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
    error::ApiError,
    models::LinkItem,
    state::AppState,
    store::{find_item, save_item, update_item_cover},
};

use super::RefreshQuery;

const COVER_REFRESH_COOLDOWN: Duration = Duration::hours(6);

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
        &format!("cover-{}.jpg", safe_file_id(item_id)),
        remote_url,
        900,
    )
    .await
}

async fn cache_item_media(state: &AppState, item_id: &str) -> anyhow::Result<()> {
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
        .take(6)
        .collect::<Vec<_>>();
    let mut changed = false;
    for index in image_indexes {
        let remote_url = item.media[index].url.clone();
        let file_name = format!("asset-{}-{index}.jpg", safe_file_id(item_id));
        match cache_remote_image(state, &file_name, &remote_url, 1200).await {
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
    file_name: &str,
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
    let bytes = response.bytes().await.context("read cover bytes")?;
    if bytes.len() > 12 * 1024 * 1024 {
        anyhow::bail!("cover image too large");
    }

    let jpeg = compress_image_jpeg(&bytes, max_dimension)?;
    tokio::fs::create_dir_all(&state.config.media_path)
        .await
        .with_context(|| {
            format!(
                "create media directory {}",
                state.config.media_path.display()
            )
        })?;
    let path = state.config.media_path.join(file_name);
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
    } else if host.contains("weread.qq.com") || host.contains("wrimg.com") {
        "https://weread.qq.com/"
    } else {
        "https://www.google.com/"
    }
}

fn safe_file_id(id: &str) -> String {
    id.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::referer_for_cover;

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
