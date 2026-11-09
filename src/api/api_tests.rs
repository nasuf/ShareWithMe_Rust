use std::{io::Cursor, net::SocketAddr, sync::Arc};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use reqwest::Client;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

use super::{
    RefreshQuery,
    auth::{bearer_token, constant_time_eq},
    build_router,
    media::refresh_cover,
    refresh::{
        merge_duplicate_into, merge_reanalyzed_item, merge_refreshed_item,
        preserve_existing_transcript, refresh_item_content,
    },
};
use crate::{
    config::{AppConfig, StoreConfig},
    models::LinkItem,
    state::AppState,
    store::{Store, find_item},
};

#[test]
fn bearer_auth_requires_the_exact_configured_token() {
    assert_eq!(bearer_token("Bearer secret-token"), Some("secret-token"));
    assert_eq!(bearer_token("bearer secret-token"), Some("secret-token"));
    assert_eq!(bearer_token("Basic secret-token"), None);
    assert!(constant_time_eq(b"secret-token", b"secret-token"));
    assert!(!constant_time_eq(b"secret-token", b"secret"));
    assert!(!constant_time_eq(b"secret-token", b"other-token"));
}

#[tokio::test]
async fn force_refresh_cover_recovers_expired_remote_image_from_source_page() {
    let fixture_base = spawn_cover_fixture().await;
    let item_id = format!("old-cover-{}", Uuid::new_v4());
    let store_path = std::env::temp_dir().join(format!("{item_id}.json"));
    let media_path = std::env::temp_dir().join(format!("{item_id}-media"));
    let now = Utc::now();
    let item = LinkItem {
        id: item_id.clone(),
        source_url: format!("{fixture_base}/page"),
        final_url: format!("{fixture_base}/page"),
        source_app: Some("小红书".to_string()),
        platform: "小红书".to_string(),
        title: "旧封面卡片".to_string(),
        description: "远程 CDN 封面已失效".to_string(),
        author: Some("测试作者".to_string()),
        image_url: Some(format!("{fixture_base}/expired.png")),
        remote_image_url: Some(format!("{fixture_base}/expired.png")),
        cover_cached_at: None,
        cover_checked_at: Some(now),
        content_text: "远程 CDN 封面已失效".to_string(),
        original_text: "远程 CDN 封面已失效".to_string(),
        shared_text: String::new(),
        original_content: Default::default(),
        media: Vec::new(),
        source_facts: Vec::new(),
        source_id: None,
        published_at: None,
        extraction: Default::default(),
        archive: None,
        content_omitted: false,
        summary: "旧封面需要重新解析".to_string(),
        category: "测试".to_string(),
        keywords: vec!["封面刷新".to_string()],
        entities: vec![],
        sentiment: "neutral".to_string(),
        content_type: "note".to_string(),
        importance_score: 60,
        notes: String::new(),
        analysis_pending: false,
        user_edited_at: None,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
    };
    let store = Arc::new(Store::local_for_tests(store_path.clone(), vec![item]));
    let state = AppState {
        store,
        http: test_http_client(),
        config: Arc::new(AppConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            store: StoreConfig::Local {
                path: store_path.clone(),
            },
            media_path: media_path.clone(),
            api_token: None,
            deepseek_api_key: None,
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            deepseek_model: "test".to_string(),
        }),
    };

    let Json(updated) = refresh_cover(
        State(state),
        Path(item_id.clone()),
        Query(RefreshQuery { force: Some(true) }),
    )
    .await
    .expect("refresh cover");

    let image_url = updated.image_url.expect("cached image url");
    assert!(image_url.starts_with("/media/cover-"));
    assert_eq!(
        updated.remote_image_url,
        Some(format!("{fixture_base}/cover.png"))
    );
    assert!(updated.cover_cached_at.is_some());
    assert!(updated.cover_checked_at.is_some());
    let file_name = image_url.strip_prefix("/media/").unwrap();
    assert!(file_name.starts_with(&format!("cover-{item_id}-")));
    assert!(media_path.join(file_name).exists());

    let _ = std::fs::remove_file(store_path);
    let _ = std::fs::remove_dir_all(media_path);
}

