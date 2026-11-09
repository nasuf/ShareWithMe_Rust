//! schema.org JSON-LD: products, books/movies/reviews, places and articles.
//!
//! Platforms pick the enrichment that matches their content kind in
//! [`crate::platforms::Platform::enrich_from_html`].

use serde_json::Value;

use crate::{models::LinkMetadata, text::clean_text};

/// `Product` name, description, image, SKU, brand, price and availability.
pub(crate) fn enrich_product(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(product) = find_typed(values, &["Product"]) else {
        return;
    };
    merge_text_fields(metadata, product);
    metadata.set_fact("sku", "商品编号", string_by_path(product, &["sku"]));
    metadata.set_fact("brand", "品牌", nested_name(product.get("brand")));
    let offers = product
        .get("offers")
        .and_then(first_object)
        .unwrap_or(product);
    metadata.set_fact(
        "price",
        "价格",
        string_by_path(offers, &["price", "lowPrice"]),
    );
    metadata.set_fact(
        "currency",
        "货币",
        string_by_path(offers, &["priceCurrency"]),
    );
    metadata.set_fact(
        "availability",
        "库存状态",
        string_by_path(offers, &["availability"]).map(short_schema_value),
    );
}

/// `Book` / `Movie` / `Review` / `Article` text, author, ISBN, rating, date.
pub(crate) fn enrich_catalog(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(item) = find_typed(values, &["Book", "Movie", "Review", "Article"]) else {
        return;
    };
    merge_text_fields(metadata, item);
    metadata.set_fact("author", "作者", nested_name(item.get("author")));
    metadata.set_fact("isbn", "ISBN", string_by_path(item, &["isbn"]));
    metadata.set_fact("rating", "评分", rating(item));
    metadata.set_fact(
        "published_at",
        "发布时间",
        string_by_path(item, &["datePublished"]),
    );
}

/// `Place` / `LocalBusiness` name, description, image, address, phone, rating
/// and coordinates.
pub(crate) fn enrich_place(metadata: &mut LinkMetadata, values: &[Value]) {
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
        metadata.set_description_and_body(&description);
    }
    if let Some(image) = value_to_string(place.get("image")) {
        metadata.set_cover_image(normalize_http_url(image));
    }
    metadata.set_fact(
        "address",
        "地址",
        place
            .get("address")
            .and_then(|address| string_by_path(address, &["streetAddress"])),
    );
    metadata.set_fact("telephone", "电话", string_by_path(place, &["telephone"]));
    metadata.set_fact("rating", "评分", rating(place));
    if let Some(geo) = place.get("geo")
        && let (Some(latitude), Some(longitude)) = (
            string_by_path(geo, &["latitude"]),
            string_by_path(geo, &["longitude"]),
        )
    {
        metadata.set_fact("location", "坐标", format!("{latitude},{longitude}"));
    }
}

/// Generic `Article` / `NewsArticle` / `VideoObject` / `SocialMediaPosting`.
pub(crate) fn enrich_common(metadata: &mut LinkMetadata, values: &[Value]) {
    let Some(item) = find_typed(
        values,
        &[
            "Article",
            "NewsArticle",
            "VideoObject",
            "SocialMediaPosting",
        ],
    ) else {
        return;
    };
    merge_text_fields(metadata, item);
    metadata.set_fact("author", "作者", nested_name(item.get("author")));
    metadata.set_fact(
        "published_at",
        "发布时间",
        string_by_path(item, &["datePublished", "uploadDate"]),
    );
}

/// Use the (marketplace-provided) description as the item body.
pub(crate) fn prefer_description_as_original(metadata: &mut LinkMetadata) {
    if metadata.description.trim().is_empty() {
        return;
    }
    let description = metadata.description.clone();
    metadata.set_description_and_body(&description);
}

