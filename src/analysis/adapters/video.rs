use std::{env, path::Path, time::Duration};

use chrono::{TimeZone, Utc};
use reqwest::Client;
use serde_json::Value;
use tokio::{process::Command, time::timeout};
use url::Url;

use crate::{
    content::{MediaAsset, OriginalContent, SourceFact},
    models::LinkMetadata,
};

use super::super::classify::{clean_original_text, clean_text};

const VIDEO_PLATFORMS: &[&str] = &["YouTube", "哔哩哔哩", "抖音", "X", "Instagram"];

pub(super) async fn enrich(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    if !VIDEO_PLATFORMS.contains(&metadata.platform.as_str()) {
        return;
    }
    if metadata.platform == "抖音" {
        enrich_douyin_public_preview(metadata, http).await;
    }
    if metadata.platform == "哔哩哔哩" {
        enrich_bilibili(metadata, http).await;
    }
    let Some(value) = yt_dlp_json(&metadata.final_url).await else {
        return;
    };
    merge_metadata(metadata, &value);
    let mut transcript_added = false;
    if let Some(caption_url) = caption_url(&value)
        && let Some(transcript) = fetch_transcript(http, &caption_url).await
    {
        apply_transcript(metadata, transcript, "平台字幕");
        transcript_added = true;
    }
    if allow_video_transcription
        && !transcript_added
        && let Some(transcript) = super::asr::transcribe_if_configured(
            http,
            &metadata.final_url,
            value.get("duration").and_then(Value::as_f64),
        )
        .await
    {
        apply_transcript(metadata, transcript, "语音转写");
    }
    metadata.extraction.method = "yt-dlp".to_string();
}

fn apply_transcript(metadata: &mut LinkMetadata, transcript: String, source: &str) {
    metadata.original_text = transcript.clone();
    metadata.content_text = clean_text(&transcript);
    metadata.original_content = OriginalContent::from_plain_text(transcript);
    fact(
        metadata,
        "transcript_source",
        "视频正文",
        source.to_string(),
    );
}

async fn enrich_douyin_public_preview(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata.source_id.as_deref() else {
        return;
    };
    let url = format!("https://www.iesdouyin.com/share/video/{id}/");
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
    if super::super::platform_metadata::merge_platform_state_metadata(metadata, &html) {
        metadata.extraction.method = "public-preview".to_string();
    }
}

async fn yt_dlp_json(url: &str) -> Option<Value> {
    let executable = env::var("SHARE_WITH_ME_YTDLP").unwrap_or_else(|_| "yt-dlp".to_string());
    tracing::debug!(%executable, url, "run yt-dlp metadata extractor");
    let mut command = Command::new(executable);
    command.args([
        "--dump-single-json",
        "--skip-download",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "12",
        "--retries",
        "1",
        "--js-runtimes",
        "node",
        "--format",
        "bestaudio/best",
    ]);
    if let Some(cookies) = env::var("SHARE_WITH_ME_YTDLP_COOKIES")
        .ok()
        .filter(|path| Path::new(path).is_file())
    {
        tracing::debug!(cookies, "use configured yt-dlp cookie file");
        command.arg("--cookies").arg(cookies);
    }
    command.arg(url);
    let output = match timeout(Duration::from_secs(40), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            tracing::debug!(%error, "yt-dlp could not start");
            return None;
        }
        Err(_) => {
            tracing::debug!(url, "yt-dlp metadata request timed out");
            return None;
        }
    };
    if !output.status.success() {
        tracing::debug!(
            status = ?output.status.code(),
            stderr = %String::from_utf8_lossy(&output.stderr),
            "yt-dlp metadata request failed"
        );
        return None;
    }
    if output.stdout.len() > 20 * 1024 * 1024 {
        tracing::debug!(
            bytes = output.stdout.len(),
            "yt-dlp metadata response too large"
        );
        return None;
    }
    match serde_json::from_slice(&output.stdout) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::debug!(%error, "yt-dlp returned invalid JSON");
            None
        }
    }
}

fn merge_metadata(metadata: &mut LinkMetadata, value: &Value) {
    if let Some(title) = string(value, "title") {
        metadata.title = clean_text(title);
    }
    if let Some(description) = string(value, "description") {
        let description = clean_original_text(description);
        if matches!(metadata.platform.as_str(), "YouTube" | "哔哩哔哩" | "抖音")
            || description.chars().count() > metadata.original_text.chars().count()
        {
            metadata.description = clean_text(&description);
            metadata.original_text = description.clone();
            metadata.content_text = clean_text(&description);
            metadata.original_content = OriginalContent::from_plain_text(description);
        }
    }
    if let Some(author) = ["uploader", "channel", "creator"]
        .iter()
        .find_map(|key| string(value, key))
    {
        metadata.author = Some(clean_text(author));
    }
    if let Some(thumbnail) = string(value, "thumbnail") {
        metadata.image_url = Some(thumbnail.to_string());
        push_media(metadata, "image", thumbnail, None);
    }
    if let Some(webpage_url) = string(value, "webpage_url") {
        metadata.final_url = webpage_url.to_string();
    }
    if let Some(duration) = value.get("duration").and_then(Value::as_f64) {
        fact(metadata, "duration", "时长", format_duration(duration));
        if let Some(thumbnail) = metadata.image_url.clone()
            && let Some(asset) = metadata
                .media
                .iter_mut()
                .find(|asset| asset.url == thumbnail)
        {
            asset.duration_seconds = Some(duration);
        }
    }
    if let Some(view_count) = value.get("view_count").and_then(Value::as_u64) {
        fact(metadata, "view_count", "播放量", view_count.to_string());
    }
    if let Some(tags) = value.get("tags").and_then(Value::as_array) {
        let tags = tags
            .iter()
            .filter_map(Value::as_str)
            .take(12)
            .collect::<Vec<_>>()
            .join("、");
        fact(metadata, "source_tags", "原标签", tags);
    }
}

