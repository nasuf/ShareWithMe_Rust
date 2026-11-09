//! Helpers shared by the map platforms ([`super::amap`], [`super::baidu_maps`]).

use std::env;

use serde_json::Value;
use url::Url;

use crate::{
    extraction::UNTITLED,
    models::LinkMetadata,
    text::{clean_text, first_meaningful_line},
};

/// Name, address and coordinates carried in the share URL's query string.
pub(super) fn enrich_from_url(metadata: &mut LinkMetadata) {
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
        metadata.set_fact("name", "地点", name);
    } else if matches!(metadata.title.as_str(), UNTITLED | "百度地图" | "高德地图")
        && let Some(title) = first_meaningful_line(&metadata.shared_text)
    {
        metadata.title = title;
    }
    if let Some(address) = query(&["address", "addr"]) {
        metadata.set_fact("address", "地址", address);
    }
    if let Some(location) = query(&["location", "latlng", "center"]) {
        metadata.set_fact("location", "坐标", location);
    }
}

/// Merge a place object from the AMap or Baidu Place API.
pub(super) fn merge_place(metadata: &mut LinkMetadata, place: &Value) {
    if let Some(name) = string(place, "name") {
        metadata.title = clean_text(name);
        metadata.set_fact("name", "地点", name.to_string());
    }
    for (key, label, source_key) in [
        ("address", "地址", "address"),
        ("type", "类别", "type"),
        ("tel", "电话", "tel"),
        ("rating", "评分", "overall_rating"),
        ("opening_hours", "营业时间", "open_time"),
    ] {
        if let Some(value) = deep_string(place, source_key) {
            metadata.set_fact(key, label, value);
        }
    }
    if let Some(location) = place.get("location") {
        let lon = location.get("lng").or_else(|| location.get("longitude"));
        let lat = location.get("lat").or_else(|| location.get("latitude"));
        if let (Some(lon), Some(lat)) = (lon.and_then(number_string), lat.and_then(number_string)) {
            metadata.set_fact("location", "坐标", format!("{lat},{lon}"));
        }
    }
}

pub(super) async fn successful_json(response: Option<reqwest::Response>) -> Option<Value> {
    let response = response?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
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

pub(super) fn env_value(name: &str) -> Option<String> {
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
