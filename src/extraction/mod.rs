//! Turn a shared link into a durable content snapshot ([`LinkMetadata`]).
//!
//! The pipeline is platform-agnostic; source-specific behaviour is reached
//! through the [`Platform`] hooks:
//!
//! 1. Seed the snapshot from the share-sheet text (always kept as fallback).
//! 2. Fetch the page ([`fetch`]) and parse it: platform page parser first,
//!    generic Open Graph/HTML otherwise ([`html`]), then structured data.
//! 3. Platform public API ([`Platform::fetch_api`]).
//! 4. Headless browser render for pages that need it ([`rendered`]).
//! 5. Remote enrichment: oEmbed, platform APIs, yt-dlp ([`Platform::enrich_remote`]).
//! 6. Replace login/verification shells with the share text ([`shell`]).
//! 7. Normalise, fingerprint and grade completeness ([`finalize`]).

mod asr;
mod document;
mod embedded_json;
mod fetch;
mod finalize;
mod html;
pub(crate) mod oembed;
mod platform_state;
mod rendered;
mod shell;
mod snapshot;
pub(crate) mod structured_data;
pub(crate) mod video;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use reqwest::Client;

use crate::{
    content::{ExtractionState, OriginalContent},
    models::LinkMetadata,
    platforms::{Platform, platform_for_url, xiaohongshu},
    render::render_extract,
    text::{clean_original_text, clean_text, first_meaningful_line, plain_text_preview},
};

pub(crate) use document::document_from_root;
pub(crate) use embedded_json::{
    json_ld_values, json_value_to_clean_string, json_value_to_image_url,
    json_value_to_original_string, json_values_after_markers, json_values_from_script_id,
    jsonp_value, platform_metadata_from_values,
};
pub(crate) use html::meta_content;
pub(crate) use platform_state::{PlatformMetadata, merge_platform_metadata};

/// Placeholder title used until a real one is found.
pub(crate) const UNTITLED: &str = "未命名链接";

pub(crate) async fn extract_metadata(
    http: &Client,
    source_url: &str,
    shared_text: Option<&str>,
) -> LinkMetadata {
    extract_metadata_with_video_transcription(http, source_url, shared_text, true).await
}

pub(crate) async fn extract_metadata_with_video_transcription(
    http: &Client,
    source_url: &str,
    shared_text: Option<&str>,
    allow_video_transcription: bool,
) -> LinkMetadata {
    let fetched_at = Utc::now();
    let mut metadata =
        seed_from_share_text(source_url, shared_text.unwrap_or_default(), fetched_at);

    fetch_and_parse_page(http, &mut metadata).await;
    finalize::update_source_id(&mut metadata);

    let platform = Platform::of(&metadata);
    if let Some(api_metadata) = platform.fetch_api(&metadata, http).await {
        merge_platform_metadata(&mut metadata, api_metadata);
        metadata.extraction.method = "platform-api".to_string();
    }
    if rendered::should_try(&metadata)
        && let Some(page) = render_extract(source_url).await
    {
        rendered::merge(&mut metadata, page);
    }
    Platform::of(&metadata)
        .enrich_remote(&mut metadata, http, allow_video_transcription)
        .await;
    shell::replace_platform_shell(&mut metadata);

    metadata.title = clean_text(&metadata.title);
    metadata.description = clean_text(&metadata.description);
    metadata.content_text = clean_text(&metadata.content_text);
    metadata.original_text = clean_original_text(&metadata.original_text);
    if metadata.original_text.is_empty() && !metadata.content_text.is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
    finalize::finalize(&mut metadata, fetched_at);
    metadata
}

