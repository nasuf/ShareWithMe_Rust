use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, patch, post},
};
use chrono::{Duration, Utc};
use image::{ColorType, codecs::jpeg::JpegEncoder, imageops::FilterType};
use tower_http::cors::{Any, CorsLayer};
use url::Url;

use crate::{
    analysis::{
        analyze_with_deepseek_or_fallback, build_item, check_deepseek, extract_metadata,
        resolve_request_url, resolve_request_urls,
    },
    digest::{collection_digest, count_by},
    error::ApiError,
    models::{
        AnalyzeLinkRequest, AnalyzerCheckResponse, CollectionDigest, CollectionStats,
        HealthResponse, JsonExport, JsonImportRequest, JsonImportResponse, LinkItem, LinkMetadata,
        ListItemsQuery, UpdateItemRequest, UpdateStatusRequest,
    },
    state::AppState,
    store::{
        all_items, delete_item_by_id, find_item, import_items, save_item, update_item_cover,
        update_item_metadata, update_item_status,
    },
};

const COVER_REFRESH_COOLDOWN: Duration = Duration::hours(6);

#[derive(Default, serde::Deserialize)]
struct RefreshCoverQuery {
    force: Option<bool>,
}

pub(crate) fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/items", get(list_items))
        .route(
            "/api/items/{id}",
            get(get_item).patch(update_item).delete(delete_item),
        )
        .route("/api/items/{id}/status", patch(update_status))
        .route("/api/items/{id}/reanalyze", post(reanalyze_item))
        .route("/api/items/{id}/cover/refresh", post(refresh_cover))
        .route("/api/links/analyze", post(analyze_link))
        .route("/api/links/analyze-many", post(analyze_links))
        .route("/media/{file}", get(media_file))
        .route("/api/stats", get(stats))
        .route("/api/digest", get(digest))
        .route("/api/export/markdown", get(export_markdown))
        .route("/api/export/json", get(export_json))
        .route("/api/import/json", post(import_json))
        .route("/api/analyzer/check", post(check_analyzer))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        analyzer_provider: "deepseek",
        deepseek_configured: state.config.deepseek_api_key.is_some(),
        deepseek_model: state.config.deepseek_model.clone(),
        storage_path: state.config.storage_path.display().to_string(),
    })
}

async fn check_analyzer(
    State(state): State<AppState>,
) -> Result<Json<AnalyzerCheckResponse>, ApiError> {
    Ok(Json(check_deepseek(&state).await))
}

async fn analyze_link(
    State(state): State<AppState>,
    Json(request): Json<AnalyzeLinkRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let source_url = resolve_request_url(&request)?;
    let item = analyze_one_url(&state, &source_url, &request).await?;
    Ok(Json(item))
}

async fn analyze_links(
    State(state): State<AppState>,
    Json(request): Json<AnalyzeLinkRequest>,
) -> Result<Json<Vec<LinkItem>>, ApiError> {
    let source_urls = resolve_request_urls(&request)?;
    let mut items = Vec::new();
    for source_url in source_urls {
        items.push(analyze_one_url(&state, &source_url, &request).await?);
    }
    Ok(Json(items))
}

async fn analyze_one_url(
    state: &AppState,
    source_url: &str,
    request: &AnalyzeLinkRequest,
) -> Result<LinkItem, ApiError> {
    let metadata = extract_metadata(&state.http, source_url, request.shared_text.as_deref()).await;
    let analysis = analyze_with_deepseek_or_fallback(state, &metadata).await;
    let item = build_item(metadata, analysis, request.source_app.clone());
    save_item(&state.store, item.clone()).map_err(ApiError::internal)?;
    Ok(item)
}

async fn reanalyze_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    let current = find_item(&state.store, &id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    let metadata = LinkMetadata {
        source_url: current.source_url.clone(),
        final_url: current.final_url.clone(),
        platform: current.platform.clone(),
        title: current.title.clone(),
        description: current.description.clone(),
        author: current.author.clone(),
        image_url: current.remote_image_url.clone().or_else(|| {
            current
                .image_url
                .clone()
                .filter(|url| !is_local_media_url(url))
        }),
        content_text: current.content_text.clone(),
        original_text: if current.original_text.trim().is_empty() {
            current.content_text.clone()
        } else {
            current.original_text.clone()
        },
    };
    let analysis = analyze_with_deepseek_or_fallback(&state, &metadata).await;
    let mut updated = build_item(metadata, analysis, current.source_app.clone());
    updated.id = current.id;
    updated.created_at = current.created_at;
    updated.status = current.status;
    save_item(&state.store, updated.clone()).map_err(ApiError::internal)?;
    Ok(Json(updated))
}

async fn refresh_cover(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<RefreshCoverQuery>,
) -> Result<Json<LinkItem>, ApiError> {
    let current = find_item(&state.store, &id)
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
            .map_err(ApiError::internal)?;
            return Ok(Json(updated));
        }
    }

    let updated = update_item_cover(&state.store, &current.id, None, None, None, now)
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

