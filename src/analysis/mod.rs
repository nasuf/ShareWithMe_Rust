//! Organise a captured snapshot: summary, category, keywords and importance,
//! via the DeepSeek analyzer with a heuristic fallback.

mod ai;
mod classify;
#[cfg(test)]
mod tests;

pub(crate) use ai::{
    analyze_with_deepseek_or_fallback, build_item, check_deepseek, heuristic_analysis,
};
#[cfg(test)]
pub(crate) use classify::{infer_category, infer_content_type};
