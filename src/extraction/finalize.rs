//! Last pipeline step: canonical identity, fingerprint and completeness grade.

use chrono::{DateTime, Utc};

use crate::{
    content::{MediaAsset, OriginalContent, settled_refresh_after, snapshot_fingerprint},
    models::LinkMetadata,
    platforms::{Platform, SourceIdentity, source_identity},
};

use super::{UNTITLED, is_blocked_without_content};

/// Extraction methods that read the platform's own data rather than a generic
/// page, so even a short result can be a complete snapshot.
const SOURCE_SPECIFIC_METHODS: &[&str] = &[
    "embedded-state",
    "shared_text",
    "platform-api",
    "official-api",
    "public-preview",
    "oembed",
    "yt-dlp",
    "xhs-initial-state",
];

/// Content kinds whose snapshot is naturally short (a post, a product card).
const SHORT_CONTENT_KINDS: &[&str] = &["post", "video", "product", "place", "book", "review"];

/// Body length from which any snapshot counts as complete.
const COMPLETE_TEXT_CHARS: usize = 120;

pub(super) fn update_source_id(metadata: &mut LinkMetadata) {
    metadata.source_id = preferred_source_identity(metadata).source_id;
}

pub(super) fn finalize(metadata: &mut LinkMetadata, fetched_at: DateTime<Utc>) {
    let identity = preferred_source_identity(metadata);
    metadata.final_url = identity.canonical_url;
    metadata.source_id = identity.source_id;

    if metadata.original_content.is_empty() {
        metadata.original_content = OriginalContent::from_plain_text(&metadata.original_text);
    }
    if metadata.original_text.trim().is_empty() {
        metadata.original_text = metadata.original_content.plain_text.clone();
    }
    if let Some(image_url) = metadata.image_url.clone()
        && !metadata.media.iter().any(|asset| asset.url == image_url)
    {
        metadata.media.insert(
            0,
            MediaAsset {
                kind: "image".to_string(),
                url: image_url,
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            },
        );
    }

    let platform = Platform::of(metadata);
    let profile = platform.profile();
    metadata.extraction.adapter = profile.adapter.to_string();
    metadata.extraction.fetched_at = Some(fetched_at);
    metadata.extraction.refresh_after = Some(profile.refresh_after(fetched_at));
    record_missing_content_hint(metadata, platform);
    metadata.extraction.content_fingerprint = snapshot_fingerprint(
        &metadata.title,
        metadata.author.as_deref(),
        metadata.original_content.plain_text.trim(),
        &metadata.source_facts,
    );
    grade_completeness(metadata, profile.content_kind, fetched_at);
    metadata.add_fact_if_absent("content_kind", "内容类型", profile.content_kind);
}

/// Set `extraction.status` and `completeness`:
/// * blocked  — a verification page with no content (40 with long share text, else 15)
/// * complete — enough text or structure (80, +5 author, +5 cover); never auto-refreshed
/// * partial  — anything else (50 with some text, else 20)
fn grade_completeness(metadata: &mut LinkMetadata, content_kind: &str, fetched_at: DateTime<Utc>) {
    let blocked = is_blocked_without_content(metadata);
    let text = metadata.original_content.plain_text.trim();
    let text_chars = text.chars().count();
    let short_snapshot_complete = SOURCE_SPECIFIC_METHODS
        .contains(&metadata.extraction.method.as_str())
        && SHORT_CONTENT_KINDS.contains(&content_kind)
        && !text.is_empty()
        && metadata.title != UNTITLED
        && (metadata.extraction.method == "shared_text"
            || metadata.author.is_some()
            || metadata.image_url.is_some()
            || metadata.source_facts.len() >= 2);

    let has_blocks = metadata.original_content.blocks.len() >= 2;
    let bonus =
        u8::from(metadata.author.is_some()) * 5 + u8::from(metadata.image_url.is_some()) * 5;
    let long_share_text = metadata.shared_text.chars().count() >= 80;

    let extraction = &mut metadata.extraction;
    if blocked {
        extraction.status = "blocked".to_string();
        extraction.completeness = if long_share_text { 40 } else { 15 };
    } else if text_chars >= COMPLETE_TEXT_CHARS || has_blocks || short_snapshot_complete {
        extraction.status = "complete".to_string();
        extraction.completeness = 80 + bonus;
        if extraction.method != "blocked-page" {
            extraction.last_error = None;
        }
        extraction.refresh_after = Some(settled_refresh_after(fetched_at));
    } else {
        extraction.status = "partial".to_string();
        extraction.completeness = if text_chars == 0 { 20 } else { 50 };
    }
}

fn record_missing_content_hint(metadata: &mut LinkMetadata, platform: Platform) {
    if !metadata.original_content.plain_text.trim().is_empty() {
        return;
    }
    let Some(hint) = platform.missing_content_hint() else {
        return;
    };
    if metadata.extraction.last_error.is_none() {
        metadata.extraction.last_error = Some(hint.to_string());
    }
    if metadata.extraction.method == "html" {
        metadata.extraction.method = "metadata-only".to_string();
    }
    metadata.add_fact_if_absent("completion_requirement", "补全方式", hint);
}

/// The final URL's identity, or the source URL's when only it carries an ID.
fn preferred_source_identity(metadata: &LinkMetadata) -> SourceIdentity {
    let final_identity = source_identity(&metadata.final_url, &metadata.platform);
    if final_identity.source_id.is_some() {
        return final_identity;
    }
    let source_identity = source_identity(&metadata.source_url, &metadata.platform);
    if source_identity.source_id.is_some() {
        source_identity
    } else {
        final_identity
    }
}