fn merge_text_fields(metadata: &mut LinkMetadata, item: &Value) {
    if let Some(title) = string_by_path(item, &["name", "headline"]) {
        metadata.title = clean_text(&title);
    }
    if let Some(description) = string_by_path(item, &["articleBody", "reviewBody", "description"]) {
        metadata.set_description_and_body_if_longer(&description);
    }
    if let Some(image) = value_to_string(item.get("image")) {
        metadata.set_cover_image(image);
    }
}

fn rating(item: &Value) -> Option<String> {
    item.get("aggregateRating")
        .and_then(first_object)
        .and_then(|rating| string_by_path(rating, &["ratingValue"]))
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

/// A scalar as text; arrays yield their first scalar, objects their
/// `url`/`name`/`text`.
pub(crate) fn value_to_string(value: Option<&Value>) -> Option<String> {
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

/// Depth-first search for the first value under any of `keys`.
pub(crate) fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
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
    use serde_json::json;

    use super::{enrich_catalog, enrich_common, enrich_place, enrich_product};
    use crate::models::LinkMetadata;

    fn fact<'a>(metadata: &'a LinkMetadata, key: &str) -> Option<&'a str> {
        metadata
            .source_facts
            .iter()
            .find(|fact| fact.key == key)
            .map(|fact| fact.value.as_str())
    }

    #[test]
    fn extracts_product_facts() {
        let mut metadata = LinkMetadata::default();
        enrich_product(
            &mut metadata,
            &[
                json!({"@type":"Product","name":"降噪耳机","sku":"1001","brand":{"name":"Acme"},
                "image":"https://example.com/p.jpg",
                "offers":[{"price":"699","priceCurrency":"CNY","availability":"https://schema.org/InStock"}]}),
            ],
        );
        assert_eq!(metadata.title, "降噪耳机");
        assert_eq!(fact(&metadata, "price"), Some("699"));
        assert_eq!(fact(&metadata, "brand"), Some("Acme"));
        assert_eq!(fact(&metadata, "availability"), Some("InStock"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://example.com/p.jpg")
        );
    }

    #[test]
    fn extracts_catalog_and_nested_graph_items() {
        let mut metadata = LinkMetadata::default();
        enrich_catalog(
            &mut metadata,
            &[
                json!({"@graph":[{"@type":["Book"],"name":"三体","isbn":"978",
                "author":[{"name":"刘慈欣"}],"description":"科幻小说简介",
                "aggregateRating":{"ratingValue":9.3}}]}),
            ],
        );
        assert_eq!(metadata.title, "三体");
        assert_eq!(fact(&metadata, "author"), Some("刘慈欣"));
        assert_eq!(fact(&metadata, "rating"), Some("9.3"));
        assert_eq!(metadata.original_text, "科幻小说简介");
    }

    #[test]
    fn extracts_place_with_geo_and_secure_image() {
        let mut metadata = LinkMetadata::default();
        enrich_place(
            &mut metadata,
            &[
                json!({"@type":"TouristAttraction","name":"故宫","description":"明清皇宫",
                "image":"http://example.com/g.jpg","telephone":"010",
                "address":{"streetAddress":"景山前街4号"},
                "geo":{"latitude":39.9,"longitude":116.4}}),
            ],
        );
        assert_eq!(metadata.title, "故宫");
        assert_eq!(fact(&metadata, "address"), Some("景山前街4号"));
        assert_eq!(fact(&metadata, "location"), Some("39.9,116.4"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://example.com/g.jpg")
        );
    }

    #[test]
    fn common_articles_only_replace_shorter_bodies() {
        let mut metadata = LinkMetadata {
            original_text: "已经抓到的很长很长的正文内容".to_string(),
            ..LinkMetadata::default()
        };
        enrich_common(
            &mut metadata,
            &[
                json!({"@type":"NewsArticle","headline":"新闻","description":"短",
                "author":{"name":"记者"},"datePublished":"2026-01-01"}),
            ],
        );
        assert_eq!(metadata.title, "新闻");
        assert_eq!(metadata.original_text, "已经抓到的很长很长的正文内容");
        assert_eq!(fact(&metadata, "published_at"), Some("2026-01-01"));
    }
}
