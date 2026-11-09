//! Pipeline tests: page parsing, verification walls, browser render merge,
//! shell replacement, and the parse -> analyse -> store chain.

use std::{env, fs, sync::Arc};

use uuid::Uuid;

use super::{parse_page, rendered, shell};
use crate::{
    analysis::{build_item, heuristic_analysis},
    links::extract_first_url,
    models::{LinkItem, LinkMetadata},
    platforms::{platform_for_url, xiaohongshu},
    render::RenderedPage,
    store::{Store, all_items, save_item},
};

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
        ..LinkMetadata::default()
    };

    parse_page(
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
        ..LinkMetadata::default()
    };

    parse_page(
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
        ..LinkMetadata::default()
    };

    parse_page(
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
fn extracts_xiaohongshu_note_from_mobile_page_state() {
    // Shape served to mobile Safari: noteData.data.noteData, images under
    // `url`, author under `nickName`, plus a commenter whose `nickname` must
    // not be mistaken for the author, and JS `undefined` values.
    let mut metadata = LinkMetadata {
        source_url: "https://xhslink.cn/o/43YTL1DvUZu".to_string(),
        final_url:
            "https://www.xiaohongshu.com/discovery/item/6aa8cb9d000000000b00f876?xsec_token=t"
                .to_string(),
        platform: "小红书".to_string(),
        title: "未命名链接".to_string(),
        ..LinkMetadata::default()
    };

    parse_page(
        &mut metadata,
        r#"<script>window.__INITIAL_STATE__={"global":{"x":undefined},"noteData":{"routeQuery":{"noteId":"6aa8cb9d000000000b00f876"},"data":{"noteData":{"noteId":"6aa8cb9d000000000b00f876","type":"normal","desc":"本来囤了几罐打算带去工位分给小伙伴🥱\n倒几颗出来像宝石一样","user":{"nickName":"西塔星球Theta Star","userId":"u1"},"tagList":[{"name":"软糖"}],"time":1789447069000,"title":"拆开瞬间就不想分给同事了🍬✨","imageList":[{"width":1080,"url":"http:\/\/sns-webpic-qc.xhscdn.com\/a\/1","infoList":[{"imageScene":"WB_PRV","url":"http:\/\/sns-webpic-qc.xhscdn.com\/a\/prv"}],"livePhoto":undefined},{"url":"http:\/\/sns-webpic-qc.xhscdn.com\/a\/2"}]},"commentData":{"comments":[{"user":{"nickname":"评论者"},"content":"好吃吗"}]}}}};</script>"#,
    );

    assert_eq!(metadata.title, "拆开瞬间就不想分给同事了🍬✨");
    assert_eq!(metadata.author, Some("西塔星球Theta Star".to_string()));
    assert!(metadata.original_text.contains("倒几颗出来像宝石一样"));
    assert_eq!(
        metadata.image_url,
        Some("https://sns-webpic-qc.xhscdn.com/a/1".to_string())
    );
    let image_urls = metadata
        .media
        .iter()
        .map(|asset| asset.url.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        image_urls,
        [
            "https://sns-webpic-qc.xhscdn.com/a/1",
            "https://sns-webpic-qc.xhscdn.com/a/2"
        ]
    );
    assert!(metadata.published_at.is_some());
}

#[test]
fn recognizes_new_xiaohongshu_domains() {
    for url in [
        "https://xhslink.cn/o/43YTL1DvUZu",
        "https://www.rednote.com/discovery/item/6aa8cb9d000000000b00f876",
        "https://xhslink.com/o/abc",
    ] {
        assert_eq!(platform_for_url(url), "小红书", "{url}");
    }
    assert_eq!(
        extract_first_url(
            "拆开瞬间就不想分给同事了🍬✨ 本来囤了几罐打算带去... https://xhslink.cn/o/43YTL1DvUZu 把这段复制好，然后去【小红书】就能看笔记。"
        ),
        Some("https://xhslink.cn/o/43YTL1DvUZu".to_string())
    );
}

#[test]
fn recovers_note_url_from_xiaohongshu_login_redirect() {
    let login = "https://www.rednote.com/login?redirectPath=http%3A%2F%2Fwww.rednote.com%2Fdiscovery%2Fitem%2F6af04c1b0000000012009b5a%3Fxsec_token%3DCBnb%3D%26xsec_source%3Dapp_share";
    assert_eq!(
        xiaohongshu::login_redirect_target(login).as_deref(),
        Some(
            "https://www.rednote.com/discovery/item/6af04c1b0000000012009b5a?xsec_token=CBnb=&xsec_source=app_share"
        )
    );
    // Not a login page, or a login redirect that is not a note.
    assert_eq!(
        xiaohongshu::login_redirect_target(
            "https://www.xiaohongshu.com/explore/6af04c1b0000000012009b5a"
        ),
        None
    );
    assert_eq!(
        xiaohongshu::login_redirect_target(
            "https://www.xiaohongshu.com/login?redirectPath=https%3A%2F%2Fevil.example%2Fdiscovery%2Fitem%2F1"
        ),
        None
    );
}

#[test]
fn recognizes_expired_xiaohongshu_share_page() {
    assert!(xiaohongshu::is_dead_end(
        "https://www.xiaohongshu.com/404/sec_iYPxltRU?source=xhs_sec_server"
    ));
    assert!(!xiaohongshu::is_dead_end(
        "https://www.xiaohongshu.com/explore/6aa8cb9d000000000b00f876"
    ));
    assert!(!xiaohongshu::is_dead_end("https://example.com/404/page"));
}

#[test]
fn xiaohongshu_login_wall_skips_browser_fallback() {
    let metadata = LinkMetadata {
        platform: "小红书".to_string(),
        extraction: crate::content::ExtractionState {
            method: "login-wall".to_string(),
            ..Default::default()
        },
        ..LinkMetadata::default()
    };
    assert!(!rendered::should_try(&metadata));
}

#[test]
fn xiaohongshu_note_keeps_sharp_image_and_drops_blurry_preview() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.xiaohongshu.com/explore/abc123".to_string(),
        final_url: "https://www.xiaohongshu.com/explore/abc123".to_string(),
        platform: "小红书".to_string(),
        title: "未命名链接".to_string(),
        ..LinkMetadata::default()
    };

    parse_page(
        &mut metadata,
        r#"<script>window.__INITIAL_STATE__={"note":{"noteDetailMap":{"abc123":{"note":{"title":"番茄酿肉","user":{"nickname":"厨师"},"imageList":[{"urlPre":"http:\/\/example.com\/blurry.jpg","urlDefault":"http:\/\/example.com\/sharp.jpg"}],"desc":"家常菜做法","noteId":"abc123"}}}}};</script>"#,
    );

    let image_urls = metadata
        .media
        .iter()
        .map(|asset| asset.url.as_str())
        .collect::<Vec<_>>();
    assert!(image_urls.contains(&"http://example.com/sharp.jpg"));
    assert!(!image_urls.contains(&"http://example.com/blurry.jpg"));
    assert_eq!(
        metadata
            .media
            .iter()
            .filter(|asset| asset.kind == "image")
            .count(),
        1
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
        ..LinkMetadata::default()
    };

    rendered::merge(
            &mut metadata,
            RenderedPage {
                ok: true,
                final_url: Some("https://www.toutiao.com/article/7651359327906710016/".to_string()),
                title: Some("世界杯｜竞彩推荐".to_string()),
                description: Some("北京时间6月16日9时，伊朗队将在洛杉矶对阵新西兰队。".to_string()),
                author: Some("上观新闻".to_string()),
                image_url: Some("https://example.com/cover.jpg".to_string()),
                content_text: Some("北京时间6月16日9时，伊朗队将在洛杉矶对阵新西兰队。伊朗队无疑是本届世界杯最受外界关注的球队之一。".to_string()),
                content_html: None,
                extractor: Some("browser-render".to_string()),
                snapshot_width: None,
                snapshot_height: None,
                snapshot_error: None,
            },
        );

    assert_eq!(metadata.title, "世界杯｜竞彩推荐");
    assert_eq!(metadata.author, Some("上观新闻".to_string()));
    assert!(metadata.content_text.contains("伊朗队"));
    assert!(metadata.original_text.contains("伊朗队"));
    assert!(!metadata.content_text.contains("无法从公开网页直接抓取正文"));
}

