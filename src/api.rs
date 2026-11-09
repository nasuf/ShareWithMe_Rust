use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::Utc;
use std::sync::Arc;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
};

use crate::{
    analysis::{analyze_with_deepseek_or_fallback, build_item, check_deepseek, heuristic_analysis},
    config::AppConfig,
    content::{media_identity, snapshot_fingerprint},
    digest::{collection_digest, count_by},
    error::ApiError,
    extraction::{extract_metadata, extract_metadata_with_video_transcription},
    links::{is_safe_remote_url, resolve_request_url, resolve_request_urls},
    models::{
        AnalyzeLinkRequest, AnalyzerCheckResponse, CollectionDigest, CollectionStats,
        HealthResponse, JsonImportRequest, JsonImportResponse, LinkAnalysis, LinkItem,
        LinkMetadata, ListItemsQuery, UpdateItemRequest, UpdateStatusRequest,
    },
    state::AppState,
    store::{
        all_items, delete_item_by_id, find_item, import_items, save_item, update_item_metadata,
        update_item_status,
    },
};

mod collection;
mod media;

pub(crate) use collection::{build_json_export, build_markdown_export, matches_filter, sort_items};
pub(crate) use media::repair_wechat_placeholders;
use media::{
    ensure_archive, is_local_media_url, media_file, recapture_archive, refresh_cover,
    schedule_media_cache,
};

#[derive(Default, serde::Deserialize)]
struct RefreshQuery {
    force: Option<bool>,
}

pub(crate) fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/items", get(list_items))
        .route(
            "/api/items/{id}",
            get(get_item).patch(update_item).delete(delete_item),
        )
        .route("/api/items/{id}/status", patch(update_status))
        .route("/api/items/{id}/reanalyze", post(reanalyze_item))
        .route("/api/items/{id}/refresh", post(refresh_item_content))
        .route("/api/items/{id}/cover/refresh", post(refresh_cover))
        .route("/api/items/{id}/archive", post(recapture_archive))
        .route("/api/links/analyze", post(analyze_link))
        .route("/api/links/analyze-many", post(analyze_links))
        .route("/api/stats", get(stats))
        .route("/api/digest", get(digest))
        .route("/api/export/markdown", get(export_markdown))
        .route("/api/export/json", get(export_json))
        .route("/api/import/json", post(import_json))
        .route("/api/analyzer/check", post(check_analyzer))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.config),
            require_api_token,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/media/{file}", get(media_file))
        .merge(protected)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        // JSON shrinks ~6x; the default predicate leaves JPEG media alone.
        .layer(CompressionLayer::new())
        .with_state(state)
}

async fn require_api_token(
    State(config): State<Arc<AppConfig>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(expected) = config.api_token.as_deref() else {
        return Ok(next.run(request).await);
    };
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    if !supplied.is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        analyzer_provider: "deepseek",
        deepseek_configured: state.config.deepseek_api_key.is_some(),
        deepseek_model: state.config.deepseek_model.clone(),
        storage_path: state.config.storage_label(),
        storage_backend: state.store.backend_name().to_string(),
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

    // LLM analysis dominates the request time (~6-7s vs ~1s for extraction), so
    // when DeepSeek is configured we return a fast local snapshot immediately and
    // run the real analysis in the background, clearing `analysis_pending` when done.
    let defer_ai = state.config.deepseek_api_key.is_some();
    let analysis = if defer_ai {
        heuristic_analysis(&metadata, "AI 分析生成中…")
    } else {
        analyze_with_deepseek_or_fallback(state, &metadata).await
    };
    let mut item = build_item(metadata.clone(), analysis, request.source_app.clone());
    item.analysis_pending = defer_ai;
    // Use the persisted item: when the final_url already exists the store reuses
    // the existing id, and the background task must target that real id.
    let stored = save_item(&state.store, item)
        .await
        .map_err(ApiError::internal)?;
    if defer_ai {
        spawn_deferred_analysis(state.clone(), stored.id.clone(), metadata);
    } else {
        schedule_media_cache(state.clone(), stored.id.clone());
    }
    Ok(stored)
}

