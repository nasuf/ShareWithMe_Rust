//! Optional bearer-token guard for `/api` routes (`SHARE_WITH_ME_API_TOKEN`).

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};

use crate::config::AppConfig;

pub(super) async fn require_api_token(
    State(config): State<Arc<AppConfig>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(expected) = config.api_token.as_deref() else {
        return Ok(next.run(request).await);
    };
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    if !supplied.is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

pub(super) fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

pub(super) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}