#[test]
fn rendered_html_restores_inline_images_and_subheadings() {
    let mut metadata = LinkMetadata {
        source_url: "https://m.toutiao.com/is/Tq5RwoXBnYM/".to_string(),
        final_url: "https://www.toutiao.com/article/7513821185688355362/".to_string(),
        platform: "今日头条".to_string(),
        ..LinkMetadata::default()
    };
    let text = "自6日开始的美非法移民“围捕战”似乎难以落幕。\n\n催泪瓦斯、震爆弹和橡皮子弹齐发\n\n7日，上百名全副武装的联邦机构执法人员与当地社区数百名抗议者对峙后发生冲突。";
    let html = r#"<article class="syl-page-article"><p>自6日开始的美非法移民“围捕战”似乎难以落幕。</p><p><strong>催泪瓦斯、震爆弹和橡皮子弹齐发</strong></p><div class="pgc-img"><img src="data:image/gif;base64,R0" data-src="https://p3-sign.toutiaoimg.com/inline.jpeg"></div><p>7日，上百名全副武装的联邦机构执法人员与当地社区数百名抗议者对峙后发生冲突。</p></article>"#;

    rendered::merge(
        &mut metadata,
        RenderedPage {
            ok: true,
            final_url: Some("https://www.toutiao.com/article/7513821185688355362/".to_string()),
            title: Some("特朗普下令：解放洛杉矶！".to_string()),
            description: None,
            author: None,
            image_url: None,
            content_text: Some(text.to_string()),
            content_html: Some(html.to_string()),
            extractor: Some("browser-render".to_string()),
            snapshot_width: None,
            snapshot_height: None,
            snapshot_error: None,
        },
    );

    let kinds = metadata
        .original_content
        .blocks
        .iter()
        .map(|block| block.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["paragraph", "heading", "image", "paragraph"]);
    assert!(
        metadata
            .media
            .iter()
            .any(|asset| asset.url == "https://p3-sign.toutiaoimg.com/inline.jpeg")
    );
}

