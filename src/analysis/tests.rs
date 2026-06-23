use std::{
    env, fs,
    sync::{Arc, Mutex},
};

use super::*;
use crate::{
    analysis::{
        ai::{blocked_page_analysis, heuristic_analysis},
        metadata::{merge_rendered_metadata, should_try_rendered_extraction},
        metadata_json::{
            jsonp_value, weibo_metadata_from_api_value, weibo_status_id, zhihu_answer_id,
            zhihu_metadata_from_api_value,
        },
    },
    models::{LinkItem, LinkMetadata},
    render::RenderedPage,
    store::{Store, all_items, save_item},
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn recognizes_toutiao_browser_challenge_pages() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        final_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        platform: "今日头条".to_string(),
        title: "未命名链接".to_string(),
        description: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        author: None,
        image_url: None,
        content_text: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        original_text: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
    };

    merge_html_metadata(
        &mut metadata,
        r#"<html><body></body><script>
            window.byted_acrawler.init({aid:99999999});
            var __ac_signature=window.byted_acrawler.sign("", "__ac_nonce");
            window.location.reload();
            </script></html>"#,
    );

    assert_eq!(metadata.title, "今日头条链接（网页验证拦截）");
    assert!(metadata.description.contains("浏览器验证页面"));
    assert!(metadata.content_text.contains("无法从公开网页直接抓取正文"));
}

#[test]
fn keeps_shared_text_when_challenge_page_has_user_context() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        final_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        platform: "今日头条".to_string(),
        title: "这是一条用户分享时带上的标题".to_string(),
        description: "这是一条用户分享时带上的摘要".to_string(),
        author: None,
        image_url: None,
        content_text: "这是一条用户分享时带上的摘要".to_string(),
        original_text: "这是一条用户分享时带上的摘要".to_string(),
    };

    merge_html_metadata(
        &mut metadata,
        r#"<script>document.cookie="__ac_nonce=1"; window.location.reload();</script>"#,
    );

    assert_eq!(metadata.title, "这是一条用户分享时带上的标题");
    assert!(
        metadata
            .description
            .starts_with("这是一条用户分享时带上的摘要")
    );
    assert_eq!(metadata.content_text, "这是一条用户分享时带上的摘要");
}

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
    };

    let analysis = blocked_page_analysis(&metadata);

    assert_eq!(analysis.category, "新闻");
    assert_eq!(analysis.content_type, "article");
    assert!(analysis.summary.contains("暂时未能抓取正文"));
}

#[test]
fn extracts_xiaohongshu_note_from_initial_state() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.xiaohongshu.com/explore/6450ee19000000000800f6c3".to_string(),
        final_url: "https://www.xiaohongshu.com/explore/6450ee19000000000800f6c3".to_string(),
        platform: "小红书".to_string(),
        title: "未命名链接".to_string(),
        description: String::new(),
        author: None,
        image_url: None,
        content_text: String::new(),
        original_text: String::new(),
    };

    merge_html_metadata(
        &mut metadata,
        r#"<script>window.__INITIAL_STATE__={"note":{"noteDetailMap":{"6450ee19000000000800f6c3":{"note":{"title":"可可爱爱，格格脑袋～👧🏻","user":{"nickname":"易梦玲"},"imageList":[{"urlDefault":"http:\/\/example.com\/cover.jpg"}],"desc":"我的摄影学生帮我拍的\n\t\n大家可以辣评一下🎙️","noteId":"6450ee19000000000800f6c3"}}}}};</script>"#,
    );

    assert_eq!(metadata.title, "可可爱爱，格格脑袋～👧🏻");
    assert_eq!(
        metadata.description,
        "我的摄影学生帮我拍的 大家可以辣评一下🎙️"
    );
    assert_eq!(metadata.author, Some("易梦玲".to_string()));
    assert_eq!(
        metadata.image_url,
        Some("http://example.com/cover.jpg".to_string())
    );
    assert!(metadata.content_text.contains("可可爱爱"));
    assert!(metadata.content_text.contains("辣评"));
    assert_eq!(
        metadata.original_text,
        "我的摄影学生帮我拍的\n\n大家可以辣评一下🎙️"
    );
}

