//! Video metadata and transcripts via yt-dlp, shared by every video platform.
//!
//! Order of preference for the video body: the platform's own captions, then
//! yt-dlp's subtitle tracks, then (if configured) speech-to-text ([`super::asr`]).

use std::{env, path::Path, time::Duration};

use reqwest::Client;
use serde_json::Value;
use tokio::{process::Command, time::timeout};

use crate::{
    models::LinkMetadata,
    text::{clean_original_text, clean_text},
};

/// Run yt-dlp for the snapshot's URL and merge title, description, uploader,
/// thumbnail, duration and a transcript.
///
/// `description_is_body`: the platform's video description is the item body
/// even when shorter than what was captured before (YouTube, Bilibili, Douyin).
pub(crate) async fn enrich_with_yt_dlp(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
    description_is_body: bool,
) {
    let Some(value) = yt_dlp_json(&metadata.final_url).await else {
        return;
    };
    merge_metadata(metadata, &value, description_is_body);
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

/// Use a transcript as the item body and record where it came from.
pub(crate) fn apply_transcript(metadata: &mut LinkMetadata, transcript: String, source: &str) {
    metadata.set_body(transcript);
    metadata.set_fact("transcript_source", "视频正文", source.to_string());
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

fn merge_metadata(metadata: &mut LinkMetadata, value: &Value, description_is_body: bool) {
    if let Some(title) = string(value, "title") {
        metadata.title = clean_text(title);
    }
    if let Some(description) = string(value, "description") {
        let description = clean_original_text(description);
        if description_is_body
            || description.chars().count() > metadata.original_text.chars().count()
        {
            metadata.description = clean_text(&description);
            metadata.set_body(description);
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
        metadata.add_image(thumbnail);
    }
    if let Some(webpage_url) = string(value, "webpage_url") {
        metadata.final_url = webpage_url.to_string();
    }
    if let Some(duration) = value.get("duration").and_then(Value::as_f64) {
        metadata.set_fact("duration", "时长", format_duration(duration));
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
        metadata.set_fact("view_count", "播放量", view_count.to_string());
    }
    if let Some(tags) = value.get("tags").and_then(Value::as_array) {
        let tags = tags
            .iter()
            .filter_map(Value::as_str)
            .take(12)
            .collect::<Vec<_>>()
            .join("、");
        metadata.set_fact("source_tags", "原标签", tags);
    }
}

/// `//host/x` and `http://host/x` -> `https://host/x`.
pub(crate) fn normalize_https(value: &str) -> String {
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

/// `125.0` -> `2:05`, `3725.0` -> `1:02:05`.
pub(crate) fn format_duration(seconds: f64) -> String {
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
        caption_url, format_duration, merge_metadata, transcript_from_json3, transcript_from_vtt,
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
            true,
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
    fn formats_durations() {
        assert_eq!(format_duration(125.0), "2:05");
        assert_eq!(format_duration(3725.0), "1:02:05");
        assert_eq!(format_duration(-3.0), "0:00");
    }
}
