use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use reqwest::{Client, redirect::Policy};
use tracing::info;

mod analysis;
mod api;
mod config;
mod digest;
mod error;
mod models;
mod render;
mod state;
mod store;

use api::build_router;
use config::AppConfig;
use state::AppState;
use store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::from_filename("../.env").ok();
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Arc::new(AppConfig::from_env()?);
    let store = Store::load(&config).await?;

    let http = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(22))
        .redirect(Policy::limited(8))
        .user_agent("ShareWithMe/0.1 (+personal link organizer)")
        .build()
        .context("build http client")?;

    let state = AppState {
        store: Arc::new(store),
        http,
        config: Arc::clone(&config),
    };

    let app = build_router(state);

    info!("ShareWithMe backend listening on {}", config.bind_addr);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use std::{env, fs, sync::Arc};
    use uuid::Uuid;

    use crate::{
        analysis::{
            extract_first_url, extract_urls, infer_category, infer_content_type, platform_for_url,
        },
        api::{build_json_export, build_markdown_export, matches_filter, sort_items},
        digest::collection_digest,
        models::{LinkItem, ListItemsQuery},
        store::{Store, all_items, import_items, save_item, update_item_metadata},
    };

    #[test]
    fn extracts_http_and_bare_urls_from_shared_text() {
        assert_eq!(
            extract_first_url("复制这段内容后打开 https://xhslink.com/a1b2c3。"),
            Some("https://xhslink.com/a1b2c3".to_string())
        );
        assert_eq!(
            extract_first_url("看这个 www.example.com/path?x=1，挺有用"),
            Some("https://www.example.com/path?x=1".to_string())
        );
        assert_eq!(
            extract_first_url("98 小红书 xhslink.com/a1B2c3，复制本条信息"),
            Some("https://xhslink.com/a1B2c3".to_string())
        );
        assert_eq!(
            extract_first_url("2.33 v.douyin.com/iLxyz9/ 复制此链接"),
            Some("https://v.douyin.com/iLxyz9/".to_string())
        );
        assert_eq!(
            extract_first_url("这个视频不错 b23.tv/BV1234。"),
            Some("https://b23.tv/BV1234".to_string())
        );
        assert_eq!(
            extract_first_url("淘宝口令 m.tb.cn/h.abc123?tk=abcd CZ0001"),
            Some("https://m.tb.cn/h.abc123?tk=abcd".to_string())
        );
        assert_eq!(
            extract_first_url("资料在 pan.baidu.com/s/abc123 提取码 test"),
            Some("https://pan.baidu.com/s/abc123".to_string())
        );
        assert_eq!(extract_first_url("版本号 abc.def/ghi 不是链接"), None);
        assert_eq!(
            extract_urls("合集 xhslink.com/a1 b23.tv/BV1234 再看 www.example.com/a xhslink.com/a1"),
            [
                "https://xhslink.com/a1",
                "https://b23.tv/BV1234",
                "https://www.example.com/a"
            ]
        );
        assert_eq!(
            extract_urls("视频 youtu.be/abc123 问答 zhihu.com/question/1 微博 weibo.com/123/abc"),
            [
                "https://youtu.be/abc123",
                "https://zhihu.com/question/1",
                "https://weibo.com/123/abc"
            ]
        );
    }

    #[test]
    fn maps_common_platforms() {
        let cases = [
            ("https://xhslink.com/a/b", "小红书"),
            ("https://www.toutiao.com/article/1", "今日头条"),
            ("https://v.douyin.com/abc", "抖音"),
            ("https://b23.tv/abc", "哔哩哔哩"),
            ("https://www.youtube.com/watch?v=abc", "YouTube"),
            ("https://youtu.be/abc", "YouTube"),
            ("https://www.zhihu.com/question/1", "知乎"),
            ("https://zhuanlan.zhihu.com/p/1", "知乎"),
            ("https://weibo.com/123/abc", "微博"),
            ("https://weibo.cn/status/abc", "微博"),
            ("https://maps.apple.com/?q=coffee", "Apple Maps"),
            ("https://music.163.com/song?id=1", "网易云音乐"),
            ("https://docs.google.com/document/d/1", "Google Docs/Drive"),
            ("https://pan.baidu.com/s/abc", "百度网盘"),
            ("https://m.tb.cn/h.abc123", "淘宝/天猫"),
        ];

        for (url, expected) in cases {
            assert_eq!(platform_for_url(url), expected);
        }
    }

    #[test]
    fn infers_categories_for_specialized_sources() {
        assert_eq!(infer_category("上海咖啡店 地图 导航", "高德地图"), "地点");
        assert_eq!(infer_category("Rust API 架构设计", "GitHub"), "技术");
        assert_eq!(infer_category("双十一优惠券 商品下单", "淘宝/天猫"), "购物");
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

    #[tokio::test]
    async fn store_deduplicates_by_final_url() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));

        let first = test_item("first-id", "https://example.com/a", "First title");
        let second = test_item("second-id", "https://example.com/a", "Second title");

        save_item(&store, first).await.expect("save first");
        save_item(&store, second).await.expect("save second");

        let items = all_items(&store).await.expect("read items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "first-id");
        assert_eq!(items[0].title, "Second title");

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn import_items_merges_existing_final_urls() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));

        let original = test_item("original-id", "https://example.com/a", "Original");
        let imported = test_item("imported-id", "https://example.com/a", "Imported");

        save_item(&store, original).await.expect("save original");
        let (created, merged) = import_items(&store, vec![imported])
            .await
            .expect("import items");

        let items = all_items(&store).await.expect("read items");
        assert_eq!((created, merged), (0, 1));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "original-id");
        assert_eq!(items[0].title, "Imported");

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn updates_user_editable_item_metadata() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));
        let item = test_item("manual-edit", "https://example.com/a", "Original");
        let original_updated_at = item.updated_at;
        save_item(&store, item).await.expect("save item");

        let updated = update_item_metadata(
            &store,
            "manual-edit",
            crate::models::UpdateItemRequest {
                category: Some("旅行".to_string()),
                keywords: Some(vec!["路线".to_string(), "攻略".to_string()]),
                notes: Some("周末整理".to_string()),
            },
        )
        .await
        .expect("update item");

        assert_eq!(updated.category, "旅行");
        assert_eq!(updated.keywords, ["路线", "攻略"]);
        assert_eq!(updated.notes, "周末整理");
        assert!(updated.updated_at >= original_updated_at);

        let persisted = all_items(&store).await.expect("read items");
        assert_eq!(persisted[0].category, "旅行");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn digest_summarizes_active_collection() {
        let mut items = vec![
            test_item("one", "https://github.com/example/repo", "Rust API 架构"),
            test_item(
                "two",
                "https://github.com/example/other",
                "Flutter API 示例",
            ),
            test_item("three", "https://item.jd.com/1.html", "优惠券 商品"),
        ];
        items[0].category = "技术".to_string();
        items[0].keywords = vec!["Rust".to_string(), "API".to_string()];
        items[1].category = "技术".to_string();
        items[1].keywords = vec!["Flutter".to_string(), "API".to_string()];
        items[2].category = "购物".to_string();
        items[2].keywords = vec!["优惠".to_string(), "券".to_string()];

        let mut archived = test_item("archived", "https://example.com/old", "Old");
        archived.status = "archived".to_string();
        items.push(archived);

        let digest = collection_digest(&items);
        assert_eq!(digest.total, 3);
        assert_eq!(digest.recent_items.len(), 3);
        assert!(digest.focus_summary.contains("技术 2"));
        assert!(digest.suggestions.iter().any(|item| item.contains("优惠")));
    }

    #[test]
    fn item_filter_respects_status_query() {
        let mut item = test_item("favorite", "https://example.com/favorite", "Favorite");
        item.status = "favorite".to_string();
        item.notes = "周末复盘".to_string();

        let favorite_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some("favorite".to_string()),
            sort: None,
        };
        let active_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some("active".to_string()),
            sort: None,
        };
        let all_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some(String::new()),
            sort: None,
        };
        let platform_query = ListItemsQuery {
            query: None,
            category: None,
            platform: Some("example.com".to_string()),
            status: Some(String::new()),
            sort: None,
        };
        let other_platform_query = ListItemsQuery {
            query: None,
            category: None,
            platform: Some("GitHub".to_string()),
            status: Some(String::new()),
            sort: None,
        };

        assert!(matches_filter(&item, &favorite_query));
        assert!(!matches_filter(&item, &active_query));
        assert!(matches_filter(&item, &all_query));
        assert!(matches_filter(&item, &platform_query));
        assert!(!matches_filter(&item, &other_platform_query));

        for term in ["example.com/favorite", "example.com", "周末复盘"] {
            let query = ListItemsQuery {
                query: Some(term.to_string()),
                category: None,
                platform: None,
                status: Some(String::new()),
                sort: None,
            };
            assert!(matches_filter(&item, &query), "term should match: {term}");
        }
    }

    #[test]
    fn sort_items_supports_recency_and_importance() {
        let mut low_old = test_item("low-old", "https://example.com/low", "Low");
        let mut high_new = test_item("high-new", "https://example.com/high", "High");
        let mut mid = test_item("mid", "https://example.com/mid", "Mid");
        low_old.importance_score = 20;
        high_new.importance_score = 90;
        mid.importance_score = 50;
        low_old.created_at -= chrono::Duration::minutes(2);
        mid.created_at -= chrono::Duration::minutes(1);

        let mut items = vec![mid.clone(), low_old.clone(), high_new.clone()];
        sort_items(&mut items, Some("oldest"));
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["low-old", "mid", "high-new"]
        );

        sort_items(&mut items, Some("importance"));
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["high-new", "mid", "low-old"]
        );
    }

    #[test]
    fn markdown_export_contains_summary_and_items() {
        let mut item = test_item("export", "https://example.com/export", "Export Title");
        item.category = "资料".to_string();
        item.keywords = vec!["备份".to_string(), "复盘".to_string()];
        item.notes = "手动备注".to_string();

        let markdown = build_markdown_export(&[item]);

        assert!(markdown.contains("# ShareWithMe Export"));
        assert!(markdown.contains("Export Title"));
        assert!(markdown.contains("https://example.com/export"));
        assert!(markdown.contains("Keywords: 备份, 复盘"));
        assert!(markdown.contains("Notes: 手动备注"));
    }

    #[test]
    fn json_export_preserves_items_and_schema_version() {
        let mut item = test_item("json-export", "https://example.com/json", "JSON Export");
        item.category = "资料".to_string();
        item.keywords = vec!["迁移".to_string()];

        let export = build_json_export(vec![item.clone()]);

        assert_eq!(export.schema_version, 1);
        assert_eq!(export.item_count, 1);
        assert_eq!(export.digest.total, 1);
        assert_eq!(export.items[0].id, "json-export");
        assert_eq!(export.items[0].keywords, ["迁移"]);
    }

    fn test_item(id: &str, final_url: &str, title: &str) -> LinkItem {
        let now = Utc::now();
        LinkItem {
            id: id.to_string(),
            source_url: final_url.to_string(),
            final_url: final_url.to_string(),
            source_app: None,
            platform: platform_for_url(final_url),
            title: title.to_string(),
            description: String::new(),
            author: None,
            image_url: None,
            remote_image_url: None,
            cover_cached_at: None,
            cover_checked_at: None,
            content_text: String::new(),
            original_text: String::new(),
            summary: title.to_string(),
            category: "待整理".to_string(),
            keywords: vec![],
            entities: vec![],
            sentiment: "neutral".to_string(),
            content_type: "link".to_string(),
            importance_score: 50,
            notes: String::new(),
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}
