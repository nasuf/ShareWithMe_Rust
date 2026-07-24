use chrono::{TimeZone, Utc};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use crate::{
    content::{ContentBlock, MediaAsset, OriginalContent, SourceFact},
    models::LinkMetadata,
};

use super::super::{
    classify::{
        clean_original_text, clean_text, first_meaningful_line, html_to_formatted_text,
        is_url_only_text, truncate_chars,
    },
    document::document_from_root,
};

/// Parse a WeChat Official Account article (`mp.weixin.qq.com/s/...`).
///
/// WeChat serves two layouts:
/// * Classic articles are server-rendered with the title in `#activity-name` and
///   the body in `#js_content` (lazy `data-src` images).
/// * Newer articles and picture messages are rendered client-side from a
///   `window.cgiData` object; the body text, account name and publish time can
///   still be recovered directly from that inline data without a browser.
///
/// Returns `true` when the page is recognised (article, picture message, or an
/// unavailable article), so the generic Open Graph path is skipped.
pub(in crate::analysis) fn merge(metadata: &mut LinkMetadata, html: &str) -> bool {
    if metadata.platform != "微信公众号" && !metadata.final_url.contains("mp.weixin.qq.com") {
        return false;
    }

    if let Some(reason) = unavailable_reason(html) {
        apply_reason(metadata, reason);
        return true;
    }

    let document = Html::parse_document(html);

    if let Some(content_element) = select_one(&document, "#js_content") {
        merge_classic_article(metadata, &document, content_element, html);
        return true;
    }

    // New-style / picture messages keep the payload in `window.cgiData`.
    if merge_from_cgi_data(metadata, html) {
        return true;
    }

    // Last resort: some servers (e.g. datacenter IPs) only receive the Open
    // Graph meta page. WeChat crams the whole share-card text into `og:title`
    // with literal `\n` escapes, so normalise those and derive a clean title.
    merge_from_open_graph(metadata, &document)
}

fn merge_classic_article(
    metadata: &mut LinkMetadata,
    document: &Html,
    content_element: ElementRef<'_>,
    html: &str,
) {
    if let Some(title) = select_text(document, "#activity-name")
        .or_else(|| {
            meta_content(document, "meta[property='og:title']").map(|v| unescape_literal_ws(&v))
        })
        .filter(|value| !value.is_empty())
    {
        metadata.title = title;
    }

    let account = select_text(document, "#js_name")
        .or_else(|| select_text(document, ".profile_nickname"))
        .filter(|value| !value.is_empty());
    let byline = select_text(document, "#js_author_name")
        .filter(|value| !value.is_empty() && !value.contains('年'));

    set_author_facts(metadata, byline, account);

    let (content, media) = document_from_root(content_element, &metadata.final_url);
    if !content.plain_text.trim().is_empty() {
        metadata.original_content = content;
        metadata.original_text = metadata.original_content.plain_text.clone();
        metadata.content_text = clean_text(&metadata.original_text);
    }

    let og_description = meta_content(document, "meta[property='og:description']")
        .or_else(|| meta_content(document, "meta[name='description']"))
        .map(|value| unescape_literal_ws(&value));
    set_description(metadata, og_description);

    let cover = meta_content(document, "meta[property='og:image']")
        .or_else(|| js_url_var(html, "msg_cdn_url"))
        .or_else(|| js_url_var(html, "cover"));
    set_cover(metadata, cover);
    for asset in media {
        push_media(metadata, asset);
    }

    if let Some(published_at) = publish_time_from_ct(html) {
        set_published_at(metadata, published_at);
    }
}

