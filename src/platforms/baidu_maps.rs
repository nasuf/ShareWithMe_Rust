//! 百度地图 (Baidu Maps): places from the share URL and, when `BAIDU_MAPS_AK`
//! is set, the Place API. Short links usually hide everything else behind a
//! login, so the share-sheet text is the main source.

use reqwest::Client;
use url::Url;

use crate::models::LinkMetadata;

use super::{
    identity::{nested_query_value, query_value},
    maps::{enrich_from_url, env_value, merge_place, successful_json},
};

pub(super) const MISSING_CONTENT_HINT: &str =
    "当前链接只公开了地点名称；配置 BAIDU_MAPS_AK 后可补全地址、坐标、电话和营业信息。";

pub(super) fn identity(url: &Url) -> Option<(String, String)> {
    let id = query_value(url, &["id", "uid", "poiid", "poiShareUid"])
        .or_else(|| nested_query_value(url, &["uid", "poiid", "poiShareUid"]))?;
    Some((id.clone(), format!("https://map.baidu.com/?uid={id}")))
}

pub(super) async fn enrich_remote(metadata: &mut LinkMetadata, http: &Client) {
    enrich_from_url(metadata);
    enrich_from_place_api(metadata, http).await;
}

async fn enrich_from_place_api(metadata: &mut LinkMetadata, http: &Client) {
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
    let Some(value) = successful_json(http.get(url).send().await.ok()).await else {
        return;
    };
    if let Some(result) = value.get("result") {
        merge_place(metadata, result);
        metadata.extraction.method = "official-api".to_string();
    }
}
