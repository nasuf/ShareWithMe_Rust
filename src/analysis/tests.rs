//! Heuristic analysis: categories, content types and blocked-page handling.

use super::{ai::blocked_page_analysis, heuristic_analysis, infer_category, infer_content_type};
use crate::models::LinkMetadata;

#[test]
fn blocked_toutiao_analysis_keeps_item_as_article() {
    let metadata = LinkMetadata {
        source_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        final_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        platform: "今日头条".to_string(),
        title: "今日头条链接（网页验证拦截）".to_string(),
        description: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
        author: None,
        image_url: None,
        content_text: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
        original_text: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
        ..LinkMetadata::default()
    };

    let analysis = blocked_page_analysis(&metadata);

    assert_eq!(analysis.category, "新闻");
    assert_eq!(analysis.content_type, "article");
    assert!(analysis.summary.contains("暂时未能抓取正文"));
}

#[test]
fn heuristic_analysis_uses_platform_and_text_signals() {
    let metadata = LinkMetadata {
        source_url: "https://www.bilibili.com/video/BV1xx411c7mD/".to_string(),
        final_url: "https://www.bilibili.com/video/BV1xx411c7mD/".to_string(),
        platform: "哔哩哔哩".to_string(),
        title: "Rust 异步编程入门".to_string(),
        description: "从 tokio 到 axum 的完整实践".to_string(),
        content_text: "从 tokio 到 axum 的完整实践".to_string(),
        ..LinkMetadata::default()
    };
    let analysis = heuristic_analysis(&metadata, "");
    assert_eq!(analysis.category, "技术");
    assert_eq!(analysis.content_type, "video");
    assert!(analysis.keywords.iter().any(|keyword| keyword == "Rust"));
    assert!(!analysis.summary.is_empty());
}

#[test]
fn social_platforms_stay_social_unless_explicitly_technical() {
    assert_eq!(infer_category("今天天气很好", "微博"), "社交动态");
    assert_eq!(infer_category("Rust 新版本发布", "微博"), "技术");
    assert_eq!(infer_category("随便看看", "知乎"), "知识问答");
    assert_eq!(infer_category("", "京东"), "购物");
    assert_eq!(infer_category("", "未知平台"), "待整理");
}

#[test]
fn content_type_follows_url_and_text() {
    assert_eq!(
        infer_content_type("https://www.amap.com/place/1", ""),
        "place"
    );
    assert_eq!(
        infer_content_type("https://item.jd.com/1.html", ""),
        "product"
    );
    assert_eq!(
        infer_content_type("https://mp.weixin.qq.com/s/abc", ""),
        "article"
    );
    assert_eq!(infer_content_type("https://example.com", "短"), "link");
}