fn merge_from_cgi_data(metadata: &mut LinkMetadata, html: &str) -> bool {
    // `content_noencode` is mostly plain text but can embed inline HTML such as
    // mini-program `<a data-miniprogram-applink=...>` links. Keep the readable
    // text and preserve those links as structured `link` blocks instead of
    // leaking raw markup or dropping the link entirely.
    let raw_body = cgi_string(html, "content_noencode")
        .or_else(|| cgi_string(html, "content").filter(|value| value.contains('<')))
        .filter(|value| !value.trim().is_empty());
    let (body_text, links) = match raw_body {
        Some(raw) if raw.contains('<') => split_inline_links(&raw, &metadata.final_url),
        Some(raw) => (clean_original_text(&raw), Vec::new()),
        None => (String::new(), Vec::new()),
    };
    let has_content = !body_text.trim().is_empty() || !links.is_empty();

    let raw_title = cgi_string(html, "msg_title").or_else(|| cgi_string(html, "title"));
    let title = raw_title
        .as_deref()
        .and_then(first_meaningful_line)
        .filter(|value| !value.is_empty());

    if title.is_none() && !has_content {
        return false;
    }

    if let Some(title) = title {
        metadata.title = title;
    }

    let account = cgi_string(html, "nick_name").filter(|value| !value.trim().is_empty());
    let byline = cgi_string(html, "author").filter(|value| !value.trim().is_empty());
    set_author_facts(metadata, byline, account);

    if has_content {
        apply_body_with_links(metadata, &body_text, &links);
    }
    set_description(metadata, None);

    let cover = meta_content_from_html(html, "og:image")
        .or_else(|| cgi_url(html, "cdn_url"))
        .or_else(|| js_url_var(html, "msg_cdn_url"));
    set_cover(metadata, cover);
    for image in cgi_urls(html, "cdn_url") {
        push_media(
            metadata,
            MediaAsset {
                kind: "image".to_string(),
                url: image,
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            },
        );
    }

    if let Some(published_at) = cgi_string(html, "create_timestamp")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
    {
        set_published_at(metadata, published_at);
    }

    true
}

fn merge_from_open_graph(metadata: &mut LinkMetadata, document: &Html) -> bool {
    let og_title = meta_content(document, "meta[property='og:title']")
        .map(|value| unescape_literal_ws(&value))
        .filter(|value| !value.trim().is_empty());
    let og_description = meta_content(document, "meta[property='og:description']")
        .or_else(|| meta_content(document, "meta[name='description']"))
        .map(|value| unescape_literal_ws(&value))
        .filter(|value| !value.trim().is_empty());

    if og_title.is_none() && og_description.is_none() {
        return false;
    }

    // The body is whichever field carries the most text; the title is its first
    // meaningful line so a long share-card caption never becomes the headline.
    let body = [og_title.as_deref(), og_description.as_deref()]
        .into_iter()
        .flatten()
        .max_by_key(|value| value.chars().count())
        .map(clean_original_text)
        .filter(|value| !value.trim().is_empty());

    if let Some(title) = og_title
        .as_deref()
        .or(body.as_deref())
        .and_then(first_meaningful_line)
        .filter(|value| !value.is_empty())
    {
        metadata.title = title;
    }
    if let Some(body) = body {
        metadata.original_text = body.clone();
        metadata.original_content = OriginalContent::from_plain_text(&body);
        metadata.content_text = clean_text(&body);
    }
    set_description(metadata, og_description);
    set_cover(
        metadata,
        meta_content(document, "meta[property='og:image']"),
    );

    true
}

fn set_author_facts(metadata: &mut LinkMetadata, byline: Option<String>, account: Option<String>) {
    let byline = byline
        .map(|value| clean_text(&value))
        .filter(|v| !v.is_empty());
    let account = account
        .map(|value| clean_text(&value))
        .filter(|v| !v.is_empty());
    if let Some(author) = byline.clone().or_else(|| account.clone()) {
        metadata.author = Some(author);
    }
    if let Some(account) = account {
        push_fact(metadata, "account", "公众号", account);
    }
    // Add a distinct byline only when it differs from the display author.
    if let Some(byline) = byline.filter(|value| metadata.author.as_deref() != Some(value)) {
        push_fact(metadata, "author", "作者", byline);
    }
}

fn set_description(metadata: &mut LinkMetadata, candidate: Option<String>) {
    if let Some(description) = candidate.filter(|value| !value.trim().is_empty()) {
        metadata.description = clean_text(&description);
    } else if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
        metadata.description = truncate_chars(&clean_text(&metadata.content_text), 160);
    }
}

fn set_cover(metadata: &mut LinkMetadata, cover: Option<String>) {
    let Some(cover) = cover.filter(|value| value.starts_with("http")) else {
        return;
    };
    metadata.image_url = Some(cover.clone());
    if !metadata.media.iter().any(|asset| asset.url == cover) {
        metadata.media.insert(
            0,
            MediaAsset {
                kind: "image".to_string(),
                url: cover,
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            },
        );
    }
}

