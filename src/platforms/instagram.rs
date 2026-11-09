//! Instagram: caption from the public Open Graph tags, the oEmbed API when
//! `INSTAGRAM_OEMBED_TOKEN` is set, and yt-dlp for reels.

use std::env;

use reqwest::Client;
use scraper::Html;

use crate::{
    extraction::{PlatformMetadata, meta_content, oembed, video},
    models::LinkMetadata,
    text::truncate_chars,
};

use super::identity::id_after;

const OEMBED_ENDPOINT: &str = "https://graph.facebook.com/v25.0/instagram_oembed";

pub(super) const MISSING_CONTENT_HINT: &str =
    "该帖子可能是私密、已删除或受地区限制，公开页面未返回 caption。";

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    ["p", "reel", "tv"].into_iter().find_map(|kind| {
        id_after(segments, &[kind]).map(|id| {
            (
                id.clone(),
                format!("https://www.instagram.com/{kind}/{id}/"),
            )
        })
    })
}

/// `og:description` reads `<n> likes, <m> comments - <author> on <date>: "<caption>"`.
pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
    let document = Html::parse_document(html);
    let og_title = meta_content(&document, "meta[property='og:title']");
    let og_description = meta_content(&document, "meta[property='og:description']");
    let caption = og_description
        .as_deref()
        .and_then(caption)
        .filter(|value| !value.is_empty())?;
    let author = og_title
        .as_deref()
        .and_then(author)
        .or_else(|| og_description.as_deref().and_then(author));
    PlatformMetadata {
        title: Some(truncate_chars(&caption, 100)),
        description: Some(caption.clone()),
        author,
        image_url: meta_content(&document, "meta[property='og:image']"),
        original_text: Some(caption),
    }
    .into_meaningful()
}

pub(super) fn shell_reason(page_text: &str) -> Option<&'static str> {
    (page_text.contains("Log in")
        && (page_text.contains("Sign up") || page_text.contains("Instagram")))
    .then_some("Instagram 只返回了登录页面，未取得帖子正文。")
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    let token = env::var("INSTAGRAM_OEMBED_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    oembed::enrich(metadata, http, OEMBED_ENDPOINT, token.as_deref()).await;
    video::enrich_with_yt_dlp(metadata, http, allow_video_transcription, false).await;
}

fn caption(description: &str) -> Option<String> {
    for marker in [": \"", ": “", "：\"", "：“"] {
        if let Some((_, caption)) = description.rsplit_once(marker) {
            let caption = caption
                .trim()
                .trim_end_matches(['.', '。'])
                .trim_matches(['\"', '“', '”'])
                .trim();
            if !caption.is_empty() {
                return Some(caption.to_string());
            }
        }
    }
    None
}

fn author(value: &str) -> Option<String> {
    if let Some(author) = value
        .strip_prefix("Instagram 用户 ")
        .and_then(|value| value.split_once(" :").map(|(author, _)| author))
    {
        return Some(author.trim().to_string());
    }
    if let Some((author, _)) = value.split_once(" on Instagram") {
        return Some(author.trim().to_string());
    }
    if let Some((_, tail)) = value.split_once(" - ") {
        for marker in [" on ", " 在 ", " 于 "] {
            if let Some((author, _)) = tail.split_once(marker) {
                let author = author.trim();
                if !author.is_empty() {
                    return Some(author.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{author, caption, embedded_state, shell_reason};

    #[test]
    fn reads_caption_and_author_from_open_graph() {
        let html = r#"<meta property="og:title" content="Chef Lin on Instagram: &quot;Dumplings&quot;">
            <meta property="og:description" content="120 likes, 4 comments - chef.lin on May 1, 2026: &quot;Handmade dumplings tonight&quot;.">
            <meta property="og:image" content="https://scontent.cdninstagram.com/a.jpg">"#;
        let metadata = embedded_state(html).expect("caption");
        assert_eq!(
            metadata.original_text.as_deref(),
            Some("Handmade dumplings tonight")
        );
        assert_eq!(metadata.author.as_deref(), Some("Chef Lin"));
        assert!(embedded_state("<meta property=\"og:title\" content=\"Instagram\">").is_none());
    }

    #[test]
    fn parses_localized_descriptions() {
        assert_eq!(
            caption("1 次赞 - a 于 2026年：“你好”").as_deref(),
            Some("你好")
        );
        assert_eq!(
            author("Instagram 用户 小王 : 帖子").as_deref(),
            Some("小王")
        );
        assert_eq!(
            author("12 likes - chef.lin on May 1").as_deref(),
            Some("chef.lin")
        );
        assert!(shell_reason("Log in • Instagram").is_some());
    }
}
