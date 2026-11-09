//! Small, uniform mutations on a [`LinkMetadata`] snapshot used by every
//! adapter, so each one does not re-implement fact and media bookkeeping.

use crate::{
    content::{MediaAsset, OriginalContent, SourceFact},
    models::LinkMetadata,
    text::{clean_original_text, clean_text},
};

impl LinkMetadata {
    /// Insert or overwrite the fact `key`. Blank values are ignored.
    pub(crate) fn set_fact(&mut self, key: &str, label: &str, value: impl Into<Option<String>>) {
        let Some(value) = value.into().filter(|value| !value.trim().is_empty()) else {
            return;
        };
        if let Some(existing) = self.source_facts.iter_mut().find(|fact| fact.key == key) {
            existing.value = value;
            return;
        }
        self.source_facts.push(SourceFact {
            key: key.to_string(),
            label: label.to_string(),
            value,
        });
    }

    /// Add the fact `key` unless one already exists. Blank values are ignored.
    pub(crate) fn add_fact_if_absent(&mut self, key: &str, label: &str, value: &str) {
        if value.trim().is_empty() || self.source_facts.iter().any(|fact| fact.key == key) {
            return;
        }
        self.source_facts.push(SourceFact {
            key: key.to_string(),
            label: label.to_string(),
            value: value.to_string(),
        });
    }

    /// Append a media asset unless one with the same URL is already present.
    pub(crate) fn add_media(&mut self, asset: MediaAsset) {
        if !self.media.iter().any(|existing| existing.url == asset.url) {
            self.media.push(asset);
        }
    }

    /// Append an image asset (alt text = title) unless already present.
    pub(crate) fn add_image(&mut self, url: &str) {
        let asset = MediaAsset {
            kind: "image".to_string(),
            url: url.to_string(),
            alt: Some(self.title.clone()),
            ..MediaAsset::default()
        };
        self.add_media(asset);
    }

    /// Use `url` as the cover and keep it in the media list. Non-HTTP values
    /// (data URIs, relative paths) are ignored.
    pub(crate) fn set_cover_image(&mut self, url: String) {
        if !url.starts_with("http") {
            return;
        }
        self.add_image(&url);
        self.image_url = Some(url);
    }

    /// Replace the body (original text, search text and blocks) with `text`.
    /// The description is left alone.
    pub(crate) fn set_body(&mut self, text: String) {
        self.content_text = clean_text(&text);
        self.original_text = text.clone();
        self.original_content = OriginalContent::from_plain_text(text);
    }

    /// Replace description and body with `text` after normalising it. Empty
    /// text is ignored.
    pub(crate) fn set_description_and_body(&mut self, text: &str) {
        let text = clean_original_text(text);
        if text.is_empty() {
            return;
        }
        self.description = clean_text(&text);
        self.set_body(text);
    }

    /// Like [`Self::set_description_and_body`], but only when `text` is longer
    /// than the body already captured.
    pub(crate) fn set_description_and_body_if_longer(&mut self, text: &str) {
        let text = clean_original_text(text);
        if text.chars().count() > self.original_text.chars().count() {
            self.set_description_and_body(&text);
        }
    }

    /// Append a diagnostic note to the description without duplicating it.
    pub(crate) fn append_note(&mut self, text: String) {
        if self.description.is_empty() {
            self.description = text;
        } else if !self.description.contains(&text) {
            self.description = format!("{}\n{}", self.description, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    #[test]
    fn set_fact_overwrites_and_add_fact_keeps_first_value() {
        let mut metadata = LinkMetadata::default();
        metadata.set_fact("price", "价格", "99".to_string());
        metadata.set_fact("price", "价格", "109".to_string());
        metadata.set_fact("brand", "品牌", None);
        metadata.set_fact("empty", "空", "  ".to_string());
        metadata.add_fact_if_absent("price", "价格", "1");
        metadata.add_fact_if_absent("kind", "类型", "post");
        let facts = metadata
            .source_facts
            .iter()
            .map(|fact| (fact.key.as_str(), fact.value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(facts, [("price", "109"), ("kind", "post")]);
    }

    #[test]
    fn media_is_deduplicated_and_cover_requires_http() {
        let mut metadata = LinkMetadata {
            title: "标题".to_string(),
            ..LinkMetadata::default()
        };
        metadata.set_cover_image("data:image/png;base64,abc".to_string());
        assert!(metadata.image_url.is_none());
        metadata.set_cover_image("https://example.com/a.jpg".to_string());
        metadata.add_image("https://example.com/a.jpg");
        assert_eq!(metadata.media.len(), 1);
        assert_eq!(metadata.media[0].alt.as_deref(), Some("标题"));
    }

    #[test]
    fn body_updates_only_grow_when_asked() {
        let mut metadata = LinkMetadata::default();
        metadata.set_description_and_body("一段  较长的\n\n正文");
        assert_eq!(metadata.description, "一段 较长的 正文");
        assert_eq!(metadata.original_text, "一段 较长的\n\n正文");
        metadata.set_description_and_body_if_longer("短");
        assert_eq!(metadata.original_text, "一段 较长的\n\n正文");
        metadata.append_note("提示".to_string());
        metadata.append_note("提示".to_string());
        assert_eq!(metadata.description, "一段 较长的 正文\n提示");
    }
}
