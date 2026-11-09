//! 高德地图 (AMap): places from the share URL, the public place preview page
//! and, when `AMAP_WEB_SERVICE_KEY` is set, the Web Service place API.

use reqwest::Client;
use scraper::Html;
use serde_json::Value;
use url::Url;

use crate::{extraction::meta_content, models::LinkMetadata};

use super::{
    Platform, SEARCH_CRAWLER_UA,
    identity::{id_after, nested_query_value, query_value},
    maps::{enrich_from_url, env_value, merge_place, successful_json},
};

pub(super) fn identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id = id_after(segments, &["place"])
        .or_else(|| query_value(url, &["id", "uid", "poiid", "poiShareUid"]))
        .or_else(|| nested_query_value(url, &["uid", "poiid", "poiShareUid"]))?;
    Some((id.clone(), format!("https://www.amap.com/place/{id}")))
}

pub(super) async fn enrich_remote(metadata: &mut LinkMetadata, http: &Client) {
    enrich_from_url(metadata);
    enrich_from_public_preview(metadata, http).await;
    enrich_from_place_api(metadata, http).await;
}

/// `ditu.amap.com/place/<id>` serves crawlers a description and JSON-LD.
async fn enrich_from_public_preview(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata.source_id.as_deref() else {
        return;
    };
    let url = format!("https://ditu.amap.com/place/{id}");
    let Ok(response) = http
        .get(url)
        .header("user-agent", SEARCH_CRAWLER_UA)
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
    let description = meta_content(&Html::parse_document(&html), "meta[name='description']");
    if let Some(description) = description {
        metadata.description = description.clone();
        metadata.content_text = description.clone();
        metadata.original_text = description.clone();
        metadata.original_content = crate::content::OriginalContent::from_plain_text(description);
    }
    Platform::Amap.enrich_from_html(metadata, &html);
    if metadata.source_facts.len() > previous_facts && !metadata.original_content.is_empty() {
        metadata.extraction.method = "public-preview".to_string();
    }
}

async fn enrich_from_place_api(metadata: &mut LinkMetadata, http: &Client) {
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
    let Some(value) = successful_json(http.get(url).send().await.ok()).await else {
        return;
    };
    let Some(poi) = value
        .get("pois")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    else {
        return;
    };
    merge_place(metadata, poi);
    metadata.extraction.method = "official-api".to_string();
}