#[test]
fn content_refresh_preserves_user_edited_classification() {
    let mut current = test_analysis_item("旧摘要", "自动分类", vec!["自动标签"]);
    current.id = "stable-id".to_string();
    current.category = "我的分类".to_string();
    current.keywords = vec!["保留标签".to_string()];
    current.notes = "我的备注".to_string();
    current.user_edited_at = Some(Utc::now());
    current.media = vec![crate::content::MediaAsset {
        kind: "image".to_string(),
        url: "https://example.com/image.jpg".to_string(),
        cached_url: Some("/media/asset-stable-0.jpg".to_string()),
        ..crate::content::MediaAsset::default()
    }];

    let mut fresh = test_analysis_item("新摘要", "新自动分类", vec!["新自动标签"]);
    fresh.media = vec![crate::content::MediaAsset {
        kind: "image".to_string(),
        url: "https://example.com/image.jpg".to_string(),
        ..crate::content::MediaAsset::default()
    }];
    let updated = merge_refreshed_item(current, fresh);

    assert_eq!(updated.summary, "新摘要");
    assert_eq!(updated.category, "我的分类");
    assert_eq!(updated.keywords, ["保留标签"]);
    assert_eq!(updated.notes, "我的备注");
    assert!(updated.user_edited_at.is_some());
    assert_eq!(
        updated.media[0].cached_url.as_deref(),
        Some("/media/asset-stable-0.jpg")
    );
}

#[test]
fn reanalysis_preserves_user_edited_classification() {
    let mut current = test_analysis_item("旧摘要", "自动分类", vec!["自动标签"]);
    current.id = "stable-id".to_string();
    current.created_at = Utc::now() - chrono::Duration::days(3);
    current.status = "favorite".to_string();
    current.category = "我的收藏夹".to_string();
    current.keywords = vec!["保留标签".to_string()];
    current.notes = "我的备注".to_string();
    current.user_edited_at = Some(Utc::now());
    let created_at = current.created_at;

    let fresh = test_analysis_item("新摘要", "新自动分类", vec!["新自动标签"]);
    let updated = merge_reanalyzed_item(current, fresh);

    assert_eq!(updated.summary, "新摘要");
    assert_eq!(updated.id, "stable-id");
    assert_eq!(updated.status, "favorite");
    assert_eq!(updated.created_at, created_at);
    assert_eq!(updated.category, "我的收藏夹");
    assert_eq!(updated.keywords, ["保留标签"]);
    assert_eq!(updated.notes, "我的备注");
    assert!(updated.user_edited_at.is_some());
}

#[test]
fn reanalysis_replaces_classification_the_user_never_touched() {
    let mut current = test_analysis_item("旧摘要", "自动分类", vec!["自动标签"]);
    current.id = "stable-id".to_string();
    current.user_edited_at = None;

    let fresh = test_analysis_item("新摘要", "新自动分类", vec!["新自动标签"]);
    let updated = merge_reanalyzed_item(current, fresh);

    assert_eq!(updated.category, "新自动分类");
    assert_eq!(updated.keywords, ["新自动标签"]);
    assert!(updated.user_edited_at.is_none());
}

#[test]
fn metadata_refresh_keeps_existing_video_transcript() {
    let transcript = "这是已经保存的视频字幕正文，自动刷新只更新标题和统计，不能用较短简介覆盖它。";
    let mut current = test_analysis_item("摘要", "视频", vec!["字幕"]);
    current.content_text = transcript.to_string();
    current.original_text = transcript.to_string();
    current.original_content = crate::content::OriginalContent::from_plain_text(transcript);
    current.source_facts = vec![crate::content::SourceFact {
        key: "transcript_source".to_string(),
        label: "视频正文".to_string(),
        value: "语音转写".to_string(),
    }];
    current.extraction.completeness = 90;

    let mut fresh = crate::models::LinkMetadata {
        title: "更新后的标题".to_string(),
        content_text: "短简介".to_string(),
        original_text: "短简介".to_string(),
        original_content: crate::content::OriginalContent::from_plain_text("短简介"),
        ..crate::models::LinkMetadata::default()
    };
    fresh.extraction.completeness = 50;
    preserve_existing_transcript(&current, &mut fresh);

    assert_eq!(fresh.original_content.plain_text, transcript);
    assert_eq!(fresh.extraction.completeness, 90);
    assert!(
        fresh
            .source_facts
            .iter()
            .any(|fact| fact.value == "语音转写")
    );
    assert!(fresh.extraction.content_fingerprint.is_some());
}

