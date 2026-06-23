use regex::Regex;

use super::{
    classify::{
        clean_original_text, clean_text, first_meaningful_line, html_to_formatted_text,
        plain_text_preview, truncate_chars,
    },
    metadata::PlatformMetadata,
};

pub(super) fn zhihu_answer_id(raw_url: &str) -> Option<String> {
    Regex::new(r"/answer/(\d+)")
        .ok()?
        .captures(raw_url)?
        .get(1)
        .map(|matched| matched.as_str().to_string())
}

pub(super) fn weibo_status_id(raw_url: &str) -> Option<String> {
    Regex::new(r"/(\d{10,})")
        .ok()?
        .captures_iter(raw_url)
        .filter_map(|captures| captures.get(1).map(|matched| matched.as_str().to_string()))
        .last()
}

pub(super) fn zhihu_metadata_from_api_value(value: &serde_json::Value) -> Option<PlatformMetadata> {
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
    let metadata = PlatformMetadata {
        title,
        description,
        author,
        image_url,
        original_text,
    };
    metadata.is_meaningful().then_some(metadata)
}

pub(super) fn weibo_metadata_from_api_value(value: &serde_json::Value) -> Option<PlatformMetadata> {
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
    let title = description
        .as_deref()
        .map(|description| truncate_chars(description, 80));
    let author = value
        .get("user")
        .and_then(|user| user.get("screen_name"))
        .and_then(json_value_to_clean_string);
    let image_url = weibo_image_url(value);
    let metadata = PlatformMetadata {
        title,
        description,
        author,
        image_url,
        original_text,
    };
    metadata.is_meaningful().then_some(metadata)
}

fn weibo_image_url(value: &serde_json::Value) -> Option<String> {
    value
        .get("pic_infos")
        .and_then(serde_json::Value::as_object)
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

pub(super) fn jsonp_value(input: &str) -> Option<serde_json::Value> {
    let start = input.find('{')?;
    let json = balanced_json_from(input, start)?;
    serde_json::from_str(&json).ok()
}

pub(super) fn platform_metadata_from_values(
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

pub(super) fn json_ld_values(html: &str) -> Vec<serde_json::Value> {
    script_bodies_by_attr(html, "type", "application/ld+json")
        .into_iter()
        .flat_map(|body| parse_json_values(&body))
        .collect()
}

pub(super) fn json_values_from_script_id(html: &str, id: &str) -> Vec<serde_json::Value> {
    script_bodies_by_attr(html, "id", id)
        .into_iter()
        .flat_map(|body| {
            let decoded = percent_decode(&body);
            parse_json_values(&decoded)
        })
        .collect()
}

pub(super) fn json_values_after_markers(html: &str, markers: &[&str]) -> Vec<serde_json::Value> {
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

fn json_value_to_image_url(value: &serde_json::Value) -> Option<String> {
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

fn json_value_to_clean_string(value: &serde_json::Value) -> Option<String> {
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

fn json_value_to_original_string(value: &serde_json::Value) -> Option<String> {
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
