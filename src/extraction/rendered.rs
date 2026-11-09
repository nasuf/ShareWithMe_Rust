//! Headless browser fallback for pages whose content is client-rendered or
//! hidden behind a JavaScript challenge.

use crate::{
    content::OriginalContent,
    models::LinkMetadata,
    platforms::{Platform, platform_for_url},
    render::RenderedPage,
    text::{clean_original_text, clean_text, is_url_only_text},
};

use super::{document::document_from_fragment, is_blocked_without_content};

/// Below this many characters an article is considered not yet captured.
const SHORT_ARTICLE_CHARS: usize = 120;

pub(crate) fn should_try(metadata: &LinkMetadata) -> bool {
    // A login wall served to a mobile UA is served to the browser too; rendering
    // it would only replace the share text with the login shell.
    if matches!(
        metadata.extraction.method.as_str(),
        "login-wall" | "link-expired"
    ) {
        return false;
    }
    if is_blocked_without_content(metadata) {
        return true;
    }
    let platform = Platform::of(metadata);
    if platform == Platform::Xiaohongshu
        && !metadata
            .content_text
            .contains("小红书返回了笔记不可浏览页面")
        && !metadata.content_text.trim().is_empty()
        && !is_url_only_text(&metadata.content_text)
    {
        return false;
    }
    // A WeChat article that already carries a definitive "deleted/unavailable"
    // reason must not be re-fetched by the browser (that would overwrite it).
    if platform == Platform::WeChat && metadata.extraction.last_error.is_some() {
        return false;
    }
    platform.needs_browser_render() && metadata.content_text.chars().count() < SHORT_ARTICLE_CHARS
}

pub(crate) fn merge(metadata: &mut LinkMetadata, rendered: RenderedPage) {
    if let Some(final_url) = rendered
        .final_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata.final_url = final_url.to_string();
        metadata.platform = platform_for_url(final_url);
    }

    if let Some(title) = clean_optional(rendered.title) {
        metadata.title = title;
    }
    if let Some(description) = clean_optional(rendered.description) {
        metadata.description = description;
    }
    if let Some(author) = clean_optional(rendered.author) {
        metadata.author = Some(author);
    }
    if let Some(image_url) = clean_optional(rendered.image_url) {
        metadata.image_url = Some(image_url);
    }
    if let Some(content_text) = rendered
        .content_text
        .map(|value| clean_original_text(&value))
        .filter(|value| !value.is_empty())
        && clean_text(&content_text).chars().count() >= metadata.content_text.chars().count()
    {
        metadata.content_text = clean_text(&content_text);
        metadata.original_text = content_text.clone();
        metadata.original_content = OriginalContent::from_plain_text(&content_text);
        if let Some(html) = rendered
            .content_html
            .as_deref()
            .filter(|html| !html.is_empty())
        {
            apply_rendered_document(metadata, html, &content_text);
        }
    }
    metadata.extraction.method = rendered
        .extractor
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "browser-render".to_string());
}

// Prefer structured blocks (headings, inline images, captions) parsed from the
// rendered article HTML, but only when they cover most of the rendered text, so
// a wrong container never silently drops part of the article.
fn apply_rendered_document(metadata: &mut LinkMetadata, html: &str, rendered_text: &str) {
    let (content, media) = document_from_fragment(html, &metadata.final_url);
    let rendered_chars = compact_len(rendered_text);
    let block_chars = compact_len(&content.plain_text);
    if content.blocks.is_empty() || block_chars * 10 < rendered_chars * 7 {
        return;
    }
    metadata.original_content = content;
    for asset in media {
        metadata.add_media(asset);
    }
}

fn compact_len(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| clean_text(&value))
        .filter(|value| !value.is_empty())
}
