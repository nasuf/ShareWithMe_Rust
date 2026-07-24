use regex::Regex;
use serde_json::Value;

use crate::{
    content::{MediaAsset, SourceFact},
    models::LinkMetadata,
};

use super::super::{
    classify::{clean_original_text, clean_text},
    metadata_json::json_ld_values,
};

pub(super) fn enrich(metadata: &mut LinkMetadata, html: &str) {
    let values = json_ld_values(html);
    match metadata.platform.as_str() {
        "京东" => {
            enrich_product(metadata, &values);
            enrich_jd_html(metadata, html);
            prefer_description_as_original(metadata);
        }
        "淘宝" | "天猫" => {
            enrich_product(metadata, &values);
            prefer_description_as_original(metadata);
        }
        "豆瓣" => enrich_catalog(metadata, &values),
        "微信读书" => {
            enrich_catalog(metadata, &values);
            enrich_weread_state(metadata, html);
        }
        "高德地图" | "百度地图" => enrich_place(metadata, &values),
        _ => enrich_common(metadata, &values),
    }
}

fn enrich_place(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(place) = find_typed(
        values,
        &[
            "Place",
            "TouristAttraction",
            "LocalBusiness",
            "LandmarksOrHistoricalBuildings",
        ],
    ) else {
        return;
    };
    if let Some(title) = string_by_path(place, &["name", "headline"]) {
        metadata.title = clean_text(&title);
    }
    if let Some(description) = string_by_path(place, &["description"]) {
        set_original(metadata, description);
    }
    if let Some(image) = value_to_string(place.get("image")) {
        push_image(metadata, normalize_http_url(image));
    }
    push_fact(
        metadata,
        "address",
        "地址",
        place
            .get("address")
            .and_then(|address| string_by_path(address, &["streetAddress"])),
    );
    push_fact(
        metadata,
        "telephone",
        "电话",
        string_by_path(place, &["telephone"]),
    );
    push_fact(
        metadata,
        "rating",
        "评分",
        place
            .get("aggregateRating")
            .and_then(first_object)
            .and_then(|rating| string_by_path(rating, &["ratingValue"])),
    );
    if let Some(geo) = place.get("geo") {
        let latitude = string_by_path(geo, &["latitude"]);
        let longitude = string_by_path(geo, &["longitude"]);
        if let (Some(latitude), Some(longitude)) = (latitude, longitude) {
            push_fact(
                metadata,
                "location",
                "坐标",
                Some(format!("{latitude},{longitude}")),
            );
        }
    }
}

fn enrich_product(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(product) = find_typed(values, &["Product"]) else {
        return;
    };
    merge_text_fields(metadata, product);
    push_fact(
        metadata,
        "sku",
        "商品编号",
        string_by_path(product, &["sku"]),
    );
    push_fact(metadata, "brand", "品牌", nested_name(product.get("brand")));
    let offers = product
        .get("offers")
        .and_then(first_object)
        .unwrap_or(product);
    push_fact(
        metadata,
        "price",
        "价格",
        string_by_path(offers, &["price", "lowPrice"]),
    );
    push_fact(
        metadata,
        "currency",
        "货币",
        string_by_path(offers, &["priceCurrency"]),
    );
    push_fact(
        metadata,
        "availability",
        "库存状态",
        string_by_path(offers, &["availability"]).map(short_schema_value),
    );
}

fn enrich_jd_html(metadata: &mut LinkMetadata, html: &str) {
    if let Some(image) = capture(html, r#"imageList\s*:\s*\[\s*\"([^\"]+)\""#) {
        push_image(metadata, format!("https://img10.360buyimg.com/n1/{image}"));
    }
    if let Some(categories) = capture(html, r#"catName\s*:\s*\[([^\]]+)\]"#) {
        let categories = categories
            .split(',')
            .map(|value| value.trim().trim_matches(['\"', '\'']))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        push_fact(metadata, "category", "商品分类", Some(categories));
    }
}

fn prefer_description_as_original(metadata: &mut LinkMetadata) {
    if metadata.description.trim().is_empty() {
        return;
    }
    set_original(metadata, metadata.description.clone());
}

fn set_original(metadata: &mut LinkMetadata, text: String) {
    let text = clean_original_text(&text);
    if text.is_empty() {
        return;
    }
    metadata.description = clean_text(&text);
    metadata.original_text = text.clone();
    metadata.content_text = clean_text(&text);
    metadata.original_content = crate::content::OriginalContent::from_plain_text(text);
}

fn enrich_catalog(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(item) = find_typed(values, &["Book", "Movie", "Review", "Article"]) else {
        return;
    };
    merge_text_fields(metadata, item);
    push_fact(metadata, "author", "作者", nested_name(item.get("author")));
    push_fact(metadata, "isbn", "ISBN", string_by_path(item, &["isbn"]));
    push_fact(
        metadata,
        "rating",
        "评分",
        item.get("aggregateRating")
            .and_then(first_object)
            .and_then(|rating| string_by_path(rating, &["ratingValue"])),
    );
    push_fact(
        metadata,
        "published_at",
        "发布时间",
        string_by_path(item, &["datePublished"]),
    );
}

