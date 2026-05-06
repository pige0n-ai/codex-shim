use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::config::AuthConfig;

/// Middleware that performs bearer token validation.
/// - `/healthz` is always allowed (no auth required).
/// - If `accepted_bearer_tokens` is empty: all other requests pass (no-op).
/// - If tokens are configured, every other request MUST carry a valid Bearer token.
pub async fn auth_middleware(
    config: &'static AuthConfig,
    request: Request,
    next: Next,
) -> Response {
    // Health check is always allowed
    if request.uri().path() == "/healthz" {
        return next.run(request).await;
    }

    if config.accepted_bearer_tokens.is_empty() {
        return next.run(request).await;
    }

    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header_value) => {
            let token = header_value.strip_prefix("Bearer ").unwrap_or(header_value);
            if config.accepted_bearer_tokens.iter().any(|t| t == token) {
                return next.run(request).await;
            }
            (axum::http::StatusCode::UNAUTHORIZED, "Invalid bearer token").into_response()
        }
        None => (
            axum::http::StatusCode::UNAUTHORIZED,
            "Authorization header required",
        )
            .into_response(),
    }
}