async fn media_file(
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
        let metadata = extract_metadata(&state.http, &source, None).await;
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
    Url::parse(remote_url).context("parse cover URL")?;
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

    let jpeg = compress_cover_jpeg(&bytes)?;
    tokio::fs::create_dir_all(&state.config.media_path)
        .await
        .with_context(|| {
            format!(
                "create media directory {}",
                state.config.media_path.display()
            )
        })?;
    let file_name = format!("cover-{}.jpg", safe_file_id(item_id));
    let path = state.config.media_path.join(&file_name);
    tokio::fs::write(&path, jpeg)
        .await
        .with_context(|| format!("write cover {}", path.display()))?;
    Ok(format!("/media/{file_name}"))
}

fn compress_cover_jpeg(bytes: &Bytes) -> anyhow::Result<Vec<u8>> {
    let image = image::load_from_memory(bytes).context("decode cover image")?;
    let resized = image.resize(900, 900, FilterType::Lanczos3).to_rgb8();
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
    } else {
        "https://www.google.com/"
    }
}

fn is_local_media_url(url: &str) -> bool {
    url.starts_with("/media/")
}

fn safe_file_id(id: &str) -> String {
    id.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>()
}

async fn list_items(
    State(state): State<AppState>,
    Query(query): Query<ListItemsQuery>,
) -> Result<Json<Vec<LinkItem>>, ApiError> {
    let mut items = all_items(&state.store).map_err(ApiError::internal)?;
    items.retain(|item| matches_filter(item, &query));
    sort_items(&mut items, query.sort.as_deref());
    Ok(Json(items))
}

async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    let item = find_item(&state.store, &id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    Ok(Json(item))
}

async fn delete_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !delete_item_by_id(&state.store, &id).map_err(ApiError::internal)? {
        return Err(ApiError::not_found("item not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateStatusRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let allowed = ["active", "archived", "favorite"];
    if !allowed.contains(&request.status.as_str()) {
        return Err(ApiError::bad_request(
            "status must be active, archived, or favorite",
        ));
    }
    update_item_status(&state.store, &id, &request.status).map_err(ApiError::internal)?;
    get_item(State(state), Path(id)).await
}

async fn update_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateItemRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let request = normalize_update_request(request)?;
    let item = update_item_metadata(&state.store, &id, request).map_err(|error| {
        if error.to_string().contains("item not found") {
            ApiError::not_found("item not found")
        } else {
            ApiError::internal(error)
        }
    })?;
    Ok(Json(item))
}

async fn stats(
    State(state): State<AppState>,
    Query(query): Query<ListItemsQuery>,
) -> Result<Json<CollectionStats>, ApiError> {
    let mut items = all_items(&state.store).map_err(ApiError::internal)?;
    items.retain(|item| matches_filter(item, &query));
    let categories = count_by(items.iter().map(|item| item.category.clone()));
    let platforms = count_by(items.iter().map(|item| item.platform.clone()));
    let keywords = count_by(items.iter().flat_map(|item| item.keywords.clone()));

    Ok(Json(CollectionStats {
        total: items.len(),
        categories,
        platforms,
        keywords,
    }))
}

fn normalize_update_request(mut request: UpdateItemRequest) -> Result<UpdateItemRequest, ApiError> {
    if let Some(category) = request.category {
        let category = category.trim().to_string();
        if category.is_empty() {
            return Err(ApiError::bad_request("category cannot be empty"));
        }
        request.category = Some(category);
    }

    if let Some(keywords) = request.keywords {
        let mut normalized = Vec::new();
        for keyword in keywords {
            let keyword = keyword.trim().to_string();
            if !keyword.is_empty() && !normalized.contains(&keyword) {
                normalized.push(keyword);
            }
            if normalized.len() >= 20 {
                break;
            }
        }
        request.keywords = Some(normalized);
    }

    if let Some(notes) = request.notes {
        request.notes = Some(notes.trim().to_string());
    }

    Ok(request)
}

async fn digest(State(state): State<AppState>) -> Result<Json<CollectionDigest>, ApiError> {
    let items = all_items(&state.store).map_err(ApiError::internal)?;
    Ok(Json(collection_digest(&items)))
}

async fn export_markdown(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let items = all_items(&state.store).map_err(ApiError::internal)?;
    let markdown = build_markdown_export(&items);
    Ok((
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"share-with-me-export.md\"",
            ),
        ],
        markdown,
    ))
}

async fn export_json(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let items = all_items(&state.store).map_err(ApiError::internal)?;
    let body = serde_json::to_string_pretty(&build_json_export(items))
        .map_err(|error| ApiError::internal(error.into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"share-with-me-export.json\"",
            ),
        ],
        body,
    ))
}

async fn import_json(
    State(state): State<AppState>,
    Json(request): Json<JsonImportRequest>,
) -> Result<Json<JsonImportResponse>, ApiError> {
    let request = validate_json_import(request)?;
    let imported_count = request.items.len();
    let (created_count, merged_count) =
        import_items(&state.store, request.items).map_err(ApiError::internal)?;
    let total_count = all_items(&state.store).map_err(ApiError::internal)?.len();
    Ok(Json(JsonImportResponse {
        imported_count,
        created_count,
        merged_count,
        total_count,
    }))
}

