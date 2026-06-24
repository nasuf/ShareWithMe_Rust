use std::{env, fs, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use url::form_urlencoded;

#[derive(Debug, Clone)]
pub(crate) struct AppConfig {
    pub(crate) bind_addr: SocketAddr,
    pub(crate) store: StoreConfig,
    pub(crate) media_path: PathBuf,
    pub(crate) deepseek_api_key: Option<String>,
    pub(crate) deepseek_base_url: String,
    pub(crate) deepseek_model: String,
}

#[derive(Debug, Clone)]
pub(crate) enum StoreConfig {
    Local { path: PathBuf },
    Supabase { database_url: String },
}

impl AppConfig {
    pub(crate) fn from_env() -> Result<Self> {
        let bind_addr = env::var("SHARE_WITH_ME_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("parse SHARE_WITH_ME_BIND")?;
        let store = load_store_config()?;
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
            store,
            media_path,
            deepseek_api_key,
            deepseek_base_url,
            deepseek_model,
        })
    }

    pub(crate) fn storage_label(&self) -> String {
        match &self.store {
            StoreConfig::Local { path } => format!("local:{}", path.display()),
            StoreConfig::Supabase { .. } => "supabase:public.items".to_string(),
        }
    }
}

fn load_store_config() -> Result<StoreConfig> {
    let requested = env::var("SHARE_WITH_ME_STORE_BACKEND")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());
    match requested.as_deref() {
        Some("local") | Some("json") | Some("file") => Ok(local_store_config()),
        Some("supabase") | Some("postgres") | Some("postgresql") => supabase_store_config(),
        Some(other) => Err(anyhow!(
            "unsupported SHARE_WITH_ME_STORE_BACKEND '{other}', expected local or supabase"
        )),
        None if has_supabase_config() => supabase_store_config(),
        None => Ok(local_store_config()),
    }
}

fn local_store_config() -> StoreConfig {
    StoreConfig::Local {
        path: env::var("SHARE_WITH_ME_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("share_with_me_store.json")),
    }
}

fn has_supabase_config() -> bool {
    env::var("SUPABASE_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
        || env::var("SUPABASE_PASSWORD")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
}

fn supabase_store_config() -> Result<StoreConfig> {
    if let Some(database_url) = env::var("SUPABASE_DATABASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(StoreConfig::Supabase { database_url });
    }

    let project_ref = supabase_project_ref()?;
    let password = required_env("SUPABASE_PASSWORD")?;
    let pooler_host = env::var("SUPABASE_POOLER_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            env::var("SUPABASE_REGION")
                .ok()
                .map(|region| format!("aws-0-{}.pooler.supabase.com", region.trim()))
        })
        .ok_or_else(|| {
            anyhow!("SUPABASE_POOLER_HOST or SUPABASE_REGION is required for Supabase storage")
        })?;
    let port = env::var("SUPABASE_DB_PORT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "5432".to_string());
    let encoded_password = form_urlencoded::byte_serialize(password.as_bytes()).collect::<String>();
    let database_url = format!(
        "postgres://postgres.{project_ref}:{encoded_password}@{pooler_host}:{port}/postgres"
    );
    Ok(StoreConfig::Supabase { database_url })
}

fn required_env(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{name} is required for Supabase storage"))
}

fn supabase_project_ref() -> Result<String> {
    if let Some(project_ref) = env::var("SUPABASE_PROJECT_REF")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(project_ref);
    }

    let url = required_env("SUPABASE_URL")?;
    let host = url::Url::parse(&url)
        .context("parse SUPABASE_URL")?
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("SUPABASE_URL has no host"))?;
    host.strip_suffix(".supabase.co")
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("SUPABASE_URL must look like https://<project-ref>.supabase.co"))
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
