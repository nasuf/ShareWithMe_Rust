use std::{io::Cursor, net::SocketAddr, sync::Arc};

use axum::extract::{Path, Query, State};
use chrono::Utc;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use reqwest::Client;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

use super::*;
use crate::{
    config::{AppConfig, StoreConfig},
    models::LinkItem,
    store::Store,
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
    assert!(media_path.join(format!("cover-{item_id}.jpg")).exists());

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
