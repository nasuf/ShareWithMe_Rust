use reqwest::Client;
use scraper::{Html, Selector};

use crate::{content::OriginalContent, models::LinkMetadata};

use super::{
    classify::{clean_original_text, clean_text, truncate_chars},
    metadata_json::{
        json_ld_values, json_values_after_markers, json_values_from_script_id, jsonp_value,
        platform_metadata_from_values, weibo_metadata_from_api_value, weibo_status_id,
        zhihu_answer_id, zhihu_metadata_from_api_value,
    },
};

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

pub(super) fn merge_platform_state_metadata(metadata: &mut LinkMetadata, html: &str) -> bool {
    let platform_metadata = match metadata.platform.as_str() {
        "抖音" => extract_douyin_metadata(html),
        "哔哩哔哩" => extract_bilibili_metadata(html),
        "YouTube" => extract_youtube_metadata(html),
        "知乎" => extract_zhihu_metadata(html),
        "微博" => extract_weibo_metadata(html),
        "Instagram" => extract_instagram_metadata(html),
        _ => None,
    };

    let Some(platform_metadata) = platform_metadata.filter(PlatformMetadata::is_meaningful) else {
        return false;
    };
    merge_platform_metadata(metadata, platform_metadata);
    true
}

pub(super) fn merge_platform_metadata(
    metadata: &mut LinkMetadata,
    platform_metadata: PlatformMetadata,
) {
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
        metadata.original_content = OriginalContent::from_plain_text(&metadata.original_text);
    } else if metadata.original_text.trim().is_empty() && !metadata.content_text.trim().is_empty() {
        metadata.original_text = metadata.content_text.clone();
    }
}

pub(super) async fn platform_api_metadata(
    metadata: &LinkMetadata,
    http: &Client,
) -> Option<PlatformMetadata> {
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

fn extract_douyin_metadata(html: &str) -> Option<PlatformMetadata> {
    if let Some(metadata) = douyin_json_ld_metadata(html) {
        return Some(metadata);
    }
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

fn douyin_json_ld_metadata(html: &str) -> Option<PlatformMetadata> {
    let values = json_ld_values(html);
    let video = values.iter().find_map(find_video_object)?;
    let title = video
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(clean_text);
    let description = video
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(clean_original_text);
    let author = video
        .get("creator")
        .and_then(|creator| creator.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(clean_text);
    let image_url = video
        .get("thumbnailUrl")
        .and_then(first_string)
        .map(str::to_string);
    let result = PlatformMetadata {
        title,
        description: description.clone(),
        author,
        image_url,
        original_text: description,
    };
    result.is_meaningful().then_some(result)
}

fn find_video_object(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("@type").and_then(serde_json::Value::as_str) == Some("VideoObject") {
                return Some(value);
            }
            map.values().find_map(find_video_object)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_video_object),
        _ => None,
    }
}

fn first_string(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Array(values) => values.iter().find_map(first_string),
        _ => None,
    }
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

fn extract_instagram_metadata(html: &str) -> Option<PlatformMetadata> {
    let document = Html::parse_document(html);
    let og_title = meta_content(&document, "meta[property='og:title']");
    let og_description = meta_content(&document, "meta[property='og:description']");
    let caption = og_description
        .as_deref()
        .and_then(instagram_caption)
        .filter(|value| !value.is_empty())?;
    let author = og_title
        .as_deref()
        .and_then(instagram_author)
        .or_else(|| og_description.as_deref().and_then(instagram_author));
    let image_url = meta_content(&document, "meta[property='og:image']");
    let result = PlatformMetadata {
        title: Some(truncate_chars(&caption, 100)),
        description: Some(caption.clone()),
        author,
        image_url,
        original_text: Some(caption),
    };
    result.is_meaningful().then_some(result)
}

fn instagram_caption(description: &str) -> Option<String> {
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

fn instagram_author(value: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::extract_douyin_metadata;

    #[test]
    fn reads_douyin_video_object_without_confusing_publisher_and_creator() {
        let metadata = extract_douyin_metadata(
            r#"<script type="application/ld+json">{
                "@type":"VideoObject",
                "name":"韭菜盒子 - 抖音",
                "description":"家常美食记录",
                "thumbnailUrl":["https://example.com/cover.jpg"],
                "creator":{"@type":"Person","name":"吃货自然萌"},
                "publisher":{"@type":"Organization","name":"抖音"}
            }</script>"#,
        )
        .expect("video metadata");

        assert_eq!(metadata.title.as_deref(), Some("韭菜盒子 - 抖音"));
        assert_eq!(metadata.author.as_deref(), Some("吃货自然萌"));
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://example.com/cover.jpg")
        );
    }
}