fn set_published_at(metadata: &mut LinkMetadata, published_at: chrono::DateTime<Utc>) {
    metadata.published_at = Some(published_at);
    push_fact(
        metadata,
        "published_at",
        "发布时间",
        published_at.format("%Y-%m-%d").to_string(),
    );
}

fn push_media(metadata: &mut LinkMetadata, asset: MediaAsset) {
    if !asset.url.starts_with("http") {
        return;
    }
    if !metadata
        .media
        .iter()
        .any(|existing| existing.url == asset.url)
    {
        metadata.media.push(asset);
    }
}

/// Detect deleted/removed/migrated articles so we store an honest reason instead
/// of a misleading empty snapshot.
fn unavailable_reason(html: &str) -> Option<&'static str> {
    const MARKERS: &[(&str, &str)] = &[
        (
            "该内容已被发布者删除",
            "该公众号文章已被发布者删除，后端无法再抓取正文。",
        ),
        (
            "此内容因违规无法查看",
            "该公众号文章因违规已被平台限制，无法查看正文。",
        ),
        (
            "此内容发送失败无法查看",
            "该公众号文章发送失败或已撤回，无法查看正文。",
        ),
        (
            "该公众号已迁移",
            "该公众号已迁移，原文章链接无法再抓取正文。",
        ),
        (
            "该账号已自主注销",
            "该公众号已注销，原文章链接无法再抓取正文。",
        ),
        (
            "该内容已被投诉",
            "该公众号文章因被投诉已被平台处理，无法查看正文。",
        ),
    ];
    MARKERS
        .iter()
        .find(|(marker, _)| html.contains(marker))
        .map(|(_, reason)| *reason)
}

fn apply_reason(metadata: &mut LinkMetadata, reason: &str) {
    if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
        metadata.description = reason.to_string();
    } else if !metadata.description.contains(reason) {
        metadata.description = format!("{}\n{reason}", metadata.description);
    }
    if metadata.content_text.trim().is_empty() || is_url_only_text(&metadata.content_text) {
        metadata.content_text = reason.to_string();
    }
    if metadata.original_text.trim().is_empty() || is_url_only_text(&metadata.original_text) {
        metadata.original_text = reason.to_string();
        metadata.original_content = OriginalContent::from_plain_text(reason);
    }
    // Marks the snapshot as a definitive dead end so the browser-render fallback
    // does not re-fetch and overwrite this reason.
    metadata.extraction.last_error = Some(reason.to_string());
}

