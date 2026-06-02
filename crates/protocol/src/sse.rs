use serde::{Deserialize, Serialize};

use crate::common::Usage;
use crate::error::ApiErrorBody;
use crate::responses::ResponseOutputItem;

/// All SSE events the adapter can emit on a Responses stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseSseEvent {
    #[serde(rename = "response.created")]
    ResponseCreated { response: SseResponseShell },

    #[serde(rename = "response.in_progress")]
    ResponseInProgress { response: SseResponseShell },

    #[serde(rename = "response.output_item.added")]
    ResponseOutputItemAdded {
        output_index: u32,
        item: ResponseOutputItem,
    },

    #[serde(rename = "response.reasoning_summary_part.added")]
    ResponseReasoningSummaryPartAdded {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: crate::responses::SummaryPart,
    },

    #[serde(rename = "response.reasoning_summary_text.delta")]
    ResponseReasoningSummaryTextDelta {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        delta: String,
    },

    #[serde(rename = "response.reasoning_summary_text.done")]
    ResponseReasoningSummaryTextDone {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        text: String,
    },

    #[serde(rename = "response.reasoning_summary_part.done")]
    ResponseReasoningSummaryPartDone {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: crate::responses::SummaryPart,
    },

    #[serde(rename = "response.content_part.added")]
    ResponseContentPartAdded {
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: crate::common::ContentPart,
    },

    #[serde(rename = "response.output_text.delta")]
    ResponseOutputTextDelta {
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
    },

    #[serde(rename = "response.output_text.done")]
    ResponseOutputTextDone {
        item_id: String,
        output_index: u32,
        content_index: u32,
        text: String,
    },

    #[serde(rename = "response.content_part.done")]
    ResponseContentPartDone {
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: crate::common::ContentPart,
    },

    #[serde(rename = "response.output_item.done")]
    ResponseOutputItemDone {
        output_index: u32,
        item: ResponseOutputItem,
    },

    #[serde(rename = "response.function_call_arguments.delta")]
    ResponseFunctionCallArgumentsDelta {
        item_id: String,
        output_index: u32,
        delta: String,
    },

    #[serde(rename = "response.function_call_arguments.done")]
    ResponseFunctionCallArgumentsDone {
        item_id: String,
        output_index: u32,
        arguments: String,
        name: String,
    },

    #[serde(rename = "response.custom_tool_call_input.delta")]
    ResponseCustomToolCallInputDelta {
        item_id: String,
        output_index: u32,
        delta: String,
    },

    #[serde(rename = "response.completed")]
    ResponseCompleted { response: SseResponseShell },

    #[serde(rename = "response.failed")]
    ResponseFailed { response: SseResponseShell },

    #[serde(rename = "error")]
    Error { error: crate::error::ApiErrorBody },
}

/// Lightweight response shell used in SSE lifecycle events.
/// Only `response.created` and `response.completed` include it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseResponseShell {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<ResponseOutputItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiErrorBody>,
}

impl SseResponseShell {
    pub fn minimal(id: String, model: String, created_at: i64) -> Self {
        Self {
            id,
            object: "response".into(),
            created_at,
            status: "in_progress".into(),
            model,
            output: None,
            output_text: None,
            usage: None,
            previous_response_id: None,
            error: None,
        }
    }
}
