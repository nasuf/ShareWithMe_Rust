//! Generic page parsing: verification walls, Open Graph and the article body.

use scraper::{Html, Selector};

use crate::{
    models::LinkMetadata,
    platforms::Platform,
    text::{clean_text, first_non_empty, is_url_only_text},
};

use super::{UNTITLED, document::document_from_html};

/// Parse a page that no platform parser recognised.
pub(super) fn merge_generic_page(metadata: &mut LinkMetadata, html: &str) {
    if let Some(reason) = verification_wall_reason(html, Platform::of(metadata)) {
        apply_verification_wall(metadata, reason);
        return;
    }

    let document = Html::parse_document(html);
    metadata.title = first_non_empty([
        meta_content(&document, "meta[property='og:title']"),
        meta_content(&document, "meta[name='twitter:title']"),
        select_text(&document, "title"),
        select_text(&document, "h1"),
        Some(metadata.title.clone()),
    ])
    .unwrap_or_else(|| UNTITLED.to_string());
    metadata.description = first_non_empty([
        meta_content(&document, "meta[property='og:description']"),
        meta_content(&document, "meta[name='description']"),
        meta_content(&document, "meta[name='twitter:description']"),
        Some(metadata.description.clone()),
    ])
    .unwrap_or_default();
    metadata.author = first_non_empty([
        meta_content(&document, "meta[name='author']"),
        meta_content(&document, "meta[property='article:author']"),
    ]);
    metadata.image_url = first_non_empty([
        meta_content(&document, "meta[property='og:image']"),
        meta_content(&document, "meta[name='twitter:image']"),
    ]);

    let (original_content, media) = document_from_html(&document, &metadata.final_url);
    // Keep the share text unless the page body is clearly the fuller source.
    let candidate_chars = original_content.plain_text.chars().count();
    let shared_chars = metadata.shared_text.chars().count();
    let shared_is_meaningful =
        !metadata.shared_text.trim().is_empty() && !is_url_only_text(&metadata.shared_text);
    if candidate_chars > 0
        && (!shared_is_meaningful || candidate_chars >= shared_chars.saturating_mul(2).max(200))
    {
        metadata.content_text = clean_text(&original_content.plain_text);
        metadata.original_text = original_content.plain_text.clone();
        metadata.original_content = original_content;
        metadata.extraction.method = "html".to_string();
    }
    for asset in media {
        metadata.add_media(asset);
    }
}

fn apply_verification_wall(metadata: &mut LinkMetadata, reason: String) {
    if metadata.title.trim().is_empty() || metadata.title == UNTITLED {
        metadata.title = format!("{}链接（网页验证拦截）", metadata.platform);
    }
    if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
        metadata.description = reason.clone();
    } else {
        metadata.append_note(reason.clone());
    }
    if metadata.content_text.trim().is_empty() || is_url_only_text(&metadata.content_text) {
        metadata.content_text = reason.clone();
    }
    if metadata.original_text.trim().is_empty() || is_url_only_text(&metadata.original_text) {
        metadata.original_text = reason.clone();
    }
    metadata.extraction.method = "blocked-page".to_string();
    metadata.extraction.last_error = Some(reason);
}

/// Recognise anti-bot / verification pages by their markers.
fn verification_wall_reason(html: &str, platform: Platform) -> Option<String> {
    let lower = html.to_lowercase();
    let reason = if lower.contains("byted_acrawler")
        || lower.contains("__ac_signature")
        || lower.contains("__ac_nonce")
    {
        // ByteDance (Douyin / Toutiao) browser challenge.
        if platform == Platform::Douyin {
            "抖音返回了浏览器验证页面，正在尝试视频元数据和字幕兜底。"
        } else {
            "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。"
        }
    } else if lower.contains("sina visitor system") || lower.contains("visitor.passport.weibo") {
        "微博返回了访客验证页面，后端无法从普通网页直接抓取正文，正在尝试平台接口兜底。"
    } else if platform == Platform::WeChat
        && (html.contains("环境异常") || html.contains("去验证") || html.contains("完成验证"))
    {
        "微信公众号返回了环境异常验证页面，正在尝试浏览器渲染兜底；若仍失败可从原文分享带正文的文本。"
    } else if lower.contains("zh-zse-ck") || lower.contains("zse-ck") {
        "知乎返回了访问验证页面，后端无法从普通网页直接抓取正文，正在尝试回答接口兜底。"
    } else if lower.contains("captcha") || lower.contains("访问验证") || lower.contains("安全验证")
    {
        "目标网页返回了访问验证页面，后端无法直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。"
    } else {
        return None;
    };
    Some(reason.to_string())
}

/// `content` of the first matching `<meta>` tag.
pub(crate) fn meta_content(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .find_map(|node| node.value().attr("content"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Whitespace-collapsed text of the first matching element.
fn select_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .map(|text| clean_text(&text))
        .filter(|value| !value.is_empty())
}
