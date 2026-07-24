use crate::{models::LinkMetadata, platforms::source_identity};

use super::{
    adapters::enrich_from_html, classify::platform_for_url, metadata::merge_html_metadata,
};

#[test]
fn recognizes_every_supported_target_platform() {
    let cases = [
        ("https://www.xiaohongshu.com/explore/abc", "小红书"),
        ("https://www.youtube.com/watch?v=abc", "YouTube"),
        ("https://www.toutiao.com/article/123/", "今日头条"),
        ("https://www.bilibili.com/video/BV1abc", "哔哩哔哩"),
        ("https://www.zhihu.com/question/1/answer/2", "知乎"),
        ("https://weibo.com/123/AbCd", "微博"),
        ("https://www.douyin.com/video/123", "抖音"),
        ("https://x.com/OpenAI/status/123", "X"),
        ("https://www.instagram.com/p/ABC/", "Instagram"),
        ("https://movie.douban.com/subject/1292052/", "豆瓣"),
        ("https://www.amap.com/place/B001", "高德地图"),
        ("https://map.baidu.com/?uid=B001", "百度地图"),
        ("https://item.jd.com/1001.html", "京东"),
        ("https://item.taobao.com/item.htm?id=1001", "淘宝"),
        ("https://detail.tmall.com/item.htm?id=1001", "天猫"),
        ("https://weread.qq.com/web/bookDetail/abc", "微信读书"),
    ];

    for (url, expected) in cases {
        assert_eq!(platform_for_url(url), expected, "{url}");
        let identity = source_identity(url, expected);
        assert!(!identity.canonical_url.is_empty(), "{url}");
    }
}

#[test]
fn preserves_article_and_social_post_structure() {
    for (platform, url, title) in [
        (
            "今日头条",
            "https://www.toutiao.com/article/123/",
            "头条文章",
        ),
        ("X", "https://x.com/OpenAI/status/123", "X post"),
        (
            "Instagram",
            "https://www.instagram.com/p/ABC/",
            "Instagram post",
        ),
    ] {
        let mut metadata = fixture_metadata(platform, url);
        let html = format!(
            r#"<html><head><meta property="og:title" content="{title}"><meta property="og:description" content="公开摘要"><meta property="og:image" content="https://example.com/{platform}.jpg"></head><body><article><h1>{title}</h1><p>第一段正文，保留段落结构。</p><blockquote>一段引用内容。</blockquote><ul><li>列表项目</li></ul></article></body></html>"#
        );
        merge_html_metadata(&mut metadata, &html);
        enrich_from_html(&mut metadata, &html);

        assert_eq!(metadata.title, title);
        assert!(
            metadata
                .original_content
                .blocks
                .iter()
                .any(|block| block.kind == "heading")
        );
        assert!(
            metadata
                .original_content
                .blocks
                .iter()
                .any(|block| block.kind == "quote")
        );
        assert!(
            metadata
                .original_content
                .blocks
                .iter()
                .any(|block| block.kind == "list-item")
        );
    }
}

#[test]
fn preserves_video_metadata_for_chinese_and_global_sources() {
    let cases = [
        (
            "抖音",
            "https://www.douyin.com/video/123",
            r#"<script id="RENDER_DATA" type="application/json">{"aweme":{"desc":"城市夜跑训练记录","author":{"nickname":"跑步小林"},"video":{"cover":{"url_list":["https://example.com/douyin.jpg"]}}}}</script>"#,
        ),
        (
            "哔哩哔哩",
            "https://www.bilibili.com/video/BV1abc",
            r#"<script>window.__INITIAL_STATE__={"videoData":{"title":"Rust 所有权讲解","desc":"从实际代码理解借用。","owner":{"name":"程序员小明"},"pic":"https://example.com/bili.jpg"}};</script>"#,
        ),
        (
            "YouTube",
            "https://www.youtube.com/watch?v=abc",
            r#"<script type="application/ld+json">{"@type":"VideoObject","name":"Rust ownership","description":"A practical walkthrough","author":{"name":"Example Channel"},"thumbnailUrl":"https://example.com/youtube.jpg"}</script>"#,
        ),
    ];

    for (platform, url, html) in cases {
        let mut metadata = fixture_metadata(platform, url);
        merge_html_metadata(&mut metadata, html);
        enrich_from_html(&mut metadata, html);
        assert_ne!(metadata.title, "未命名链接", "{platform}");
        assert!(!metadata.original_text.trim().is_empty(), "{platform}");
        assert!(metadata.image_url.is_some(), "{platform}");
    }
}