async fn enrich_bilibili(metadata: &mut LinkMetadata, http: &Client) {
    let Some(bvid) = bilibili_bvid(&metadata.final_url) else {
        return;
    };
    let api_url = format!("https://api.bilibili.com/x/web-interface/view?bvid={bvid}");
    let Ok(response) = http
        .get(api_url)
        .header("referer", "https://www.bilibili.com/")
        .send()
        .await
    else {
        return;
    };
    let Ok(response) = response.error_for_status() else {
        return;
    };
    let Ok(value) = response.json::<Value>().await else {
        return;
    };
    if merge_bilibili_value(metadata, &value) {
        metadata.extraction.method = "platform-api".to_string();
        if let Some(cid) = value
            .get("data")
            .and_then(|data| data.get("cid"))
            .and_then(Value::as_u64)
            && let Some(transcript) = bilibili_transcript(http, &bvid, cid).await
        {
            apply_transcript(metadata, transcript, "平台字幕");
        }
    }
}

async fn bilibili_transcript(http: &Client, bvid: &str, cid: u64) -> Option<String> {
    let player_url = format!("https://api.bilibili.com/x/player/v2?bvid={bvid}&cid={cid}");
    let value = http
        .get(player_url)
        .header("referer", "https://www.bilibili.com/")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    let subtitle_url = bilibili_subtitle_url(&value)?;
    let value = http
        .get(subtitle_url)
        .header("referer", "https://www.bilibili.com/")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    bilibili_subtitle_text(&value)
}

