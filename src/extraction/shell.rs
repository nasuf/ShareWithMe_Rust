//! Login, visitor and security pages that slipped through as "content".
//!
//! When a platform recognises its own shell ([`Platform::shell_reason`]), the
//! shell text is replaced by the share-sheet text (or dropped) and the reason
//! is recorded, so a login page is never saved as the item.

use crate::{
    content::OriginalContent,
    models::LinkMetadata,
    platforms::Platform,
    text::{clean_original_text, clean_text, is_url_only_text},
};

use super::UNTITLED;

pub(super) fn replace_platform_shell(metadata: &mut LinkMetadata) {
    // Content from an API or yt-dlp never comes from the page shell.
    if matches!(
        metadata.extraction.method.as_str(),
        "platform-api" | "yt-dlp"
    ) {
        return;
    }
    let platform = Platform::of(metadata);
    let Some(reason) = platform.shell_reason(metadata) else {
        return;
    };
    if platform == Platform::Douban && metadata.title.trim() == "豆瓣" {
        metadata.title = "豆瓣条目".to_string();
    }

    let shared_text = metadata.shared_text.trim();
    let fallback = if !shared_text.is_empty() && !is_url_only_text(shared_text) {
        Some(shared_text.to_string())
    } else {
        douyin_title_fallback(platform, &metadata.title)
    };
    if let Some(fallback) = fallback.filter(|value| !value.is_empty()) {
        metadata.content_text = clean_text(&fallback);
        metadata.original_text = clean_original_text(&fallback);
        metadata.original_content =
            OriginalContent::from_plain_text(metadata.original_text.clone());
        metadata.extraction.method = "metadata-only".to_string();
    } else {
        metadata.content_text.clear();
        metadata.original_text.clear();
        metadata.original_content = OriginalContent::default();
        metadata.extraction.method = "blocked-page".to_string();
    }
    metadata.extraction.last_error = Some(reason.to_string());
}

/// Douyin's login shell still carries the video title ("<title> - 抖音").
fn douyin_title_fallback(platform: Platform, title: &str) -> Option<String> {
    (platform == Platform::Douyin && title != UNTITLED && !title.contains("网页验证拦截"))
        .then(|| title.trim_end_matches(" - 抖音").trim().to_string())
}
