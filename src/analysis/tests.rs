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

#[test]
fn infers_categories_for_specialized_sources() {
    assert_eq!(infer_category("上海咖啡店 地图 导航", "高德地图"), "地点");
    assert_eq!(infer_category("Rust API 架构设计", "GitHub"), "技术");
    assert_eq!(infer_category("双十一优惠券 商品下单", "淘宝"), "购物");
    assert_eq!(infer_category("播客 专辑 音乐", "小宇宙"), "影音娱乐");
    assert_eq!(infer_category("项目文档 Notion 资料", "Notion"), "文档资料");
    assert_eq!(infer_category("资料链接 提取码", "百度网盘"), "文档资料");
}

#[test]
fn infers_content_types() {
    assert_eq!(
        infer_content_type("https://maps.apple.com/?q=park", ""),
        "place"
    );
    assert_eq!(
        infer_content_type("https://music.163.com/song?id=1", ""),
        "audio"
    );
    assert_eq!(
        infer_content_type("https://docs.google.com/document/d/1", ""),
        "document"
    );
    assert_eq!(
        infer_content_type("https://pan.baidu.com/s/abc", "提取码"),
        "document"
    );
    assert_eq!(
        infer_content_type("https://www.youtube.com/watch?v=1", ""),
        "video"
    );
    assert_eq!(
        infer_content_type(
            "https://www.youtube.com/watch?v=1",
            "Official video with Apple Music and Spotify links"
        ),
        "video"
    );
    assert_eq!(
        infer_content_type("https://www.zhihu.com/question/1/answer/2", "回答"),
        "article"
    );
    assert_eq!(
        infer_content_type("https://weibo.com/123/abc", "微博正文"),
        "post"
    );
    assert_eq!(
        infer_content_type("https://item.jd.com/1.html", "商品"),
        "product"
    );
    assert_eq!(
        infer_content_type("https://m.tb.cn/h.abc123?tk=abcd", "淘宝口令"),
        "product"
    );
}
