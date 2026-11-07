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
    document_from_root(root, base_url)
}

// Parse an article container's HTML captured by the browser renderer.
pub(super) fn document_from_fragment(
    html: &str,
    base_url: &str,
) -> (OriginalContent, Vec<MediaAsset>) {
    let fragment = Html::parse_fragment(html);
    document_from_root(fragment.root_element(), base_url)
}

// Extract semantic blocks and media from a specific root element. Dedicated
// adapters (e.g. WeChat `#js_content`) reuse this so lazy-image and block logic
// stays in one place.
pub(super) fn document_from_root(
    root: ElementRef<'_>,
    base_url: &str,
) -> (OriginalContent, Vec<MediaAsset>) {
    let selector = Selector::parse("h1,h2,h3,h4,h5,h6,p,blockquote,pre,li,figcaption,img").ok();
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
            let follows_image = blocks
                .last()
                .is_some_and(|previous: &ContentBlock| previous.kind == "image");
            let (kind, level, text) = if name == "p" && is_caption(&element, &text, follows_image) {
                ("caption", None, strip_caption_marker(&text))
            } else if name == "p" && is_bold_subheading(&element, &text) {
                ("heading", Some(3), text)
            } else {
                (block_kind(name), heading_level(name), text)
            };
            blocks.push(ContentBlock {
                kind: kind.to_string(),
                level,
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
    // `data-src` / `data-croporisrc` cover WeChat and other lazy-loaded images
    // whose real URL is not in the placeholder `src`.
    let raw_url = ["data-src", "data-croporisrc", "data-original", "src"]
        .iter()
        .find_map(|name| {
            element
                .value()
                .attr(name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?
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
        "figcaption" => "caption",
        _ => "paragraph",
    }
}

const CAPTION_MARKERS: &[char] = &['△', '▲', '▼', '↑'];
const MAX_CAPTION_CHARS: usize = 60;
const MAX_SUBHEADING_CHARS: usize = 40;

// Image captions: an explicit caption class, the "△…" convention used by
// Chinese news sites, or a short unpunctuated line right after an image.
fn is_caption(element: &ElementRef<'_>, text: &str, follows_image: bool) -> bool {
    let chars = text.chars().count();
    if chars > MAX_CAPTION_CHARS {
        return false;
    }
    let class = element.value().attr("class").unwrap_or_default();
    if class.contains("caption") || text.starts_with(CAPTION_MARKERS) {
        return true;
    }
    follows_image
        && !is_bold_subheading(element, text)
        && !text.ends_with(['。', '！', '？', '；', '：', '.', '!', '?', ';', ':'])
}

fn strip_caption_marker(text: &str) -> String {
    text.trim_start_matches(CAPTION_MARKERS).trim().to_string()
}

// Many editors (Toutiao, WeChat) mark section titles as a bold paragraph
// instead of an <h*> tag.
fn is_bold_subheading(element: &ElementRef<'_>, text: &str) -> bool {
    if text.chars().count() > MAX_SUBHEADING_CHARS {
        return false;
    }
    let mut bold_text = String::new();
    for node in element.descendants() {
        let Some(child) = ElementRef::wrap(node) else {
            continue;
        };
        if is_bold_element(&child)
            && !child
                .ancestors()
                .filter_map(ElementRef::wrap)
                .take_while(|ancestor| ancestor.id() != element.id())
                .any(|ancestor| is_bold_element(&ancestor))
        {
            bold_text.extend(child.text());
        }
    }
    compact(&bold_text) == compact(text)
}

fn is_bold_element(element: &ElementRef<'_>) -> bool {
    let value = element.value();
    if matches!(value.name(), "strong" | "b") {
        return true;
    }
    let style = value
        .attr("style")
        .unwrap_or_default()
        .replace(' ', "")
        .to_ascii_lowercase();
    style.contains("font-weight:bold")
        || style.contains("font-weight:bolder")
        || style.contains("font-weight:600")
        || style.contains("font-weight:700")
        || style.contains("font-weight:800")
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
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

    use super::{document_from_fragment, document_from_html};

    #[test]
    fn rendered_fragment_keeps_lazy_images_captions_and_bold_subheadings() {
        let html = r#"<article class="syl-page-article">
            <p><span>自6日开始的美非法移民“围捕战”似乎难以落幕。</span></p>
            <p><span><strong>催泪瓦斯、震爆弹和橡皮子弹齐发</strong></span></p>
            <div class="pgc-img"><img src="data:image/gif;base64,R0lGOD" data-src="https://p3-sign.toutiaoimg.com/a.jpeg"></div>
            <p>伊朗队球员在墨西哥蒂华纳进行赛前训练</p>
            <p>正文继续，这是一段完整的句子。</p>
            <div class="pgc-img"><img data-src="https://p3-sign.toutiaoimg.com/b.jpeg"></div>
            <p>正文紧跟图片但以句号结尾，所以不是图注。</p>
            <p>△美国白宫边境事务主管霍曼（资料图）</p>
        </article>"#;
        let (content, media) = document_from_fragment(html, "https://www.toutiao.com/article/1/");
        let kinds = content
            .blocks
            .iter()
            .map(|block| block.kind.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            [
                "paragraph",
                "heading",
                "image",
                "caption",
                "paragraph",
                "image",
                "paragraph",
                "caption"
            ]
        );
        assert_eq!(content.blocks[1].level, Some(3));
        assert_eq!(content.blocks[7].text, "美国白宫边境事务主管霍曼（资料图）");
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].url, "https://p3-sign.toutiaoimg.com/a.jpeg");
    }

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