// Cache media, then run the LLM analysis, then apply it with a fresh read so the
// two background writes (media cache and AI) do not clobber each other.
fn spawn_deferred_analysis(state: AppState, item_id: String, metadata: LinkMetadata) {
    tokio::spawn(async move {
        if let Err(error) = media::cache_item_media(&state, &item_id).await {
            tracing::warn!(item_id, %error, "cache item media failed");
        }
        let analysis = analyze_with_deepseek_or_fallback(&state, &metadata).await;
        match find_item(&state.store, &item_id).await {
            Ok(Some(mut item)) => {
                apply_deferred_analysis(&mut item, analysis);
                if let Err(error) = save_item(&state.store, item).await {
                    tracing::warn!(item_id, %error, "save deferred analysis failed");
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(item_id, %error, "reload for deferred analysis failed"),
        }
        ensure_archive(&state, &item_id).await;
    });
}

fn apply_deferred_analysis(item: &mut LinkItem, analysis: LinkAnalysis) {
    item.summary = analysis.summary;
    item.sentiment = analysis.sentiment;
    item.content_type = analysis.content_type;
    item.importance_score = analysis.importance_score;
    item.notes = analysis.notes;
    // Preserve any classification the user edited before analysis finished.
    if item.user_edited_at.is_none() {
        item.category = analysis.category;
        item.keywords = analysis.keywords;
        item.entities = analysis.entities;
    }
    item.analysis_pending = false;
    item.updated_at = Utc::now();
}

async fn reanalyze_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    let current = find_item(&state.store, &id)
        .await
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
        shared_text: current.shared_text.clone(),
        original_content: current.original_content.clone(),
        media: current.media.clone(),
        source_facts: current.source_facts.clone(),
        source_id: current.source_id.clone(),
        published_at: current.published_at,
        extraction: current.extraction.clone(),
    };
    let analysis = analyze_with_deepseek_or_fallback(&state, &metadata).await;
    let fresh = build_item(metadata, analysis, current.source_app.clone());
    let updated = merge_reanalyzed_item(current, fresh);
    save_item(&state.store, updated.clone())
        .await
        .map_err(ApiError::internal)?;
    schedule_media_cache(state, updated.id.clone());
    Ok(Json(updated))
}

async fn refresh_item_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<RefreshQuery>,
) -> Result<Json<LinkItem>, ApiError> {
    let current = find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    let now = Utc::now();
    if !query.force.unwrap_or(false) && !current.extraction.is_stale_at(now) {
        // Complete items saved before refreshes stopped still carry a past
        // `refresh_after`, which makes the app ask again on every launch.
        // Push it out once so the app stops asking.
        if current.extraction.is_settled()
            && current
                .extraction
                .refresh_after
                .is_none_or(|refresh_after| refresh_after <= now)
        {
            let mut settled = current;
            settled.extraction.refresh_after = Some(crate::content::settled_refresh_after(now));
            save_item(&state.store, settled.clone())
                .await
                .map_err(ApiError::internal)?;
            return Ok(Json(settled));
        }
        return Ok(Json(current));
    }

    let shared_text =
        (!current.shared_text.trim().is_empty()).then_some(current.shared_text.as_str());
    let existing_transcript = current
        .source_facts
        .iter()
        .find(|fact| fact.key == "transcript_source")
        .cloned();
    let mut metadata = extract_metadata_with_video_transcription(
        &state.http,
        &current.source_url,
        shared_text,
        existing_transcript.is_none(),
    )
    .await;
    if existing_transcript.is_some() {
        preserve_existing_transcript(&current, &mut metadata);
    }
    let current_has_snapshot =
        !current.original_content.is_empty() || current.original_text.trim().chars().count() >= 40;
    let existing_completeness = if current.extraction.status == "legacy" && current_has_snapshot {
        70
    } else {
        current.extraction.completeness
    };
    let fresh_is_better =
        metadata.extraction.completeness >= existing_completeness || !current_has_snapshot;

    let updated = if fresh_is_better {
        let content_changed =
            metadata.extraction.content_fingerprint != current.extraction.content_fingerprint;
        let analysis = if content_changed || current.extraction.status == "legacy" {
            analyze_with_deepseek_or_fallback(&state, &metadata).await
        } else {
            analysis_from_item(&current)
        };
        merge_refreshed_item(current, build_item(metadata, analysis, None))
    } else {
        preserve_last_good_snapshot(current, &metadata)
    };

    // The refresh resolved to a source another item already holds (e.g. a
    // share link first saved as a login wall, then saved again once fixed).
    // `final_url` is unique, so saving would fail: fold this duplicate into
    // the existing item instead and drop it.
    if let Some(owner) = all_items(&state.store)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .find(|item| item.final_url == updated.final_url && item.id != updated.id)
    {
        let merged = merge_duplicate_into(owner, &updated);
        save_item(&state.store, merged.clone())
            .await
            .map_err(ApiError::internal)?;
        delete_item_by_id(&state.store, &updated.id)
            .await
            .map_err(ApiError::internal)?;
        schedule_media_cache(state, merged.id.clone());
        return Ok(Json(merged));
    }

    save_item(&state.store, updated.clone())
        .await
        .map_err(ApiError::internal)?;
    schedule_media_cache(state, updated.id.clone());
    Ok(Json(updated))
}

