//! 微信读书 (WeRead): book title, author, introduction and cover from JSON-LD
//! and the page's initial state.

use serde_json::Value;
use url::Url;

use crate::{
    extraction::{
        json_values_after_markers,
        structured_data::{self, find_string},
    },
    models::LinkMetadata,
    text::clean_text,
};

use super::identity::{id_after, query_value};

pub(super) fn identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id =
        id_after(segments, &["bookDetail"]).or_else(|| query_value(url, &["bookId", "bookid"]))?;
    Some((
        id.clone(),
        format!("https://weread.qq.com/web/bookDetail/{id}"),
    ))
}

pub(super) fn enrich_from_html(metadata: &mut LinkMetadata, html: &str, json_ld: &[Value]) {
    structured_data::enrich_catalog(metadata, json_ld);
    enrich_from_initial_state(metadata, html);
}

fn enrich_from_initial_state(metadata: &mut LinkMetadata, html: &str) {
    let markers = ["window.__INITIAL_STATE__=", "window.__NUXT__="];
    for value in &json_values_after_markers(html, &markers) {
        if let Some(title) = find_string(value, &["title", "bookName"]) {
            metadata.title = clean_text(&title);
        }
        if let Some(author) = find_string(value, &["author", "authorName"]) {
            metadata.author = Some(clean_text(&author));
            metadata.set_fact("author", "作者", author);
        }
        if let Some(intro) = find_string(value, &["intro", "description"]) {
            metadata.set_description_and_body_if_longer(&intro);
        }
        if let Some(cover) = find_string(value, &["cover", "coverUrl"]) {
            metadata.set_cover_image(cover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::enrich_from_html;
    use crate::{extraction::json_ld_values, models::LinkMetadata};

    #[test]
    fn reads_book_from_initial_state() {
        let html = r#"<script>window.__INITIAL_STATE__={"reader":{"bookInfo":{"title":"置身事内","author":"兰小欢","intro":"中国政府与经济发展","cover":"https://wfqqreader.myapp.com/cover.jpg"}}};</script>"#;
        let mut metadata = LinkMetadata::default();
        enrich_from_html(&mut metadata, html, &json_ld_values(html));
        assert_eq!(metadata.title, "置身事内");
        assert_eq!(metadata.author.as_deref(), Some("兰小欢"));
        assert_eq!(metadata.original_text, "中国政府与经济发展");
        assert_eq!(
            metadata.image_url.as_deref(),
            Some("https://wfqqreader.myapp.com/cover.jpg")
        );
    }
}
