//! Locate and decode JSON embedded in pages: JSON-LD blocks, `<script id=..>`
//! payloads, `window.__STATE__ = {...}` assignments and JSONP responses.

use regex::Regex;

use crate::text::{
    clean_original_text, clean_text, first_meaningful_line, html_to_formatted_text,
    plain_text_preview, truncate_chars,
};

use super::PlatformMetadata;

pub(crate) fn jsonp_value(input: &str) -> Option<serde_json::Value> {
    let start = input.find('{')?;
    let json = balanced_json_from(input, start)?;
    serde_json::from_str(&json).ok()
}

pub(crate) fn platform_metadata_from_values(
    values: Vec<serde_json::Value>,
    title_keys: &[&str],
    description_keys: &[&str],
    author_keys: &[&str],
    image_keys: &[&str],
) -> Option<PlatformMetadata> {
    let mut metadata = PlatformMetadata::default();
    for value in &values {
        metadata.title = metadata.title.or_else(|| {
            find_original_string_by_keys(value, title_keys)
                .and_then(|value| first_meaningful_line(&value))
                .or_else(|| find_string_by_keys(value, title_keys))
        });
        metadata.description = metadata.description.or_else(|| {
            find_string_by_keys(value, description_keys).map(|value| plain_text_preview(&value))
        });
        metadata.original_text = metadata.original_text.or_else(|| {
            find_original_string_by_keys(value, description_keys)
                .map(|value| html_to_formatted_text(&value))
                .filter(|value| !value.is_empty())
        });
        metadata.author = metadata
            .author
            .or_else(|| find_author_string(value, author_keys));
        metadata.image_url = metadata
            .image_url
            .or_else(|| find_image_string(value, image_keys));
        if metadata.is_meaningful()
            && metadata.title.is_some()
            && metadata.description.is_some()
            && metadata.image_url.is_some()
        {
            break;
        }
    }
    if metadata.original_text.is_none() {
        metadata.original_text = metadata
            .description
            .as_deref()
            .map(clean_original_text)
            .filter(|value| !value.is_empty());
    }
    if metadata.is_meaningful() {
        Some(metadata)
    } else {
        None
    }
}

pub(crate) fn json_ld_values(html: &str) -> Vec<serde_json::Value> {
    script_bodies_by_attr(html, "type", "application/ld+json")
        .into_iter()
        .flat_map(|body| parse_json_values(&body))
        .collect()
}

pub(crate) fn json_values_from_script_id(html: &str, id: &str) -> Vec<serde_json::Value> {
    script_bodies_by_attr(html, "id", id)
        .into_iter()
        .flat_map(|body| {
            let decoded = percent_decode(&body);
            parse_json_values(&decoded)
        })
        .collect()
}

pub(crate) fn json_values_after_markers(html: &str, markers: &[&str]) -> Vec<serde_json::Value> {
    markers
        .iter()
        .filter_map(|marker| balanced_json_after_marker(html, marker))
        .flat_map(|body| parse_json_values(&body))
        .collect()
}

fn parse_json_values(input: &str) -> Vec<serde_json::Value> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Array(values)) => values,
        Ok(value) => {
            if let Some(graph) = value.get("@graph").and_then(|value| value.as_array()) {
                let mut values = vec![value.clone()];
                values.extend(graph.iter().cloned());
                values
            } else {
                vec![value]
            }
        }
        Err(_) => Vec::new(),
    }
}

fn script_bodies_by_attr(html: &str, attr: &str, expected: &str) -> Vec<String> {
    let Ok(regex) = Regex::new(r#"(?is)<script\b(?P<attrs>[^>]*)>(?P<body>.*?)</script>"#) else {
        return Vec::new();
    };
    regex
        .captures_iter(html)
        .filter_map(|captures| {
            let attrs = captures.name("attrs")?.as_str();
            let body = captures.name("body")?.as_str();
            if script_attr_matches(attrs, attr, expected) {
                Some(body.trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

fn script_attr_matches(attrs: &str, attr: &str, expected: &str) -> bool {
    let pattern = format!(
        r#"(?is)\b{}\s*=\s*["']{}["']"#,
        regex::escape(attr),
        regex::escape(expected)
    );
    Regex::new(&pattern)
        .map(|regex| regex.is_match(attrs))
        .unwrap_or(false)
}

fn balanced_json_after_marker(html: &str, marker: &str) -> Option<String> {
    let marker_start = html.find(marker)?;
    let start = html[marker_start + marker.len()..].find('{')? + marker_start + marker.len();
    balanced_json_from(html, start)
}

fn balanced_json_from(input: &str, start: usize) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in input[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return input
                        .get(start..start + offset + ch.len_utf8())
                        .map(str::to_string);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = find_string_by_key(value, key) {
            return Some(found);
        }
    }
    None
}

fn find_original_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = find_original_string_by_key(value, key) {
            return Some(found);
        }
    }
    None
}

fn find_original_string_by_key(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(json_value_to_original_string) {
                return Some(found);
            }
            map.values()
                .find_map(|value| find_original_string_by_key(value, key))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_original_string_by_key(value, key)),
        _ => None,
    }
}

fn find_string_by_key(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(json_value_to_clean_string) {
                return Some(found);
            }
            map.values()
                .find_map(|value| find_string_by_key(value, key))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_string_by_key(value, key)),
        _ => None,
    }
}

fn find_author_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_string_by_keys(value, keys)
        .map(|value| truncate_chars(&plain_text_preview(&value), 80))
        .filter(|value| !value.is_empty())
}

fn find_image_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = find_image_by_key(value, key) {
            return Some(found);
        }
    }
    None
}

fn find_image_by_key(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(json_value_to_image_url) {
                return Some(found);
            }
            map.values().find_map(|value| find_image_by_key(value, key))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_image_by_key(value, key)),
        _ => None,
    }
}

pub(crate) fn json_value_to_image_url(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => normalize_image_url(value),
        serde_json::Value::Array(values) => values.iter().find_map(json_value_to_image_url),
        serde_json::Value::Object(map) => ["url", "urlDefault", "urlPre", "src"]
            .iter()
            .find_map(|key| map.get(*key).and_then(json_value_to_image_url))
            .or_else(|| map.values().find_map(json_value_to_image_url)),
        _ => None,
    }
}

fn normalize_image_url(value: &str) -> Option<String> {
    let value = clean_text(value);
    if value.starts_with("//") {
        return Some(format!("https:{value}"));
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        return Some(value);
    }
    None
}

pub(crate) fn json_value_to_clean_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(clean_text(value)),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Object(map) => ["text", "simpleText", "name", "title", "url"]
            .iter()
            .find_map(|key| map.get(*key).and_then(json_value_to_clean_string)),
        serde_json::Value::Array(values) => values.iter().find_map(json_value_to_clean_string),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

pub(crate) fn json_value_to_original_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(clean_original_text(value)),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Object(map) => ["text", "simpleText", "name", "title"]
            .iter()
            .find_map(|key| map.get(*key).and_then(json_value_to_original_string)),
        serde_json::Value::Array(values) => {
            let joined = values
                .iter()
                .filter_map(json_value_to_original_string)
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            output.push(high * 16 + low);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| input.to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