#[test]
fn rendered_html_is_ignored_when_it_misses_most_of_the_text() {
    let mut metadata = LinkMetadata {
        final_url: "https://www.toutiao.com/article/1/".to_string(),
        platform: "今日头条".to_string(),
        ..LinkMetadata::default()
    };
    let text = "第一段正文内容比较长，用来模拟完整的渲染文本。".repeat(10);

    rendered::merge(
        &mut metadata,
        RenderedPage {
            ok: true,
            final_url: None,
            title: None,
            description: None,
            author: None,
            image_url: None,
            content_text: Some(text.clone()),
            content_html: Some("<div><p>只有一小段</p></div>".to_string()),
            extractor: None,
            snapshot_width: None,
            snapshot_height: None,
            snapshot_error: None,
        },
    );

    assert!(
        metadata
            .original_content
            .blocks
            .iter()
            .all(|block| block.kind == "paragraph")
    );
    assert!(metadata.original_content.plain_text.contains("第一段正文"));
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
        ..LinkMetadata::default()
    };

    assert!(!rendered::should_try(&metadata));
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

#[tokio::test]
async fn persists_target_platform_items_after_parsing() {
    let path = env::temp_dir().join(format!(
        "share_with_me_platform_chain_{}.json",
        Uuid::new_v4()
    ));
    let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));

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
        save_item(&store, parse_platform_fixture(url, platform, html))
            .await
            .expect("save item");
    }

    let items = all_items(&store).await.expect("read items");
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
        ..LinkMetadata::default()
    };

    parse_page(&mut metadata, html);
    let analysis = heuristic_analysis(&metadata, "fixture");
    build_item(metadata, analysis, None)
}

#[test]
fn douban_security_wall_keeps_shared_text() {
    let mut metadata = LinkMetadata {
        source_url: "https://www.douban.com/doubanapp/dispatch/movie/35524447?dt_dapp=1"
            .to_string(),
        final_url:
            "https://sec.douban.com/c?r=https%3A%2F%2Fmovie.douban.com%2Fsubject%2F35524447%2F"
                .to_string(),
        platform: "豆瓣".to_string(),
        title: "豆瓣".to_string(),
        shared_text:
            "推荐一部韩剧《怪物》 https://www.douban.com/doubanapp/dispatch/movie/35524447"
                .to_string(),
        ..LinkMetadata::default()
    };
    shell::replace_platform_shell(&mut metadata);
    assert_eq!(metadata.title, "豆瓣条目");
    assert_eq!(metadata.extraction.method, "metadata-only");
    assert!(metadata.original_text.contains("怪物"));
    assert!(metadata.extraction.last_error.is_some());
}
