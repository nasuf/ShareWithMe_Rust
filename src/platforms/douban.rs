//! 豆瓣 (Douban): movie/TV/book/music subjects via the public rexxar API.

use reqwest::Client;
use serde_json::Value;
use url::Url;

use crate::{
    models::LinkMetadata,
    text::{clean_original_text, clean_text},
};

use super::identity::{id_after, path_segments, query_value};

pub(super) fn identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    // Anti-bot wall: https://sec.douban.com/c?r=<encoded original URL>
    if url.host_str() == Some("sec.douban.com") {
        let target = Url::parse(&query_value(url, &["r"])?).ok()?;
        if target.host_str() == Some("sec.douban.com") {
            return None;
        }
        return identity(&target, &path_segments(&target));
    }
    // App share link: https://www.douban.com/doubanapp/dispatch/<kind>/<id>
    if let ["doubanapp", "dispatch", kind, id, ..] = segments
        && !id.is_empty()
        && id.chars().all(|ch| ch.is_ascii_digit())
    {
        let host = match *kind {
            "movie" | "tv" => "movie.douban.com",
            "book" => "book.douban.com",
            "music" => "music.douban.com",
            _ => return None,
        };
        return Some((
            format!("subject:{id}"),
            format!("https://{host}/subject/{id}/"),
        ));
    }
    for kind in ["subject", "review", "note"] {
        if let Some(id) = id_after(segments, &[kind]) {
            let host = url.host_str().unwrap_or("www.douban.com");
            return Some((
                format!("{kind}:{id}"),
                format!("https://{host}/{kind}/{id}/"),
            ));
        }
    }
    None
}

/// Douban bounces non-browser clients to sec.douban.com; that page's only
/// content is the site name. Reached only if the subject API also failed.
pub(super) fn shell_reason(final_url: &str) -> Option<&'static str> {
    final_url
        .contains("sec.douban.com")
        .then_some("豆瓣返回了安全验证页面，条目接口也未返回内容，已保存分享文字。")
}

/// Subject details from `m.douban.com/rexxar/api/v2/<kind>/<id>`.
pub(super) async fn enrich_remote(metadata: &mut LinkMetadata, http: &Client) {
    let Some(id) = metadata
        .source_id
        .as_deref()
        .and_then(|value| value.strip_prefix("subject:"))
    else {
        return;
    };
    let kind = subject_kind(&[&metadata.source_url, &metadata.final_url]);
    let url = format!("https://m.douban.com/rexxar/api/v2/{kind}/{id}?ck=&for_mobile=1");
    let Ok(response) = http
        .get(url)
        .header(
            "referer",
            format!("https://m.douban.com/{kind}/subject/{id}/"),
        )
        .header("accept", "application/json")
        .send()
        .await
    else {
        return;
    };
    let Ok(response) = response.error_for_status() else {
        return;
    };
    let Ok(value) = response.json::<Value>().await else {
        return;
    };
    if merge(metadata, &value) {
        metadata.extraction.method = "platform-api".to_string();
    }
}

// The rexxar API is split by subject kind. Web links carry it in the host
// (book./music.douban.com), App share links in the path
// (/doubanapp/dispatch/<kind>/<id>). A movie request for a TV series is
// redirected to the tv endpoint, so "movie" is a safe default.
fn subject_kind(urls: &[&str]) -> &'static str {
    let has = |needle: &str| urls.iter().any(|url| url.contains(needle));
    if has("book.douban.com") || has("/dispatch/book/") {
        "book"
    } else if has("music.douban.com") || has("/dispatch/music/") {
        "music"
    } else if has("/dispatch/tv/") {
        "tv"
    } else {
        "movie"
    }
}

fn merge(metadata: &mut LinkMetadata, value: &Value) -> bool {
    let Some(title) = value.get("title").and_then(Value::as_str) else {
        return false;
    };
    metadata.title = clean_text(title);
    if let Some(intro) = value.get("intro").and_then(Value::as_str) {
        let intro = clean_original_text(intro);
        metadata.description = clean_text(&intro);
        metadata.set_body(intro);
    }
    if let Some(image) = value
        .get("pic")
        .and_then(|pic| pic.get("large").or_else(|| pic.get("normal")))
        .and_then(Value::as_str)
    {
        metadata.image_url = Some(image.to_string());
        metadata.add_image(image);
    }
    metadata.set_fact(
        "rating",
        "豆瓣评分",
        value
            .get("rating")
            .and_then(|rating| rating.get("value"))
            .and_then(value_string),
    );
    metadata.set_fact("year", "年份", value.get("year").and_then(value_string));
    metadata.set_fact(
        "subtitle",
        "条目信息",
        value
            .get("card_subtitle")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    for (key, label) in [
        ("directors", "导演"),
        ("actors", "主演"),
        ("author", "作者"),
    ] {
        metadata.set_fact(key, label, names(value.get(key)));
    }
    true
}

fn names(value: Option<&Value>) -> Option<String> {
    let values = value?.as_array()?;
    let names = values
        .iter()
        .filter_map(|value| {
            value
                .as_str()
                .or_else(|| value.get("name").and_then(Value::as_str))
        })
        .take(12)
        .collect::<Vec<_>>()
        .join("、");
    (!names.is_empty()).then_some(names)
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::{merge, subject_kind};

    #[test]
    fn picks_api_kind_from_web_and_app_links() {
        assert_eq!(
            subject_kind(&["https://www.douban.com/doubanapp/dispatch/book/1?dt_dapp=1"]),
            "book"
        );
        assert_eq!(
            subject_kind(&["https://www.douban.com/doubanapp/dispatch/tv/2"]),
            "tv"
        );
        assert_eq!(
            subject_kind(&["https://music.douban.com/subject/3/"]),
            "music"
        );
        assert_eq!(
            subject_kind(&[
                "https://www.douban.com/doubanapp/dispatch/movie/35524447?dt_dapp=1",
                "https://sec.douban.com/c?r=https%3A%2F%2Fmovie.douban.com%2Fsubject%2F35524447%2F"
            ]),
            "movie"
        );
    }

    #[test]
    fn merges_public_subject_payload() {
        let mut metadata = LinkMetadata::default();
        assert!(merge(
            &mut metadata,
            &serde_json::json!({
                "title": "肖申克的救赎",
                "intro": "希望让人自由。",
                "year": "1994",
                "rating": {"value": 9.7},
                "pic": {"large": "https://example.com/poster.jpg"},
                "directors": [{"name": "弗兰克·德拉邦特"}]
            })
        ));
        assert_eq!(metadata.title, "肖申克的救赎");
        assert_eq!(metadata.original_text, "希望让人自由。");
        assert!(
            metadata
                .source_facts
                .iter()
                .any(|fact| fact.key == "rating")
        );
    }
}
