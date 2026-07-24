use std::{env, path::PathBuf, time::Duration};

use reqwest::{Client, multipart::Form};
use serde_json::Value;
use tokio::{fs, process::Command, time::timeout};
use uuid::Uuid;

use super::super::classify::clean_original_text;

const DEFAULT_MAX_AUDIO_BYTES: u64 = 96 * 1024 * 1024;
const DEFAULT_MAX_VIDEO_SECONDS: f64 = 7_200.0;

#[derive(Debug, Clone)]
struct AsrSettings {
    url: String,
    api_key: Option<String>,
    model: String,
    language: Option<String>,
    max_audio_bytes: u64,
    max_video_seconds: f64,
}

impl AsrSettings {
    fn from_env() -> Option<Self> {
        let url = env_value("SHARE_WITH_ME_ASR_URL")?;
        Some(Self {
            url,
            api_key: env_value("SHARE_WITH_ME_ASR_API_KEY"),
            model: env_value("SHARE_WITH_ME_ASR_MODEL").unwrap_or_else(|| "whisper-1".to_string()),
            language: env_value("SHARE_WITH_ME_ASR_LANGUAGE"),
            max_audio_bytes: env_value("SHARE_WITH_ME_ASR_MAX_BYTES")
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_MAX_AUDIO_BYTES),
            max_video_seconds: env_value("SHARE_WITH_ME_ASR_MAX_SECONDS")
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_MAX_VIDEO_SECONDS),
        })
    }
}

pub(super) async fn transcribe_if_configured(
    http: &Client,
    source_url: &str,
    duration_seconds: Option<f64>,
) -> Option<String> {
    let settings = AsrSettings::from_env()?;
    if duration_seconds.is_some_and(|duration| duration > settings.max_video_seconds) {
        return None;
    }

    let temp_dir = env::temp_dir().join(format!("sharewithme-asr-{}", Uuid::new_v4()));
    let audio_path = download_audio(source_url, &temp_dir, settings.max_audio_bytes).await?;
    let result = transcribe_file(http, &settings, &audio_path).await;
    let _ = fs::remove_dir_all(temp_dir).await;
    result
}

async fn download_audio(source_url: &str, temp_dir: &PathBuf, max_bytes: u64) -> Option<PathBuf> {
    fs::create_dir_all(temp_dir).await.ok()?;
    let executable = env::var("SHARE_WITH_ME_YTDLP").unwrap_or_else(|_| "yt-dlp".to_string());
    let output_template = temp_dir.join("audio.%(ext)s");
    let max_size = format!("{}M", (max_bytes / 1024 / 1024).max(1));
    let mut command = Command::new(executable);
    command.kill_on_drop(true).args([
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "15",
        "--retries",
        "1",
        "--js-runtimes",
        "node",
        "--max-filesize",
        &max_size,
        "--format",
        "bestaudio/best",
        "--output",
    ]);
    command.arg(output_template);
    if let Some(cookies) =
        env_value("SHARE_WITH_ME_YTDLP_COOKIES").filter(|path| std::path::Path::new(path).is_file())
    {
        command.arg("--cookies").arg(cookies);
    }
    command.arg(source_url);

    let output = timeout(Duration::from_secs(150), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(temp_dir).await;
        return None;
    }

    let mut entries = fs::read_dir(temp_dir).await.ok()?;
    while let Some(entry) = entries.next_entry().await.ok()? {
        let path = entry.path();
        let metadata = entry.metadata().await.ok()?;
        if metadata.is_file() && metadata.len() > 0 && metadata.len() <= max_bytes {
            return Some(path);
        }
    }
    let _ = fs::remove_dir_all(temp_dir).await;
    None
}

async fn transcribe_file(
    http: &Client,
    settings: &AsrSettings,
    audio_path: &PathBuf,
) -> Option<String> {
    let mut form = Form::new()
        .text("model", settings.model.clone())
        .text("response_format", "json")
        .file("file", audio_path)
        .await
        .ok()?;
    if let Some(language) = settings.language.clone() {
        form = form.text("language", language);
    }

    let mut request = http
        .post(&settings.url)
        .timeout(Duration::from_secs(180))
        .multipart(form);
    if let Some(api_key) = settings.api_key.as_deref() {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await.ok()?.error_for_status().ok()?;
    let body = response.text().await.ok()?;
    let transcript = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or(body);
    let transcript = clean_original_text(&transcript);
    (transcript.chars().count() >= 40).then_some(transcript)
}

fn env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{AsrSettings, DEFAULT_MAX_AUDIO_BYTES, DEFAULT_MAX_VIDEO_SECONDS};

    #[test]
    fn asr_defaults_are_bounded() {
        assert_eq!(DEFAULT_MAX_AUDIO_BYTES, 96 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_VIDEO_SECONDS, 7_200.0);
        assert!(std::mem::size_of::<AsrSettings>() > 0);
    }
}
