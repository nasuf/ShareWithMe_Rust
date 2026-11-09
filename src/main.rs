use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use reqwest::Client;
use tracing::info;

mod analysis;
mod api;
mod config;
mod content;
mod digest;
mod error;
mod extraction;
mod links;
mod models;
mod network;
mod platforms;
mod probe;
mod render;
mod state;
mod store;
mod text;

use api::build_router;
use config::AppConfig;
use network::safe_redirect_policy;
use state::AppState;
use store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::from_filename("../.env").ok();
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|argument| {
        matches!(
            argument.as_str(),
            "--probe-shared" | "--probe-shared-strict"
        )
    }) {
        if args.len() != 3 {
            anyhow::bail!("usage: backend --probe-shared[-strict] <url> <shared text>");
        }
        return probe::run_shared(&args[1], &args[2], args[0] == "--probe-shared-strict").await;
    }
    if args
        .first()
        .is_some_and(|argument| matches!(argument.as_str(), "--probe" | "--probe-strict"))
    {
        return probe::run(&args[1..], args[0] == "--probe-strict").await;
    }

    let config = Arc::new(AppConfig::from_env()?);
    let store = Store::load(&config).await?;

    let http = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(22))
        .redirect(safe_redirect_policy(8))
        .user_agent("ShareWithMe/0.1 (+personal link organizer)")
        .build()
        .context("build http client")?;

    let state = AppState {
        store: Arc::new(store),
        http,
        config: Arc::clone(&config),
    };

    let repair_state = state.clone();
    tokio::spawn(async move { api::repair_wechat_placeholders(&repair_state).await });

    let app = build_router(state);

    info!("ShareWithMe backend listening on {}", config.bind_addr);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
