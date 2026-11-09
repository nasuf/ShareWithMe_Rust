//! Archive screenshots: a JPEG of the article as it looked when saved, so the
//! item stays viewable in its original layout after the link dies.

use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use url::Url;

use crate::{
    content::ArchiveSnapshot,
    error::ApiError,
    links::is_safe_remote_url,
    models::LinkItem,
    platforms::Platform,
    render::render_snapshot,
    state::AppState,
    store::{find_item, save_item},
};

use super::media::safe_file_id;

// Capture the visual archive once, in the background, while the source is
// still reachable. Failures are logged; the next refresh will retry.
pub(super) async fn ensure_archive(state: &AppState, item_id: &str) {
    if let Err(error) = capture_archive(state, item_id, false).await {
        tracing::warn!(item_id, %error, "capture archive snapshot failed");
    }
}

pub(super) async fn recapture_archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    match capture_archive(&state, &id, true).await {
        Ok(true) => {}
        Ok(false) => return Err(ApiError::bad_request("原链接当前无法生成存档快照")),
        Err(error) => return Err(ApiError::internal(error)),
    }
    let item = find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    Ok(Json(item))
}

static ARCHIVES_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

// Returns Ok(true) when a new snapshot was stored.
async fn capture_archive(state: &AppState, item_id: &str, force: bool) -> anyhow::Result<bool> {
    let Some(item) = find_item(&state.store, item_id).await? else {
        return Ok(false);
    };
    if (!force && item.archive.is_some()) || !archivable(&item) {
        return Ok(false);
    }
    {
        let mut in_flight = ARCHIVES_IN_FLIGHT
            .lock()
            .map_err(|_| anyhow::anyhow!("archive lock poisoned"))?;
        if !in_flight.insert(item_id.to_string()) {
            return Ok(false);
        }
    }
    let result = capture_archive_file(state, &item).await;
    if let Ok(mut in_flight) = ARCHIVES_IN_FLIGHT.lock() {
        in_flight.remove(item_id);
    }
    result
}

fn archivable(item: &LinkItem) -> bool {
    // A captcha wall or a "deleted" notice is not worth archiving.
    !matches!(item.extraction.status.as_str(), "blocked")
        && Url::parse(&item.final_url)
            .ok()
            .is_some_and(|url| is_safe_remote_url(&url))
}

// Xiaohongshu's canonical /explore/{id} URL lacks the share's xsec_token and
// shows a login wall; the original share link redirects with the token.
fn archive_source_url(item: &LinkItem) -> &str {
    if Platform::from_label(&item.platform) == Platform::Xiaohongshu
        && !item.source_url.trim().is_empty()
    {
        &item.source_url
    } else {
        &item.final_url
    }
}

async fn capture_archive_file(state: &AppState, item: &LinkItem) -> anyhow::Result<bool> {
    tokio::fs::create_dir_all(&state.config.media_path)
        .await
        .context("create media directory")?;
    // A fresh name per capture: media is served as immutable, so overwriting a
    // file in place would leave clients showing the stale image.
    let captured_at = Utc::now();
    let file_name = format!(
        "snapshot-{}-{}.jpg",
        safe_file_id(&item.id),
        captured_at.timestamp()
    );
    let path = state.config.media_path.join(&file_name);
    let Some(rendered) = render_snapshot(archive_source_url(item), &path).await else {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(false);
    };
    let (Some(width), Some(height)) = (rendered.snapshot_width, rendered.snapshot_height) else {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(false);
    };

    // Re-read so the background media/AI writes are not overwritten.
    let Some(mut current) = find_item(&state.store, &item.id).await? else {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(false);
    };
    let previous = current.archive.replace(ArchiveSnapshot {
        kind: "screenshot".to_string(),
        url: format!("/media/{file_name}"),
        width,
        height,
        source_url: item.final_url.clone(),
        captured_at,
    });
    current.updated_at = Utc::now();
    save_item(&state.store, current).await?;
    if let Some(old_file) = previous
        .as_ref()
        .and_then(|archive| archive.url.strip_prefix("/media/"))
        .filter(|old| *old != file_name && !old.contains('/'))
    {
        let _ = tokio::fs::remove_file(state.config.media_path.join(old_file)).await;
    }
    Ok(true)
}
