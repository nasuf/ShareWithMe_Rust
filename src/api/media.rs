use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

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
    analysis::{extract_metadata_with_video_transcription, is_safe_remote_url},
    content::{ArchiveSnapshot, bytes_fingerprint, content_fingerprint, media_identity},
    error::ApiError,
    models::LinkItem,
    render::render_snapshot,
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
        ensure_archive(&state, &item_id).await;
    });
}

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
    if item.platform == "小红书" && !item.source_url.trim().is_empty() {
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
    if host.contains("xhscdn.com") || host.contains("xiaohongshu.com") {
        "https://www.xiaohongshu.com/"
    } else if host.contains("toutiao") || host.contains("byteimg") {
        "https://www.toutiao.com/"
    } else if host.contains("hdslb.com") || host.contains("bilibili.com") {
        "https://www.bilibili.com/"
    } else if host.contains("douyinpic.com") || host.contains("douyinvod.com") {
        "https://www.douyin.com/"
    } else if host.contains("zhimg.com") || host.contains("zhihu.com") {
        "https://www.zhihu.com/"
    } else if host.contains("sinaimg.cn") || host.contains("weibo.com") {
        "https://weibo.com/"
    } else if host.contains("doubanio.com") || host.contains("douban.com") {
        "https://www.douban.com/"
    } else if host.contains("instagram.com") || host.contains("cdninstagram.com") {
        "https://www.instagram.com/"
    } else if host.contains("twimg.com") {
        "https://x.com/"
    } else if host.contains("ytimg.com") || host.contains("googleusercontent.com") {
        "https://www.youtube.com/"
    } else if host.contains("autonavi.com") || host.contains("amap.com") {
        "https://www.amap.com/"
    } else if host.contains("bdimg.com") || host.contains("baidu.com") {
        "https://map.baidu.com/"
    } else if host.contains("360buyimg.com") || host.contains("jd.com") {
        "https://item.jd.com/"
    } else if host.contains("alicdn.com") || host.contains("taobao.com") {
        "https://www.taobao.com/"
    } else if is_wechat_media_host(&host) {
        // mmbiz.qpic.cn answers a foreign Referer with a 200 "此图片来自微信公众平台
        // 未经允许不可引用" placeholder instead of the image.
        "https://mp.weixin.qq.com/"
    } else if host.contains("weread.qq.com") || host.contains("wrimg.com") {
        "https://weread.qq.com/"
    } else {
        "https://www.google.com/"
    }
}

fn is_wechat_media_host(host: &str) -> bool {
    host.ends_with("qpic.cn") || host.ends_with("qlogo.cn") || host == "mp.weixin.qq.com"
}

fn is_wechat_media_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(is_wechat_media_host))
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

fn safe_file_id(id: &str) -> String {
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

    #[test]
    fn uses_platform_referers_for_protected_media_hosts() {
        assert_eq!(
            referer_for_cover("https://i0.hdslb.com/bfs/archive/cover.jpg"),
            "https://www.bilibili.com/"
        );
        assert_eq!(
            referer_for_cover("https://p3-sign.douyinpic.com/tos-cn/cover.jpeg"),
            "https://www.douyin.com/"
        );
        assert_eq!(
            referer_for_cover("https://img.alicdn.com/bao/uploaded/item.jpg"),
            "https://www.taobao.com/"
        );
    }
}
