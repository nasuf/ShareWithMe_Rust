//! Saving new links: extract the snapshot, store it, analyse it in the background.

use axum::{Json, extract::State};
use chrono::Utc;

use crate::{
    analysis::{analyze_with_deepseek_or_fallback, build_item, check_deepseek, heuristic_analysis},
    error::ApiError,
    extraction::extract_metadata,
    links::{resolve_request_url, resolve_request_urls},
    models::{AnalyzeLinkRequest, AnalyzerCheckResponse, LinkAnalysis, LinkItem, LinkMetadata},
    state::AppState,
    store::{find_item, save_item},
};

use super::{archive::ensure_archive, media::schedule_media_cache};

pub(super) async fn check_analyzer(
    State(state): State<AppState>,
) -> Result<Json<AnalyzerCheckResponse>, ApiError> {
    Ok(Json(check_deepseek(&state).await))
}

pub(super) async fn analyze_link(
    State(state): State<AppState>,
    Json(request): Json<AnalyzeLinkRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let source_url = resolve_request_url(&request)?;
    let item = analyze_one_url(&state, &source_url, &request).await?;
    Ok(Json(item))
}

pub(super) async fn analyze_links(
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
        if let Err(error) = super::media::cache_item_media(&state, &item_id).await {
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
