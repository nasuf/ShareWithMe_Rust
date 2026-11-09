//! 抖音 (Douyin): video caption and creator from the page or the public share
//! preview, then yt-dlp for subtitles.

use reqwest::Client;
use serde_json::Value;

use crate::{
    extraction::{
        PlatformMetadata, json_ld_values, json_values_after_markers, json_values_from_script_id,
        platform_metadata_from_values, video,
    },
    models::LinkMetadata,
    text::{clean_original_text, clean_text},
};

use super::{Platform, SEARCH_CRAWLER_UA, identity::id_after};

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    id_after(segments, &["video"])
        .map(|id| (id.clone(), format!("https://www.douyin.com/video/{id}")))
}

/// The `VideoObject` JSON-LD first (it separates creator from publisher),
/// then the app's render data.
pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
    if let Some(metadata) = video_object(html) {
        return Some(metadata);
    }
    let mut candidates = json_values_from_script_id(html, "RENDER_DATA");
    candidates.extend(json_values_after_markers(
        html,
        &[
            "window.__INIT_PROPS__=",
            "window._ROUTER_DATA=",
            "window.__data=",
        ],
    ));
    candidates.extend(json_ld_values(html));
    platform_metadata_from_values(
        candidates,
        &["title", "desc", "description"],
        &["desc", "description", "seoDescription"],
        &["nickname", "authorName", "name"],
        &["cover", "poster", "thumbnailUrl", "urlDefault", "url"],
    )
}

pub(super) fn shell_reason(page_text: &str) -> Option<&'static str> {
    (page_text.contains("登录后免费畅享高清视频")
        || (page_text.contains("扫码登录") && page_text.contains("验证码登录")))
    .then_some("抖音只返回了登录页面，未取得视频说明或字幕。")
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    enrich_from_public_preview(metadata, http).await;
    video::enrich_with_yt_dlp(metadata, http, allow_video_transcription, true).await;
}

/// `iesdouyin.com/share/video/<id>/` serves crawlers the video's JSON-LD.
async fn enrich_from_public_preview(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata.source_id.as_deref() else {
        return;
    };
    let url = format!("https://www.iesdouyin.com/share/video/{id}/");
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
    if Platform::Douyin.merge_embedded_state(metadata, &html) {
        metadata.extraction.method = "public-preview".to_string();
    }
}

fn video_object(html: &str) -> Option<PlatformMetadata> {
    let values = json_ld_values(html);
    let video = values.iter().find_map(find_video_object)?;
    let description = video
        .get("description")
        .and_then(Value::as_str)
        .map(clean_original_text);
    PlatformMetadata {
        title: video.get("name").and_then(Value::as_str).map(clean_text),
        description: description.clone(),
        author: video
            .get("creator")
            .and_then(|creator| creator.get("name"))
            .and_then(Value::as_str)
            .map(clean_text),
        image_url: video
            .get("thumbnailUrl")
            .and_then(first_string)
            .map(str::to_string),
        original_text: description,
    }
    .into_meaningful()
}

fn find_video_object(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => {
            if map.get("@type").and_then(Value::as_str) == Some("VideoObject") {
                return Some(value);
            }
            map.values().find_map(find_video_object)
        }
        Value::Array(values) => values.iter().find_map(find_video_object),
        _ => None,
    }
}

fn first_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value),
        Value::Array(values) => values.iter().find_map(first_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{embedded_state, shell_reason};

    #[test]
    fn reads_video_object_without_confusing_publisher_and_creator() {
        let metadata = embedded_state(
            r#"<script type="application/ld+json">{
                "@type":"VideoObject",
                "name":"韭菜盒子 - 抖音",
                "description":"家常美食记录",
                "thumbnailUrl":["https://example.com/cover.jpg"],
                "creator":{"@type":"Person","name":"吃货自然萌"},
                "publisher":{"@type":"Organization","name":"抖音"}
            }</script>"#,
        )
        .expect("video metadata");

        assert_eq!(metadata.title.as_deref(), Some("韭菜盒子 - 抖音"));
        assert_eq!(metadata.author.as_deref(), Some("吃货自然萌"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://example.com/cover.jpg")
        );
    }

    #[test]
    fn falls_back_to_render_data() {
        let html = r#"<script id="RENDER_DATA" type="application/json">%7B%22app%22%3A%7B%22videoDetail%22%3A%7B%22desc%22%3A%22%E6%8A%96%E9%9F%B3%E8%A7%86%E9%A2%91%E8%AF%B4%E6%98%8E%22%2C%22authorInfo%22%3A%7B%22nickname%22%3A%22%E5%88%9B%E4%BD%9C%E8%80%85%22%7D%7D%7D%7D</script>"#;
        let metadata = embedded_state(html).expect("render data");
        assert_eq!(metadata.description.as_deref(), Some("抖音视频说明"));
        assert_eq!(metadata.author.as_deref(), Some("创作者"));
    }

    #[test]
    fn recognises_login_shell() {
        assert!(shell_reason("登录后免费畅享高清视频").is_some());
        assert!(shell_reason("扫码登录 验证码登录").is_some());
        assert!(shell_reason("家常美食记录").is_none());
    }
}
