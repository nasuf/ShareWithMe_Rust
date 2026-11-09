use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::content::{ArchiveSnapshot, ExtractionState, MediaAsset, OriginalContent, SourceFact};

#[derive(Debug, Deserialize)]
pub(crate) struct AnalyzeLinkRequest {
    pub(crate) url: Option<String>,
    pub(crate) shared_text: Option<String>,
    pub(crate) source_app: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListItemsQuery {
    pub(crate) query: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) platform: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) sort: Option<String>,
    // "summary" returns card fields only (no article text).
    pub(crate) fields: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateStatusRequest {
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateItemRequest {
    pub(crate) category: Option<String>,
    pub(crate) keywords: Option<Vec<String>>,
    pub(crate) notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct LinkItem {
    pub(crate) id: String,
    pub(crate) source_url: String,
    pub(crate) final_url: String,
    pub(crate) source_app: Option<String>,
    pub(crate) platform: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) author: Option<String>,
    pub(crate) image_url: Option<String>,
    #[serde(default)]
    pub(crate) remote_image_url: Option<String>,
    #[serde(default)]
    pub(crate) cover_cached_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) cover_checked_at: Option<DateTime<Utc>>,
    pub(crate) content_text: String,
    #[serde(default)]
    pub(crate) original_text: String,
    #[serde(default)]
    pub(crate) shared_text: String,
    #[serde(default)]
    pub(crate) original_content: OriginalContent,
    #[serde(default)]
    pub(crate) media: Vec<MediaAsset>,
    #[serde(default)]
    pub(crate) source_facts: Vec<SourceFact>,
    #[serde(default)]
    pub(crate) source_id: Option<String>,
    #[serde(default)]
    pub(crate) published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) extraction: ExtractionState,
    #[serde(default)]
    pub(crate) archive: Option<ArchiveSnapshot>,
    // Response-only marker for list rows with article text stripped; never
    // persisted, so it is skipped unless true.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) content_omitted: bool,
    pub(crate) summary: String,
    pub(crate) category: String,
    pub(crate) keywords: Vec<String>,
    pub(crate) entities: Vec<String>,
    pub(crate) sentiment: String,
    pub(crate) content_type: String,
    pub(crate) importance_score: u8,
    pub(crate) notes: String,
    // True while AI analysis is still running in the background; the client can
    // show a pending state and refresh when it clears.
    #[serde(default)]
    pub(crate) analysis_pending: bool,
    #[serde(default)]
    pub(crate) user_edited_at: Option<DateTime<Utc>>,
    pub(crate) status: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub(crate) struct LinkMetadata {
    pub(crate) source_url: String,
    pub(crate) final_url: String,
    pub(crate) platform: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) author: Option<String>,
    pub(crate) image_url: Option<String>,
    pub(crate) content_text: String,
    #[serde(default)]
    pub(crate) original_text: String,
    #[serde(default)]
    pub(crate) shared_text: String,
    #[serde(default)]
    pub(crate) original_content: OriginalContent,
    #[serde(default)]
    pub(crate) media: Vec<MediaAsset>,
    #[serde(default)]
    pub(crate) source_facts: Vec<SourceFact>,
    #[serde(default)]
    pub(crate) source_id: Option<String>,
    #[serde(default)]
    pub(crate) published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) extraction: ExtractionState,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct LinkAnalysis {
    pub(crate) summary: String,
    pub(crate) category: String,
    pub(crate) keywords: Vec<String>,
    pub(crate) entities: Vec<String>,
    pub(crate) sentiment: String,
    pub(crate) content_type: String,
    pub(crate) importance_score: u8,
    pub(crate) notes: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) analyzer_provider: &'static str,
    pub(crate) deepseek_configured: bool,
    pub(crate) deepseek_model: String,
    pub(crate) storage_path: String,
    pub(crate) storage_backend: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AnalyzerCheckResponse {
    pub(crate) provider: &'static str,
    pub(crate) configured: bool,
    pub(crate) ok: bool,
    pub(crate) model: String,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CollectionStats {
    pub(crate) total: usize,
    pub(crate) categories: Vec<CountBucket>,
    pub(crate) platforms: Vec<CountBucket>,
    pub(crate) keywords: Vec<CountBucket>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CountBucket {
    pub(crate) label: String,
    pub(crate) count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct CollectionDigest {
    pub(crate) total: usize,
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) focus_summary: String,
    pub(crate) category_summaries: Vec<CountBucket>,
    pub(crate) recent_items: Vec<DigestItem>,
    pub(crate) suggestions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DigestItem {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) platform: String,
    pub(crate) category: String,
    pub(crate) summary: String,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonExport {
    pub(crate) schema_version: u8,
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) item_count: usize,
    pub(crate) digest: CollectionDigest,
    pub(crate) items: Vec<LinkItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonImportRequest {
    pub(crate) schema_version: u8,
    pub(crate) items: Vec<LinkItem>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonImportResponse {
    pub(crate) imported_count: usize,
    pub(crate) created_count: usize,
    pub(crate) merged_count: usize,
    pub(crate) total_count: usize,
}

/// A minimal saved item for tests.
#[cfg(test)]
pub(crate) fn test_item(id: &str, final_url: &str, title: &str) -> LinkItem {
    let now = Utc::now();
    LinkItem {
        id: id.to_string(),
        source_url: final_url.to_string(),
        final_url: final_url.to_string(),
        source_app: None,
        platform: crate::platforms::platform_for_url(final_url),
        title: title.to_string(),
        description: String::new(),
        author: None,
        image_url: None,
        remote_image_url: None,
        cover_cached_at: None,
        cover_checked_at: None,
        content_text: String::new(),
        original_text: String::new(),
        shared_text: String::new(),
        original_content: Default::default(),
        media: Vec::new(),
        source_facts: Vec::new(),
        source_id: None,
        published_at: None,
        extraction: Default::default(),
        archive: None,
        content_omitted: false,
        summary: title.to_string(),
        category: "待整理".to_string(),
        keywords: vec![],
        entities: vec![],
        sentiment: "neutral".to_string(),
        content_type: "link".to_string(),
        importance_score: 50,
        notes: String::new(),
        analysis_pending: false,
        user_edited_at: None,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
    }
}