#[test]
fn preserves_knowledge_book_and_review_content() {
    let cases = [
        (
            "知乎",
            "https://www.zhihu.com/question/1/answer/2",
            r#"<script type="application/ld+json">{"@type":"Article","headline":"程序员为什么需要理解所有权","articleBody":"第一段回答。\n\n第二段回答。","author":{"name":"示例作者"}}</script>"#,
        ),
        (
            "豆瓣",
            "https://movie.douban.com/subject/1292052/",
            r#"<script type="application/ld+json">{"@type":"Review","name":"经典电影短评","reviewBody":"这是一段完整短评。","author":{"name":"豆瓣用户"},"aggregateRating":{"ratingValue":"9.7"}}</script>"#,
        ),
        (
            "微信读书",
            "https://weread.qq.com/web/bookDetail/abc",
            r#"<script type="application/ld+json">{"@type":"Book","name":"设计中的设计","description":"关于设计本质的思考。","author":{"name":"原研哉"},"isbn":"9780000000000"}</script>"#,
        ),
    ];

    for (platform, url, html) in cases {
        let mut metadata = fixture_metadata(platform, url);
        merge_html_metadata(&mut metadata, html);
        enrich_from_html(&mut metadata, html);
        assert_ne!(metadata.title, "未命名链接", "{platform}");
        assert!(!metadata.original_text.trim().is_empty(), "{platform}");
        assert!(!metadata.source_facts.is_empty(), "{platform}");
    }
}

#[test]
fn preserves_product_fields_for_each_marketplace() {
    for (platform, url) in [
        ("京东", "https://item.jd.com/1001.html"),
        ("淘宝", "https://item.taobao.com/item.htm?id=1001"),
        ("天猫", "https://detail.tmall.com/item.htm?id=1001"),
    ] {
        let mut metadata = fixture_metadata(platform, url);
        let html = format!(
            r#"<script type="application/ld+json">{{"@type":"Product","name":"{platform}降噪耳机","description":"轻量舒适，支持主动降噪。","sku":"1001","brand":{{"name":"Acme"}},"image":"https://example.com/product.jpg","offers":{{"price":"699","priceCurrency":"CNY","availability":"https://schema.org/InStock"}}}}</script>"#
        );
        merge_html_metadata(&mut metadata, &html);
        enrich_from_html(&mut metadata, &html);
        assert!(metadata.title.contains("降噪耳机"));
        assert!(metadata.source_facts.iter().any(|fact| fact.key == "price"));
        assert!(metadata.source_facts.iter().any(|fact| fact.key == "brand"));
    }
}

#[test]
fn preserves_place_share_payloads_for_both_map_platforms() {
    for (platform, url) in [
        (
            "高德地图",
            "https://www.amap.com/place/B001?name=%E6%95%85%E5%AE%AB&address=%E5%8C%97%E4%BA%AC&location=116.4,39.9",
        ),
        (
            "百度地图",
            "https://map.baidu.com/?uid=B001&name=%E6%95%85%E5%AE%AB&address=%E5%8C%97%E4%BA%AC&location=116.4,39.9",
        ),
    ] {
        let metadata = fixture_metadata(platform, url);
        assert_eq!(metadata.platform, platform);
        assert!(!metadata.final_url.is_empty());
    }
}

fn fixture_metadata(platform: &str, url: &str) -> LinkMetadata {
    LinkMetadata {
        source_url: url.to_string(),
        final_url: url.to_string(),
        platform: platform.to_string(),
        title: "未命名链接".to_string(),
        ..LinkMetadata::default()
    }
}
