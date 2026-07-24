use std::env;

use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use url::Url;

use crate::{content::SourceFact, models::LinkMetadata};

use super::super::classify::{clean_text, first_meaningful_line};

pub(super) async fn enrich(metadata: &mut LinkMetadata, http: &Client) {
    if !matches!(metadata.platform.as_str(), "高德地图" | "百度地图") {
        return;
    }
    enrich_from_url(metadata);
    match metadata.platform.as_str() {
        "高德地图" => {
            enrich_amap_public_preview(metadata, http).await;
            enrich_amap(metadata, http).await;
        }
        "百度地图" => enrich_baidu(metadata, http).await,
        _ => {}
    }
}

fn enrich_from_url(metadata: &mut LinkMetadata) {
    let Ok(url) = Url::parse(&metadata.final_url) else {
        return;
    };
    let query = |keys: &[&str]| {
        url.query_pairs()
            .find(|(key, value)| keys.contains(&key.as_ref()) && !value.is_empty())
            .map(|(_, value)| value.into_owned())
    };
    if let Some(name) = query(&["name", "q", "query", "title", "wd"]) {
        metadata.title = clean_text(&name);
        fact(metadata, "name", "地点", name);
    } else if matches!(
        metadata.title.as_str(),
        "未命名链接" | "百度地图" | "高德地图"
    ) && let Some(title) = first_meaningful_line(&metadata.shared_text)
    {
        metadata.title = title;
    }
    if let Some(address) = query(&["address", "addr"]) {
        fact(metadata, "address", "地址", address);
    }
    if let Some(location) = query(&["location", "latlng", "center"]) {
        fact(metadata, "location", "坐标", location);
    }
}

async fn enrich_amap_public_preview(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata.source_id.as_deref() else {
        return;
    };
    let url = format!("https://ditu.amap.com/place/{id}");
    let Ok(response) = http
        .get(url)
        .header(
            "user-agent",
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
        )
        .send()
        .await
    else {
        return;
    };
    let Ok(response) = response.error_for_status() else {
        return;
    };
    let Ok(html) = response.text().await else {
        return;
    };
    let previous_facts = metadata.source_facts.len();
    let document = Html::parse_document(&html);
    if let Some(description) = meta_content(&document, "meta[name='description']") {
        metadata.description = description.clone();
        metadata.content_text = description.clone();
        metadata.original_text = description.clone();
        metadata.original_content = crate::content::OriginalContent::from_plain_text(description);
    }
    super::structured::enrich(metadata, &html);
    if metadata.source_facts.len() > previous_facts && !metadata.original_content.is_empty() {
        metadata.extraction.method = "public-preview".to_string();
    }
}

fn meta_content(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .find_map(|node| node.value().attr("content"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn enrich_amap(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata.source_id.as_deref() else {
        return;
    };
    let Some(key) = env_value("AMAP_WEB_SERVICE_KEY") else {
        return;
    };
    let mut url = Url::parse("https://restapi.amap.com/v5/place/detail").expect("static URL");
    url.query_pairs_mut()
        .append_pair("key", &key)
        .append_pair("id", id)
        .append_pair("show_fields", "business,photos");
    let response = http.get(url).send().await.ok();
    let Some(value) = successful_json(response).await else {
        return;
    };
    let poi = value
        .get("pois")
        .and_then(Value::as_array)
        .and_then(|values| values.first());
    let Some(poi) = poi else {
        return;
    };
    merge_place(metadata, poi);
    metadata.extraction.method = "official-api".to_string();
}

async fn enrich_baidu(metadata: &mut LinkMetadata, http: &Client) {
    let Some(uid) = metadata.source_id.as_deref() else {
        return;
    };
    let Some(key) = env_value("BAIDU_MAPS_AK") else {
        return;
    };
    let mut url = Url::parse("https://api.map.baidu.com/place/v2/detail").expect("static URL");
    url.query_pairs_mut()
        .append_pair("uid", uid)
        .append_pair("output", "json")
        .append_pair("scope", "2")
        .append_pair("ak", &key);
    let response = http.get(url).send().await.ok();
    let Some(value) = successful_json(response).await else {
        return;
    };
    if let Some(result) = value.get("result") {
        merge_place(metadata, result);
        metadata.extraction.method = "official-api".to_string();
    }
}

fn merge_place(metadata: &mut LinkMetadata, place: &Value) {
    if let Some(name) = string(place, "name") {
        metadata.title = clean_text(name);
        fact(metadata, "name", "地点", name.to_string());
    }
    for (key, label, source_key) in [
        ("address", "地址", "address"),
        ("type", "类别", "type"),
        ("tel", "电话", "tel"),
        ("rating", "评分", "overall_rating"),
        ("opening_hours", "营业时间", "open_time"),
    ] {
        if let Some(value) = deep_string(place, source_key) {
            fact(metadata, key, label, value);
        }
    }
    if let Some(location) = place.get("location") {
        let lon = location.get("lng").or_else(|| location.get("longitude"));
        let lat = location.get("lat").or_else(|| location.get("latitude"));
        if let (Some(lon), Some(lat)) = (lon.and_then(number_string), lat.and_then(number_string)) {
            fact(metadata, "location", "坐标", format!("{lat},{lon}"));
        }
    }
}

async fn successful_json(response: Option<reqwest::Response>) -> Option<Value> {
    let response = response?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

fn fact(metadata: &mut LinkMetadata, key: &str, label: &str, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if let Some(existing) = metadata
        .source_facts
        .iter_mut()
        .find(|fact| fact.key == key)
    {
        existing.value = value;
        return;
    }
    metadata.source_facts.push(SourceFact {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn deep_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(value_string)
            .or_else(|| map.values().find_map(|value| deep_string(value, key))),
        Value::Array(values) => values.iter().find_map(|value| deep_string(value, key)),
        _ => None,
    }
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn number_string(value: &Value) -> Option<String> {
    value_string(value)
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::enrich_from_url;

    #[test]
    fn preserves_map_name_address_and_location() {
        let mut metadata = LinkMetadata {
            platform: "高德地图".to_string(),
            final_url: "https://www.amap.com/place/1?name=%E6%95%85%E5%AE%AB&address=%E5%8C%97%E4%BA%AC&location=116.4,39.9".to_string(),
            ..LinkMetadata::default()
        };
        enrich_from_url(&mut metadata);
        assert_eq!(metadata.title, "故宫");
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "address")
        );
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "location")
        );
    }
}
