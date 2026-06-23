use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    pub(crate) summary: String,
    pub(crate) category: String,
    pub(crate) keywords: Vec<String>,
    pub(crate) entities: Vec<String>,
    pub(crate) sentiment: String,
    pub(crate) content_type: String,
    pub(crate) importance_score: u8,
    pub(crate) notes: String,
    pub(crate) status: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
