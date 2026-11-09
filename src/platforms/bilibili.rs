//! 哔哩哔哩 (Bilibili): the public view API, its subtitle tracks, then yt-dlp.

use chrono::{TimeZone, Utc};
use reqwest::Client;
use serde_json::Value;
use url::Url;

use crate::{
    extraction::{
        PlatformMetadata, json_ld_values, json_values_after_markers, platform_metadata_from_values,
        video::{self, apply_transcript, format_duration, normalize_https},
    },
    models::LinkMetadata,
    text::{clean_original_text, clean_text},
};

const REFERER: &str = "https://www.bilibili.com/";
/// Subtitle languages in order of preference.
const SUBTITLE_LANGUAGES: &[&str] = &["zh-Hans", "zh-CN", "zh-Hant", "zh-TW", "en"];

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    segments
        .iter()
        .find(|value| value.starts_with("BV") || value.starts_with("av"))
        .map(|id| {
            (
                (*id).to_string(),
                format!("https://www.bilibili.com/video/{id}/"),
            )
        })
}

pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
    let mut candidates =
        json_values_after_markers(html, &["window.__INITIAL_STATE__=", "window.__playinfo__="]);
    candidates.extend(json_ld_values(html));
    platform_metadata_from_values(
        candidates,
        &["title", "name"],
        &["desc", "description"],
        &["owner", "author", "name"],
        &["pic", "cover", "thumbnailUrl", "image"],
    )
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    enrich_from_view_api(metadata, http).await;
    video::enrich_with_yt_dlp(metadata, http, allow_video_transcription, true).await;
}

async fn enrich_from_view_api(metadata: &mut LinkMetadata, http: &Client) {
    let Some(bvid) = bvid(&metadata.final_url) else {
        return;
    };
    let api_url = format!("https://api.bilibili.com/x/web-interface/view?bvid={bvid}");
    let Some(value) = get_json(http, &api_url).await else {
        return;
    };
    if !merge_view(metadata, &value) {
        return;
    }
    metadata.extraction.method = "platform-api".to_string();
    if let Some(cid) = value
        .get("data")
        .and_then(|data| data.get("cid"))
        .and_then(Value::as_u64)
        && let Some(transcript) = subtitle_transcript(http, &bvid, cid).await
    {
        apply_transcript(metadata, transcript, "平台字幕");
    }
}

async fn subtitle_transcript(http: &Client, bvid: &str, cid: u64) -> Option<String> {
    let player_url = format!("https://api.bilibili.com/x/player/v2?bvid={bvid}&cid={cid}");
    let player = get_json(http, &player_url).await?;
    let subtitle = get_json(http, &subtitle_url(&player)?).await?;
    subtitle_text(&subtitle)
}

async fn get_json(http: &Client, url: &str) -> Option<Value> {
    http.get(url)
        .header("referer", REFERER)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()
}

fn subtitle_url(player: &Value) -> Option<String> {
    let tracks = player
        .get("data")?
        .get("subtitle")?
        .get("subtitles")?
        .as_array()?;
    let track = SUBTITLE_LANGUAGES
        .iter()
        .find_map(|language| {
            tracks.iter().find(|track| {
                track
                    .get("lan")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == *language)
            })
        })
        .or_else(|| tracks.first())?;
    track
        .get("subtitle_url")
        .and_then(Value::as_str)
        .map(normalize_https)
}

/// Subtitle lines joined, consecutive duplicates dropped; `None` when too short
/// to be a real transcript.
fn subtitle_text(subtitle: &Value) -> Option<String> {
    let mut lines = Vec::new();
    for text in subtitle
        .get("body")?
        .as_array()?
        .iter()
        .filter_map(|line| line.get("content").and_then(Value::as_str))
    {
        let text = clean_original_text(text);
        if !text.is_empty() && lines.last() != Some(&text) {
            lines.push(text);
        }
    }
    let transcript = clean_original_text(&lines.join("\n"));
    (transcript.chars().count() >= 40).then_some(transcript)
}

