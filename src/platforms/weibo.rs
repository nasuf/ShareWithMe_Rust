//! 微博 (Weibo): posts from the page state, or from the status API with a
//! visitor cookie when the page is the visitor-verification wall.

use regex::Regex;
use reqwest::Client;
use serde_json::Value;
use url::Url;

use crate::{
    extraction::{
        PlatformMetadata, json_ld_values, json_value_to_clean_string, json_value_to_image_url,
        json_value_to_original_string, json_values_after_markers, jsonp_value,
        platform_metadata_from_values,
    },
    text::{html_to_formatted_text, plain_text_preview, truncate_chars},
};

use super::identity::id_after;

pub(super) fn identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id =
        id_after(segments, &["status"]).or_else(|| segments.get(1).map(|id| (*id).to_string()))?;
    let author = segments.first().copied().unwrap_or("u");
    let host = if url
        .host_str()
        .is_some_and(|host| host.ends_with("weibo.cn"))
    {
        "weibo.cn"
    } else {
        "weibo.com"
    };
    Some((id.clone(), format!("https://{host}/{author}/{id}")))
}

pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
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

pub(super) fn shell_reason(page_text: &str) -> Option<&'static str> {
    (page_text.contains("微博热搜")
        && (page_text.contains("登录/注册") || page_text.contains("该微博不存在")))
    .then_some("微博只返回了访客/不存在页面，未取得这条微博的正文。")
}

/// `GET weibo.com/ajax/statuses/show` with a freshly issued visitor cookie.
pub(super) async fn fetch_status(http: &Client, raw_url: &str) -> Option<PlatformMetadata> {
    let status_id = status_id(raw_url)?;
    let (sub, subp) = visitor_tokens(http).await?;
    let api_url = format!("https://weibo.com/ajax/statuses/show?id={status_id}");
    let value = http
        .get(api_url)
        .header("user-agent", "Mozilla/5.0")
        .header("referer", "https://weibo.com/")
        .header("accept", "application/json")
        .header("cookie", format!("SUB={sub}; SUBP={subp}"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    status_from_api_value(&value)
}

/// The numeric status ID: the last run of 10+ digits in the path.
fn status_id(raw_url: &str) -> Option<String> {
    Regex::new(r"/(\d{10,})")
        .ok()?
        .captures_iter(raw_url)
        .filter_map(|captures| captures.get(1).map(|matched| matched.as_str().to_string()))
        .last()
}

/// Visitor `SUB`/`SUBP` cookie values from the passport visitor flow.
async fn visitor_tokens(http: &Client) -> Option<(String, String)> {
    let visitor_url = "https://passport.weibo.com/visitor/genvisitor?cb=gen_callback&fp=%7B%22os%22%3A%221%22%2C%22browser%22%3A%22Chrome%22%2C%22fonts%22%3A%22undefined%22%2C%22screenInfo%22%3A%221920*1080*24%22%2C%22plugins%22%3A%22%22%7D";
    let visitor = jsonp_value(&get_text(http, visitor_url).await?)?;
    let tid = visitor
        .get("data")
        .and_then(|data| data.get("tid"))
        .and_then(Value::as_str)?;
    let incarnate_url = format!(
        "https://passport.weibo.com/visitor/visitor?a=incarnate&t={tid}&w=2&c=095&gc=&cb=cross_domain&from=weibo"
    );
    let incarnate = jsonp_value(&get_text(http, &incarnate_url).await?)?;
    let data = incarnate.get("data")?;
    let sub = data.get("sub").and_then(Value::as_str)?;
    let subp = data.get("subp").and_then(Value::as_str)?;
    Some((sub.to_string(), subp.to_string()))
}

async fn get_text(http: &Client, url: &str) -> Option<String> {
    http.get(url)
        .header("user-agent", "Mozilla/5.0")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
}

fn status_from_api_value(value: &Value) -> Option<PlatformMetadata> {
    if value.get("error").is_some() {
        return None;
    }
    let description = value
        .get("text_raw")
        .and_then(json_value_to_clean_string)
        .or_else(|| {
            value
                .get("text")
                .and_then(json_value_to_clean_string)
                .map(|text| plain_text_preview(&text))
        });
    let original_text = value
        .get("text_raw")
        .and_then(json_value_to_original_string)
        .or_else(|| {
            value
                .get("text")
                .and_then(json_value_to_original_string)
                .map(|text| html_to_formatted_text(&text))
        });
    PlatformMetadata {
        title: description
            .as_deref()
            .map(|description| truncate_chars(description, 80)),
        description,
        author: value
            .get("user")
            .and_then(|user| user.get("screen_name"))
            .and_then(json_value_to_clean_string),
        image_url: image_url(value),
        original_text,
    }
    .into_meaningful()
}

fn image_url(value: &Value) -> Option<String> {
    value
        .get("pic_infos")
        .and_then(Value::as_object)
        .and_then(|pics| {
            pics.values().find_map(|pic| {
                ["largest", "large", "original", "bmiddle", "thumbnail"]
                    .iter()
                    .find_map(|key| {
                        pic.get(*key)
                            .and_then(|image| image.get("url"))
                            .and_then(json_value_to_image_url)
                    })
            })
        })
        .or_else(|| {
            value
                .get("page_info")
                .and_then(|page| page.get("page_pic"))
                .and_then(json_value_to_image_url)
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{shell_reason, status_from_api_value, status_id};
    use crate::extraction::jsonp_value;

    #[test]
    fn reads_the_numeric_status_id() {
        assert_eq!(
            status_id("https://m.weibo.cn/status/5012345678901234?from=x").as_deref(),
            Some("5012345678901234")
        );
        assert_eq!(status_id("https://weibo.com/123/AbCd"), None);
    }

    #[test]
    fn extracts_status_from_ajax_json() {
        let metadata = status_from_api_value(&json!({
            "text_raw": "今天发布了新版本\n欢迎试用",
            "user": {"screen_name": "开发者"},
            "pic_infos": {"a": {"large": {"url": "https://wx1.sinaimg.cn/large/a.jpg"}}}
        }))
        .expect("status");
        assert_eq!(metadata.author.as_deref(), Some("开发者"));
        assert_eq!(
            metadata.original_text.as_deref(),
            Some("今天发布了新版本\n欢迎试用")
        );
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://wx1.sinaimg.cn/large/a.jpg")
        );
        assert!(status_from_api_value(&json!({"error": "not found"})).is_none());
    }

    #[test]
    fn parses_visitor_jsonp_payload() {
        let value = jsonp_value(r#"gen_callback({"retcode":20000000,"data":{"tid":"abc"}});"#)
            .expect("jsonp");
        assert_eq!(value["data"]["tid"], "abc");
    }

    #[test]
    fn recognises_visitor_shell() {
        assert!(shell_reason("微博热搜 登录/注册").is_some());
        assert!(shell_reason("一条普通的微博正文").is_none());
    }
}
