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

#[cfg(test)]
mod tests {
    use super::replace_platform_shell;
    use crate::{content::OriginalContent, models::LinkMetadata};

    fn shell(platform: &str, page_text: &str, shared_text: &str, title: &str) -> LinkMetadata {
        LinkMetadata {
            platform: platform.to_string(),
            title: title.to_string(),
            shared_text: shared_text.to_string(),
            original_content: OriginalContent::from_plain_text(page_text),
            ..LinkMetadata::default()
        }
    }

    #[test]
    fn douyin_login_shell_falls_back_to_the_video_title() {
        let mut metadata = shell(
            "抖音",
            "登录后免费畅享高清视频",
            "https://v.douyin.com/abc/",
            "韭菜盒子做法 - 抖音",
        );
        replace_platform_shell(&mut metadata);
        assert_eq!(metadata.original_text, "韭菜盒子做法");
        assert_eq!(metadata.extraction.method, "metadata-only");
        assert!(metadata.extraction.last_error.is_some());
    }

    #[test]
    fn shell_without_any_fallback_is_cleared() {
        let mut metadata = shell("微博", "微博热搜 登录/注册", "", "未命名链接");
        replace_platform_shell(&mut metadata);
        assert!(metadata.original_content.is_empty());
        assert_eq!(metadata.extraction.method, "blocked-page");
    }

    #[test]
    fn api_content_is_never_treated_as_a_shell() {
        let mut metadata = shell("微博", "微博热搜 登录/注册", "", "标题");
        metadata.extraction.method = "platform-api".to_string();
        replace_platform_shell(&mut metadata);
        assert_eq!(metadata.extraction.method, "platform-api");
        assert!(!metadata.original_content.is_empty());
    }
}