#[test]
fn rendered_metadata_replaces_blocked_placeholder() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        final_url: "https://www.toutiao.com/article/7651359327906710016/".to_string(),
        platform: "今日头条".to_string(),
        title: "今日头条链接（网页验证拦截）".to_string(),
        description: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
        author: None,
        image_url: None,
        content_text: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
        original_text: "今日头条返回了浏览器验证页面，后端无法从公开网页直接抓取正文。".to_string(),
    };

    merge_rendered_metadata(
            &mut metadata,
            RenderedPage {
                ok: true,
                final_url: Some("https://www.toutiao.com/article/7651359327906710016/".to_string()),
                title: Some("世界杯｜竞彩推荐".to_string()),
                description: Some("北京时间6月16日9时，伊朗队将在洛杉矶对阵新西兰队。".to_string()),
                author: Some("上观新闻".to_string()),
                image_url: Some("https://example.com/cover.jpg".to_string()),
                content_text: Some("北京时间6月16日9时，伊朗队将在洛杉矶对阵新西兰队。伊朗队无疑是本届世界杯最受外界关注的球队之一。".to_string()),
                extractor: Some("browser-render".to_string()),
            },
        );

    assert_eq!(metadata.title, "世界杯｜竞彩推荐");
    assert_eq!(metadata.author, Some("上观新闻".to_string()));
    assert!(metadata.content_text.contains("伊朗队"));
    assert!(metadata.original_text.contains("伊朗队"));
    assert!(!metadata.content_text.contains("无法从公开网页直接抓取正文"));
}

#[test]
fn xiaohongshu_ssr_content_does_not_trigger_browser_fallback() {
    let metadata = LinkMetadata {
        source_url: "https://www.xiaohongshu.com/explore/6450ee19000000000800f6c3".to_string(),
        final_url: "https://www.xiaohongshu.com/explore/6450ee19000000000800f6c3".to_string(),
        platform: "小红书".to_string(),
        title: "可可爱爱，格格脑袋～👧🏻".to_string(),
        description: "我的摄影学生帮我拍的 大家可以辣评一下🎙️".to_string(),
        author: Some("易梦玲".to_string()),
        image_url: None,
        content_text: "可可爱爱，格格脑袋～👧🏻 我的摄影学生帮我拍的 大家可以辣评一下🎙️".to_string(),
        original_text: "可可爱爱，格格脑袋～👧🏻\n\n我的摄影学生帮我拍的\n大家可以辣评一下🎙️"
            .to_string(),
    };

    assert!(!should_try_rendered_extraction(&metadata));
}

#[test]
fn parses_douyin_render_data_through_item_chain() {
    let item = parse_platform_fixture(
        "https://v.douyin.com/iLxyz9/",
        "抖音",
        r#"<script id="RENDER_DATA" type="application/json">{
                "aweme":{"desc":"城市夜跑 5 公里训练记录\n配速稳定，心率舒服。",
                "author":{"nickname":"跑步小林"},
                "video":{"cover":{"url_list":["https://p3-sign.douyinpic.com/cover.jpeg"]}}}
            }</script>"#,
    );

    assert_eq!(item.platform, "抖音");
    assert_eq!(item.title, "城市夜跑 5 公里训练记录");
    assert_eq!(item.author, Some("跑步小林".to_string()));
    assert_eq!(item.content_type, "video");
    assert_eq!(item.category, "影音娱乐");
    assert!(
        item.original_text
            .contains("城市夜跑 5 公里训练记录\n配速稳定")
    );
    assert_eq!(
        item.image_url,
        Some("https://p3-sign.douyinpic.com/cover.jpeg".to_string())
    );
}

#[test]
fn parses_bilibili_initial_state_through_item_chain() {
    let item = parse_platform_fixture(
        "https://www.bilibili.com/video/BV1xx411c7mD/",
        "哔哩哔哩",
        r#"<script>window.__INITIAL_STATE__={
                "videoData":{"title":"Rust 异步编程入门","desc":"从 tokio 到 axum 的完整实践",
                "pic":"//i0.hdslb.com/bfs/archive/cover.jpg",
                "owner":{"name":"代码实验室"}}};</script>"#,
    );

    assert_eq!(item.platform, "哔哩哔哩");
    assert_eq!(item.title, "Rust 异步编程入门");
    assert_eq!(item.author, Some("代码实验室".to_string()));
    assert_eq!(item.content_type, "video");
    assert_eq!(item.category, "技术");
    assert_eq!(item.original_text, "从 tokio 到 axum 的完整实践");
    assert_eq!(
        item.image_url,
        Some("https://i0.hdslb.com/bfs/archive/cover.jpg".to_string())
    );
}

