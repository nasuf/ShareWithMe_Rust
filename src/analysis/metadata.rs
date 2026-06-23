use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use crate::{
    models::LinkMetadata,
    render::{RenderedPage, render_extract},
};

use super::classify::{
    clean_original_text, clean_text, first_meaningful_line, first_non_empty,
    is_blocked_without_content, is_url_only_text, plain_text_preview, platform_for_url,
    truncate_chars,
};
use super::metadata_json::{
    json_ld_values, json_values_after_markers, json_values_from_script_id, jsonp_value,
    platform_metadata_from_values, weibo_metadata_from_api_value, weibo_status_id, zhihu_answer_id,
    zhihu_metadata_from_api_value,
};

pub(crate) async fn extract_metadata(
    http: &Client,
    source_url: &str,
    shared_text: Option<&str>,
) -> LinkMetadata {
    let mut metadata = LinkMetadata {
        source_url: source_url.to_string(),
        final_url: source_url.to_string(),
        platform: platform_for_url(source_url),
        title: shared_text
            .and_then(first_meaningful_line)
            .unwrap_or_else(|| "未命名链接".to_string()),
        description: shared_text.unwrap_or_default().trim().to_string(),
        author: None,
        image_url: None,
        content_text: shared_text.unwrap_or_default().trim().to_string(),
        original_text: shared_text.unwrap_or_default().trim().to_string(),
    };

    match http.get(source_url).send().await {
        Ok(response) => {
            metadata.final_url = response.url().to_string();
            metadata.platform = platform_for_url(&metadata.final_url);
            let status = response.status();
            match response.text().await {
                Ok(html) if status.is_success() => merge_html_metadata(&mut metadata, &html),
                Ok(body) => {
                    metadata
                        .notes_text_append(format!("页面返回 HTTP {status}; 已保存可见分享文本。"));
                    if metadata.content_text.is_empty() {
                        metadata.content_text = plain_text_preview(&body);
                    }
                }
                Err(error) => metadata.notes_text_append(format!("读取页面失败: {error}")),
            }
        }
        Err(error) => metadata.notes_text_append(format!("请求页面失败: {error}")),
    }

    if should_try_rendered_extraction(&metadata)
        && let Some(rendered) = render_extract(source_url).await
    {
        merge_rendered_metadata(&mut metadata, rendered);
    }

    if let Some(api_metadata) = platform_api_metadata(&metadata, http).await {
        merge_platform_metadata(&mut metadata, api_metadata);
    }

    metadata.title = clean_text(&metadata.title);
    metadata.description = clean_text(&metadata.description);
    metadata.content_text = clean_text(&metadata.content_text);
    metadata.original_text = clean_original_text(&metadata.original_text);
    if metadata.original_text.is_empty() && !metadata.content_text.is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
    metadata
}

trait MetadataNotes {
    fn notes_text_append(&mut self, text: String);
}

impl MetadataNotes for LinkMetadata {
    fn notes_text_append(&mut self, text: String) {
        if self.description.is_empty() {
            self.description = text;
        } else if !self.description.contains(&text) {
            self.description = format!("{}\n{}", self.description, text);
        }
    }
}

pub(super) fn merge_html_metadata(metadata: &mut LinkMetadata, html: &str) {
    if merge_xiaohongshu_metadata(metadata, html) {
        return;
    }
    if merge_platform_state_metadata(metadata, html) {
        return;
    }

    if let Some(reason) = access_block_reason(html) {
        if metadata.title.trim().is_empty() || metadata.title == "未命名链接" {
            metadata.title = format!("{}链接（网页验证拦截）", metadata.platform);
        }
        if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
            metadata.description = reason.to_string();
        } else {
            metadata.notes_text_append(reason.to_string());
        }
        if metadata.content_text.trim().is_empty() || is_url_only_text(&metadata.content_text) {
            metadata.content_text = reason.to_string();
        }
        if metadata.original_text.trim().is_empty() || is_url_only_text(&metadata.original_text) {
            metadata.original_text = reason.to_string();
        }
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
    .unwrap_or_else(|| "未命名链接".to_string());
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

    let extracted_text = extract_visible_text(&document);
    if !extracted_text.is_empty() {
        metadata.content_text = extracted_text;
        metadata.original_text = extract_formatted_visible_text(&document);
    }
}