fn test_analysis_item(summary: &str, category: &str, keywords: Vec<&str>) -> LinkItem {
    crate::analysis::build_item(
        crate::models::LinkMetadata {
            source_url: "https://example.com/post".to_string(),
            final_url: "https://example.com/post".to_string(),
            platform: "Example".to_string(),
            title: "标题".to_string(),
            original_text: "一段足够稳定的原文内容".to_string(),
            ..crate::models::LinkMetadata::default()
        },
        crate::models::LinkAnalysis {
            summary: summary.to_string(),
            category: category.to_string(),
            keywords: keywords.into_iter().map(str::to_string).collect(),
            entities: Vec::new(),
            sentiment: "neutral".to_string(),
            content_type: "article".to_string(),
            importance_score: 50,
            notes: "自动备注".to_string(),
        },
        None,
    )
}

async fn spawn_cover_fixture() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let image_bytes = test_png_bytes();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let image_bytes = image_bytes.clone();
            tokio::spawn(async move {
                let mut buffer = [0_u8; 2048];
                let bytes_read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, content_type, body) = match path {
                    "/page" => (
                        "200 OK",
                        "text/html; charset=utf-8",
                        format!(
                            r#"<html><head>
                                <meta property="og:title" content="恢复后的封面">
                                <meta property="og:image" content="http://{addr}/cover.png">
                                </head><body>可重新解析的卡片正文</body></html>"#
                        )
                        .into_bytes(),
                    ),
                    "/cover.png" => ("200 OK", "image/png", image_bytes),
                    _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    format!("http://{addr}")
}

fn test_png_bytes() -> Vec<u8> {
    let image = RgbImage::from_pixel(8, 8, Rgb([248, 92, 118]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn test_http_client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn list_items_serves_gzip_and_opt_in_summary_rows() {
    let item_id = format!("summary-{}", Uuid::new_v4());
    let store_path = std::env::temp_dir().join(format!("{item_id}.json"));
    let body_text = "长文正文内容。".repeat(400);
    let mut item = test_analysis_item("摘要", "阅读", vec!["标签"]);
    item.id = item_id.clone();
    item.content_text = body_text.clone();
    item.original_text = body_text.clone();
    item.original_content = crate::content::OriginalContent::from_plain_text(&body_text);
    let store = Arc::new(Store::local_for_tests(store_path.clone(), vec![item]));
    let state = AppState {
        store,
        http: test_http_client(),
        config: Arc::new(AppConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            store: StoreConfig::Local {
                path: store_path.clone(),
            },
            media_path: std::env::temp_dir().join(format!("{item_id}-media")),
            api_token: None,
            deepseek_api_key: None,
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            deepseek_model: "test".to_string(),
        }),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });

    let (headers, _) = raw_get(addr, "/api/items", true).await;
    assert!(headers.contains("content-encoding: gzip"), "{headers}");

    let (_, body) = raw_get(addr, "/api/items?fields=summary", false).await;
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(rows[0]["content_omitted"], true);
    assert_eq!(rows[0]["content_text"], "");
    assert_eq!(rows[0]["original_text"], "");
    assert!(
        rows[0]["original_content"]["blocks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(rows[0]["summary"], "摘要");

    // Without the opt-in, older clients still get the full article.
    let (_, body) = raw_get(addr, "/api/items", false).await;
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(rows[0].get("content_omitted").is_none());
    assert_eq!(rows[0]["content_text"], body_text.as_str());

    let _ = std::fs::remove_file(store_path);
}

async fn raw_get(addr: SocketAddr, path: &str, accept_gzip: bool) -> (String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let encoding = if accept_gzip {
        "Accept-Encoding: gzip\r\n"
    } else {
        ""
    };
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n{encoding}\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let headers = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    (headers, response[split + 4..].to_vec())
}

#[test]
fn duplicate_merge_keeps_owner_content_and_users_edits() {
    let mut owner = test_analysis_item("真正的笔记", "零食", vec!["软糖"]);
    owner.id = "owner".to_string();
    owner.title = "拆开瞬间就不想分给同事了🍬✨".to_string();
    let mut duplicate = test_analysis_item("登录页", "我的收藏夹", vec!["要买"]);
    duplicate.id = "duplicate".to_string();
    duplicate.title = "Log in - rednote".to_string();
    duplicate.notes = "我的备注".to_string();
    duplicate.user_edited_at = Some(Utc::now());
    duplicate.status = "favorite".to_string();
    duplicate.created_at = owner.created_at - chrono::Duration::days(1);

    let merged = merge_duplicate_into(owner, &duplicate);

    assert_eq!(merged.id, "owner");
    assert_eq!(merged.title, "拆开瞬间就不想分给同事了🍬✨");
    assert_eq!(merged.category, "我的收藏夹");
    assert_eq!(merged.keywords, ["要买"]);
    assert_eq!(merged.notes, "我的备注");
    assert_eq!(merged.status, "favorite");
    assert_eq!(merged.created_at, duplicate.created_at);
}

#[tokio::test]
async fn auto_refresh_leaves_complete_snapshot_untouched() {
    let item_id = format!("settled-{}", Uuid::new_v4());
    let store_path = std::env::temp_dir().join(format!("{item_id}.json"));
    let now = Utc::now();
    let mut item = test_analysis_item("完整摘要", "阅读", vec!["标签"]);
    item.id = item_id.clone();
    // Unreachable source: any fetch here would degrade the snapshot.
    item.source_url = "http://127.0.0.1:9/gone".to_string();
    item.final_url = item.source_url.clone();
    item.original_text = "已保存的完整正文".to_string();
    item.extraction = crate::content::ExtractionState {
        status: "complete".to_string(),
        adapter: "generic".to_string(),
        method: "html".to_string(),
        completeness: 90,
        fetched_at: Some(now - chrono::Duration::days(30)),
        refresh_after: Some(now - chrono::Duration::days(1)),
        content_fingerprint: Some("kept".to_string()),
        last_error: None,
    };
    let store = Arc::new(Store::local_for_tests(store_path.clone(), vec![item]));
    let state = AppState {
        store,
        http: test_http_client(),
        config: Arc::new(AppConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            store: StoreConfig::Local {
                path: store_path.clone(),
            },
            media_path: std::env::temp_dir().join(format!("{item_id}-media")),
            api_token: None,
            deepseek_api_key: None,
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            deepseek_model: "test".to_string(),
        }),
    };

    let Json(updated) = refresh_item_content(
        State(state.clone()),
        Path(item_id.clone()),
        Query(RefreshQuery { force: None }),
    )
    .await
    .expect("refresh");

    assert_eq!(updated.original_text, "已保存的完整正文");
    assert_eq!(updated.extraction.status, "complete");
    assert_eq!(
        updated.extraction.content_fingerprint.as_deref(),
        Some("kept")
    );
    // Pushed far out (and saved) so the app stops asking on every launch.
    let refresh_after = updated.extraction.refresh_after.expect("refresh_after");
    assert!(refresh_after > now + chrono::Duration::days(365));
    let saved = find_item(&state.store, &item_id).await.unwrap().unwrap();
    assert_eq!(saved.extraction.refresh_after, Some(refresh_after));

    let _ = std::fs::remove_file(store_path);
}

/// Serve the router over a local store holding `items`, protected by `token`.
async fn spawn_api(items: Vec<LinkItem>, token: Option<&str>) -> (String, std::path::PathBuf) {
    let id = Uuid::new_v4();
    let store_path = std::env::temp_dir().join(format!("api-routes-{id}.json"));
    let state = AppState {
        store: Arc::new(Store::local_for_tests(store_path.clone(), items)),
        http: test_http_client(),
        config: Arc::new(AppConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            store: StoreConfig::Local {
                path: store_path.clone(),
            },
            media_path: std::env::temp_dir().join(format!("api-routes-{id}-media")),
            api_token: token.map(str::to_string),
            deepseek_api_key: None,
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            deepseek_model: "test".to_string(),
        }),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    (format!("http://{addr}"), store_path)
}

#[tokio::test]
async fn item_routes_cover_the_full_lifecycle() {
    let items = vec![
        crate::models::test_item("keep", "https://example.com/keep", "保留的文章"),
        crate::models::test_item("drop", "http://192.168.1.1/private", "要删除的文章"),
    ];
    let (base, store_path) = spawn_api(items, Some("secret")).await;
    let http = test_http_client();
    let api = |method: reqwest::Method, path: &str| {
        http.request(method, format!("{base}{path}"))
            .bearer_auth("secret")
    };

    // Health is public; everything under /api needs the token.
    let health = http.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    let anonymous = http.get(format!("{base}/api/items")).send().await.unwrap();
    assert_eq!(anonymous.status(), 401);

    let list: Vec<serde_json::Value> = api(reqwest::Method::GET, "/api/items")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    let missing = api(reqwest::Method::GET, "/api/items/nope")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    let edited: serde_json::Value = api(reqwest::Method::PATCH, "/api/items/keep")
        .json(
            &serde_json::json!({"category": "技术", "keywords": ["Rust", "Rust"], "notes": "复读"}),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edited["category"], "技术");
    assert_eq!(edited["notes"], "复读");
    let empty_category = api(reqwest::Method::PATCH, "/api/items/keep")
        .json(&serde_json::json!({"category": "  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_category.status(), 400);

    let status = api(reqwest::Method::PATCH, "/api/items/keep/status")
        .json(&serde_json::json!({"status": "favorite"}))
        .send()
        .await
        .unwrap();
    assert!(status.status().is_success());
    let bad_status = api(reqwest::Method::PATCH, "/api/items/keep/status")
        .json(&serde_json::json!({"status": "gone"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_status.status(), 400);

    let stats: serde_json::Value = api(reqwest::Method::GET, "/api/stats")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["total"], 2);
    let digest: serde_json::Value = api(reqwest::Method::GET, "/api/digest")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(digest["total"].as_u64().is_some());
    let markdown = api(reqwest::Method::GET, "/api/export/markdown")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(markdown.contains("保留的文章"));
    let export: serde_json::Value = api(reqwest::Method::GET, "/api/export/json")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(export["item_count"], 2);

    // Re-importing the export merges by URL instead of duplicating; items with
    // private URLs are rejected.
    let exported = |id: &str| {
        export["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .cloned()
            .unwrap()
    };
    let rejected = api(reqwest::Method::POST, "/api/import/json")
        .json(&serde_json::json!({"schema_version": 1, "items": [exported("drop")]}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    let imported: serde_json::Value = api(reqwest::Method::POST, "/api/import/json")
        .json(&serde_json::json!({"schema_version": 1, "items": [exported("keep")]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(imported["merged_count"], 1);
    assert_eq!(imported["total_count"], 2);

    // A private URL is neither analysed nor archived.
    let analyze = api(reqwest::Method::POST, "/api/links/analyze")
        .json(&serde_json::json!({"url": "http://192.168.1.1/admin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(analyze.status(), 400);
    let archive = api(reqwest::Method::POST, "/api/items/drop/archive")
        .send()
        .await
        .unwrap();
    assert_eq!(archive.status(), 400);

    let deleted = api(reqwest::Method::DELETE, "/api/items/drop")
        .send()
        .await
        .unwrap();
    assert!(deleted.status().is_success());
    let gone = api(reqwest::Method::GET, "/api/items/drop")
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 404);

    let _ = std::fs::remove_file(store_path);
}

#[tokio::test]
async fn analyze_link_saves_a_new_item_and_merges_a_resubmission() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page_addr = listener.local_addr().unwrap();
    let paragraph = "一篇用于验证收录接口的文章正文，内容足够长。".repeat(12);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = format!(
                r#"<html><head><meta property="og:title" content="收录测试文章"></head>
                <body><article><p>{paragraph}</p><p>{paragraph}</p></article></body></html>"#
            );
            tokio::spawn(async move {
                let mut buffer = [0_u8; 2048];
                let _ = socket.read(&mut buffer).await;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
            });
        }
    });
    let (base, store_path) = spawn_api(Vec::new(), None).await;
    let http = test_http_client();
    let page = format!("http://{page_addr}/post");
    let analyze = |shared_text: &str| {
        http.post(format!("{base}/api/links/analyze"))
            .json(&serde_json::json!({"shared_text": format!("{shared_text} {page}")}))
            .send()
    };

    let first: serde_json::Value = analyze("推荐一篇文章").await.unwrap().json().await.unwrap();
    assert_eq!(first["title"], "收录测试文章");
    assert_eq!(first["extraction"]["status"], "complete");
    assert!(!first["summary"].as_str().unwrap().is_empty());

    let second: serde_json::Value = analyze("再分享一次").await.unwrap().json().await.unwrap();
    assert_eq!(
        second["id"], first["id"],
        "same URL must not create a duplicate"
    );
    let list: Vec<serde_json::Value> = http
        .get(format!("{base}/api/items"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);

    let missing_url = http
        .post(format!("{base}/api/links/analyze"))
        .json(&serde_json::json!({"shared_text": "没有链接的文字"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_url.status(), 400);

    let _ = std::fs::remove_file(store_path);
}