#[test]
fn parses_youtube_json_ld_through_item_chain() {
    let item = parse_platform_fixture(
        "https://www.youtube.com/watch?v=abc123",
        "YouTube",
        r#"<script type="application/ld+json">{
                "@context":"https://schema.org","@type":"VideoObject",
                "name":"How LLM agents use tools",
                "description":"A practical demo of tool calling and planning.\nLinks:\nhttps://example.com",
                "thumbnailUrl":["https://i.ytimg.com/vi/abc123/hqdefault.jpg"],
                "author":{"@type":"Person","name":"AI Workshop"}
            }</script>"#,
    );

    assert_eq!(item.platform, "YouTube");
    assert_eq!(item.title, "How LLM agents use tools");
    assert_eq!(item.author, Some("AI Workshop".to_string()));
    assert_eq!(item.content_type, "video");
    assert_eq!(item.category, "影音娱乐");
    assert!(item.original_text.contains("Links:\nhttps://example.com"));
    assert_eq!(
        item.image_url,
        Some("https://i.ytimg.com/vi/abc123/hqdefault.jpg".to_string())
    );
}

#[test]
fn parses_zhihu_initial_data_through_item_chain() {
    let item = parse_platform_fixture(
        "https://www.zhihu.com/question/123/answer/456",
        "知乎",
        r#"<script id="js-initialData" type="application/json">{
                "initialState":{"entities":{"answers":{"456":{
                    "question":{"title":"如何系统学习产品设计？"},
                    "excerpt":"先建立信息架构，再打磨交互细节，最后做可用性验证。",
                    "author":{"name":"设计观察员","headline":"产品设计师"}
                }}}}
            }</script>"#,
    );

    assert_eq!(item.platform, "知乎");
    assert_eq!(item.title, "如何系统学习产品设计？");
    assert_eq!(item.author, Some("设计观察员".to_string()));
    assert_eq!(item.content_type, "article");
    assert_eq!(item.category, "知识问答");
    assert!(item.original_text.contains("信息架构"));
    assert!(item.summary.contains("信息架构"));
}

#[test]
fn parses_weibo_json_ld_through_item_chain() {
    let item = parse_platform_fixture(
        "https://weibo.com/123456/NabcDEF",
        "微博",
        r#"<script type="application/ld+json">{
                "@context":"https://schema.org","@type":"SocialMediaPosting",
                "headline":"微博正文",
                "articleBody":"今天的 AI 产品发布会信息量很大，几个功能值得继续关注。",
                "image":"https://wx1.sinaimg.cn/large/cover.jpg",
                "author":{"@type":"Person","name":"科技博主"}
            }</script>"#,
    );

    assert_eq!(item.platform, "微博");
    assert_eq!(item.title, "微博正文");
    assert_eq!(item.author, Some("科技博主".to_string()));
    assert_eq!(item.content_type, "post");
    assert_eq!(item.category, "社交动态");
    assert!(item.original_text.contains("AI 产品发布会"));
    assert_eq!(
        item.image_url,
        Some("https://wx1.sinaimg.cn/large/cover.jpg".to_string())
    );
}

#[test]
fn extracts_zhihu_answer_metadata_from_api_json() {
    assert_eq!(
        zhihu_answer_id(
            "https://www.zhihu.com/question/2017210364081754127/answer/2028521919280858844"
        ),
        Some("2028521919280858844".to_string())
    );

    let value = json!({
        "question": {"title": "程序员为啥突然会变成这么辣鸡的一个职业？"},
        "excerpt": "最近公司在推claude code和codex，免费给我们充值。",
        "content": "<p>最近公司在推claude code和codex。</p><p>第二段继续讲嵌入式驱动。</p>",
        "author": {
            "name": "小小罗",
            "avatar_url": "https://picx.zhimg.com/avatar.jpg"
        }
    });
    let metadata = zhihu_metadata_from_api_value(&value).expect("zhihu metadata");

    assert_eq!(
        metadata.title,
        Some("程序员为啥突然会变成这么辣鸡的一个职业？".to_string())
    );
    assert_eq!(metadata.author, Some("小小罗".to_string()));
    assert!(
        metadata
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("claude code")
    );
    assert_eq!(
        metadata.image_url,
        Some("https://picx.zhimg.com/avatar.jpg".to_string())
    );
    assert_eq!(
        metadata.original_text,
        Some("最近公司在推claude code和codex。\n\n第二段继续讲嵌入式驱动。".to_string())
    );
}

#[test]
fn extracts_weibo_status_metadata_from_ajax_json() {
    assert_eq!(
        weibo_status_id("https://weibo.com/1686789045/5311397441306950"),
        Some("5311397441306950".to_string())
    );

    let value = json!({
        "text_raw": "今天看了一个很有意思的 AI 工具分享，值得继续关注。\n第二行保留原格式。",
        "user": {"screen_name": "大老王重生在微博"},
        "pic_infos": {
            "abc": {
                "largest": {
                    "url": "https://wx1.sinaimg.cn/large/abc.jpg"
                }
            }
        }
    });
    let metadata = weibo_metadata_from_api_value(&value).expect("weibo metadata");

    assert_eq!(metadata.author, Some("大老王重生在微博".to_string()));
    assert!(
        metadata
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("AI 工具分享")
    );
    assert_eq!(
        metadata.image_url,
        Some("https://wx1.sinaimg.cn/large/abc.jpg".to_string())
    );
    assert_eq!(
        metadata.original_text,
        Some("今天看了一个很有意思的 AI 工具分享，值得继续关注。\n第二行保留原格式。".to_string())
    );
}

