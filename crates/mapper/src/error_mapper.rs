use protocol::error::ApiError;

/// Map an upstream HTTP error (status code + body) to an ApiError.
pub fn map_upstream_error(status: u16, body: &str) -> ApiError {
    match status {
        401 => ApiError::upstream_auth_error(first_line_or(body, "Upstream authentication failed")),
        429 => ApiError::upstream_rate_limited(first_line_or(body, "Upstream rate limit exceeded")),
        500..=599 => ApiError::upstream_error(first_line_or(body, "Upstream internal error")),
        400 => {
            // Try to extract a useful message
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body)
                && let Some(msg) = parsed
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
            {
                return ApiError::new(msg.to_string(), "invalid_request_error");
            }
            ApiError::new(
                first_line_or(body, "Bad request to upstream"),
                "invalid_request_error",
            )
        }
        _ => ApiError::upstream_error(first_line_or(
            body,
            &format!("Upstream returned HTTP {status}"),
        )),
    }
}

fn first_line_or<'a>(text: &'a str, default: &'a str) -> String {
    let line = text.lines().next().unwrap_or(default);
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 200 {
        // Truncate long error bodies
        default.into()
    } else {
        trimmed.into()
    }
}
