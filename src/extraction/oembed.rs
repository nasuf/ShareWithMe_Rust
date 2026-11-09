//! oEmbed lookups (YouTube, X, Instagram).

use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use url::Url;

use crate::{
    models::LinkMetadata,
    text::{clean_original_text, clean_text, truncate_chars},
};

use super::UNTITLED;

/// Query an oEmbed `endpoint` for the snapshot's URL and merge the result.
pub(crate) async fn enrich(
    metadata: &mut LinkMetadata,
    http: &Client,
    endpoint: &str,
    access_token: Option<&str>,
) {
    let Some(value) = fetch(http, endpoint, &metadata.final_url, access_token).await else {
        return;
    };
    if merge(metadata, &value) {
        metadata.extraction.method = "oembed".to_string();
    }
}

async fn fetch(http: &Client, endpoint: &str, url: &str, token: Option<&str>) -> Option<Value> {
    let mut endpoint = Url::parse(endpoint).ok()?;
    endpoint
        .query_pairs_mut()
        .append_pair("url", url)
        .append_pair("format", "json");
    if let Some(token) = token {
        endpoint
            .query_pairs_mut()
            .append_pair("access_token", token);
    }
    let response = http.get(endpoint).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

fn merge(metadata: &mut LinkMetadata, value: &Value) -> bool {
    let mut changed = false;
    if let Some(title) = string(value, "title")
        .map(clean_text)
        .filter(|value| !value.is_empty())
    {
        metadata.title = title;
        changed = true;
    }
    if let Some(author) = string(value, "author_name")
        .map(clean_text)
        .filter(|value| !value.is_empty())
    {
        metadata.author = Some(author);
        changed = true;
    }
    if let Some(thumbnail) = string(value, "thumbnail_url")
        .map(str::to_string)
        .filter(|value| value.starts_with("http"))
    {
        metadata.add_image(&thumbnail);
        metadata.image_url = Some(thumbnail);
        changed = true;
    }
    if let Some(html) = string(value, "html") {
        let text = embed_text(html);
        if !is_embed_placeholder(&text)
            && text.chars().count() >= metadata.original_text.chars().count()
            && !text.is_empty()
        {
            metadata.set_body(text);
            changed = true;
        }
    }
    if metadata.title == UNTITLED && !metadata.original_text.trim().is_empty() {
        metadata.title = truncate_chars(&metadata.original_text, 80);
    }
    changed
}

fn is_embed_placeholder(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    normalized == "view this post on instagram" || normalized == "view this post on instagram."
}

fn embed_text(html: &str) -> String {
    let document = Html::parse_fragment(html);
    let preferred = Selector::parse("blockquote p").ok().and_then(|selector| {
        document
            .select(&selector)
            .next()
            .map(|element| element.text().collect::<Vec<_>>().join(" "))
    });
    clean_original_text(
        preferred
            .unwrap_or_else(|| document.root_element().text().collect::<Vec<_>>().join(" "))
            .trim(),
    )
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use crate::models::LinkMetadata;

    use super::merge;

    #[test]
    fn x_oembed_preserves_post_text() {
        let mut metadata = LinkMetadata {
            platform: "X".to_string(),
            title: "未命名链接".to_string(),
            ..LinkMetadata::default()
        };
        merge(
            &mut metadata,
            &serde_json::json!({
                "author_name": "OpenAI",
                "html": "<blockquote><p>A useful product update with <a href='https://x.com/hashtag/AI'>#AI</a>.</p></blockquote>"
            }),
        );

        assert_eq!(metadata.author.as_deref(), Some("OpenAI"));
        assert!(metadata.original_text.contains("product update"));
        assert!(!metadata.original_text.contains("<blockquote"));
    }
}