#[test]
fn parses_weibo_visitor_jsonp_payload() {
    let value = jsonp_value(
            r#"window.cross_domain && cross_domain({"retcode":20000000,"data":{"sub":"s","subp":"p"}});"#,
        )
        .expect("jsonp");

    assert_eq!(value["data"]["sub"], "s");
    assert_eq!(value["data"]["subp"], "p");
}

#[test]
fn persists_target_platform_items_after_parsing() {
    let path = env::temp_dir().join(format!(
        "share_with_me_platform_chain_{}.json",
        Uuid::new_v4()
    ));
    let store = Arc::new(Mutex::new(Store {
        path: path.clone(),
        items: Vec::new(),
    }));

    let fixtures = [
        (
            "https://v.douyin.com/iLxyz9/",
            "抖音",
            r#"<script id="RENDER_DATA" type="application/json">{
                    "aweme":{"desc":"城市夜跑 5 公里训练记录","author":{"nickname":"跑步小林"},
                    "video":{"cover":{"url_list":["https://p3-sign.douyinpic.com/cover.jpeg"]}}}
                }</script>"#,
            "影音娱乐",
            "video",
        ),
        (
            "https://www.bilibili.com/video/BV1xx411c7mD/",
            "哔哩哔哩",
            r#"<script>window.__INITIAL_STATE__={
                    "videoData":{"title":"Rust 异步编程入门","desc":"从 tokio 到 axum 的完整实践",
                    "pic":"//i0.hdslb.com/bfs/archive/cover.jpg","owner":{"name":"代码实验室"}}};</script>"#,
            "技术",
            "video",
        ),
        (
            "https://www.youtube.com/watch?v=abc123",
            "YouTube",
            r#"<script type="application/ld+json">{
                    "@type":"VideoObject","name":"How LLM agents use tools",
                    "description":"A practical demo of tool calling and planning.",
                    "thumbnailUrl":["https://i.ytimg.com/vi/abc123/hqdefault.jpg"],
                    "author":{"name":"AI Workshop"}}</script>"#,
            "影音娱乐",
            "video",
        ),
        (
            "https://www.zhihu.com/question/123/answer/456",
            "知乎",
            r#"<script id="js-initialData" type="application/json">{
                    "initialState":{"entities":{"answers":{"456":{"question":{"title":"如何系统学习产品设计？"},
                    "excerpt":"先建立信息架构，再打磨交互细节。","thumbnail":"https://picx.zhimg.com/cover.jpg",
                    "author":{"name":"设计观察员"}}}}}
                }</script>"#,
            "知识问答",
            "article",
        ),
        (
            "https://weibo.com/123456/NabcDEF",
            "微博",
            r#"<script type="application/ld+json">{
                    "@type":"SocialMediaPosting","headline":"微博正文",
                    "articleBody":"今天的 AI 产品发布会信息量很大。",
                    "image":"https://wx1.sinaimg.cn/large/cover.jpg",
                    "author":{"name":"科技博主"}}</script>"#,
            "社交动态",
            "post",
        ),
    ];

    for (url, platform, html, _, _) in fixtures.iter().copied() {
        save_item(&store, parse_platform_fixture(url, platform, html)).expect("save item");
    }

    let items = all_items(&store).expect("read items");
    assert_eq!(items.len(), fixtures.len());
    for (_, platform, _, category, content_type) in fixtures.iter().copied() {
        let item = items
            .iter()
            .find(|item| item.platform == platform)
            .unwrap_or_else(|| panic!("missing platform {platform}"));
        assert_eq!(item.category, category);
        assert_eq!(item.content_type, content_type);
        assert!(!item.title.trim().is_empty());
        assert!(
            item.image_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("http")
        );
    }

    let _ = fs::remove_file(path);
}

fn parse_platform_fixture(url: &str, platform: &str, html: &str) -> LinkItem {
    let mut metadata = LinkMetadata {
        source_url: url.to_string(),
        final_url: url.to_string(),
        platform: platform.to_string(),
        title: "未命名链接".to_string(),
        description: String::new(),
        author: None,
        image_url: None,
        content_text: String::new(),
        original_text: String::new(),
    };

    merge_html_metadata(&mut metadata, html);
    let analysis = heuristic_analysis(&metadata, "fixture");
    build_item(metadata, analysis, None)
}
