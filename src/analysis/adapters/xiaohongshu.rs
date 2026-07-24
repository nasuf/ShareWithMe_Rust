use regex::Regex;
use url::Url;

use crate::{
    content::{MediaAsset, OriginalContent},
    models::LinkMetadata,
};

use super::super::classify::{clean_original_text, clean_text, is_url_only_text};

pub(in crate::analysis) fn merge(metadata: &mut LinkMetadata, html: &str) -> bool {
    if metadata.platform != "小红书" && !metadata.final_url.contains("xiaohongshu.com") {
        return false;
    }

    if html.contains("当前笔记暂时无法浏览") || html.contains("你访问的页面不见了")
    {
        let reason = "小红书返回了笔记不可浏览页面，后端无法抓取该笔记正文。请换一个可公开访问的分享链接，或从原 App 分享带标题/正文的文本。";
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
        }
        return true;
    }

    let Some(section) = note_section(html, &metadata.final_url) else {
        return false;
    };
    let title = string_field(section, "title").or_else(|| string_field(section, "displayTitle"));
    let description = string_field(section, "desc");
    let original_description = raw_string_field(section, "desc");
    let author = string_field(section, "nickname").or_else(|| string_field(section, "nickName"));
    let image_url = string_field(section, "urlDefault").or_else(|| string_field(section, "urlPre"));

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
        metadata.original_content = OriginalContent::from_plain_text(&metadata.original_text);
    }
    if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
        metadata.author = Some(author);
    }
    if let Some(image_url) = image_url.filter(|value| !value.trim().is_empty()) {
        metadata.image_url = Some(image_url);
    }
    for image_url in string_fields(section, &["urlDefault", "urlPre"]) {
        if !metadata.media.iter().any(|asset| asset.url == image_url) {
            metadata.media.push(MediaAsset {
                kind: "image".to_string(),
                url: image_url,
                alt: Some(metadata.title.clone()),
                ..MediaAsset::default()
            });
        }
    }
    true
}

fn note_section<'a>(html: &'a str, final_url: &str) -> Option<&'a str> {
    let start = note_id(final_url)
        .and_then(|note_id| html.find(&format!("\"{note_id}\"")))
        .or_else(|| html.find("\"noteDetailMap\""))
        .or_else(|| html.find("\"noteCard\""))?;
    let end = (start + 120_000).min(html.len());
    html.get(start..end)
}

fn note_id(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let note_id = match segments.as_slice() {
        ["explore", id, ..] => Some(*id),
        ["discovery", "item", id, ..] => Some(*id),
        _ => None,
    }?;
    (!note_id.is_empty()).then(|| note_id.to_string())
}

fn string_field(section: &str, key: &str) -> Option<String> {
    raw_string_field(section, key)
        .map(|value| clean_text(&value))
        .filter(|value| !value.is_empty())
}

fn raw_string_field(section: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
    let regex = Regex::new(&pattern).ok()?;
    let raw = regex.captures(section)?.get(1)?.as_str();
    serde_json::from_str::<String>(&format!("\"{raw}\""))
        .ok()
        .filter(|value| !value.is_empty())
}

fn string_fields(section: &str, keys: &[&str]) -> Vec<String> {
    let key_pattern = keys
        .iter()
        .map(|key| regex::escape(key))
        .collect::<Vec<_>>()
        .join("|");
    let pattern = format!(r#"\"(?:{key_pattern})\"\s*:\s*\"(?P<value>(?:\\.|[^\"\\])*)\""#);
    let Ok(regex) = Regex::new(&pattern) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for captures in regex.captures_iter(section) {
        let Some(raw) = captures.name("value").map(|value| value.as_str()) else {
            continue;
        };
        let Some(decoded) = serde_json::from_str::<String>(&format!("\"{raw}\"")).ok() else {
            continue;
        };
        if (decoded.starts_with("http://") || decoded.starts_with("https://"))
            && !values.contains(&decoded)
        {
            values.push(decoded);
        }
    }
    values
}
