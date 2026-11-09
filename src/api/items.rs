//! Reading and editing saved items.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::{
    error::ApiError,
    models::{HealthResponse, LinkItem, ListItemsQuery, UpdateItemRequest, UpdateStatusRequest},
    state::AppState,
    store::{all_items, delete_item_by_id, find_item, update_item_metadata, update_item_status},
};

use super::collection::{matches_filter, sort_items};

pub(super) async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        analyzer_provider: "deepseek",
        deepseek_configured: state.config.deepseek_api_key.is_some(),
        deepseek_model: state.config.deepseek_model.clone(),
        storage_path: state.config.storage_label(),
        storage_backend: state.store.backend_name().to_string(),
    })
}

pub(super) async fn list_items(
    State(state): State<AppState>,
    Query(query): Query<ListItemsQuery>,
) -> Result<Json<Vec<LinkItem>>, ApiError> {
    let mut items = all_items(&state.store).await.map_err(ApiError::internal)?;
    items.retain(|item| matches_filter(item, &query));
    sort_items(&mut items, query.sort.as_deref());
    if query.fields.as_deref() == Some("summary") {
        items = items.into_iter().map(summary_item).collect();
    }
    Ok(Json(items))
}

// The library list only needs card fields. Full article text (stored in up to
// four forms) is ~80% of the payload, so `fields=summary` drops it and clients
// fetch GET /api/items/{id} when an item is opened. Opt-in, so older app builds
// that read content from the list keep working.
fn summary_item(mut item: LinkItem) -> LinkItem {
    item.content_text.clear();
    item.original_text.clear();
    item.original_content = Default::default();
    item.content_omitted = true;
    item
}

pub(super) async fn get_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LinkItem>, ApiError> {
    let item = find_item(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("item not found"))?;
    Ok(Json(item))
}

pub(super) async fn delete_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !delete_item_by_id(&state.store, &id)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::not_found("item not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateStatusRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let allowed = ["active", "archived", "favorite"];
    if !allowed.contains(&request.status.as_str()) {
        return Err(ApiError::bad_request(
            "status must be active, archived, or favorite",
        ));
    }
    update_item_status(&state.store, &id, &request.status)
        .await
        .map_err(ApiError::internal)?;
    get_item(State(state), Path(id)).await
}

pub(super) async fn update_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateItemRequest>,
) -> Result<Json<LinkItem>, ApiError> {
    let request = normalize_update_request(request)?;
    let item = update_item_metadata(&state.store, &id, request)
        .await
        .map_err(|error| {
            if error.to_string().contains("item not found") {
                ApiError::not_found("item not found")
            } else {
                ApiError::internal(error)
            }
        })?;
    Ok(Json(item))
}

fn normalize_update_request(mut request: UpdateItemRequest) -> Result<UpdateItemRequest, ApiError> {
    if let Some(category) = request.category {
        let category = category.trim().to_string();
        if category.is_empty() {
            return Err(ApiError::bad_request("category cannot be empty"));
        }
        request.category = Some(category);
    }

    if let Some(keywords) = request.keywords {
        let mut normalized = Vec::new();
        for keyword in keywords {
            let keyword = keyword.trim().to_string();
            if !keyword.is_empty() && !normalized.contains(&keyword) {
                normalized.push(keyword);
            }
            if normalized.len() >= 20 {
                break;
            }
        }
        request.keywords = Some(normalized);
    }

    if let Some(notes) = request.notes {
        request.notes = Some(notes.trim().to_string());
    }

    Ok(request)
}
