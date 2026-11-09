//! Cached images: item covers and inline article images, downloaded once with
//! the source CDN's Referer and stored as bounded JPEGs under `/media`.

use anyhow::Context;
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use chrono::{Duration, Utc};
use image::{ColorType, codecs::jpeg::JpegEncoder, imageops::FilterType};
use url::Url;

use crate::{
    content::{bytes_fingerprint, content_fingerprint, media_identity},
    error::ApiError,
    extraction::extract_metadata_with_video_transcription,
    links::is_safe_remote_url,
    models::LinkItem,
    platforms::media_referer,
    state::AppState,
    store::{all_items, find_item, save_item, update_item_cover},
};

use super::RefreshQuery;

const COVER_REFRESH_COOLDOWN: Duration = Duration::hours(6);
// Inline article images; source CDNs (e.g. Toutiao) sign URLs that expire, so
// anything not cached locally eventually stops rendering.
const MAX_CACHED_IMAGES: usize = 40;

pub(super) async fn refresh_cover(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<RefreshQuery>,
) -> Result<Json<LinkItem>, ApiError> {
    let current = find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    let now = Utc::now();
    let force = query.force.unwrap_or(false);

    if !force
        && current.cover_checked_at.is_some_and(|checked_at| {
            now.signed_duration_since(checked_at) < COVER_REFRESH_COOLDOWN
        })
    {
        return Ok(Json(current));
    }

    let candidates = cover_candidates(&state, &current).await;
    for remote_url in candidates {
        if let Ok(local_url) = cache_cover_image(&state, &current.id, &remote_url).await {
            let updated = update_item_cover(
                &state.store,
                &current.id,
                Some(local_url),
                Some(remote_url),
                Some(now),
                now,
            )
            .await
            .map_err(ApiError::internal)?;
            return Ok(Json(updated));
        }
    }

    let updated = update_item_cover(&state.store, &current.id, None, None, None, now)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

pub(super) async fn media_file(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if file.contains('/') || file.contains('\\') || file.starts_with('.') {
        return Err(ApiError::not_found("media not found"));
    }
    let path = state.config.media_path.join(file);
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| ApiError::not_found("media not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    ))
}

pub(super) fn schedule_media_cache(state: AppState, item_id: String) {
    tokio::spawn(async move {
        if let Err(error) = cache_item_media(&state, &item_id).await {
            tracing::warn!(item_id, %error, "cache item media failed");
        }
        super::archive::ensure_archive(&state, &item_id).await;
    });
}

pub(super) fn is_local_media_url(url: &str) -> bool {
    url.starts_with("/media/")
}

async fn cover_candidates(state: &AppState, current: &LinkItem) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut metadata_urls = Vec::new();

    for source in [&current.source_url, &current.final_url] {
        let trimmed = source.trim();
        if trimmed.is_empty() || metadata_urls.iter().any(|url| url == trimmed) {
            continue;
        }
        metadata_urls.push(trimmed.to_string());
    }

    for source in metadata_urls {
        let metadata =
            extract_metadata_with_video_transcription(&state.http, &source, None, false).await;
        push_cover_candidate(&mut candidates, metadata.image_url.as_deref());
    }

    for value in [
        current.remote_image_url.as_deref(),
        current
            .image_url
            .as_deref()
            .filter(|url| !is_local_media_url(url)),
    ]
    .into_iter()
    .flatten()
    {
        push_cover_candidate(&mut candidates, Some(value));
    }
    candidates
}

fn push_cover_candidate(candidates: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if !candidates.iter().any(|candidate| candidate == value) {
        candidates.push(value.to_string());
    }
}

async fn cache_cover_image(
    state: &AppState,
    item_id: &str,
    remote_url: &str,
) -> anyhow::Result<String> {
    cache_remote_image(
        state,
        &format!("cover-{}", safe_file_id(item_id)),
        remote_url,
        900,
    )
    .await
}

pub(super) async fn cache_item_media(state: &AppState, item_id: &str) -> anyhow::Result<()> {
    let Some(mut item) = find_item(&state.store, item_id).await? else {
        return Ok(());
    };
    if let Some(image_url) = item
        .image_url
        .clone()
        .filter(|url| !is_local_media_url(url))
        && !item.media.iter().any(|asset| asset.url == image_url)
    {
        item.media.insert(
            0,
            crate::content::MediaAsset {
                kind: "image".to_string(),
                url: image_url,
                alt: Some(item.title.clone()),
                ..crate::content::MediaAsset::default()
            },
        );
    }

    let image_indexes = item
        .media
        .iter()
        .enumerate()
        .filter(|(_, asset)| {
            asset.kind == "image" && asset.cached_url.is_none() && !is_local_media_url(&asset.url)
        })
        .map(|(index, _)| index)
        .take(MAX_CACHED_IMAGES)
        .collect::<Vec<_>>();
    let mut changed = false;
    for index in image_indexes {
        let remote_url = item.media[index].url.clone();
        // Keyed by URL, not position: a refresh can reorder `media`, and an
        // index-based name would overwrite a file another asset still uses.
        let file_stem = format!(
            "asset-{}-{}",
            safe_file_id(item_id),
            content_fingerprint(&media_identity(&remote_url))
        );
        match cache_remote_image(state, &file_stem, &remote_url, 1200).await {
            Ok(local_url) => {
                item.media[index].cached_url = Some(local_url.clone());
                if index == 0 || item.image_url.as_deref() == Some(remote_url.as_str()) {
                    item.remote_image_url = Some(remote_url);
                    item.image_url = Some(local_url);
                    item.cover_cached_at = Some(Utc::now());
                    item.cover_checked_at = Some(Utc::now());
                }
                changed = true;
            }
            Err(error) => tracing::debug!(item_id, index, %error, "skip unavailable media"),
        }
    }
    if changed {
        item.updated_at = Utc::now();
        save_item(&state.store, item).await?;
    }
    Ok(())
}

async fn cache_remote_image(
    state: &AppState,
    file_stem: &str,
    remote_url: &str,
    max_dimension: u32,
) -> anyhow::Result<String> {
    let parsed_url = Url::parse(remote_url).context("parse cover URL")?;
    if !is_safe_remote_url(&parsed_url) {
        anyhow::bail!("refuse non-public cover URL");
    }
    let response = state
        .http
        .get(remote_url)
        .header(header::REFERER, referer_for_cover(remote_url))
        .send()
        .await
        .context("download cover")?;
    if !response.status().is_success() {
        anyhow::bail!("download cover returned HTTP {}", response.status());
    }
    if is_hotlink_placeholder(response.headers()) {
        anyhow::bail!("source CDN returned its hotlink placeholder");
    }
    let bytes = response.bytes().await.context("read cover bytes")?;
    if bytes.len() > 12 * 1024 * 1024 {
        anyhow::bail!("cover image too large");
    }

    let jpeg = compress_image_jpeg(&bytes, max_dimension)?;
    // `/media` is served as immutable, so a file whose bytes change (a fixed
    // placeholder, a new cover) must get a new name or clients keep the old one.
    let file_name = format!("{file_stem}-{}.jpg", &bytes_fingerprint(&jpeg)[..8]);
    tokio::fs::create_dir_all(&state.config.media_path)
        .await
        .with_context(|| {
            format!(
                "create media directory {}",
                state.config.media_path.display()
            )
        })?;
    let path = state.config.media_path.join(&file_name);
    tokio::fs::write(&path, jpeg)
        .await
        .with_context(|| format!("write cover {}", path.display()))?;
    Ok(format!("/media/{file_name}"))
}

fn compress_image_jpeg(bytes: &Bytes, max_dimension: u32) -> anyhow::Result<Vec<u8>> {
    let image = image::load_from_memory(bytes).context("decode cover image")?;
    let resized = image
        .resize(max_dimension, max_dimension, FilterType::Lanczos3)
        .to_rgb8();
    let (width, height) = resized.dimensions();
    let mut output = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut output, 82);
    encoder
        .encode(&resized, width, height, ColorType::Rgb8.into())
        .context("encode cover jpeg")?;
    Ok(output)
}