fn merge_xiaohongshu_metadata(metadata: &mut LinkMetadata, html: &str) -> bool {
    if metadata.platform != "小红书" && !metadata.final_url.contains("xiaohongshu.com") {
        return false;
    }

    if html.contains("当前笔记暂时无法浏览") || html.contains("你访问的页面不见了")
    {
        let reason = "小红书返回了笔记不可浏览页面，后端无法抓取该笔记正文。请换一个可公开访问的分享链接，或从原 App 分享带标题/正文的文本。";
        if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
            metadata.description = reason.to_string();
        } else {
            metadata.notes_text_append(reason.to_string());
        }
        if metadata.content_text.trim().is_empty() || is_url_only_text(&metadata.content_text) {
            metadata.content_text = reason.to_string();
        }
        if metadata.original_text.trim().is_empty() || is_url_only_text(&metadata.original_text) {
            metadata.original_text = reason.to_string();
        }
        return true;
    }

    let Some(section) = xiaohongshu_note_section(html, &metadata.final_url) else {
        return false;
    };

    let title =
        json_string_field(section, "title").or_else(|| json_string_field(section, "displayTitle"));
    let description = json_string_field(section, "desc");
    let original_description = json_raw_string_field(section, "desc");
    let author =
        json_string_field(section, "nickname").or_else(|| json_string_field(section, "nickName"));
    let image_url =
        json_string_field(section, "urlDefault").or_else(|| json_string_field(section, "urlPre"));

    if title.is_none() && description.is_none() {
        return false;
    }

    if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
        metadata.title = title;
    }
    if let Some(description) = description.filter(|value| !value.trim().is_empty()) {
        metadata.description = clean_text(&description);
        metadata.content_text = clean_text(&format!("{}\n\n{}", metadata.title, description));
        let original = original_description
            .as_deref()
            .map(clean_original_text)
            .unwrap_or_else(|| clean_original_text(&description));
        metadata.original_text = if original.is_empty() {
            metadata.content_text.clone()
        } else {
            original
        };
    }
    if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
        metadata.author = Some(author);
    }
    if let Some(image_url) = image_url.filter(|value| !value.trim().is_empty()) {
        metadata.image_url = Some(image_url);
    }
    true
}

#[derive(Debug, Default)]
pub(super) struct PlatformMetadata {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) author: Option<String>,
    pub(super) image_url: Option<String>,
    pub(super) original_text: Option<String>,
}

impl PlatformMetadata {
    pub(super) fn is_meaningful(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.image_url.is_some()
            || self.original_text.is_some()
    }
}

fn merge_platform_state_metadata(metadata: &mut LinkMetadata, html: &str) -> bool {
    let platform_metadata = match metadata.platform.as_str() {
        "抖音" => extract_douyin_metadata(html),
        "哔哩哔哩" => extract_bilibili_metadata(html),
        "YouTube" => extract_youtube_metadata(html),
        "知乎" => extract_zhihu_metadata(html),
        "微博" => extract_weibo_metadata(html),
        _ => None,
    };

    let Some(platform_metadata) = platform_metadata.filter(PlatformMetadata::is_meaningful) else {
        return false;
    };

    merge_platform_metadata(metadata, platform_metadata);
    true
}

