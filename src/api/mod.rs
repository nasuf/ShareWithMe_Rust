//! HTTP API: routing, authentication and the request handlers.
//!
//! * [`items`]   — list, read, edit and delete saved items
//! * [`ingest`]  — save new links (`/api/links/...`)
//! * [`refresh`] — re-fetch / re-analyse items without losing saved content
//! * [`media`]   — cached cover and article images
//! * [`archive`] — archive screenshots of the original page
//! * [`library`] — stats, digest, export and import

use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, patch, post},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
};

use crate::state::AppState;

mod archive;
mod auth;
mod collection;
mod ingest;
mod items;
mod library;
mod media;
mod refresh;

#[cfg(test)]
mod api_tests;

pub(crate) use media::repair_wechat_placeholders;

/// `?force=true` bypasses refresh cooldowns.
#[derive(Default, serde::Deserialize)]
pub(super) struct RefreshQuery {
    pub(super) force: Option<bool>,
}

pub(crate) fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/items", get(items::list_items))
        .route(
            "/api/items/{id}",
            get(items::get_item)
                .patch(items::update_item)
                .delete(items::delete_item),
        )
        .route("/api/items/{id}/status", patch(items::update_status))
        .route("/api/items/{id}/reanalyze", post(refresh::reanalyze_item))
        .route(
            "/api/items/{id}/refresh",
            post(refresh::refresh_item_content),
        )
        .route("/api/items/{id}/cover/refresh", post(media::refresh_cover))
        .route("/api/items/{id}/archive", post(archive::recapture_archive))
        .route("/api/links/analyze", post(ingest::analyze_link))
        .route("/api/links/analyze-many", post(ingest::analyze_links))
        .route("/api/stats", get(library::stats))
        .route("/api/digest", get(library::digest))
        .route("/api/export/markdown", get(library::export_markdown))
        .route("/api/export/json", get(library::export_json))
        .route("/api/import/json", post(library::import_json))
        .route("/api/analyzer/check", post(ingest::check_analyzer))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.config),
            auth::require_api_token,
        ));

    Router::new()
        .route("/health", get(items::health))
        .route("/media/{file}", get(media::media_file))
        .merge(protected)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        // JSON shrinks ~6x; the default predicate leaves JPEG media alone.
        .layer(CompressionLayer::new())
        .with_state(state)
}