fn referer_for_cover(remote_url: &str) -> &'static str {
    let host = Url::parse(remote_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    media_referer(&host)
}

fn is_wechat_media_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(crate::platforms::wechat::is_media_host))
        .unwrap_or(false)
}

// Hotlink-protected CDNs answer HTTP 200 with a stand-in image; Tencent marks
// it with `x-errinfo: banned`. Caching that would hide the real image forever.
fn is_hotlink_placeholder(headers: &header::HeaderMap) -> bool {
    headers
        .get("x-errinfo")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("banned"))
}

const WECHAT_REPAIR_MARKER: &str = ".repair-wechat-referer-v1";

// One-off: WeChat images cached before the mp.weixin.qq.com Referer was used
// are the hotlink placeholder. Drop those caches once and download them again.
pub(crate) async fn repair_wechat_placeholders(state: &AppState) {
    let marker = state.config.media_path.join(WECHAT_REPAIR_MARKER);
    if tokio::fs::try_exists(&marker).await.unwrap_or(false) {
        return;
    }
    let items = match all_items(&state.store).await {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(%error, "wechat media repair: load items failed");
            return;
        }
    };
    let mut repaired = 0;
    for mut item in items {
        let mut stale_files = Vec::new();
        for asset in &mut item.media {
            if is_wechat_media_url(&asset.url)
                && let Some(cached) = asset.cached_url.take()
            {
                stale_files.push(cached);
            }
        }
        if let Some(remote) = item
            .remote_image_url
            .clone()
            .filter(|remote| is_wechat_media_url(remote))
            && item.image_url.as_deref().is_some_and(is_local_media_url)
        {
            stale_files.extend(item.image_url.replace(remote));
        }
        if stale_files.is_empty() {
            continue;
        }
        let item_id = item.id.clone();
        item.updated_at = Utc::now();
        if let Err(error) = save_item(&state.store, item).await {
            tracing::warn!(item_id, %error, "wechat media repair: save failed");
            continue;
        }
        if let Err(error) = cache_item_media(state, &item_id).await {
            tracing::warn!(item_id, %error, "wechat media repair: re-cache failed");
        }
        remove_unreferenced_media(state, &item_id, stale_files).await;
        repaired += 1;
    }
    tracing::info!(repaired, "wechat media repair finished");
    if let Err(error) = tokio::fs::write(&marker, b"done\n").await {
        tracing::warn!(%error, "wechat media repair: write marker failed");
    }
}