/// WeChat exposes the classic publish timestamp as `var ct = "1700000000"`
/// (unix seconds). The DOM `#publish_time` node is filled in by JavaScript, so
/// the inline variable is the reliable server-rendered source.
fn publish_time_from_ct(html: &str) -> Option<chrono::DateTime<Utc>> {
    let seconds = Regex::new(r#"var\s+ct\s*=\s*"(\d{10,})""#)
        .ok()?
        .captures(html)?
        .get(1)?
        .as_str()
        .parse::<i64>()
        .ok()?;
    Utc.timestamp_opt(seconds, 0).single()
}

fn js_url_var(html: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"{}\s*=\s*"([^"]+)""#, regex::escape(name));
    let raw = Regex::new(&pattern).ok()?.captures(html)?.get(1)?.as_str();
    let url = normalize_wechat_url(raw);
    url.starts_with("http").then_some(url)
}

/// Read a single-quoted `cgiData` field. WeChat uses `\x0a` style escapes and
/// single quotes, so this decodes the raw capture into readable text.
fn cgi_string(html: &str, key: &str) -> Option<String> {
    let raw = Regex::new(&cgi_field_pattern(key))
        .ok()?
        .captures(html)?
        .get(1)?
        .as_str();
    let decoded = decode_js_string(raw);
    (!decoded.trim().is_empty()).then_some(decoded)
}

fn cgi_url(html: &str, key: &str) -> Option<String> {
    cgi_urls(html, key).into_iter().next()
}

/// Match a `cgiData` field precisely: the key must not be embedded in a longer
/// identifier on either side, so `content` does not match `content_noencode`
/// (trailing `\b`) and `cdn_url` does not match `head_img_cdn_url` (leading
/// boundary).
fn cgi_field_pattern(key: &str) -> String {
    format!(
        r#"(?:\A|[^0-9A-Za-z_]){}\b[^']{{0,40}}'((?:\\.|[^'\\])*)'"#,
        regex::escape(key)
    )
}

fn cgi_urls(html: &str, key: &str) -> Vec<String> {
    let Ok(regex) = Regex::new(&cgi_field_pattern(key)) else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for captures in regex.captures_iter(html) {
        let Some(raw) = captures.get(1) else {
            continue;
        };
        let url = normalize_wechat_url(raw.as_str());
        if url.starts_with("http") && !urls.contains(&url) {
            urls.push(url);
        }
        if urls.len() >= 6 {
            break;
        }
    }
    urls
}

fn normalize_wechat_url(raw: &str) -> String {
    raw.replace("\\x26amp;", "&")
        .replace("\\x26", "&")
        .replace("&amp;", "&")
        .replace("\\/", "/")
}

/// Decode WeChat's JS string escapes (`\x0a`, `\uXXXX`, `\'`, `\/`, ...) and
/// HTML entities into plain text.
///
/// `\xNN` escapes are raw UTF-8 bytes, so consecutive `\xNN` sequences are
/// buffered and flushed together (lossy) to reconstruct multi-byte characters.
/// Invalid escapes are dropped rather than turned into NUL, which keeps the text
/// safe to persist.
fn decode_js_string(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut byte_buffer: Vec<u8> = Vec::new();
    let mut chars = raw.chars().peekable();

    fn flush_bytes(output: &mut String, byte_buffer: &mut Vec<u8>) {
        if !byte_buffer.is_empty() {
            output.push_str(&String::from_utf8_lossy(byte_buffer));
            byte_buffer.clear();
        }
    }

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            flush_bytes(&mut output, &mut byte_buffer);
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('x') => {
                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                // Drop malformed byte escapes instead of emitting a NUL byte.
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    byte_buffer.push(byte);
                }
                continue;
            }
            escape => {
                flush_bytes(&mut output, &mut byte_buffer);
                match escape {
                    Some('n') => output.push('\n'),
                    Some('r') => output.push('\r'),
                    Some('t') => output.push('\t'),
                    Some('/') => output.push('/'),
                    Some('\'') => output.push('\''),
                    Some('"') => output.push('"'),
                    Some('\\') => output.push('\\'),
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                        if let Some(decoded) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            output.push(decoded);
                        }
                    }
                    Some(other) => output.push(other),
                    None => output.push('\\'),
                }
            }
        }
    }
    flush_bytes(&mut output, &mut byte_buffer);
    clean_original_text(&output)
}

/// Split inline `<a>` links out of a WeChat body fragment. Returns the plain
/// body text (anchors removed) plus the extracted `(text, target)` links.
///
/// A standard `http(s)` href is used as-is; a mini-program link (no usable web
/// href, e.g. `data-miniprogram-applink="#小程序://..."`) falls back to the
/// article URL so the client can hand off to WeChat, mirroring how the browser
/// prompts "open in WeChat" for these links.
fn split_inline_links(raw: &str, article_url: &str) -> (String, Vec<(String, String)>) {
    let Ok(anchor) = Regex::new(r"(?is)<a\b([^>]*)>(.*?)</a>") else {
        return (html_to_formatted_text(raw), Vec::new());
    };
    let mut links = Vec::new();
    let stripped = anchor.replace_all(raw, |caps: &regex::Captures<'_>| {
        let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let inner = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let text = html_to_formatted_text(inner);
        if text.trim().is_empty() {
            return String::new();
        }
        let target = anchor_http_href(attrs).unwrap_or_else(|| article_url.to_string());
        links.push((text, target));
        // Remove the anchor from the flowing text; it is re-added as a block.
        String::new()
    });
    (html_to_formatted_text(&stripped), links)
}