fn merge_view(metadata: &mut LinkMetadata, value: &Value) -> bool {
    let Some(data) = value
        .get("data")
        .filter(|_| value.get("code").and_then(Value::as_i64) == Some(0))
    else {
        return false;
    };

    if let Some(title) = string(data, "title") {
        metadata.title = clean_text(title);
    }
    if let Some(description) = string(data, "desc") {
        let description = clean_original_text(description);
        metadata.description = clean_text(&description);
        metadata.set_body(description);
    }
    if let Some(owner) = data.get("owner").and_then(|owner| string(owner, "name")) {
        metadata.author = Some(clean_text(owner));
    }
    if let Some(image) = string(data, "pic") {
        let image = normalize_https(image);
        metadata.add_image(&image);
        metadata.image_url = Some(image);
    }
    if let Some(timestamp) = data.get("pubdate").and_then(Value::as_i64) {
        metadata.published_at = Utc.timestamp_opt(timestamp, 0).single();
    }
    if let Some(duration) = data.get("duration").and_then(Value::as_f64) {
        metadata.set_fact("duration", "时长", format_duration(duration));
    }
    if let Some(stat) = data.get("stat") {
        for (key, label) in [
            ("view", "播放量"),
            ("danmaku", "弹幕"),
            ("like", "点赞"),
            ("favorite", "收藏"),
        ] {
            if let Some(value) = stat.get(key).and_then(Value::as_u64) {
                metadata.set_fact(key, label, value.to_string());
            }
        }
    }
    true
}

fn bvid(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    url.path_segments()?
        .find(|segment| segment.starts_with("BV") && segment.len() >= 10)
        .map(str::to_string)
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{bvid, embedded_state, merge_view, subtitle_text, subtitle_url};
    use crate::models::LinkMetadata;

    #[test]
    fn parses_public_view_api_payload() {
        let mut metadata = LinkMetadata::default();
        assert!(merge_view(
            &mut metadata,
            &json!({
                "code": 0,
                "data": {
                    "title": "视频标题",
                    "desc": "视频简介",
                    "pic": "http://i0.hdslb.com/cover.jpg",
                    "pubdate": 1_700_000_000,
                    "duration": 65,
                    "owner": {"name": "UP 主"},
                    "stat": {"view": 120, "like": 12}
                }
            })
        ));
        assert_eq!(metadata.title, "视频标题");
        assert_eq!(metadata.author.as_deref(), Some("UP 主"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://i0.hdslb.com/cover.jpg")
        );
        assert!(metadata.published_at.is_some());
        assert!(!merge_view(&mut metadata, &json!({"code": -404})));
        assert_eq!(
            bvid("https://www.bilibili.com/video/BV1epwvzGEKf/"),
            Some("BV1epwvzGEKf".to_string())
        );
    }

    #[test]
    fn prefers_chinese_subtitles_and_drops_repeated_lines() {
        let player = json!({
            "data": {"subtitle": {"subtitles": [
                {"lan": "en", "subtitle_url": "//example.com/en.json"},
                {"lan": "zh-CN", "subtitle_url": "//example.com/zh.json"}
            ]}}
        });
        assert_eq!(
            subtitle_url(&player).as_deref(),
            Some("https://example.com/zh.json")
        );
        let body = json!({"body": [
            {"content": "第一句字幕内容"},
            {"content": "第一句字幕内容"},
            {"content": "第三句字幕内容，长度足够用于正文分析。"},
            {"content": "第四句继续补充视频里的关键信息和上下文。"}
        ]});
        let text = subtitle_text(&body).expect("transcript");
        assert_eq!(text.matches("第一句字幕内容").count(), 1);
        assert!(subtitle_text(&json!({"body": [{"content": "太短"}]})).is_none());
    }

    #[test]
    fn parses_initial_state() {
        let html = r#"<script>window.__INITIAL_STATE__={"videoData":{"title":"B站视频","desc":"视频简介","owner":{"name":"UP主"},"pic":"https://i0.hdslb.com/a.jpg"}};</script>"#;
        let metadata = embedded_state(html).expect("state");
        assert_eq!(metadata.title.as_deref(), Some("B站视频"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://i0.hdslb.com/a.jpg")
        );
    }
}
