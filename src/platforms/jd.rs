//! 京东 (JD.com): product JSON-LD plus the image list and category names from
//! the page's inline product config.

use regex::Regex;
use serde_json::Value;

use crate::{extraction::structured_data, models::LinkMetadata};

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    let id = segments
        .iter()
        .find_map(|segment| segment.strip_suffix(".html"))
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))?
        .to_string();
    Some((id.clone(), format!("https://item.jd.com/{id}.html")))
}

pub(super) fn enrich_from_html(metadata: &mut LinkMetadata, html: &str, json_ld: &[Value]) {
    structured_data::enrich_product(metadata, json_ld);
    enrich_from_product_config(metadata, html);
    structured_data::prefer_description_as_original(metadata);
}

/// `imageList: ["jfs/..."]` and `catName: ["手机", ...]` in the page script.
fn enrich_from_product_config(metadata: &mut LinkMetadata, html: &str) {
    if let Some(image) = capture(html, r#"imageList\s*:\s*\[\s*\"([^\"]+)\""#) {
        metadata.set_cover_image(format!("https://img10.360buyimg.com/n1/{image}"));
    }
    if let Some(categories) = capture(html, r#"catName\s*:\s*\[([^\]]+)\]"#) {
        let categories = categories
            .split(',')
            .map(|value| value.trim().trim_matches(['\"', '\'']))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        metadata.set_fact("category", "商品分类", categories);
    }
}

fn capture(input: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(input)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::enrich_from_html;
    use crate::{extraction::json_ld_values, models::LinkMetadata};

    #[test]
    fn reads_json_ld_and_product_config() {
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"降噪耳机","description":"主动降噪，续航 30 小时","offers":{"price":"699"}}</script>
            <script>var pageConfig = {product: {imageList: ["jfs/t1/abc.jpg"], catName: ["数码", "耳机"]}};</script>"#;
        let mut metadata = LinkMetadata::default();
        enrich_from_html(&mut metadata, html, &json_ld_values(html));
        assert_eq!(metadata.title, "降噪耳机");
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://img10.360buyimg.com/n1/jfs/t1/abc.jpg")
        );
        assert_eq!(metadata.original_text, "主动降噪，续航 30 小时");
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "category" && fact.value == "数码 / 耳机")
        );
    }
}
