use chrono::Utc;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use crate::{
    content::{ExtractionState, MediaAsset, OriginalContent, SourceFact, snapshot_fingerprint},
    models::LinkMetadata,
    platforms::{profile_for, source_identity},
    render::{RenderedPage, render_extract},
};

use super::adapters::{
    enrich_from_html, enrich_remote, merge_wechat_metadata, merge_xiaohongshu_metadata,
};
use super::classify::{
    clean_original_text, clean_text, first_meaningful_line, first_non_empty,
    is_blocked_without_content, is_url_only_text, is_xiaohongshu_host, plain_text_preview,
    platform_for_url,
};
use super::document::document_from_html;
use super::platform_metadata::{
    merge_platform_metadata, merge_platform_state_metadata, platform_api_metadata,
};

pub(crate) async fn extract_metadata(
    http: &Client,
    source_url: &str,
    shared_text: Option<&str>,
) -> LinkMetadata {
    extract_metadata_with_video_transcription(http, source_url, shared_text, true).await
}

pub(crate) async fn extract_metadata_with_video_transcription(
    http: &Client,
    source_url: &str,
    shared_text: Option<&str>,
    allow_video_transcription: bool,
) -> LinkMetadata {
    let fetched_at = Utc::now();
    let shared_text = shared_text.unwrap_or_default().trim().to_string();
    let mut metadata = LinkMetadata {
        source_url: source_url.to_string(),
        final_url: source_url.to_string(),
        platform: platform_for_url(source_url),
        title: first_meaningful_line(&shared_text).unwrap_or_else(|| "未命名链接".to_string()),
        description: shared_text.clone(),
        author: None,
        image_url: None,
        content_text: shared_text.clone(),
        original_text: shared_text.clone(),
        shared_text: shared_text.clone(),
        original_content: OriginalContent::from_plain_text(shared_text),
        media: Vec::new(),
        source_facts: Vec::new(),
        source_id: None,
        published_at: None,
        extraction: ExtractionState {
            status: "partial".to_string(),
            adapter: profile_for(&platform_for_url(source_url))
                .adapter
                .to_string(),
            method: "shared_text".to_string(),
            completeness: 20,
            fetched_at: Some(fetched_at),
            refresh_after: None,
            content_fingerprint: None,
            last_error: None,
        },
    };

    match fetch_page(http, source_url).await {
        Ok(response) => {
            metadata.final_url = response.url().to_string();
            metadata.platform = platform_for_url(&metadata.final_url);
            // Xiaohongshu sometimes still bounces to its login wall; the note
            // URL (with its xsec_token) is kept in `redirectPath`. Fetch it
            // directly and, if that fails too, at least keep the note URL so
            // the item gets the right identity instead of the login page's.
            let mut stuck_on_login = false;
            let response = match xiaohongshu_login_target(&metadata.final_url) {
                Some(target) => {
                    metadata.final_url = target.clone();
                    match fetch_page(http, &target).await {
                        Ok(retry) if xiaohongshu_login_target(retry.url().as_str()).is_none() => {
                            metadata.final_url = retry.url().to_string();
                            retry
                        }
                        _ => {
                            stuck_on_login = true;
                            response
                        }
                    }
                }
                None => response,
            };
            metadata.platform = platform_for_url(&metadata.final_url);
            let status = response.status();
            match response.text().await {
                // The login wall is a JS shell with no note data; parsing it
                // would save "Log in - rednote" as the item. Keep the share text.
                Ok(_) if stuck_on_login => {
                    let reason = "小红书只返回了登录页面，未取得笔记正文，已保存分享文字。";
                    metadata.extraction.method = "login-wall".to_string();
                    metadata.extraction.last_error = Some(reason.to_string());
                }
                // Expired share token or hidden note: /404/sec_... page.
                Ok(_) if is_xiaohongshu_dead_end(&metadata.final_url) => {
                    let reason =
                        "小红书分享链接已失效或笔记不可见，未取得笔记正文，已保存分享文字。";
                    metadata.extraction.method = "link-expired".to_string();
                    metadata.extraction.last_error = Some(reason.to_string());
                }
                Ok(html) if status.is_success() => {
                    merge_html_metadata(&mut metadata, &html);
                    enrich_from_html(&mut metadata, &html);
                }
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
        Err(error) => {
            metadata.extraction.last_error = Some(format!("请求页面失败: {error}"));
            metadata.notes_text_append(format!("请求页面失败: {error}"));
        }
    }
    update_source_identity(&mut metadata);

    if let Some(api_metadata) = platform_api_metadata(&metadata, http).await {
        merge_platform_metadata(&mut metadata, api_metadata);
        metadata.extraction.method = "platform-api".to_string();
    }
    if should_try_rendered_extraction(&metadata)
        && let Some(rendered) = render_extract(source_url).await
    {
        merge_rendered_metadata(&mut metadata, rendered);
    }
    enrich_remote(&mut metadata, http, allow_video_transcription).await;
    sanitize_platform_shell(&mut metadata);

    metadata.title = clean_text(&metadata.title);
    metadata.description = clean_text(&metadata.description);
    metadata.content_text = clean_text(&metadata.content_text);
    metadata.original_text = clean_original_text(&metadata.original_text);
    if metadata.original_text.is_empty() && !metadata.content_text.is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
    finalize_metadata(&mut metadata, fetched_at);
    metadata
}

// WeChat serves an "环境异常" verification wall to non-browser user agents.
// A mobile Safari UA returns the real article HTML (server-rendered `#js_content`
// or `window.cgiData`), so the direct path can work without a headless browser.
// Xiaohongshu redirects non-mobile user agents (including our default one) to
// its login wall, while mobile Safari gets the server-rendered note page.
fn fetch_user_agent(source_url: &str) -> Option<&'static str> {
    let host = Url::parse(source_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    (source_url.contains("mp.weixin.qq.com") || is_xiaohongshu_host(&host)).then_some(
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 \
         (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
    )
}

async fn fetch_page(http: &Client, url: &str) -> reqwest::Result<reqwest::Response> {
    let mut request = http.get(url);
    if let Some(user_agent) = fetch_user_agent(url) {
        request = request.header("user-agent", user_agent);
    }
    request.send().await
}

// `https://www.xiaohongshu.com/login?redirectPath=<note url>` (or the rednote.com
// equivalent) -> the note URL, when it points at a note.
pub(super) fn is_xiaohongshu_dead_end(raw_url: &str) -> bool {
    Url::parse(raw_url).is_ok_and(|url| {
        url.host_str().is_some_and(is_xiaohongshu_host) && url.path().starts_with("/404")
    })
}

pub(super) fn xiaohongshu_login_target(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    if !is_xiaohongshu_host(url.host_str()?) || !url.path().starts_with("/login") {
        return None;
    }
    let target = url
        .query_pairs()
        .find(|(key, _)| key == "redirectPath")
        .map(|(_, value)| value.into_owned())?;
    let mut target = Url::parse(&target).ok()?;
    let is_note = target.host_str().is_some_and(is_xiaohongshu_host)
        && (target.path().starts_with("/discovery/item/")
            || target.path().starts_with("/explore/"));
    if !is_note {
        return None;
    }
    let _ = target.set_scheme("https");
    Some(target.into())
}

pub(super) fn sanitize_platform_shell(metadata: &mut LinkMetadata) {
    if metadata.extraction.method == "platform-api" || metadata.extraction.method == "yt-dlp" {
        return;
    }
    let text = metadata.original_content.plain_text.as_str();
    let reason = match metadata.platform.as_str() {
        "微博"
            if text.contains("微博热搜")
                && (text.contains("登录/注册") || text.contains("该微博不存在")) =>
        {
            Some("微博只返回了访客/不存在页面，未取得这条微博的正文。")
        }
        "抖音"
            if text.contains("登录后免费畅享高清视频")
                || (text.contains("扫码登录") && text.contains("验证码登录")) =>
        {
            Some("抖音只返回了登录页面，未取得视频说明或字幕。")
        }
        "Instagram"
            if text.contains("Log in")
                && (text.contains("Sign up") || text.contains("Instagram")) =>
        {
            Some("Instagram 只返回了登录页面，未取得帖子正文。")
        }
        // Douban bounces non-browser clients to sec.douban.com; that page's
        // only content is the site name. Reached only if the API also failed.
        "豆瓣" if metadata.final_url.contains("sec.douban.com") => {
            Some("豆瓣返回了安全验证页面，条目接口也未返回内容，已保存分享文字。")
        }
        _ => None,
    };
    let Some(reason) = reason else {
        return;
    };
    if metadata.platform == "豆瓣" && metadata.title.trim() == "豆瓣" {
        metadata.title = "豆瓣条目".to_string();
    }

    let shared_text = metadata.shared_text.trim();
    let title_fallback = (metadata.platform == "抖音"
        && metadata.title != "未命名链接"
        && !metadata.title.contains("网页验证拦截"))
    .then(|| {
        metadata
            .title
            .trim_end_matches(" - 抖音")
            .trim()
            .to_string()
    });
    let fallback = if !shared_text.is_empty() && !is_url_only_text(shared_text) {
        Some(shared_text.to_string())
    } else {
        title_fallback
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

fn finalize_metadata(metadata: &mut LinkMetadata, fetched_at: chrono::DateTime<Utc>) {
    let identity = preferred_source_identity(metadata);
    metadata.final_url = identity.canonical_url;
    metadata.source_id = identity.source_id;

    if metadata.original_content.is_empty() {
        metadata.original_content = OriginalContent::from_plain_text(&metadata.original_text);
    }
    if metadata.original_text.trim().is_empty() {
        metadata.original_text = metadata.original_content.plain_text.clone();
    }
    if let Some(image_url) = metadata.image_url.clone()
        && !metadata.media.iter().any(|asset| asset.url == image_url)
    {
        metadata.media.insert(
            0,
            MediaAsset {
                kind: "image".to_string(),
                url: image_url,
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            },
        );
    }

    let profile = profile_for(&metadata.platform);
    metadata.extraction.adapter = profile.adapter.to_string();
    metadata.extraction.fetched_at = Some(fetched_at);
    metadata.extraction.refresh_after = Some(profile.refresh_after(fetched_at));
    mark_missing_content_requirement(metadata);
    let meaningful_text = metadata.original_content.plain_text.trim();
    metadata.extraction.content_fingerprint = snapshot_fingerprint(
        &metadata.title,
        metadata.author.as_deref(),
        meaningful_text,
        &metadata.source_facts,
    );
    let source_specific = matches!(
        metadata.extraction.method.as_str(),
        "embedded-state"
            | "shared_text"
            | "platform-api"
            | "official-api"
            | "public-preview"
            | "oembed"
            | "yt-dlp"
            | "xhs-initial-state"
    );
    let short_snapshot_complete = source_specific
        && matches!(
            profile.content_kind,
            "post" | "video" | "product" | "place" | "book" | "review"
        )
        && !meaningful_text.is_empty()
        && metadata.title != "未命名链接"
        && (metadata.extraction.method == "shared_text"
            || metadata.author.is_some()
            || metadata.image_url.is_some()
            || metadata.source_facts.len() >= 2);

    if is_blocked_without_content(metadata) {
        metadata.extraction.status = "blocked".to_string();
        metadata.extraction.completeness = if metadata.shared_text.chars().count() >= 80 {
            40
        } else {
            15
        };
    } else if meaningful_text.chars().count() >= 120
        || metadata.original_content.blocks.len() >= 2
        || short_snapshot_complete
    {
        metadata.extraction.status = "complete".to_string();
        metadata.extraction.completeness = 80
            + u8::from(metadata.author.is_some()) * 5
            + u8::from(metadata.image_url.is_some()) * 5;
        if metadata.extraction.method != "blocked-page" {
            metadata.extraction.last_error = None;
        }
        metadata.extraction.refresh_after = Some(crate::content::settled_refresh_after(fetched_at));
    } else {
        metadata.extraction.status = "partial".to_string();
        metadata.extraction.completeness = if meaningful_text.is_empty() { 20 } else { 50 };
    }

    push_source_fact(
        &mut metadata.source_facts,
        "content_kind",
        "内容类型",
        profile.content_kind,
    );
}

fn mark_missing_content_requirement(metadata: &mut LinkMetadata) {
    if !metadata.original_content.plain_text.trim().is_empty() {
        return;
    }
    let requirement = match metadata.platform.as_str() {
        "淘宝" | "天猫" => {
            Some("请保留 App 分享面板中的商品标题/规格文字；仅凭网页 URL 常会触发淘宝登录或风控。")
        }
        "百度地图" => Some(
            "当前链接只公开了地点名称；配置 BAIDU_MAPS_AK 后可补全地址、坐标、电话和营业信息。",
        ),
        "Instagram" => Some("该帖子可能是私密、已删除或受地区限制，公开页面未返回 caption。"),
        _ => None,
    };
    let Some(requirement) = requirement else {
        return;
    };
    if metadata.extraction.last_error.is_none() {
        metadata.extraction.last_error = Some(requirement.to_string());
    }
    if metadata.extraction.method == "html" {
        metadata.extraction.method = "metadata-only".to_string();
    }
    push_source_fact(
        &mut metadata.source_facts,
        "completion_requirement",
        "补全方式",
        requirement,
    );
}

fn update_source_identity(metadata: &mut LinkMetadata) {
    metadata.source_id = preferred_source_identity(metadata).source_id;
}

fn preferred_source_identity(metadata: &LinkMetadata) -> crate::platforms::SourceIdentity {
    let final_identity = source_identity(&metadata.final_url, &metadata.platform);
    if final_identity.source_id.is_some() {
        return final_identity;
    }
    let source_identity = source_identity(&metadata.source_url, &metadata.platform);
    if source_identity.source_id.is_some() {
        source_identity
    } else {
        final_identity
    }
}

fn push_source_fact(facts: &mut Vec<SourceFact>, key: &str, label: &str, value: &str) {
    if value.trim().is_empty() || facts.iter().any(|fact| fact.key == key) {
        return;
    }
    facts.push(SourceFact {
        key: key.to_string(),
        label: label.to_string(),
        value: value.to_string(),
    });
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
        metadata.extraction.method = "embedded-state".to_string();
        return;
    }
    if merge_wechat_metadata(metadata, html) {
        metadata.extraction.method = "wechat-article".to_string();
        return;
    }
    if merge_platform_state_metadata(metadata, html) {
        metadata.extraction.method = "embedded-state".to_string();
        return;
    }

    if let Some(reason) = access_block_reason(html, &metadata.platform) {
        if metadata.title.trim().is_empty() || metadata.title == "未命名链接" {
            metadata.title = format!("{}链接（网页验证拦截）", metadata.platform);
        }
        if metadata.description.trim().is_empty() || is_url_only_text(&metadata.description) {
            metadata.description = reason.clone();
        } else {
            metadata.notes_text_append(reason.clone());
        }
        if metadata.content_text.trim().is_empty() || is_url_only_text(&metadata.content_text) {
            metadata.content_text = reason.clone();
        }
        if metadata.original_text.trim().is_empty() || is_url_only_text(&metadata.original_text) {
            metadata.original_text = reason.clone();
        }
        metadata.extraction.method = "blocked-page".to_string();
        metadata.extraction.last_error = Some(reason);
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

    let (original_content, media) = document_from_html(&document, &metadata.final_url);
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
        if !metadata
            .media
            .iter()
            .any(|existing| existing.url == asset.url)
        {
            metadata.media.push(asset);
        }
    }
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

fn access_block_reason(html: &str, platform: &str) -> Option<String> {
    let lower = html.to_lowercase();
    if lower.contains("byted_acrawler")
        || lower.contains("__ac_signature")
        || lower.contains("__ac_nonce")
    {
        return Some(match platform {
            "抖音" => "抖音返回了浏览器验证页面，正在尝试视频元数据和字幕兜底。",
            _ => "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。",
        }
        .to_string());
    }
    if lower.contains("sina visitor system") || lower.contains("visitor.passport.weibo") {
        return Some(
            "微博返回了访客验证页面，后端无法从普通网页直接抓取正文，正在尝试平台接口兜底。"
                .to_string(),
        );
    }
    if platform == "微信公众号"
        && (html.contains("环境异常") || html.contains("去验证") || html.contains("完成验证"))
    {
        return Some(
            "微信公众号返回了环境异常验证页面，正在尝试浏览器渲染兜底；若仍失败可从原文分享带正文的文本。"
                .to_string(),
        );
    }
    if lower.contains("zh-zse-ck") || lower.contains("zse-ck") {
        return Some(
            "知乎返回了访问验证页面，后端无法从普通网页直接抓取正文，正在尝试回答接口兜底。"
                .to_string(),
        );
    }
    if lower.contains("captcha") || lower.contains("访问验证") || lower.contains("安全验证")
    {
        return Some(
            "目标网页返回了访问验证页面，后端无法直接抓取正文。建议从原 App 分享带标题的文本，或手动补充标题/摘要后再整理。"
                .to_string(),
        );
    }
    None
}

pub(super) fn should_try_rendered_extraction(metadata: &LinkMetadata) -> bool {
    // A login wall served to a mobile UA is served to the browser too; rendering
    // it would only replace the share text with the login shell.
    if matches!(
        metadata.extraction.method.as_str(),
        "login-wall" | "link-expired"
    ) {
        return false;
    }
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
    // A WeChat article that already carries a definitive "deleted/unavailable"
    // reason must not be re-fetched by the browser (that would overwrite it).
    if metadata.platform == "微信公众号" && metadata.extraction.last_error.is_some() {
        return false;
    }
    matches!(
        metadata.platform.as_str(),
        "今日头条" | "小红书" | "微信公众号"
    ) && text_len < 120
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
    let (content, media) = super::document::document_from_fragment(html, &metadata.final_url);
    let rendered_chars = compact_len(rendered_text);
    let block_chars = compact_len(&content.plain_text);
    if content.blocks.is_empty() || block_chars * 10 < rendered_chars * 7 {
        return;
    }
    metadata.original_content = content;
    for asset in media {
        if !metadata
            .media
            .iter()
            .any(|existing| existing.url == asset.url)
        {
            metadata.media.push(asset);
        }
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
