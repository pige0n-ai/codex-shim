use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error.message)
    }
}

impl std::fmt::Display for ApiErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ApiError {
    pub fn new(message: impl Into<String>, error_type: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                message: message.into(),
                error_type: error_type.into(),
                param: None,
                code: None,
            },
        }
    }

    pub fn with_param(
        message: impl Into<String>,
        error_type: impl Into<String>,
        param: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        Self {
            error: ApiErrorBody {
                message: message.into(),
                error_type: error_type.into(),
                param: Some(param.into()),
                code: Some(code.into()),
            },
        }
    }

    pub fn invalid_parameter(
        param: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::with_param(message, "invalid_request_error", param, code)
    }

    // --- Common error constructors ---

    pub fn invalid_json(details: impl Into<String>) -> Self {
        Self::with_param(details, "invalid_request_error", "body", "invalid_json")
    }

    pub fn missing_model() -> Self {
        Self::invalid_parameter("model", "missing_required_parameter", "model is required")
    }

    pub fn unsupported_tool_type(tool_type: &str, index: usize) -> Self {
        Self::invalid_parameter(
            format!("tools[{index}].type"),
            "unsupported_tool_type",
            format!("Unsupported tool type: {tool_type}"),
        )
    }

    pub fn unknown_parameter(param: &str) -> Self {
        Self::invalid_parameter(
            param,
            "unknown_parameter",
            format!("Unknown parameter: '{param}'"),
        )
    }

    pub fn unsupported_include(value: &str) -> Self {
        Self::invalid_parameter(
            "include",
            "unsupported_include",
            format!("include entry '{value}' is not supported by this adapter"),
        )
    }

    pub fn unsupported_input_item(item_type: &str) -> Self {
        Self::invalid_parameter(
            "input",
            "unsupported_input_item",
            format!("input item type '{item_type}' is not supported by this adapter"),
        )
    }

    pub fn unknown_input_item(item_type: Option<&str>) -> Self {
        let label = item_type.unwrap_or("<missing>");
        Self::invalid_parameter(
            "input",
            "unknown_input_item",
            format!("Unknown input item type: '{label}'"),
        )
    }

    pub fn unsupported_content_part(part_type: &str) -> Self {
        Self::invalid_parameter(
            "input",
            "unsupported_content_part",
            format!("content part type '{part_type}' is not supported by this adapter"),
        )
    }

    pub fn endpoint_not_implemented(detail: impl Into<String>) -> Self {
        Self::new(detail, "not_implemented")
    }

    pub fn response_not_found(id: &str) -> Self {
        Self::invalid_parameter(
            "previous_response_id",
            "response_not_found",
            format!("Response {id} not found"),
        )
    }

    pub fn debug_artifact_expired(id: &str) -> Self {
        Self::with_param(
            format!("Debug artifact for response {id} has expired"),
            "invalid_request_error",
            "response_id",
            "debug_artifact_expired",
        )
    }

    pub fn upstream_auth_error(details: impl Into<String>) -> Self {
        Self::new(details, "upstream_auth_error")
    }

    pub fn upstream_timeout() -> Self {
        Self::new("Upstream request timed out", "upstream_timeout")
    }

    pub fn upstream_error(details: impl Into<String>) -> Self {
        Self::new(details, "upstream_error")
    }

    pub fn upstream_rate_limited(details: impl Into<String>) -> Self {
        Self::new(details, "upstream_rate_limited")
    }

    pub fn stream_interrupted() -> Self {
        Self::new("Upstream stream interrupted", "stream_interrupted")
    }

    pub fn stream_interrupted_with_details(details: impl Into<String>) -> Self {
        Self::new(
            format!("Upstream stream interrupted: {}", details.into()),
            "stream_interrupted",
        )
    }

    pub fn internal(details: impl Into<String>) -> Self {
        Self::new(details, "internal_error")
    }

    pub fn file_input_not_supported() -> Self {
        Self::new(
            "input_file / file inputs are not supported.              Multipart uploads and server-side file retrieval are not implemented.              Use input_image or input_text instead.",
            "not_implemented",
        )
    }

    pub fn field_not_implemented(field: &str) -> Self {
        Self::with_param(
            format!("The field '{field}' is not implemented by this adapter"),
            "not_implemented",
            field,
            "not_implemented",
        )
    }

    pub fn not_implemented() -> Self {
        Self::new(
            "This feature is not implemented by the adapter",
            "not_implemented",
        )
    }

    pub fn hosted_tool_not_supported(tool_type: &str) -> Self {
        Self::new(
            format!(
                "OpenAI hosted tool '{tool_type}' is not available through this adapter. \
                 It requires OpenAI server-side infrastructure (vector stores, file upload, sandbox). \
                 Chat Completions backends cannot execute this tool."
            ),
            "not_implemented",
        )
    }

    pub fn previous_response_id_requires_store() -> Self {
        Self::invalid_parameter(
            "previous_response_id",
            "store_required",
            "store must be enabled to use previous_response_id",
        )
    }
}