// Keep the existing item's content and identity, but don't lose what the user
// curated on the duplicate.
fn merge_duplicate_into(mut owner: LinkItem, duplicate: &LinkItem) -> LinkItem {
    if duplicate.user_edited_at.is_some() && owner.user_edited_at.is_none() {
        owner.category = duplicate.category.clone();
        owner.keywords = duplicate.keywords.clone();
        owner.notes = duplicate.notes.clone();
        owner.user_edited_at = duplicate.user_edited_at;
    }
    if owner.status == "active" && duplicate.status != "active" {
        owner.status = duplicate.status.clone();
    }
    owner.created_at = owner.created_at.min(duplicate.created_at);
    owner.updated_at = Utc::now();
    owner
}

fn preserve_existing_transcript(current: &LinkItem, metadata: &mut LinkMetadata) {
    let Some(transcript_source) = current
        .source_facts
        .iter()
        .find(|fact| fact.key == "transcript_source")
        .cloned()
    else {
        return;
    };
    if metadata
        .source_facts
        .iter()
        .any(|fact| fact.key == "transcript_source")
    {
        return;
    }

    metadata.original_text = current.original_text.clone();
    metadata.content_text = current.content_text.clone();
    metadata.original_content = current.original_content.clone();
    metadata.source_facts.push(transcript_source);
    metadata.extraction.content_fingerprint = snapshot_fingerprint(
        &metadata.title,
        metadata.author.as_deref(),
        &metadata.original_content.plain_text,
        &metadata.source_facts,
    );
    metadata.extraction.completeness = metadata
        .extraction
        .completeness
        .max(current.extraction.completeness);
}

fn analysis_from_item(item: &LinkItem) -> crate::models::LinkAnalysis {
    crate::models::LinkAnalysis {
        summary: item.summary.clone(),
        category: item.category.clone(),
        keywords: item.keywords.clone(),
        entities: item.entities.clone(),
        sentiment: item.sentiment.clone(),
        content_type: item.content_type.clone(),
        importance_score: item.importance_score,
        notes: item.notes.clone(),
    }
}

// Re-analysis rebuilds the item from a fresh AI pass, so identity and the
// classification the user curated by hand have to be carried over explicitly.
fn merge_reanalyzed_item(current: LinkItem, mut fresh: LinkItem) -> LinkItem {
    fresh.id = current.id;
    fresh.archive = current.archive;
    fresh.created_at = current.created_at;
    fresh.status = current.status;
    fresh.user_edited_at = current.user_edited_at;
    if current.user_edited_at.is_some() {
        fresh.category = current.category;
        fresh.keywords = current.keywords;
        fresh.notes = current.notes;
    }
    fresh
}