fn bilibili_subtitle_url(value: &Value) -> Option<String> {
    let tracks = value
        .get("data")?
        .get("subtitle")?
        .get("subtitles")?
        .as_array()?;
    let track = ["zh-Hans", "zh-CN", "zh-Hant", "zh-TW", "en"]
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

fn bilibili_subtitle_text(value: &Value) -> Option<String> {
    let mut lines = Vec::new();
    for text in value
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

fn merge_bilibili_value(metadata: &mut LinkMetadata, value: &Value) -> bool {
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
        metadata.content_text = clean_text(&description);
        metadata.original_text = description.clone();
        metadata.original_content = OriginalContent::from_plain_text(description);
    }
    if let Some(owner) = data.get("owner").and_then(|owner| string(owner, "name")) {
        metadata.author = Some(clean_text(owner));
    }
    if let Some(image) = string(data, "pic") {
        let image = normalize_https(image);
        metadata.image_url = Some(image.clone());
        push_media(metadata, "image", &image, None);
    }
    if let Some(timestamp) = data.get("pubdate").and_then(Value::as_i64) {
        metadata.published_at = Utc.timestamp_opt(timestamp, 0).single();
    }
    if let Some(duration) = data.get("duration").and_then(Value::as_f64) {
        fact(metadata, "duration", "时长", format_duration(duration));
    }
    if let Some(stat) = data.get("stat") {
        for (key, label) in [
            ("view", "播放量"),
            ("danmaku", "弹幕"),
            ("like", "点赞"),
            ("favorite", "收藏"),
        ] {
            if let Some(value) = stat.get(key).and_then(Value::as_u64) {
                fact(metadata, key, label, value.to_string());
            }
        }
    }
    true
}

fn bilibili_bvid(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    url.path_segments()?
        .find(|segment| segment.starts_with("BV") && segment.len() >= 10)
        .map(str::to_string)
}

fn normalize_https(value: &str) -> String {
    if value.starts_with("//") {
        format!("https:{value}")
    } else if let Some(value) = value.strip_prefix("http://") {
        format!("https://{value}")
    } else {
        value.to_string()
    }
}

fn caption_url(value: &Value) -> Option<String> {
    for key in ["subtitles", "automatic_captions"] {
        let Some(tracks) = value.get(key).and_then(Value::as_object) else {
            continue;
        };
        for language in ["zh-Hans", "zh-CN", "zh", "en", "en-US"] {
            if let Some(url) = tracks.get(language).and_then(best_track_url) {
                return Some(url);
            }
        }
        if let Some(url) = tracks.values().find_map(best_track_url) {
            return Some(url);
        }
    }
    None
}

fn best_track_url(value: &Value) -> Option<String> {
    let tracks = value.as_array()?;
    tracks
        .iter()
        .find(|track| matches!(string(track, "ext"), Some("json3" | "vtt")))
        .or_else(|| tracks.first())
        .and_then(|track| string(track, "url"))
        .map(str::to_string)
}

async fn fetch_transcript(http: &Client, url: &str) -> Option<String> {
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().await.ok()?;
    let text = if body.trim_start().starts_with('{') {
        transcript_from_json3(&body)
    } else {
        transcript_from_vtt(&body)
    };
    let text = clean_original_text(&text);
    (text.chars().count() >= 40).then_some(text)
}

fn transcript_from_json3(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    value
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| event.get("segs").and_then(Value::as_array))
        .flat_map(|segments| segments.iter())
        .filter_map(|segment| segment.get("utf8").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn transcript_from_vtt(body: &str) -> String {
    let mut output = Vec::new();
    for line in body.lines().map(str::trim) {
        if line.is_empty()
            || line.starts_with("WEBVTT")
            || line.starts_with("NOTE")
            || line.contains("-->")
            || line.chars().all(|ch| ch.is_ascii_digit())
        {
            continue;
        }
        let text = clean_original_text(line);
        if !text.is_empty() && output.last() != Some(&text) {
            output.push(text);
        }
    }
    output.join("\n")
}

fn push_media(metadata: &mut LinkMetadata, kind: &str, url: &str, thumbnail_url: Option<String>) {
    if metadata.media.iter().any(|asset| asset.url == url) {
        return;
    }
    metadata.media.push(MediaAsset {
        kind: kind.to_string(),
        url: url.to_string(),
        thumbnail_url,
        alt: Some(metadata.title.clone()),
        ..MediaAsset::default()
    });
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

fn format_duration(seconds: f64) -> String {
    let total = seconds.round().max(0.0) as u64;
    let hours = total / 3600;
    let minutes = total % 3600 / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::{
        bilibili_bvid, bilibili_subtitle_text, bilibili_subtitle_url, caption_url,
        merge_bilibili_value, merge_metadata, transcript_from_json3, transcript_from_vtt,
    };

    #[test]
    fn merges_video_metadata_and_source_facts() {
        let mut metadata = LinkMetadata {
            platform: "YouTube".to_string(),
            ..LinkMetadata::default()
        };
        merge_metadata(
            &mut metadata,
            &serde_json::json!({
                "title":"Rust ownership explained",
                "description":"A practical walkthrough.",
                "uploader":"Example Channel",
                "thumbnail":"https://example.com/cover.jpg",
                "duration":125.0,
                "view_count":42
            }),
        );
        assert_eq!(metadata.title, "Rust ownership explained");
        assert_eq!(metadata.author.as_deref(), Some("Example Channel"));
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.value == "2:05")
        );
    }

    #[test]
    fn parses_caption_formats() {
        assert_eq!(
            transcript_from_json3(r#"{"events":[{"segs":[{"utf8":"你好"},{"utf8":"世界"}]}]}"#),
            "你好世界"
        );
        assert_eq!(
            transcript_from_vtt(
                "WEBVTT\n\n00:00.000 --> 00:01.000\n第一句\n\n00:01.000 --> 00:02.000\n第二句"
            ),
            "第一句\n第二句"
        );
    }

    #[test]
    fn falls_back_to_automatic_captions_when_manual_tracks_are_missing() {
        let value = serde_json::json!({
            "automatic_captions": {
                "zh": [{"ext": "json3", "url": "https://example.com/auto.json3"}]
            }
        });
        assert_eq!(
            caption_url(&value).as_deref(),
            Some("https://example.com/auto.json3")
        );
    }

    #[test]
    fn parses_bilibili_public_api_payload() {
        let mut metadata = LinkMetadata {
            platform: "哔哩哔哩".to_string(),
            ..LinkMetadata::default()
        };
        assert!(merge_bilibili_value(
            &mut metadata,
            &serde_json::json!({
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
        assert_eq!(
            bilibili_bvid("https://www.bilibili.com/video/BV1epwvzGEKf/"),
            Some("BV1epwvzGEKf".to_string())
        );
    }

    #[test]
    fn parses_bilibili_subtitle_track_and_body() {
        let player = serde_json::json!({
            "data": {"subtitle": {"subtitles": [
                {"lan": "en", "subtitle_url": "//example.com/en.json"},
                {"lan": "zh-CN", "subtitle_url": "//example.com/zh.json"}
            ]}}
        });
        assert_eq!(
            bilibili_subtitle_url(&player).as_deref(),
            Some("https://example.com/zh.json")
        );
        let body = serde_json::json!({"body": [
            {"content": "第一句字幕内容"},
            {"content": "第二句字幕内容"},
            {"content": "第三句字幕内容，长度足够用于正文分析。"},
            {"content": "第四句继续补充视频里的关键信息和上下文。"}
        ]});
        assert!(bilibili_subtitle_text(&body).is_some_and(|text| text.contains("第三句字幕内容")));
    }
}