async fn remove_unreferenced_media(state: &AppState, item_id: &str, files: Vec<String>) {
    let Ok(Some(item)) = find_item(&state.store, item_id).await else {
        return;
    };
    for local in files {
        let still_used = item.image_url.as_deref() == Some(local.as_str())
            || item
                .media
                .iter()
                .any(|asset| asset.cached_url.as_deref() == Some(local.as_str()));
        if let Some(name) = local
            .strip_prefix("/media/")
            .filter(|name| !still_used && !name.contains('/') && !name.starts_with('.'))
        {
            let _ = tokio::fs::remove_file(state.config.media_path.join(name)).await;
        }
    }
}

pub(super) fn safe_file_id(id: &str) -> String {
    id.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{is_hotlink_placeholder, is_wechat_media_url, referer_for_cover};

    #[test]
    fn wechat_images_use_the_official_account_referer() {
        let url = "https://mmbiz.qpic.cn/sz_mmbiz_png/abc/640?wx_fmt=png&from=appmsg";
        assert_eq!(referer_for_cover(url), "https://mp.weixin.qq.com/");
        assert!(is_wechat_media_url(url));
        assert!(is_wechat_media_url("https://wx.qlogo.cn/mmhead/abc/0"));
        assert!(!is_wechat_media_url("https://example.com/qpic.cn.jpg"));
    }

    #[test]
    fn rejects_tencent_hotlink_placeholder() {
        let mut headers = HeaderMap::new();
        assert!(!is_hotlink_placeholder(&headers));
        headers.insert("x-errinfo", HeaderValue::from_static("banned"));
        assert!(is_hotlink_placeholder(&headers));
    }
}