fn validate_json_import(request: JsonImportRequest) -> Result<JsonImportRequest, ApiError> {
    if request.schema_version != 1 {
        return Err(ApiError::bad_request(
            "unsupported JSON backup schema_version",
        ));
    }
    if request.items.is_empty() {
        return Err(ApiError::bad_request("JSON backup has no items"));
    }
    for item in &request.items {
        if item.id.trim().is_empty() {
            return Err(ApiError::bad_request("item id cannot be empty"));
        }
        if item.source_url.trim().is_empty() || item.final_url.trim().is_empty() {
            return Err(ApiError::bad_request("item URLs cannot be empty"));
        }
        if item.title.trim().is_empty() {
            return Err(ApiError::bad_request("item title cannot be empty"));
        }
        if !["active", "archived", "favorite"].contains(&item.status.as_str()) {
            return Err(ApiError::bad_request(
                "item status must be active, archived, or favorite",
            ));
        }
    }
    Ok(request)
}

pub(crate) fn matches_filter(item: &LinkItem, query: &ListItemsQuery) -> bool {
    if let Some(status) = &query.status
        && !status.is_empty()
        && item.status != *status
    {
        return false;
    }
    if let Some(category) = &query.category
        && !category.is_empty()
        && item.category != *category
    {
        return false;
    }
    if let Some(platform) = &query.platform
        && !platform.is_empty()
        && item.platform != *platform
    {
        return false;
    }
    if let Some(search) = &query.query {
        let needle = search.trim().to_lowercase();
        if !needle.is_empty() {
            let haystack = format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                item.title,
                item.description,
                item.summary,
                item.category,
                item.platform,
                item.source_url,
                item.final_url,
                item.source_app.as_deref().unwrap_or_default(),
                item.author.as_deref().unwrap_or_default(),
                item.content_type,
                item.notes,
                item.keywords.join(" "),
                item.entities.join(" "),
                item.content_text
            )
            .to_lowercase();
            return haystack.contains(&needle);
        }
    }
    true
}

pub(crate) fn build_markdown_export(items: &[LinkItem]) -> String {
    let digest = collection_digest(items);
    let mut output = String::new();
    output.push_str("# ShareWithMe Export\n\n");
    output.push_str(&format!(
        "- Generated: {}\n- Total active items: {}\n- Total stored items: {}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        digest.total,
        items.len()
    ));
    output.push_str("## Collection Summary\n\n");
    output.push_str(&format!("{}\n\n", digest.focus_summary));
    if !digest.category_summaries.is_empty() {
        output.push_str("### Categories\n\n");
        for bucket in &digest.category_summaries {
            output.push_str(&format!("- {}: {}\n", bucket.label, bucket.count));
        }
        output.push('\n');
    }
    output.push_str("## Items\n\n");
    for item in items {
        output.push_str(&format!("### {}\n\n", sanitize_markdown_line(&item.title)));
        output.push_str(&format!(
            "- Platform: {}\n- Category: {}\n- Type: {}\n- Status: {}\n- Importance: {}\n- Created: {}\n- URL: {}\n",
            sanitize_markdown_line(&item.platform),
            sanitize_markdown_line(&item.category),
            sanitize_markdown_line(&item.content_type),
            sanitize_markdown_line(&item.status),
            item.importance_score,
            item.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
            item.final_url
        ));
        if !item.keywords.is_empty() {
            output.push_str(&format!(
                "- Keywords: {}\n",
                item.keywords
                    .iter()
                    .map(|value| sanitize_markdown_line(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        output.push('\n');
        if !item.summary.trim().is_empty() {
            output.push_str(&format!("{}\n\n", sanitize_markdown_line(&item.summary)));
        }
        let original_text = if item.original_text.trim().is_empty() {
            item.content_text.trim()
        } else {
            item.original_text.trim()
        };
        if !original_text.is_empty() {
            output.push_str("#### 原文\n\n");
            output.push_str(original_text);
            output.push_str("\n\n");
        }
        if !item.notes.trim().is_empty() {
            output.push_str(&format!(
                "> Notes: {}\n\n",
                sanitize_markdown_line(&item.notes)
            ));
        }
    }
    output
}

pub(crate) fn build_json_export(items: Vec<LinkItem>) -> JsonExport {
    let digest = collection_digest(&items);
    JsonExport {
        schema_version: 1,
        generated_at: Utc::now(),
        item_count: items.len(),
        digest,
        items,
    }
}

fn sanitize_markdown_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn sort_items(items: &mut [LinkItem], sort: Option<&str>) {
    match sort.unwrap_or("newest") {
        "oldest" => items.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.title.cmp(&b.title))
        }),
        "importance" => items.sort_by(|a, b| {
            b.importance_score
                .cmp(&a.importance_score)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| a.title.cmp(&b.title))
        }),
        _ => items.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.title.cmp(&b.title))
        }),
    }
}
#[cfg(test)]
mod api_tests;
