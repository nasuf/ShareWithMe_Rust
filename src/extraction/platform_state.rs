//! Fields recovered from a platform's embedded page state or public API, and
//! how they override the snapshot built so far.

use crate::{
    content::OriginalContent,
    models::LinkMetadata,
    text::{clean_original_text, clean_text, truncate_chars},
};

use super::UNTITLED;

#[derive(Debug, Default)]
pub(crate) struct PlatformMetadata {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) image_url: Option<String>,
    pub(crate) original_text: Option<String>,
}

impl PlatformMetadata {
    pub(crate) fn is_meaningful(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.image_url.is_some()
            || self.original_text.is_some()
    }

    /// `Some(self)` when it carries anything worth merging.
    pub(crate) fn into_meaningful(self) -> Option<Self> {
        self.is_meaningful().then_some(self)
    }
}

/// Overwrite snapshot fields with every non-blank platform field.
pub(crate) fn merge_platform_metadata(metadata: &mut LinkMetadata, platform: PlatformMetadata) {
    let non_blank = |value: Option<String>| value.filter(|value| !value.trim().is_empty());

    if let Some(title) = non_blank(platform.title) {
        metadata.title = title;
    } else if metadata.title == UNTITLED
        && let Some(description) = platform
            .description
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    {
        metadata.title = truncate_chars(description, 80);
    }

    if let Some(description) = non_blank(platform.description) {
        metadata.content_text = clean_text(&format!("{}\n\n{}", metadata.title, description));
        metadata.description = description;
    } else if metadata.content_text.trim().is_empty() && metadata.title != UNTITLED {
        metadata.content_text = metadata.title.clone();
    }

    if let Some(author) = non_blank(platform.author) {
        metadata.author = Some(author);
    }
    if let Some(image_url) = non_blank(platform.image_url) {
        metadata.image_url = Some(image_url);
    }
    if let Some(original_text) = non_blank(platform.original_text) {
        metadata.original_text = clean_original_text(&original_text);
        metadata.original_content = OriginalContent::from_plain_text(&metadata.original_text);
    } else if metadata.original_text.trim().is_empty() && !metadata.content_text.trim().is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::{PlatformMetadata, merge_platform_metadata};
    use crate::models::LinkMetadata;

    #[test]
    fn platform_fields_override_but_blank_fields_do_not() {
        let mut metadata = LinkMetadata {
            title: "未命名链接".to_string(),
            author: Some("旧作者".to_string()),
            ..LinkMetadata::default()
        };
        merge_platform_metadata(
            &mut metadata,
            PlatformMetadata {
                description: Some("只有描述的帖子".to_string()),
                author: Some("  ".to_string()),
                ..PlatformMetadata::default()
            },
        );
        assert_eq!(metadata.title, "只有描述的帖子");
        assert_eq!(metadata.author.as_deref(), Some("旧作者"));
        assert_eq!(metadata.content_text, "只有描述的帖子 只有描述的帖子");
        assert_eq!(metadata.original_text, metadata.content_text);
        assert!(PlatformMetadata::default().into_meaningful().is_none());
    }
}