/// A real, openable web href from an anchor's attributes (mini-program `#小程序://`
/// links have no usable web URL, so those return `None`).
fn anchor_http_href(attrs: &str) -> Option<String> {
    let raw = Regex::new(r#"(?i)\bhref\s*=\s*["']([^"']+)["']"#)
        .ok()?
        .captures(attrs)?
        .get(1)?
        .as_str();
    let href = normalize_wechat_url(raw);
    href.starts_with("http").then_some(href)
}

/// Apply the plain body as paragraphs and append the preserved links as trailing
/// `link` blocks (WeChat drainage links sit at the end of the article).
fn apply_body_with_links(metadata: &mut LinkMetadata, body_text: &str, links: &[(String, String)]) {
    let mut content = OriginalContent::from_plain_text(body_text);
    let mut plain = content.plain_text.clone();
    for (text, url) in links {
        content.blocks.push(ContentBlock {
            kind: "link".to_string(),
            text: text.clone(),
            url: Some(url.clone()),
            ..ContentBlock::default()
        });
        if !plain.is_empty() {
            plain.push_str("\n\n");
        }
        plain.push_str(text);
    }
    if content.format.is_empty() {
        content.format = "blocks-v1".to_string();
    }
    content.plain_text = plain.clone();
    metadata.original_content = content;
    metadata.original_text = plain.clone();
    metadata.content_text = clean_text(&plain);
}

/// Turn literal escape sequences (the two characters `\` + `n`) into real
/// whitespace. WeChat's `og:title` / `og:description` embed `\n` / `\r\n` / `\t`
/// verbatim, which would otherwise be stored and displayed literally.
fn unescape_literal_ws(value: &str) -> String {
    value
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n")
        .replace("\\t", " ")
}

fn push_fact(metadata: &mut LinkMetadata, key: &str, label: &str, value: String) {
    let value = clean_text(&value);
    if value.is_empty() || metadata.source_facts.iter().any(|fact| fact.key == key) {
        return;
    }
    metadata.source_facts.push(SourceFact {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
}

fn select_one<'a>(document: &'a Html, selector: &str) -> Option<ElementRef<'a>> {
    document.select(&Selector::parse(selector).ok()?).next()
}

fn select_text(document: &Html, selector: &str) -> Option<String> {
    let element = select_one(document, selector)?;
    let text = clean_original_text(&element.text().collect::<Vec<_>>().join(" "));
    (!text.is_empty()).then(|| clean_text(&text))
}

fn meta_content(document: &Html, selector: &str) -> Option<String> {
    document
        .select(&Selector::parse(selector).ok()?)
        .find_map(|node| node.value().attr("content"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn meta_content_from_html(html: &str, property: &str) -> Option<String> {
    meta_content(
        &Html::parse_document(html),
        &format!("meta[property='{property}']"),
    )
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::merge;

    fn wechat_metadata() -> LinkMetadata {
        LinkMetadata {
            platform: "微信公众号".to_string(),
            final_url: "https://mp.weixin.qq.com/s/s4xrYzArVrkhEAfvvhR01Q".to_string(),
            ..LinkMetadata::default()
        }
    }

    #[test]
    fn extracts_classic_article_body_author_and_images() {
        let html = r#"
            <html><head>
              <meta property="og:title" content="OG 标题">
              <meta property="og:description" content="这是文章摘要。">
              <meta property="og:image" content="https://mmbiz.qpic.cn/cover.jpg">
            </head><body>
              <h1 class="rich_media_title" id="activity-name">
                  真实文章标题
              </h1>
              <a id="js_name">测试公众号</a>
              <div class="rich_media_content" id="js_content">
                <p>第一段正文内容，足够长以便通过完整度阈值判断逻辑测试测试。</p>
                <p>第二段继续补充更多细节，说明适配器可以抽取多段落正文。</p>
                <img data-src="https://mmbiz.qpic.cn/inline-1.jpg" alt="插图">
              </div>
              <script>var ct = "1700000000";</script>
            </body></html>
        "#;
        let mut metadata = wechat_metadata();
        assert!(merge(&mut metadata, html));
        assert_eq!(metadata.title, "真实文章标题");
        assert_eq!(metadata.author.as_deref(), Some("测试公众号"));
        assert!(metadata.content_text.contains("第一段正文内容"));
        assert!(metadata.content_text.contains("第二段继续补充"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://mmbiz.qpic.cn/cover.jpg")
        );
        assert!(
            metadata
                .media
                .iter()
                .any(|asset| asset.url == "https://mmbiz.qpic.cn/inline-1.jpg")
        );
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "account" && fact.value == "测试公众号")
        );
        assert!(metadata.published_at.is_some());
    }

    #[test]
    fn extracts_new_style_article_from_cgi_data() {
        // Mirrors the Vue/picture-message layout: no `#js_content`, payload lives
        // in `window.cgiData` with `\x0a` escapes and single quotes.
        let html = r##"
            <html><head>
              <meta property="og:title" content="" />
              <meta property="og:image" content="" />
            </head><body>
              <div id="js_article"></div>
              <script>
                window.cgiData = {
                  nick_name: '神仙外企',
                  create_timestamp: '1784865600',
                  author: '',
                  content_noencode: '外企真的在撤离中国吗？\x0a\x0a如果认真看了近两年的财报，会发现并不是这样的。\x0a很多外企撤掉的，其实是利润低的传统业务。\x0a<a data-miniprogram-applink="#小程序://神仙外企/Lb9WsKYl1HQ4Pxb" class="weapp_text_link">点我免费领取《New外企招聘信息表》</a>',
                };
                msg_title = window.title = '外企真的在撤离中国吗？\x0a\x0a如果认真看了近两年的财报';
              </script>
            </body></html>
        "##;
        let mut metadata = wechat_metadata();
        assert!(merge(&mut metadata, html));
        assert_eq!(metadata.title, "外企真的在撤离中国吗？");
        assert_eq!(metadata.author.as_deref(), Some("神仙外企"));
        assert!(metadata.content_text.contains("如果认真看了近两年的财报"));
        assert!(metadata.content_text.contains("利润低的传统业务"));
        // Inline mini-program link markup is stripped, keeping the readable text.
        assert!(
            metadata
                .content_text
                .contains("点我免费领取《New外企招聘信息表》")
        );
        assert!(!metadata.content_text.contains("<a "));
        assert!(!metadata.content_text.contains("data-miniprogram-applink"));
        // The link is preserved as a structured `link` block; a mini-program
        // link (no http href) targets the article URL for a WeChat hand-off.
        let link = metadata
            .original_content
            .blocks
            .iter()
            .find(|block| block.kind == "link")
            .expect("a link block");
        assert!(link.text.contains("点我免费领取《New外企招聘信息表》"));
        assert_eq!(link.url.as_deref(), Some(metadata.final_url.as_str()));
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "account" && fact.value == "神仙外企")
        );
        assert!(metadata.published_at.is_some());
    }

    #[test]
    fn decodes_hex_byte_escapes_into_utf8() {
        // \xe4\xbd\xa0 = 你, \x0a = newline, \xe5\xa5\xbd = 好
        assert_eq!(
            super::decode_js_string(r"\xe4\xbd\xa0\x0a\xe5\xa5\xbd"),
            "你\n好"
        );
        // A malformed escape is dropped instead of becoming a NUL byte.
        assert!(!super::decode_js_string(r"a\xzzb").contains('\u{0}'));
    }

    #[test]
    fn falls_back_to_open_graph_and_normalizes_literal_newlines() {
        // Datacenter/blocked variant: only `og:` meta with literal `\n` escapes
        // and the whole caption stuffed into og:title.
        let html = r#"
            <html><head>
              <meta property="og:title" content="外企真的在撤离中国吗？\n\n如果认真看了近两年的财报，会发现并不是这样的。\n很多外企撤掉的，其实是利润低的传统业务。" />
              <meta property="og:image" content="https://mmbiz.qpic.cn/cover.jpg" />
            </head><body><div id="page_content"></div></body></html>
        "#;
        let mut metadata = wechat_metadata();
        assert!(merge(&mut metadata, html));
        assert_eq!(metadata.title, "外企真的在撤离中国吗？");
        assert!(!metadata.title.contains("\\n"));
        assert!(!metadata.content_text.contains("\\n"));
        assert!(metadata.content_text.contains("如果认真看了近两年的财报"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://mmbiz.qpic.cn/cover.jpg")
        );
    }

    #[test]
    fn reports_deleted_article() {
        let html = r#"<html><body><div class="weui-msg__title warn">该内容已被发布者删除</div></body></html>"#;
        let mut metadata = wechat_metadata();
        assert!(merge(&mut metadata, html));
        assert!(metadata.content_text.contains("已被发布者删除"));
    }

    #[test]
    fn skips_non_article_pages() {
        let html =
            r#"<html><body><div id="verify">环境异常，完成验证后即可继续访问</div></body></html>"#;
        let mut metadata = wechat_metadata();
        assert!(!merge(&mut metadata, html));
    }
}
