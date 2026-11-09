//! Text normalisation shared by extraction, analysis and export.

use regex::Regex;

/// Collapse all whitespace into single spaces (one line).
pub(crate) fn clean_text(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Normalise text but keep its line and paragraph structure.
pub(crate) fn clean_original_text(input: &str) -> String {
    let decoded = input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = Vec::new();
    let mut blank_pending = false;
    for raw_line in decoded.lines() {
        let line = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_pending = !lines.is_empty();
            continue;
        }
        if blank_pending && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
        }
        lines.push(line);
        blank_pending = false;
    }
    lines.join("\n").trim().to_string()
}

pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut output = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

/// The first line with at least four characters once URLs are removed.
pub(crate) fn first_meaningful_line(text: &str) -> Option<String> {
    let url_regex = Regex::new(r#"https?://\S+|www\.\S+"#).ok();
    text.lines()
        .map(|line| {
            url_regex
                .as_ref()
                .map(|regex| regex.replace_all(line, " ").to_string())
                .unwrap_or_else(|| line.to_string())
        })
        .map(|line| clean_text(&line))
        .find(|line| line.chars().count() >= 4)
        .map(|line| truncate_chars(&line, 120))
}

/// True when the text holds nothing but URLs and punctuation.
pub(crate) fn is_url_only_text(text: &str) -> bool {
    let without_urls = Regex::new(r#"https?://\S+|www\.\S+"#)
        .map(|regex| regex.replace_all(text, " ").to_string())
        .unwrap_or_else(|_| text.to_string());
    without_urls
        .trim()
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(
                    ch,
                    '，' | '。' | '；' | '：' | '、' | '（' | '）' | '【' | '】'
                )
        })
        .trim()
        .is_empty()
}

pub(crate) fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| clean_text(&value))
        .find(|value| !value.is_empty())
}

/// Strip tags and collapse whitespace.
pub(crate) fn plain_text_preview(input: &str) -> String {
    clean_text(
        &Regex::new(r"<[^>]+>")
            .map(|regex| regex.replace_all(input, " ").to_string())
            .unwrap_or_else(|_| input.to_string()),
    )
}

/// Strip tags but turn block elements and `<br>` into line breaks.
pub(crate) fn html_to_formatted_text(input: &str) -> String {
    let mut text = input
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    if let Ok(block_regex) = Regex::new(r"(?i)</?(p|div|section|article|li|h[1-6])\b[^>]*>") {
        text = block_regex.replace_all(&text, "\n").to_string();
    }
    if let Ok(tag_regex) = Regex::new(r"(?is)<[^>]+>") {
        text = tag_regex.replace_all(&text, " ").to_string();
    }
    clean_original_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_paragraphs_but_collapses_spaces() {
        assert_eq!(
            clean_original_text("第一段  文字\r\n\r\n\r\n第二段&nbsp;文字"),
            "第一段 文字\n\n第二段 文字"
        );
        assert_eq!(clean_text("  a \n b\t c "), "a b c");
    }

    #[test]
    fn truncates_by_characters_not_bytes() {
        assert_eq!(truncate_chars("你好世界", 2), "你好...");
        assert_eq!(truncate_chars("你好", 2), "你好");
    }

    #[test]
    fn detects_url_only_share_text() {
        assert!(is_url_only_text("https://example.com/a ，"));
        assert!(!is_url_only_text("好文推荐 https://example.com/a"));
        assert_eq!(
            first_meaningful_line("https://x.com\n好文推荐 https://example.com").as_deref(),
            Some("好文推荐")
        );
    }

    #[test]
    fn converts_html_blocks_to_lines() {
        assert_eq!(
            html_to_formatted_text("<p>第一段</p><p>第二<b>段</b></p>a<br>b"),
            "第一段\n\n第二 段\na\nb"
        );
        assert_eq!(plain_text_preview("<p>你好</p> <b>世界</b>"), "你好 世界");
    }
}