/// The starting snapshot: everything the share sheet gave us.
fn seed_from_share_text(
    source_url: &str,
    shared_text: &str,
    fetched_at: DateTime<Utc>,
) -> LinkMetadata {
    let shared_text = shared_text.trim().to_string();
    let platform = platform_for_url(source_url);
    let adapter = Platform::from_label(&platform).profile().adapter;
    LinkMetadata {
        source_url: source_url.to_string(),
        final_url: source_url.to_string(),
        title: first_meaningful_line(&shared_text).unwrap_or_else(|| UNTITLED.to_string()),
        description: shared_text.clone(),
        content_text: shared_text.clone(),
        original_text: shared_text.clone(),
        original_content: OriginalContent::from_plain_text(shared_text.clone()),
        shared_text,
        platform,
        extraction: ExtractionState {
            status: "partial".to_string(),
            adapter: adapter.to_string(),
            method: "shared_text".to_string(),
            completeness: 20,
            fetched_at: Some(fetched_at),
            ..ExtractionState::default()
        },
        ..LinkMetadata::default()
    }
}

async fn fetch_and_parse_page(http: &Client, metadata: &mut LinkMetadata) {
    let response = match fetch::fetch_page(http, &metadata.source_url).await {
        Ok(response) => response,
        Err(error) => {
            metadata.extraction.last_error = Some(format!("请求页面失败: {error}"));
            metadata.append_note(format!("请求页面失败: {error}"));
            return;
        }
    };
    metadata.final_url = response.url().to_string();

    // Xiaohongshu sometimes still bounces to its login wall; the note URL (with
    // its xsec_token) is kept in `redirectPath`. Fetch it directly and, if that
    // fails too, at least keep the note URL so the item gets the right identity
    // instead of the login page's.
    let mut stuck_on_login = false;
    let response = match xiaohongshu::login_redirect_target(&metadata.final_url) {
        Some(target) => {
            metadata.final_url = target.clone();
            match fetch::fetch_page(http, &target).await {
                Ok(retry) if xiaohongshu::login_redirect_target(retry.url().as_str()).is_none() => {
                    metadata.final_url = retry.url().to_string();
                    retry
                }
                _ => {
                    stuck_on_login = true;
                    response
                }
            }
        }
        None => response,
    };
    metadata.platform = platform_for_url(&metadata.final_url);

    let status = response.status();
    match response.text().await {
        // The login wall is a JS shell with no note data; parsing it would
        // save "Log in - rednote" as the item. Keep the share text.
        Ok(_) if stuck_on_login => {
            metadata.extraction.method = "login-wall".to_string();
            metadata.extraction.last_error = Some(xiaohongshu::LOGIN_WALL_REASON.to_string());
        }
        // Expired share token or hidden note: /404/sec_... page.
        Ok(_) if xiaohongshu::is_dead_end(&metadata.final_url) => {
            metadata.extraction.method = "link-expired".to_string();
            metadata.extraction.last_error = Some(xiaohongshu::DEAD_END_REASON.to_string());
        }
        Ok(html) if status.is_success() => parse_page(metadata, &html),
        Ok(body) => {
            metadata.append_note(format!("页面返回 HTTP {status}; 已保存可见分享文本。"));
            if metadata.content_text.is_empty() {
                metadata.content_text = plain_text_preview(&body);
            }
        }
        Err(error) => metadata.append_note(format!("读取页面失败: {error}")),
    }
}

/// Parse a fetched page: platform parser, else generic HTML; then structured
/// data for every platform.
pub(crate) fn parse_page(metadata: &mut LinkMetadata, html: &str) {
    let platform = Platform::of(metadata);
    match platform.parse_page(metadata, html) {
        Some(method) => metadata.extraction.method = method.to_string(),
        None => html::merge_generic_page(metadata, html),
    }
    platform.enrich_from_html(metadata, html);
}

/// True when the snapshot is a verification page with no usable content.
pub(crate) fn is_blocked_without_content(metadata: &LinkMetadata) -> bool {
    metadata.extraction.method == "blocked-page"
        || metadata.title.contains("网页验证拦截")
        || metadata.content_text.contains("无法从公开网页直接抓取正文")
        || metadata.content_text.contains("无法直接抓取正文")
}