fn merge_platform_metadata(metadata: &mut LinkMetadata, platform_metadata: PlatformMetadata) {
    if let Some(title) = platform_metadata
        .title
        .filter(|value| !value.trim().is_empty())
    {
        metadata.title = title;
    } else if metadata.title == "未命名链接"
        && let Some(description) = platform_metadata
            .description
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    {
        metadata.title = truncate_chars(description, 80);
    }

    if let Some(description) = platform_metadata
        .description
        .filter(|value| !value.trim().is_empty())
    {
        metadata.description = description.clone();
        metadata.content_text = clean_text(&format!("{}\n\n{}", metadata.title, description));
    } else if metadata.content_text.trim().is_empty() && metadata.title != "未命名链接" {
        metadata.content_text = metadata.title.clone();
    }

    if let Some(author) = platform_metadata
        .author
        .filter(|value| !value.trim().is_empty())
    {
        metadata.author = Some(author);
    }
    if let Some(image_url) = platform_metadata
        .image_url
        .filter(|value| !value.trim().is_empty())
    {
        metadata.image_url = Some(image_url);
    }
    if let Some(original_text) = platform_metadata
        .original_text
        .filter(|value| !value.trim().is_empty())
    {
        metadata.original_text = clean_original_text(&original_text);
    } else if metadata.original_text.trim().is_empty() && !metadata.content_text.trim().is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
}

fn extract_douyin_metadata(html: &str) -> Option<PlatformMetadata> {
    let mut candidates = json_values_from_script_id(html, "RENDER_DATA");
    candidates.extend(json_values_after_markers(
        html,
        &[
            "window.__INIT_PROPS__=",
            "window._ROUTER_DATA=",
            "window.__data=",
        ],
    ));
    candidates.extend(json_ld_values(html));
    platform_metadata_from_values(
        candidates,
        &["title", "desc", "description"],
        &["desc", "description", "seoDescription"],
        &["nickname", "authorName", "name"],
        &["cover", "poster", "thumbnailUrl", "urlDefault", "url"],
    )
}

fn extract_bilibili_metadata(html: &str) -> Option<PlatformMetadata> {
    let mut candidates =
        json_values_after_markers(html, &["window.__INITIAL_STATE__=", "window.__playinfo__="]);
    candidates.extend(json_ld_values(html));
    platform_metadata_from_values(
        candidates,
        &["title", "name"],
        &["desc", "description"],
        &["owner", "author", "name"],
        &["pic", "cover", "thumbnailUrl", "image"],
    )
}

fn extract_youtube_metadata(html: &str) -> Option<PlatformMetadata> {
    let mut candidates = json_ld_values(html);
    candidates.extend(json_values_after_markers(
        html,
        &["var ytInitialPlayerResponse =", "ytInitialPlayerResponse ="],
    ));
    platform_metadata_from_values(
        candidates,
        &["title", "name", "headline"],
        &["shortDescription", "description"],
        &["author", "ownerChannelName", "name"],
        &["thumbnailUrl", "thumbnail", "image", "url"],
    )
}

fn extract_zhihu_metadata(html: &str) -> Option<PlatformMetadata> {
    let mut candidates = json_values_from_script_id(html, "js-initialData");
    candidates.extend(json_ld_values(html));
    platform_metadata_from_values(
        candidates,
        &["title", "questionTitle", "headline"],
        &["excerpt", "description", "content", "text"],
        &["author", "name", "headline"],
        &["thumbnail", "image", "avatarUrl", "url"],
    )
}

fn extract_weibo_metadata(html: &str) -> Option<PlatformMetadata> {
    let mut candidates = json_ld_values(html);
    candidates.extend(json_values_after_markers(
        html,
        &[
            "window.__INITIAL_STATE__=",
            "$render_data =",
            "render_data =",
        ],
    ));
    platform_metadata_from_values(
        candidates,
        &[
            "title",
            "headline",
            "page_title",
            "text_raw",
            "status_title",
        ],
        &["text_raw", "articleBody", "description", "text", "content"],
        &["screen_name", "author", "name"],
        &["pic", "pics", "thumbnailUrl", "image", "url"],
    )
}

