use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    models::{AnalyzerCheckResponse, LinkAnalysis, LinkItem, LinkMetadata},
    state::AppState,
};

use super::classify::{
    extract_entities, extract_keywords, fallback_summary, infer_category, infer_content_type,
    is_blocked_without_content, truncate_chars,
};

pub(crate) async fn analyze_with_deepseek_or_fallback(
    state: &AppState,
    metadata: &LinkMetadata,
) -> LinkAnalysis {
    if is_blocked_without_content(metadata) {
        return blocked_page_analysis(metadata);
    }

    let Some(api_key) = state.config.deepseek_api_key.as_deref() else {
        return heuristic_analysis(metadata, "DeepSeek API key 未配置，已使用本地启发式分析。");
    };

    let payload = json!({
        "model": state.config.deepseek_model,
        "thinking": { "type": "disabled" },
        "response_format": { "type": "json_object" },
        "max_tokens": 900,
        "temperature": 0.2,
        "messages": [
            {
                "role": "system",
                "content": "你是私人碎片信息整理助手。请只返回 JSON，不要 Markdown。字段必须是 summary, category, keywords, entities, sentiment, content_type, importance_score, notes。category 用中文短标签；keywords/entities 是字符串数组；importance_score 是 0-100 整数。"
            },
            {
                "role": "user",
                "content": serde_json::to_string(&json!({
                    "url": metadata.final_url,
                    "platform": metadata.platform,
                    "title": metadata.title,
                    "description": metadata.description,
                    "author": metadata.author,
                    "content_text": truncate_chars(&metadata.content_text, 7000),
                })).unwrap_or_default()
            }
        ]
    });

    let result = state
        .http
        .post(&state.config.deepseek_base_url)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await;

    match result {
        Ok(response) if response.status().is_success() => match response
            .json::<DeepSeekResponse>()
            .await
        {
            Ok(body) => body
                .choices
                .first()
                .and_then(|choice| {
                    serde_json::from_str::<LinkAnalysis>(&choice.message.content).ok()
                })
                .map(normalize_analysis)
                .unwrap_or_else(|| {
                    heuristic_analysis(
                        metadata,
                        "DeepSeek 返回内容无法解析，已使用本地启发式分析。",
                    )
                }),
            Err(error) => heuristic_analysis(metadata, &format!("DeepSeek 响应解析失败: {error}")),
        },
        Ok(response) => heuristic_analysis(
            metadata,
            &format!("DeepSeek 请求失败: HTTP {}", response.status()),
        ),
        Err(error) => heuristic_analysis(metadata, &format!("DeepSeek 请求失败: {error}")),
    }
}

pub(crate) async fn check_deepseek(state: &AppState) -> AnalyzerCheckResponse {
    let Some(api_key) = state.config.deepseek_api_key.as_deref() else {
        return AnalyzerCheckResponse {
            provider: "deepseek",
            configured: false,
            ok: false,
            model: state.config.deepseek_model.clone(),
            message: "DeepSeek API key 未配置".to_string(),
        };
    };

    let payload = json!({
        "model": state.config.deepseek_model,
        "thinking": { "type": "disabled" },
        "response_format": { "type": "json_object" },
        "max_tokens": 24,
        "temperature": 0,
        "messages": [
            {
                "role": "user",
                "content": "Return exactly this JSON object: {\"ok\":true}"
            }
        ]
    });

    match state
        .http
        .post(&state.config.deepseek_base_url)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => AnalyzerCheckResponse {
            provider: "deepseek",
            configured: true,
            ok: true,
            model: state.config.deepseek_model.clone(),
            message: "DeepSeek 连接和认证正常".to_string(),
        },
        Ok(response) => {
            let status = response.status();
            let message = match response.text().await {
                Ok(body) if !body.trim().is_empty() => {
                    format!(
                        "DeepSeek 检测失败: HTTP {status}; {}",
                        truncate_chars(&body, 180)
                    )
                }
                _ => format!("DeepSeek 检测失败: HTTP {status}"),
            };
            AnalyzerCheckResponse {
                provider: "deepseek",
                configured: true,
                ok: false,
                model: state.config.deepseek_model.clone(),
                message,
            }
        }
        Err(error) => AnalyzerCheckResponse {
            provider: "deepseek",
            configured: true,
            ok: false,
            model: state.config.deepseek_model.clone(),
            message: format!("DeepSeek 检测失败: {error}"),
        },
    }
}