fn enrich_common(metadata: &mut LinkMetadata, values: &[Value]) {
    if let Some(item) = find_typed(
        values,
        &[
            "Article",
            "NewsArticle",
            "VideoObject",
            "SocialMediaPosting",
        ],
    ) {
        merge_text_fields(metadata, item);
        push_fact(metadata, "author", "作者", nested_name(item.get("author")));
        push_fact(
            metadata,
            "published_at",
            "发布时间",
            string_by_path(item, &["datePublished", "uploadDate"]),
        );
    }
}

fn enrich_weread_state(metadata: &mut LinkMetadata, html: &str) {
    let markers = ["window.__INITIAL_STATE__=", "window.__NUXT__="];
    let values = super::super::metadata_json::json_values_after_markers(html, &markers);
    for value in &values {
        if let Some(title) = find_string(value, &["title", "bookName"]) {
            metadata.title = clean_text(&title);
        }
        if let Some(author) = find_string(value, &["author", "authorName"]) {
            metadata.author = Some(clean_text(&author));
            push_fact(metadata, "author", "作者", Some(author));
        }
        if let Some(intro) = find_string(value, &["intro", "description"]) {
            let intro = clean_original_text(&intro);
            if intro.chars().count() > metadata.original_text.chars().count() {
                metadata.description = clean_text(&intro);
                metadata.original_text = intro.clone();
                metadata.content_text = clean_text(&intro);
                metadata.original_content = crate::content::OriginalContent::from_plain_text(intro);
            }
        }
        if let Some(cover) = find_string(value, &["cover", "coverUrl"]) {
            push_image(metadata, cover);
        }
    }
}

fn merge_text_fields(metadata: &mut LinkMetadata, item: &Value) {
    if let Some(title) = string_by_path(item, &["name", "headline"]) {
        metadata.title = clean_text(&title);
    }
    if let Some(description) = string_by_path(item, &["articleBody", "reviewBody", "description"]) {
        let description = clean_original_text(&description);
        if description.chars().count() > metadata.original_text.chars().count() {
            metadata.description = clean_text(&description);
            metadata.original_text = description.clone();
            metadata.content_text = clean_text(&description);
            metadata.original_content =
                crate::content::OriginalContent::from_plain_text(description);
        }
    }
    if let Some(image) = value_to_string(item.get("image")) {
        push_image(metadata, image);
    }
}

fn push_image(metadata: &mut LinkMetadata, image: String) {
    if !image.starts_with("http") {
        return;
    }
    metadata.image_url = Some(image.clone());
    if !metadata.media.iter().any(|asset| asset.url == image) {
        metadata.media.push(MediaAsset {
            kind: "image".to_string(),
            url: image,
            alt: Some(metadata.title.clone()),
            ..MediaAsset::default()
        });
    }
}

fn normalize_http_url(value: String) -> String {
    if let Some(value) = value.strip_prefix("http://") {
        format!("https://{value}")
    } else if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    }
}

fn capture(input: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(input)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn push_fact(metadata: &mut LinkMetadata, key: &str, label: &str, value: Option<String>) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
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

fn find_typed<'a>(values: &'a [Value], types: &[&str]) -> Option<&'a Value> {
    values
        .iter()
        .find_map(|value| find_typed_value(value, types))
}

fn find_typed_value<'a>(value: &'a Value, types: &[&str]) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if map
                .get("@type")
                .is_some_and(|kind| type_matches(kind, types))
            {
                return Some(value);
            }
            map.values()
                .find_map(|child| find_typed_value(child, types))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_typed_value(child, types)),
        _ => None,
    }
}

fn type_matches(value: &Value, types: &[&str]) -> bool {
    match value {
        Value::String(value) => types.contains(&value.as_str()),
        Value::Array(values) => values.iter().any(|value| type_matches(value, types)),
        _ => false,
    }
}

fn string_by_path(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| value_to_string(value.get(*key)))
}

fn nested_name(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| {
        value_to_string(Some(value)).or_else(|| {
            value
                .get("name")
                .and_then(|name| value_to_string(Some(name)))
        })
    })
}

fn first_object(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(_) => Some(value),
        Value::Array(values) => values.iter().find(|value| value.is_object()),
        _ => None,
    }
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(values) => values.iter().find_map(|value| value_to_string(Some(value))),
        Value::Object(map) => ["url", "name", "text"]
            .iter()
            .find_map(|key| map.get(*key).and_then(|value| value_to_string(Some(value)))),
        _ => None,
    }
}

fn short_schema_value(value: String) -> String {
    value.rsplit('/').next().unwrap_or(&value).to_string()
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key).and_then(|value| value_to_string(Some(value))) {
                    return Some(value);
                }
            }
            map.values().find_map(|value| find_string(value, keys))
        }
        Value::Array(values) => values.iter().find_map(|value| find_string(value, keys)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::enrich;

    #[test]
    fn extracts_product_facts_from_json_ld() {
        let mut metadata = LinkMetadata {
            platform: "京东".to_string(),
            ..LinkMetadata::default()
        };
        enrich(
            &mut metadata,
            r#"<script type="application/ld+json">{"@type":"Product","name":"降噪耳机","sku":"1001","brand":{"name":"Acme"},"offers":{"price":"699","priceCurrency":"CNY","availability":"https://schema.org/InStock"}}</script>"#,
        );
        assert_eq!(metadata.title, "降噪耳机");
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "price" && fact.value == "699")
        );
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "brand" && fact.value == "Acme")
        );
    }
}