async fn platform_api_metadata(metadata: &LinkMetadata, http: &Client) -> Option<PlatformMetadata> {
    match metadata.platform.as_str() {
        "知乎" => {
            let from_source = fetch_zhihu_answer_metadata(http, &metadata.source_url).await;
            if from_source.is_some() {
                from_source
            } else {
                fetch_zhihu_answer_metadata(http, &metadata.final_url).await
            }
        }
        "微博" => {
            let from_source = fetch_weibo_status_metadata(http, &metadata.source_url).await;
            if from_source.is_some() {
                from_source
            } else {
                fetch_weibo_status_metadata(http, &metadata.final_url).await
            }
        }
        _ => None,
    }
}

async fn fetch_zhihu_answer_metadata(http: &Client, raw_url: &str) -> Option<PlatformMetadata> {
    let answer_id = zhihu_answer_id(raw_url)?;
    let api_url = format!(
        "https://www.zhihu.com/api/v4/answers/{answer_id}?include=content,excerpt,question,author"
    );
    let value = http
        .get(api_url)
        .header(
            "user-agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148",
        )
        .header("accept", "application/json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    zhihu_metadata_from_api_value(&value)
}

async fn fetch_weibo_status_metadata(http: &Client, raw_url: &str) -> Option<PlatformMetadata> {
    let status_id = weibo_status_id(raw_url)?;
    let (sub, subp) = weibo_visitor_tokens(http).await?;
    let api_url = format!("https://weibo.com/ajax/statuses/show?id={status_id}");
    let cookie = format!("SUB={sub}; SUBP={subp}");
    let value = http
        .get(api_url)
        .header("user-agent", "Mozilla/5.0")
        .header("referer", "https://weibo.com/")
        .header("accept", "application/json")
        .header("cookie", cookie)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    weibo_metadata_from_api_value(&value)
}

async fn weibo_visitor_tokens(http: &Client) -> Option<(String, String)> {
    let visitor_url = "https://passport.weibo.com/visitor/genvisitor?cb=gen_callback&fp=%7B%22os%22%3A%221%22%2C%22browser%22%3A%22Chrome%22%2C%22fonts%22%3A%22undefined%22%2C%22screenInfo%22%3A%221920*1080*24%22%2C%22plugins%22%3A%22%22%7D";
    let visitor_body = http
        .get(visitor_url)
        .header("user-agent", "Mozilla/5.0")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let visitor = jsonp_value(&visitor_body)?;
    let tid = visitor
        .get("data")
        .and_then(|data| data.get("tid"))
        .and_then(serde_json::Value::as_str)?;
    let incarnate_url = format!(
        "https://passport.weibo.com/visitor/visitor?a=incarnate&t={tid}&w=2&c=095&gc=&cb=cross_domain&from=weibo"
    );
    let incarnate_body = http
        .get(incarnate_url)
        .header("user-agent", "Mozilla/5.0")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let incarnate = jsonp_value(&incarnate_body)?;
    let data = incarnate.get("data")?;
    let sub = data.get("sub").and_then(serde_json::Value::as_str)?;
    let subp = data.get("subp").and_then(serde_json::Value::as_str)?;
    Some((sub.to_string(), subp.to_string()))
}

fn meta_content(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .find_map(|node| node.value().attr("content"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn select_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .map(|text| clean_text(&text))
        .filter(|value| !value.is_empty())
}

fn extract_visible_text(document: &Html) -> String {
    let mut chunks = Vec::new();
    for selector in ["article p", "main p", "p", "h1", "h2", "li"] {
        if let Ok(selector) = Selector::parse(selector) {
            for node in document.select(&selector).take(80) {
                let text = clean_text(&node.text().collect::<Vec<_>>().join(" "));
                if text.chars().count() >= 12 && !chunks.contains(&text) {
                    chunks.push(text);
                }
                if chunks.join("\n").chars().count() > 12_000 {
                    break;
                }
            }
        }
        if chunks.join("\n").chars().count() > 1_500 {
            break;
        }
    }
    chunks.join("\n")
}

fn extract_formatted_visible_text(document: &Html) -> String {
    let mut chunks = Vec::new();
    for selector in ["article p", "main p", "p", "h1", "h2", "li"] {
        if let Ok(selector) = Selector::parse(selector) {
            for node in document.select(&selector).take(120) {
                let text = clean_original_text(&node.text().collect::<Vec<_>>().join(" "));
                if text.chars().count() >= 12 && !chunks.contains(&text) {
                    chunks.push(text);
                }
                if chunks.join("\n\n").chars().count() > 12_000 {
                    break;
                }
            }
        }
        if chunks.join("\n\n").chars().count() > 2_000 {
            break;
        }
    }
    chunks.join("\n\n")
}

fn access_block_reason(html: &str) -> Option<&'static str> {
    let lower = html.to_lowercase();
    if lower.contains("byted_acrawler")
        || lower.contains("__ac_signature")
        || lower.contains("__ac_nonce")
    {
        return Some(
            "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。",
        );
    }
    if lower.contains("sina visitor system") || lower.contains("visitor.passport.weibo") {
        return Some(
            "微博返回了访客验证页面，后端无法从普通网页直接抓取正文，正在尝试平台接口兜底。",
        );
    }
    if lower.contains("zh-zse-ck") || lower.contains("zse-ck") {
        return Some(
            "知乎返回了访问验证页面，后端无法从普通网页直接抓取正文，正在尝试回答接口兜底。",
        );
    }
    if lower.contains("captcha") || lower.contains("访问验证") || lower.contains("安全验证")
    {
        return Some(
            "目标网页返回了访问验证页面，后端无法直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。",
        );
    }
    None
}

fn xiaohongshu_note_section<'a>(html: &'a str, final_url: &str) -> Option<&'a str> {
    let start = xiaohongshu_note_id(final_url)
        .and_then(|note_id| html.find(&format!("\"{note_id}\"")))
        .or_else(|| html.find("\"noteDetailMap\""))
        .or_else(|| html.find("\"noteCard\""))?;
    let end = (start + 120_000).min(html.len());
    html.get(start..end)
}

