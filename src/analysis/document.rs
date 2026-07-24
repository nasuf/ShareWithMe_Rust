use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::content::{ContentBlock, MediaAsset, OriginalContent};

use super::classify::clean_original_text;

const MAX_BLOCKS: usize = 300;
const MAX_TEXT_CHARS: usize = 40_000;

pub(super) fn document_from_html(
    document: &Html,
    base_url: &str,
) -> (OriginalContent, Vec<MediaAsset>) {
    let Some(root) = content_root(document) else {
        return (OriginalContent::default(), Vec::new());
    };
    let selector = Selector::parse("h1,h2,h3,h4,h5,h6,p,blockquote,pre,li,img").ok();
    let mut blocks = Vec::new();
    let mut media = Vec::new();
    let mut text_chars = 0usize;

    if let Some(selector) = selector {
        for element in root.select(&selector).take(MAX_BLOCKS * 2) {
            if blocks.len() >= MAX_BLOCKS || text_chars >= MAX_TEXT_CHARS {
                break;
            }
            let name = element.value().name();
            if name == "img" {
                if let Some(asset) = image_asset(&element, base_url)
                    && !media
                        .iter()
                        .any(|existing: &MediaAsset| existing.url == asset.url)
                {
                    blocks.push(ContentBlock {
                        kind: "image".to_string(),
                        url: Some(asset.url.clone()),
                        alt: asset.alt.clone(),
                        ..ContentBlock::default()
                    });
                    media.push(asset);
                }
                continue;
            }

            let text = element
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let text = clean_original_text(&text);
            if text.is_empty() || duplicate_text(&blocks, &text) {
                continue;
            }
            text_chars += text.chars().count();
            blocks.push(ContentBlock {
                kind: block_kind(name).to_string(),
                level: heading_level(name),
                text,
                ..ContentBlock::default()
            });
        }
    }

    if blocks.is_empty() {
        let text = clean_original_text(&root.text().collect::<Vec<_>>().join(" "));
        return (OriginalContent::from_plain_text(text), media);
    }

    let plain_text = blocks
        .iter()
        .filter(|block| block.kind != "image" && !block.text.is_empty())
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    (
        OriginalContent {
            format: "blocks-v1".to_string(),
            plain_text,
            blocks,
        },
        media,
    )
}

fn content_root(document: &Html) -> Option<ElementRef<'_>> {
    const ROOTS: &[&str] = &[
        "article",
        ".RichContent-inner",
        ".Post-RichTextContainer",
        ".syl-page-article",
        ".article-content",
        "[class*='note-content']",
        "main",
        "body",
    ];
    for selector in ROOTS {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        if let Some(root) = document.select(&selector).find(|element| {
            element
                .text()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .count()
                >= 2
        }) {
            return Some(root);
        }
    }
    None
}

fn image_asset(element: &ElementRef<'_>, base_url: &str) -> Option<MediaAsset> {
    let raw_url = ["data-src", "data-original", "src"]
        .iter()
        .find_map(|name| element.value().attr(name))?
        .trim();
    let url = absolute_url(base_url, raw_url)?;
    if url.starts_with("data:") || url.ends_with(".svg") {
        return None;
    }
    Some(MediaAsset {
        kind: "image".to_string(),
        url,
        alt: element
            .value()
            .attr("alt")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        width: element
            .value()
            .attr("width")
            .and_then(|value| value.parse().ok()),
        height: element
            .value()
            .attr("height")
            .and_then(|value| value.parse().ok()),
        ..MediaAsset::default()
    })
}

fn absolute_url(base_url: &str, raw_url: &str) -> Option<String> {
    if raw_url.starts_with("https://") || raw_url.starts_with("http://") {
        return Some(raw_url.to_string());
    }
    if raw_url.starts_with("//") {
        return Some(format!("https:{raw_url}"));
    }
    Url::parse(base_url)
        .ok()?
        .join(raw_url)
        .ok()
        .map(Into::into)
}

fn block_kind(name: &str) -> &'static str {
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "blockquote" => "quote",
        "pre" => "code",
        "li" => "list-item",
        _ => "paragraph",
    }
}

fn heading_level(name: &str) -> Option<u8> {
    name.strip_prefix('h').and_then(|value| value.parse().ok())
}

fn duplicate_text(blocks: &[ContentBlock], text: &str) -> bool {
    blocks
        .last()
        .is_some_and(|previous| previous.text == text || previous.text.contains(text))
}

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::document_from_html;

    #[test]
    fn preserves_semantic_blocks_and_images() {
        let html = Html::parse_document(
            r#"<article><h1>标题</h1><p>第一段 <strong>重点</strong></p><blockquote>引用</blockquote><ul><li>项目一</li><li>项目二</li></ul><img src="/cover.jpg" alt="封面"></article>"#,
        );
        let (content, media) = document_from_html(&html, "https://example.com/post/1");

        assert_eq!(content.blocks[0].kind, "heading");
        assert_eq!(content.blocks[1].text, "第一段 重点");
        assert_eq!(content.blocks[2].kind, "quote");
        assert_eq!(content.blocks[3].kind, "list-item");
        assert_eq!(media[0].url, "https://example.com/cover.jpg");
    }
}
