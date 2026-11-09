//! 知乎 (Zhihu): answers and column articles from the page's `js-initialData`,
//! with the public answer API as a fallback for the access-verification wall.

use regex::Regex;
use reqwest::Client;
use serde_json::Value;

use crate::{
    extraction::{
        PlatformMetadata, json_ld_values, json_value_to_clean_string, json_value_to_image_url,
        json_value_to_original_string, json_values_from_script_id, platform_metadata_from_values,
    },
    text::{html_to_formatted_text, plain_text_preview, truncate_chars},
};

use super::identity::id_after;

const API_USER_AGENT: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148";

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    if let Some(id) = id_after(segments, &["answer"]) {
        let question = id_after(segments, &["question"])?;
        return Some((
            format!("answer:{id}"),
            format!("https://www.zhihu.com/question/{question}/answer/{id}"),
        ));
    }
    id_after(segments, &["p"]).map(|id| {
        (
            format!("article:{id}"),
            format!("https://zhuanlan.zhihu.com/p/{id}"),
        )
    })
}

pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
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

/// `GET /api/v4/answers/<id>` for answer links.
pub(super) async fn fetch_answer(http: &Client, raw_url: &str) -> Option<PlatformMetadata> {
    let answer_id = answer_id(raw_url)?;
    let api_url = format!(
        "https://www.zhihu.com/api/v4/answers/{answer_id}?include=content,excerpt,question,author"
    );
    let value = http
        .get(api_url)
        .header("user-agent", API_USER_AGENT)
        .header("accept", "application/json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    answer_from_api_value(&value)
}

fn answer_id(raw_url: &str) -> Option<String> {
    Regex::new(r"/answer/(\d+)")
        .ok()?
        .captures(raw_url)?
        .get(1)
        .map(|matched| matched.as_str().to_string())
}

fn answer_from_api_value(value: &Value) -> Option<PlatformMetadata> {
    let title = value
        .get("question")
        .and_then(|question| question.get("title"))
        .and_then(json_value_to_clean_string);
    let description = value
        .get("excerpt")
        .and_then(json_value_to_clean_string)
        .or_else(|| {
            value
                .get("content")
                .and_then(json_value_to_clean_string)
                .map(|content| truncate_chars(&plain_text_preview(&content), 1_200))
        });
    let original_text = value
        .get("content")
        .and_then(json_value_to_original_string)
        .map(|content| truncate_chars(&html_to_formatted_text(&content), 12_000))
        .or_else(|| value.get("excerpt").and_then(json_value_to_original_string));
    let author = value
        .get("author")
        .and_then(|author| author.get("name"))
        .and_then(json_value_to_clean_string);
    let image_url = value
        .get("thumbnail")
        .and_then(json_value_to_image_url)
        .or_else(|| {
            value
                .get("author")
                .and_then(|author| author.get("avatar_url"))
                .and_then(json_value_to_image_url)
        });
    PlatformMetadata {
        title,
        description,
        author,
        image_url,
        original_text,
    }
    .into_meaningful()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{answer_from_api_value, answer_id, embedded_state};

    #[test]
    fn reads_answer_id_from_question_urls() {
        assert_eq!(
            answer_id("https://www.zhihu.com/question/123/answer/456789?utm=1").as_deref(),
            Some("456789")
        );
        assert_eq!(answer_id("https://zhuanlan.zhihu.com/p/1"), None);
    }

    #[test]
    fn extracts_answer_from_api_json_keeping_paragraphs() {
        let metadata = answer_from_api_value(&json!({
            "question": {"title": "如何学习 Rust？"},
            "excerpt": "先读官方书。",
            "content": "<p>先读官方书。</p><p>再写项目。</p>",
            "author": {"name": "张三", "avatar_url": "https://pic.zhimg.com/a.jpg"}
        }))
        .expect("answer");
        assert_eq!(metadata.title.as_deref(), Some("如何学习 Rust？"));
        assert_eq!(metadata.author.as_deref(), Some("张三"));
        assert_eq!(
            metadata.original_text.as_deref(),
            Some("先读官方书。\n\n再写项目。")
        );
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://pic.zhimg.com/a.jpg")
        );
        assert!(answer_from_api_value(&json!({})).is_none());
    }

    #[test]
    fn parses_initial_data_state() {
        let html = r#"<script id="js-initialData" type="text/json">{"initialState":{"entities":{"answers":{"1":{"question":{"title":"知乎问题"},"excerpt":"知乎回答摘要","author":{"name":"答主"}}}}}}</script>"#;
        let metadata = embedded_state(html).expect("state");
        assert_eq!(metadata.title.as_deref(), Some("知乎问题"));
        assert_eq!(metadata.description.as_deref(), Some("知乎回答摘要"));
    }
}
