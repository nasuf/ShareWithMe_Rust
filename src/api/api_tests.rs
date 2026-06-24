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
        summary: "旧封面需要重新解析".to_string(),
        category: "测试".to_string(),
        keywords: vec!["封面刷新".to_string()],
        entities: vec![],
        sentiment: "neutral".to_string(),
        content_type: "note".to_string(),
        importance_score: 60,
        notes: String::new(),
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
            deepseek_api_key: None,
            deepseek_base_url: "https://api.deepseek.com/chat/completions".to_string(),
            deepseek_model: "test".to_string(),
        }),
    };

    let Json(updated) = refresh_cover(
        State(state),
        Path(item_id.clone()),
        Query(RefreshCoverQuery { force: Some(true) }),
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
