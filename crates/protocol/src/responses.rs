use serde::{Deserialize, Serialize};

use crate::common::{ContentPart, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesCreateRequest {
    pub model: String,
    pub input: ResponseInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponseTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// input can be a plain string, an array of items, or a raw JSON value.
/// Order matters for untagged: try string first, then typed array, then catch-all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseInput {
    Text(String),
    Items(Vec<InputItem>),
    /// Catch-all for single objects, numbers, booleans, null, or arrays with
    /// items that don't parse as InputItem.
    Value(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputItem {
    #[serde(rename = "message")]
    Message {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        role: InputMessageRole,
        content: MessageContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        /// output can be a plain string or an array of structured content items.
        /// The mapper converts this to a plain string for Chat Completions.
        output: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "custom_tool_call")]
    CustomToolCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "custom_tool_call_output")]
    CustomToolCallOutput {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        output: String,
    },
    #[serde(rename = "local_shell_call")]
    LocalShellCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename = "local_shell_call_output")]
    LocalShellCallOutput {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        output: String,
    },
    #[serde(rename = "apply_patch_call")]
    ApplyPatchCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        patch: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    #[serde(rename = "apply_patch_call_output")]
    ApplyPatchCallOutput {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        output: String,
    },
    #[serde(rename = "mcp_call")]
    McpCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        server: String,
        tool: String,
        arguments: serde_json::Value,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "file_search_call")]
    FileSearchCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ContentPart>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<SummaryPart>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "input_file")]
    InputFile {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
    /// Catch-all for unknown item types (prevents 422 on unrecognized items).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputMessageRole {
    #[default]
    User,
    System,
    Developer,
    Assistant,
}

/// Content in a message input item can be a plain string or an array of content parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SummaryPart {
    #[serde(rename = "summary_text")]
    SummaryText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseTool {
    #[serde(rename = "function")]
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
    #[serde(rename = "web_search_preview")]
    #[serde(alias = "web_search")]
    WebSearchPreview {
        #[serde(skip_serializing_if = "Option::is_none")]
        user_location: Option<WebSearchUserLocation>,
        #[serde(skip_serializing_if = "Option::is_none")]
        search_context_size: Option<String>,
    },
    #[serde(rename = "file_search")]
    FileSearch {
        vector_store_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_num_results: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filters: Option<serde_json::Value>,
    },
    #[serde(rename = "computer_use")]
    ComputerUse {
        display_width: u32,
        display_height: u32,
        environment: String,
    },
    #[serde(rename = "code_interpreter")]
    CodeInterpreter {
        #[serde(skip_serializing_if = "Option::is_none")]
        container_image: Option<String>,
    },
    #[serde(rename = "mcp")]
    Mcp {
        server_label: String,
        server_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_description: Option<String>,
    },
    /// Catch-all for future/unknown tool types.
    #[serde(other)]
    UnknownTool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchUserLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Auto(String), // "auto", "none", "required"
    Specific {
        #[serde(rename = "type")]
        choice_type: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextConfig {
    pub format: TextFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TextFormat {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "json_object")]
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        schema_: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}

// --- Response types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesObject {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_details: Option<StatusDetails>,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::ApiErrorBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseOutputItem {
    #[serde(rename = "message")]
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ContentPart>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        status: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ContentPart>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<SummaryPart>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "custom_tool_call")]
    CustomToolCall {
        id: String,
        status: String,
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "local_shell_call")]
    LocalShellCall {
        id: String,
        status: String,
        call_id: String,
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename = "apply_patch_call")]
    ApplyPatchCall {
        id: String,
        status: String,
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        patch: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    #[serde(rename = "mcp_call")]
    McpCall {
        id: String,
        status: String,
        call_id: String,
        server: String,
        tool: String,
        arguments: serde_json::Value,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall { id: String, status: String },
    #[serde(rename = "file_search_call")]
    FileSearchCall { id: String, status: String },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall { id: String, status: String },
    /// Catch-all for future/unknown output item types.
    #[serde(other)]
    UnknownOutputItem,
}
