//! 今日头条 (Toutiao): articles use the generic HTML path; the page is often
//! a JavaScript challenge, so the browser render fallback does the real work
//! (see [`super::Platform::needs_browser_render`]).

use super::identity::id_after;

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    id_after(segments, &["article"])
        .map(|id| (id.clone(), format!("https://www.toutiao.com/article/{id}/")))
}