#[derive(Debug, Deserialize)]
struct DeepSeekResponse {
    choices: Vec<DeepSeekChoice>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    message: DeepSeekMessage,
}

#[derive(Debug, Deserialize)]
struct DeepSeekMessage {
    content: String,
}

pub(super) fn heuristic_analysis(metadata: &LinkMetadata, note: &str) -> LinkAnalysis {
    let text = format!(
        "{} {} {}",
        metadata.title, metadata.description, metadata.content_text
    );
    let category = infer_category(&text, &metadata.platform);
    let mut keywords = vec![metadata.platform.clone(), category.clone()];
    keywords.extend(extract_keywords(&text));
    keywords.sort();
    keywords.dedup();

    LinkAnalysis {
        summary: fallback_summary(metadata),
        category,
        keywords: keywords.into_iter().take(10).collect(),
        entities: extract_entities(&text),
        sentiment: "neutral".to_string(),
        content_type: infer_content_type(&metadata.final_url, &text),
        importance_score: if metadata.content_text.chars().count() > 400 {
            66
        } else {
            48
        },
        notes: note.to_string(),
    }
}

pub(super) fn blocked_page_analysis(metadata: &LinkMetadata) -> LinkAnalysis {
    let category = infer_category("", &metadata.platform);
    let mut keywords = vec![
        metadata.platform.clone(),
        category.clone(),
        "访问验证".to_string(),
        "原链接".to_string(),
    ];
    keywords.sort();
    keywords.dedup();

    LinkAnalysis {
        summary: format!(
            "{} 页面触发访问验证，暂时未能抓取正文；已保存原链接。",
            metadata.platform
        ),
        category,
        keywords,
        entities: vec![metadata.platform.clone()],
        sentiment: "neutral".to_string(),
        content_type: infer_content_type(&metadata.final_url, &metadata.title),
        importance_score: 36,
        notes: "源站返回访问验证页面，本次只保存链接和可用分享文本。".to_string(),
    }
}

fn normalize_analysis(mut analysis: LinkAnalysis) -> LinkAnalysis {
    if analysis.summary.trim().is_empty() {
        analysis.summary = "暂无摘要".to_string();
    }
    if analysis.category.trim().is_empty() {
        analysis.category = "未分类".to_string();
    }
    analysis.keywords.retain(|value| !value.trim().is_empty());
    analysis.entities.retain(|value| !value.trim().is_empty());
    analysis.importance_score = analysis.importance_score.clamp(0, 100);
    analysis
}

pub(crate) fn build_item(
    metadata: LinkMetadata,
    analysis: LinkAnalysis,
    source_app: Option<String>,
) -> LinkItem {
    let now = Utc::now();
    LinkItem {
        id: Uuid::new_v4().to_string(),
        source_url: metadata.source_url,
        final_url: metadata.final_url,
        source_app,
        platform: metadata.platform,
        title: metadata.title,
        description: metadata.description,
        author: metadata.author,
        image_url: metadata.image_url,
        remote_image_url: None,
        cover_cached_at: None,
        cover_checked_at: None,
        content_text: metadata.content_text,
        original_text: metadata.original_text,
        summary: analysis.summary,
        category: analysis.category,
        keywords: analysis.keywords,
        entities: analysis.entities,
        sentiment: analysis.sentiment,
        content_type: analysis.content_type,
        importance_score: analysis.importance_score,
        notes: analysis.notes,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
    }
}