fn xiaohongshu_note_id(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let note_id = match segments.as_slice() {
        ["explore", id, ..] => Some(*id),
        ["discovery", "item", id, ..] => Some(*id),
        _ => None,
    }?;
    if note_id.is_empty() {
        None
    } else {
        Some(note_id.to_string())
    }
}

fn json_string_field(section: &str, key: &str) -> Option<String> {
    json_raw_string_field(section, key)
        .map(|value| clean_text(&value))
        .filter(|value| !value.is_empty())
}

fn json_raw_string_field(section: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
    let regex = Regex::new(&pattern).ok()?;
    let raw = regex.captures(section)?.get(1)?.as_str();
    serde_json::from_str::<String>(&format!("\"{raw}\""))
        .ok()
        .filter(|value| !value.is_empty())
}

pub(super) fn should_try_rendered_extraction(metadata: &LinkMetadata) -> bool {
    let text_len = metadata.content_text.chars().count();
    if is_blocked_without_content(metadata) {
        return true;
    }
    if metadata.platform == "小红书"
        && !metadata
            .content_text
            .contains("小红书返回了笔记不可浏览页面")
        && !metadata.content_text.trim().is_empty()
        && !is_url_only_text(&metadata.content_text)
    {
        return false;
    }
    matches!(metadata.platform.as_str(), "今日头条" | "小红书") && text_len < 120
}

pub(super) fn merge_rendered_metadata(metadata: &mut LinkMetadata, rendered: RenderedPage) {
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
        metadata.original_text = content_text;
    }
    let _ = rendered.extractor;
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| clean_text(&value))
        .filter(|value| !value.is_empty())
}
