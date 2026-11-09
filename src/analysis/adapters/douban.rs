use reqwest::Client;
use serde_json::Value;

use crate::{
    content::{MediaAsset, OriginalContent, SourceFact},
    models::LinkMetadata,
};

use super::super::classify::{clean_original_text, clean_text};

pub(super) async fn enrich(metadata: &mut LinkMetadata, http: &Client) {
    if metadata.platform != "豆瓣" {
        return;
    }
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
        metadata.content_text = clean_text(&intro);
        metadata.original_text = intro.clone();
        metadata.original_content = OriginalContent::from_plain_text(intro);
    }
    if let Some(image) = value
        .get("pic")
        .and_then(|pic| pic.get("large").or_else(|| pic.get("normal")))
        .and_then(Value::as_str)
    {
        metadata.image_url = Some(image.to_string());
        if !metadata.media.iter().any(|asset| asset.url == image) {
            metadata.media.push(MediaAsset {
                kind: "image".to_string(),
                url: image.to_string(),
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            });
        }
    }
    fact(
        metadata,
        "rating",
        "豆瓣评分",
        value
            .get("rating")
            .and_then(|rating| rating.get("value"))
            .and_then(value_string),
    );
    fact(
        metadata,
        "year",
        "年份",
        value.get("year").and_then(value_string),
    );
    fact(
        metadata,
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
        fact(metadata, key, label, names(value.get(key)));
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

fn fact(metadata: &mut LinkMetadata, key: &str, label: &str, value: Option<String>) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    if let Some(existing) = metadata
        .source_facts
        .iter_mut()
        .find(|fact| fact.key == key)
    {
        existing.value = value;
        return;
    }
    metadata.source_facts.push(SourceFact {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
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
