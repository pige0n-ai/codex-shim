use protocol::error::ApiError;

/// Map an upstream HTTP error (status code + body) to an ApiError.
pub fn map_upstream_error(status: u16, body: &str) -> ApiError {
    match status {
        401 => ApiError::upstream_auth_error(upstream_error_message(
            body,
            "Upstream authentication failed",
        )),
        429 => ApiError::upstream_rate_limited(upstream_error_message(
            body,
            "Upstream rate limit exceeded",
        )),
        500..=599 => {
            ApiError::upstream_error(upstream_error_message(body, "Upstream internal error"))
        }
        400 => ApiError::new(
            upstream_error_message(body, "Bad request to upstream"),
            "invalid_request_error",
        ),
        _ => ApiError::upstream_error(first_line_or(
            body,
            &format!("Upstream returned HTTP {status}"),
        )),
    }
}

pub fn is_retryable_upstream_error(status: u16, _body: &str) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn upstream_error_message(body: &str, default: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
        if let Some(msg) = parsed.get("message").and_then(|m| m.as_str()) {
            return msg.to_string();
        }
    }
    first_line_or(body, default)
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
