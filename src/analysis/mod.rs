mod ai;
mod classify;
mod metadata;
mod metadata_json;
#[cfg(test)]
mod tests;
mod url;

pub(crate) use ai::{analyze_with_deepseek_or_fallback, build_item, check_deepseek};
pub(crate) use classify::{clean_text, truncate_chars};
#[cfg(test)]
pub(crate) use classify::{infer_category, infer_content_type, platform_for_url};
pub(crate) use metadata::extract_metadata;
#[cfg(test)]
pub(crate) use url::{extract_first_url, extract_urls};
pub(crate) use url::{resolve_request_url, resolve_request_urls};

#[cfg(test)]
use metadata::merge_html_metadata;