fn merge_refreshed_item(current: LinkItem, mut fresh: LinkItem) -> LinkItem {
    for asset in &mut fresh.media {
        let identity = media_identity(&asset.url);
        if asset.cached_url.is_none()
            && let Some(cached_url) = current
                .media
                .iter()
                .find(|current_asset| media_identity(&current_asset.url) == identity)
                .and_then(|current_asset| current_asset.cached_url.clone())
        {
            asset.cached_url = Some(cached_url);
        }
    }
    fresh.id = current.id;
    fresh.archive = current.archive;
    fresh.source_app = current.source_app;
    fresh.created_at = current.created_at;
    fresh.status = current.status;
    fresh.user_edited_at = current.user_edited_at;
    if current.user_edited_at.is_some() {
        fresh.category = current.category;
        fresh.keywords = current.keywords;
        fresh.notes = current.notes;
    }
    if current.image_url.as_deref().is_some_and(is_local_media_url) {
        fresh.remote_image_url = fresh.image_url.clone().or(current.remote_image_url);
        fresh.image_url = current.image_url;
        fresh.cover_cached_at = current.cover_cached_at;
        fresh.cover_checked_at = current.cover_checked_at;
    }
    fresh.updated_at = Utc::now();
    fresh
}

fn preserve_last_good_snapshot(mut current: LinkItem, attempted: &LinkMetadata) -> LinkItem {
    if current.original_content.is_empty() {
        current.original_content = crate::content::OriginalContent::from_plain_text(
            if current.original_text.trim().is_empty() {
                &current.content_text
            } else {
                &current.original_text
            },
        );
    }
    current.extraction.status = "stale".to_string();
    current.extraction.fetched_at = attempted.extraction.fetched_at;
    // A kept complete snapshot stays settled; anything else retries on schedule.
    current.extraction.refresh_after = if current.extraction.is_settled() {
        attempted
            .extraction
            .fetched_at
            .map(crate::content::settled_refresh_after)
    } else {
        attempted.extraction.refresh_after
    };
    current.extraction.last_error = attempted.extraction.last_error.clone().or_else(|| {
        Some(format!(
            "{} 本次刷新只获得不完整内容，已保留上一次完整快照。",
            attempted.platform
        ))
    });
    current.updated_at = Utc::now();
    current
}

async fn list_items(
    State(state): State<AppState>,
    Query(query): Query<ListItemsQuery>,
) -> Result<Json<Vec<LinkItem>>, ApiError> {
    let mut items = all_items(&state.store).await.map_err(ApiError::internal)?;
    items.retain(|item| matches_filter(item, &query));
    sort_items(&mut items, query.sort.as_deref());
    if query.fields.as_deref() == Some("summary") {
        items = items.into_iter().map(summary_item).collect();
    }
    Ok(Json(items))
}

// The library list only needs card fields. Full article text (stored in up to
// four forms) is ~80% of the payload, so `fields=summary` drops it and clients
// fetch GET /api/items/{id} when an item is opened. Opt-in, so older app builds
// that read content from the list keep working.
fn summary_item(mut item: LinkItem) -> LinkItem {
    item.content_text.clear();
    item.original_text.clear();
    item.original_content = Default::default();
    item.content_omitted = true;
    item
}

async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    let item = find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    Ok(Json(item))
}

async fn delete_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !delete_item_by_id(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
    {
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
    update_item_status(&state.store, &id, &request.status)
        .await
        .map_err(ApiError::internal)?;
    get_item(State(state), Path(id)).await
}

async fn update_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateItemRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let request = normalize_update_request(request)?;
    let item = update_item_metadata(&state.store, &id, request)
        .await
        .map_err(|error| {
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
    let mut items = all_items(&state.store).await.map_err(ApiError::internal)?;
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
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
    Ok(Json(collection_digest(&items)))
}

async fn export_markdown(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
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
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
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
    let (created_count, merged_count) = import_items(&state.store, request.items)
        .await
        .map_err(ApiError::internal)?;
    let total_count = all_items(&state.store)
        .await
        .map_err(ApiError::internal)?
        .len();
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
        for raw_url in [&item.source_url, &item.final_url] {
            let url = url::Url::parse(raw_url)
                .map_err(|_| ApiError::bad_request("item URLs must be valid"))?;
            if !is_safe_remote_url(&url) {
                return Err(ApiError::bad_request(
                    "item URLs must point to public network hosts",
                ));
            }
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

#[cfg(test)]
mod api_tests;
