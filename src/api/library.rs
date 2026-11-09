//! Collection-wide endpoints: stats, digest, export and import.

use axum::{
    Json,
    extract::{Query, State},
    http::header,
    response::IntoResponse,
};

use crate::{
    digest::{collection_digest, count_by},
    error::ApiError,
    links::is_safe_remote_url,
    models::{
        CollectionDigest, CollectionStats, JsonImportRequest, JsonImportResponse, ListItemsQuery,
    },
    state::AppState,
    store::{all_items, import_items},
};

use super::collection::{build_json_export, build_markdown_export, matches_filter};

pub(super) async fn stats(
    State(state): State<AppState>,
    Query(query): Query<ListItemsQuery>,
) -> Result<Json<CollectionStats>, ApiError> {
    let mut items = all_items(&state.store).await.map_err(ApiError::internal)?;
    items.retain(|item| matches_filter(item, &query));
    let categories = count_by(items.iter().map(|item| item.category.clone()));
    let platforms = count_by(items.iter().map(|item| item.platform.clone()));
    let keywords = count_by(items.iter().flat_map(|item| item.keywords.clone()));

    Ok(Json(CollectionStats {
        total: items.len(),
        categories,
        platforms,
        keywords,
    }))
}

pub(super) async fn digest(
    State(state): State<AppState>,
) -> Result<Json<CollectionDigest>, ApiError> {
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
    Ok(Json(collection_digest(&items)))
}

pub(super) async fn export_markdown(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
    let markdown = build_markdown_export(&items);
    Ok((
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"share-with-me-export.md\"",
            ),
        ],
        markdown,
    ))
}

pub(super) async fn export_json(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let items = all_items(&state.store).await.map_err(ApiError::internal)?;
    let body = serde_json::to_string_pretty(&build_json_export(items))
        .map_err(|error| ApiError::internal(error.into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"share-with-me-export.json\"",
            ),
        ],
        body,
    ))
}

pub(super) async fn import_json(
    State(state): State<AppState>,
    Json(request): Json<JsonImportRequest>,
) -> Result<Json<JsonImportResponse>, ApiError> {
    let request = validate_json_import(request)?;
    let imported_count = request.items.len();
    let (created_count, merged_count) = import_items(&state.store, request.items)
        .await
        .map_err(ApiError::internal)?;
    let total_count = all_items(&state.store)
        .await
        .map_err(ApiError::internal)?
        .len();
    Ok(Json(JsonImportResponse {
        imported_count,
        created_count,
        merged_count,
        total_count,
    }))
}

fn validate_json_import(request: JsonImportRequest) -> Result<JsonImportRequest, ApiError> {
    if request.schema_version != 1 {
        return Err(ApiError::bad_request(
            "unsupported JSON backup schema_version",
        ));
    }
    if request.items.is_empty() {
        return Err(ApiError::bad_request("JSON backup has no items"));
    }
    for item in &request.items {
        if item.id.trim().is_empty() {
            return Err(ApiError::bad_request("item id cannot be empty"));
        }
        if item.source_url.trim().is_empty() || item.final_url.trim().is_empty() {
            return Err(ApiError::bad_request("item URLs cannot be empty"));
        }
        for raw_url in [&item.source_url, &item.final_url] {
            let url = url::Url::parse(raw_url)
                .map_err(|_| ApiError::bad_request("item URLs must be valid"))?;
            if !is_safe_remote_url(&url) {
                return Err(ApiError::bad_request(
                    "item URLs must point to public network hosts",
                ));
            }
        }
        if item.title.trim().is_empty() {
            return Err(ApiError::bad_request("item title cannot be empty"));
        }
        if !["active", "archived", "favorite"].contains(&item.status.as_str()) {
            return Err(ApiError::bad_request(
                "item status must be active, archived, or favorite",
            ));
        }
    }
    Ok(request)
}
