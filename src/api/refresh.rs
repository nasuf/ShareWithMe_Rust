//! Re-fetching and re-analysing saved items without losing what was saved.
//!
//! A refresh never replaces a more complete snapshot with a worse one, keeps
//! user edits and transcripts, and merges items that turn out to be the same
//! source.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;

use crate::{
    analysis::{analyze_with_deepseek_or_fallback, build_item},
    content::{media_identity, snapshot_fingerprint},
    error::ApiError,
    extraction::extract_metadata_with_video_transcription,
    models::{LinkItem, LinkMetadata},
    state::AppState,
    store::{all_items, delete_item_by_id, find_item, save_item},
};

use super::{
    RefreshQuery,
    media::{is_local_media_url, schedule_media_cache},
};

pub(super) async fn reanalyze_item(
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

pub(super) async fn refresh_item_content(
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
pub(super) fn merge_duplicate_into(mut owner: LinkItem, duplicate: &LinkItem) -> LinkItem {
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

pub(super) fn preserve_existing_transcript(current: &LinkItem, metadata: &mut LinkMetadata) {
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
pub(super) fn merge_reanalyzed_item(current: LinkItem, mut fresh: LinkItem) -> LinkItem {
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

pub(super) fn merge_refreshed_item(current: LinkItem, mut fresh: LinkItem) -> LinkItem {
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
