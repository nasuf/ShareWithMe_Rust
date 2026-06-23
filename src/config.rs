use std::{env, fs, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub(crate) struct AppConfig {
    pub(crate) bind_addr: SocketAddr,
    pub(crate) storage_path: PathBuf,
    pub(crate) media_path: PathBuf,
    pub(crate) deepseek_api_key: Option<String>,
    pub(crate) deepseek_base_url: String,
    pub(crate) deepseek_model: String,
}

impl AppConfig {
    pub(crate) fn from_env() -> Result<Self> {
        let bind_addr = env::var("SHARE_WITH_ME_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("parse SHARE_WITH_ME_BIND")?;
        let storage_path = env::var("SHARE_WITH_ME_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("share_with_me_store.json"));
        let media_path = env::var("SHARE_WITH_ME_MEDIA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("media"));
        let deepseek_api_key = env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(read_key_txt);
        let deepseek_base_url = env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".to_string());
        let deepseek_model =
            env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());

        Ok(Self {
            bind_addr,
            storage_path,
            media_path,
            deepseek_api_key,
            deepseek_base_url,
            deepseek_model,
        })
    }
}

fn read_key_txt() -> Option<String> {
    for candidate in ["../key.txt", "key.txt"] {
        if let Ok(value) = fs::read_to_string(candidate) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}
