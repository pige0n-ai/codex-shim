use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::{
    ChatCompletionRequest, ChatContent, ChatFunctionCall, ChatImageUrl, ChatMessage, ChatTool,
    ChatToolCall,
};
use crate::common::ContentPart;
use crate::error::ApiError;
use crate::provider_caps::{ProviderCapabilities, ReasoningPolicy, ToolPolicy};
use crate::responses::{
    InputItem, InputMessageRole, MessageContent, NamespaceTool, ResponseInput, ResponseTool,
    ResponsesCreateRequest, TextFormat, ToolChoice as ResponsesToolChoice,
};

// ── Canonical IR ──────────────────────────────────────────────────

/// Unified internal representation of a Responses API request.
/// All three endpoint modes derive their upstream payload from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalRequest {
    pub model: String,
    pub instructions: Option<String>,
    pub items: Vec<CanonicalItem>,
    pub tools: Vec<CanonicalTool>,
    pub tool_choice: ToolChoice,
    pub sampling: CanonicalSampling,
    pub reasoning: Option<CanonicalReasoning>,
    pub text_format: Option<CanonicalTextFormat>,
    pub parallel_tool_calls: Option<bool>,
    pub stream: bool,
    pub previous_response_id: Option<String>,
    pub store: Option<bool>,
    pub metadata: Value,
    /// Preserved from request for Native/Stateless passthrough.
    /// Chat shim ignores this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<serde_json::Value>,
    /// History messages prepended from a prior store lookup.
    pub history_messages: Vec<ChatMessage>,
    /// Warnings from the mapping process (e.g. filtered hosted tools).
    #[serde(default)]
    pub host_tool_warnings: Vec<String>,
    /// Raw ResponseTool list preserved for Native/Stateless proxy paths.
    /// Chat shim filters these; proxy forwards them directly.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub response_tools_raw: Vec<ResponseTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CanonicalItem {
    Message(CanonicalMessage),
    FunctionCall(CanonicalFunctionCall),
    FunctionCallOutput(CanonicalFunctionCallOutput),
    Reasoning(CanonicalReasoningItem),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMessage {
    pub role: CanonicalMessageRole,
    pub content: Vec<CanonicalContentPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CanonicalMessageRole {
    User,
    System,
    Developer,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CanonicalContentPart {
    Text(String),
    ImageUrl(String, Option<String>), // url, detail
    OutputText(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalFunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalFunctionCallOutput {
    pub call_id: String,
    pub output: String, // already flattened to text
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalReasoningItem {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalTool {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Specific(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalSampling {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalReasoning {
    pub effort: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CanonicalTextFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        description: Option<String>,
        schema: Option<Value>,
        strict: Option<bool>,
    },
}

// ── Parser ────────────────────────────────────────────────────────

impl CanonicalRequest {
    /// Parse a `ResponsesCreateRequest` into canonical IR.
    /// `history_messages` comes from the store via `previous_response_id`.
    pub fn from_request(
        req: &ResponsesCreateRequest,
        history_messages: Vec<ChatMessage>,
    ) -> Result<Self, String> {
        let items = parse_input_items(&req.input)?;
        validate_reasoning_items(&items)?;
        let mut host_tool_warnings: Vec<String> = Vec::new();
        let raw_tools = req.tools.clone().unwrap_or_default();
        let tools = req
            .tools
            .as_ref()
            .map(|t| parse_tools(t, &mut host_tool_warnings))
            .transpose()?
            .unwrap_or_default();
        let tool_choice = req.tool_choice.as_ref().map(parse_tool_choice);
        let reasoning = req.reasoning.as_ref().map(|r| CanonicalReasoning {
            effort: r.effort.clone(),
            summary: r.summary.clone(),
        });
        let text_format = req.text.as_ref().map(|t| parse_text_format(&t.format));

        Ok(Self {
            model: req.model.clone(),
            instructions: req.instructions.clone(),
            items,
            host_tool_warnings,
            response_tools_raw: raw_tools,
            tools,
            tool_choice: tool_choice.unwrap_or(ToolChoice::Auto),
            sampling: CanonicalSampling {
                temperature: req.temperature,
                top_p: req.top_p,
                max_output_tokens: req.max_output_tokens,
            },
            reasoning,
            text_format,
            parallel_tool_calls: req.parallel_tool_calls,
            stream: req.stream.unwrap_or(false),
            previous_response_id: req.previous_response_id.clone(),
            store: req.store,
            metadata: req.metadata.clone().unwrap_or(Value::Null),
            include: req.include.clone(),
            history_messages,
        })
    }

    // ── Derivations ────────────────────────────────────────────

    /// Derive a Chat Completion request from this canonical IR.
    pub fn into_chat_request(&self, caps: &ProviderCapabilities) -> ChatCompletionRequest {
        let mut messages: Vec<ChatMessage> = Vec::new();

        // 1. Prepend history messages
        messages.extend(self.history_messages.clone());

        // 2. Instructions → system message
        if let Some(ref inst) = self.instructions {
            messages.insert(
                0,
                ChatMessage::System {
                    content: ChatContent::Text(inst.clone()),
                    name: None,
                },
            );
        }

        // 3. Map canonical items → chat messages
        let mut pending_reasoning: Option<String> = None;
        for item in &self.items {
            match item {
                CanonicalItem::Message(msg) => {
                    let chat_msg = canonical_message_to_chat(msg, &mut pending_reasoning);
                    messages.push(chat_msg);
                }
                CanonicalItem::FunctionCall(call) => {
                    messages.push(ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(vec![ChatToolCall {
                            id: call.call_id.clone(),
                            call_type: "function".into(),
                            function: ChatFunctionCall {
                                name: Some(call.name.clone()),
                                arguments: call.arguments.clone(),
                            },
                        }]),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                CanonicalItem::FunctionCallOutput(output) => {
                    messages.push(ChatMessage::Tool {
                        content: ChatContent::Text(output.output.clone()),
                        tool_call_id: output.call_id.clone(),
                    });
                }
                CanonicalItem::Reasoning(reasoning) => {
                    let existing = pending_reasoning.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&reasoning.text);
                }
            }
        }
        // 4. Enforce tool call adjacency required by OpenAI/DeepSeek API
        enforce_tool_call_adjacency(&mut messages);

        let tools = if caps.tool_policy == ToolPolicy::NoTools || self.tools.is_empty() {
            None
        } else {
            let chat_tools: Vec<ChatTool> = self
                .tools
                .iter()
                .map(|t| ChatTool {
                    tool_type: "function".into(),
                    function: crate::chat::ChatFunctionDef {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                        strict: t.strict,
                    },
                })
                .collect();
            Some(chat_tools)
        };

        let response_format = self.text_format.as_ref().and_then(|tf| match tf {
            CanonicalTextFormat::JsonObject => Some(serde_json::json!({"type": "json_object"})),
            CanonicalTextFormat::JsonSchema {
                name,
                description,
                schema,
                strict,
            } => {
                let mut obj =
                    serde_json::json!({"type": "json_schema", "json_schema": {"name": name}});
                if let Some(d) = description {
                    obj["json_schema"]["description"] = Value::String(d.clone());
                }
                if let Some(s) = schema {
                    obj["json_schema"]["schema"] = s.clone();
                }
                if let Some(s) = strict {
                    obj["json_schema"]["strict"] = Value::Bool(*s);
                }
                Some(obj)
            }
            CanonicalTextFormat::Text => None,
        });

        let tool_choice = match self.tool_choice {
            ToolChoice::Auto => None,
            ToolChoice::None => Some(serde_json::json!("none")),
            ToolChoice::Required => Some(serde_json::json!("required")),
            ToolChoice::Specific(ref name) => {
                Some(serde_json::json!({"type": "function", "function": {"name": name}}))
            }
        };

        let reasoning_effort = if caps.reasoning_policy != ReasoningPolicy::None {
            self.reasoning.as_ref().and_then(|r| r.effort.clone())
        } else {
            None
        };

        let thinking = match caps.reasoning_policy {
            ReasoningPolicy::DeepSeekReasoningContent if self.reasoning.is_some() => {
                Some(crate::chat::ThinkingConfig {
                    thinking_type: "enabled".into(),
                })
            }
            _ => None,
        };

        ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: Some(self.stream),
            max_tokens: self.sampling.max_output_tokens,
            temperature: self.sampling.temperature,
            top_p: self.sampling.top_p,
            presence_penalty: None,
            frequency_penalty: None,
            stop: None,
            tools,
            tool_choice,
            parallel_tool_calls: if caps.supports_parallel_tool_calls {
                self.parallel_tool_calls // preserve user intent
            } else {
                Some(false) // explicitly disable parallel calls
            },
            response_format,
            reasoning_effort,
            thinking,
            extra_body: serde_json::json!({}),
        }
    }

    /// Serialize this canonical request back to OpenAI Responses JSON.
    /// Used for Native / Stateless Responses proxy.
    pub fn into_native_responses_json(&self) -> Value {
        let mut input: Vec<Value> = Vec::new();

        // Reconstruct input items
        for item in &self.items {
            match item {
                CanonicalItem::Message(msg) => {
                    let mut obj = serde_json::Map::new();
                    obj.insert("type".into(), "message".into());
                    obj.insert("role".into(), role_str(&msg.role).into());
                    let content: Vec<Value> = msg
                        .content
                        .iter()
                        .map(|p| match p {
                            CanonicalContentPart::Text(t) => {
                                serde_json::json!({"type": "input_text", "text": t})
                            }
                            CanonicalContentPart::ImageUrl(url, detail) => {
                                let mut img =
                                    serde_json::json!({"type": "input_image", "image_url": url});
                                if let Some(d) = detail {
                                    img["image_url"] = serde_json::json!({"url": url, "detail": d});
                                }
                                img
                            }
                            CanonicalContentPart::OutputText(t) => {
                                serde_json::json!({"type": "output_text", "text": t})
                            }
                        })
                        .collect();
                    obj.insert("content".into(), Value::Array(content));
                    input.push(Value::Object(obj));
                }
                CanonicalItem::FunctionCall(call) => {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": call.call_id,
                        "name": call.name,
                        "arguments": call.arguments
                    }));
                }
                CanonicalItem::FunctionCallOutput(output) => {
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": output.call_id,
                        "output": output.output
                    }));
                }
                CanonicalItem::Reasoning(reasoning) => {
                    input.push(serde_json::json!({
                        "type": "reasoning",
                        "summary": [{"type": "summary_text", "text": reasoning.text}]
                    }));
                }
            }
        }

        let mut req = serde_json::json!({
            "model": self.model,
            "input": input,
        });
        if let Some(ref inst) = self.instructions {
            req["instructions"] = Value::String(inst.clone());
        }
        if let Some(ref inc) = self.include {
            req["include"] = inc.clone();
        }
        if let Some(ref prev) = self.previous_response_id {
            req["previous_response_id"] = Value::String(prev.clone());
        }
        if let Some(store) = self.store {
            req["store"] = Value::Bool(store);
        }
        req["stream"] = Value::Bool(self.stream);
        if let Some(ref r) = self.reasoning {
            let mut obj = serde_json::json!({});
            if let Some(ref e) = r.effort {
                obj["effort"] = Value::String(e.clone());
            }
            if let Some(ref s) = r.summary {
                obj["summary"] = Value::String(s.clone());
            }
            if !obj.as_object().is_none_or(|o| o.is_empty()) {
                req["reasoning"] = obj;
            }
        }
        if let Some(t) = self.sampling.max_output_tokens {
            req["max_output_tokens"] = Value::Number(t.into());
        }
        if let Some(t) = self.sampling.temperature {
            req["temperature"] =
                Value::Number(serde_json::Number::from_f64(t as f64).unwrap_or(0.into()));
        }
        if let Some(t) = self.sampling.top_p {
            req["top_p"] =
                Value::Number(serde_json::Number::from_f64(t as f64).unwrap_or(0.into()));
        }
        let flattened_tools: Vec<ResponseTool> = flatten_response_tools(&self.response_tools_raw);
        if !flattened_tools.is_empty() {
            req["tools"] = flattened_tools
                .iter()
                .map(|t| serde_json::to_value(t).unwrap_or(serde_json::Value::Null))
                .collect();
        }
        if self.metadata != Value::Null {
            req["metadata"] = self.metadata.clone();
        }
        Value::Object(req.as_object().cloned().unwrap_or_default())
    }

    /// Flatten canonical items + history into a Vec of ChatMessage for store.
    pub fn into_canonical_messages(&self) -> Vec<ChatMessage> {
        let mut msgs: Vec<ChatMessage> = Vec::new();
        msgs.extend(self.history_messages.clone());
        let mut pending_reasoning: Option<String> = None;
        for item in &self.items {
            match item {
                CanonicalItem::Message(msg) => {
                    msgs.push(canonical_message_to_chat(msg, &mut pending_reasoning));
                }
                CanonicalItem::FunctionCall(call) => {
                    msgs.push(ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(vec![ChatToolCall {
                            id: call.call_id.clone(),
                            call_type: "function".into(),
                            function: ChatFunctionCall {
                                name: Some(call.name.clone()),
                                arguments: call.arguments.clone(),
                            },
                        }]),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                CanonicalItem::FunctionCallOutput(output) => {
                    msgs.push(ChatMessage::Tool {
                        content: ChatContent::Text(output.output.clone()),
                        tool_call_id: output.call_id.clone(),
                    });
                }
                CanonicalItem::Reasoning(reasoning) => {
                    let existing = pending_reasoning.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&reasoning.text);
                }
            }
        }
        msgs
    }

    /// Detect if reasoning recovery is needed (tool outputs present, no reasoning items in input).
    pub fn needs_reasoning_recovery(&self, caps: &ProviderCapabilities) -> bool {
        if caps.reasoning_policy == ReasoningPolicy::None {
            return false;
        }
        let has_tool_outputs = self
            .items
            .iter()
            .any(|i| matches!(i, CanonicalItem::FunctionCallOutput { .. }));
        let has_reasoning = self
            .items
            .iter()
            .any(|i| matches!(i, CanonicalItem::Reasoning { .. }));
        let has_missing = self.history_messages.iter().any(|m| {
            matches!(
                m,
                ChatMessage::Assistant {
                    reasoning_content: None,
                    tool_calls: Some(_),
                    ..
                }
            )
        });
        (has_tool_outputs && !has_reasoning) || has_missing
    }
}

/// Validate a canonical request against provider capabilities.
/// Returns Ok(()) if all requested features are supported by the provider.
/// Returns Err for unsupported hosted tools, file inputs, and other
/// Responses-specific features that Chat Completions cannot handle.
pub fn validate_against_caps(
    canonical: &CanonicalRequest,
    caps: &ProviderCapabilities,
) -> Result<(), crate::error::ApiError> {
    // tools come from the request, validated separately in route handler

    // 0. Check structured output support
    if let Some(ref tf) = canonical.text_format {
        match tf {
            CanonicalTextFormat::JsonObject if !caps.supports_json_object => {
                return Err(crate::error::ApiError::invalid_json(
                    "text.format json_object is not supported",
                ));
            }
            CanonicalTextFormat::JsonSchema { .. } if !caps.supports_json_schema => {
                return Err(crate::error::ApiError::invalid_json(
                    "text.format json_schema is not supported",
                ));
            }
            _ => {}
        }
    }

    // 1. Hosted tools: reject if the provider doesn't support them
    for tool in &canonical.response_tools_raw {
        match tool {
            ResponseTool::WebSearchPreview { .. } if !caps.supports_hosted_web_search => {
                return Err(crate::error::ApiError::hosted_tool_not_supported(
                    "web_search",
                ));
            }
            ResponseTool::FileSearch { .. } if !caps.supports_hosted_file_search => {
                return Err(crate::error::ApiError::hosted_tool_not_supported(
                    "file_search",
                ));
            }
            ResponseTool::CodeInterpreter { .. } if !caps.supports_code_interpreter => {
                return Err(crate::error::ApiError::hosted_tool_not_supported(
                    "code_interpreter",
                ));
            }
            ResponseTool::ComputerUse { .. } => {
                return Err(crate::error::ApiError::hosted_tool_not_supported(
                    "computer_use",
                ));
            }
            ResponseTool::Mcp { .. } => {
                return Err(crate::error::ApiError::hosted_tool_not_supported("mcp"));
            }
            _ => {}
        }
    }

    // 3. Structured output guard
    // (checked in mapping layer — if json_schema requested and not supported, error there)

    // 3. Structured output guard
    // Check is done in the mapping layer by caller

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────

fn parse_input_items(input: &ResponseInput) -> Result<Vec<CanonicalItem>, String> {
    let raw_items: Vec<&InputItem> = match input {
        ResponseInput::Text(text) => {
            return Ok(vec![CanonicalItem::Message(CanonicalMessage {
                role: CanonicalMessageRole::User,
                content: vec![CanonicalContentPart::Text(text.clone())],
            })]);
        }
        ResponseInput::Items(items) => items.iter().collect(),
        ResponseInput::Value(val) => match val {
            Value::Object(_) => {
                let parsed: InputItem = serde_json::from_value(val.clone())
                    .map_err(|e| format!("input object is not a valid Responses item: {e}"))?;
                return map_input_item(&parsed).map(|item| vec![item]);
            }
            Value::Array(arr) => {
                let mut items = Vec::with_capacity(arr.len());
                for value in arr {
                    let item: InputItem = serde_json::from_value(value.clone()).map_err(|e| {
                        format!("input array contains an invalid Responses item: {e}")
                    })?;
                    items.push(map_input_item(&item)?);
                }
                return Ok(items);
            }
            _ => return Err(
                "input must be a string, a Responses item object, or an array of Responses items"
                    .into(),
            ),
        },
    };
    let mut items = Vec::with_capacity(raw_items.len());
    for item in raw_items {
        items.push(map_input_item(item)?);
    }
    Ok(items)
}

fn validate_reasoning_items(items: &[CanonicalItem]) -> Result<(), String> {
    let mut pending_reasoning = false;
    for item in items {
        match item {
            CanonicalItem::Reasoning(_) => pending_reasoning = true,
            CanonicalItem::FunctionCall(_) => pending_reasoning = false,
            CanonicalItem::Message(message)
                if matches!(message.role, CanonicalMessageRole::Assistant) =>
            {
                pending_reasoning = false;
            }
            _ => {}
        }
    }
    if pending_reasoning {
        Err("reasoning items must be followed by an assistant or function_call item".into())
    } else {
        Ok(())
    }
}

fn map_input_item(item: &InputItem) -> Result<CanonicalItem, String> {
    match item {
        InputItem::Message { role, content, .. } => {
            let role = match role {
                InputMessageRole::User => CanonicalMessageRole::User,
                InputMessageRole::System => CanonicalMessageRole::System,
                InputMessageRole::Developer => CanonicalMessageRole::Developer,
                InputMessageRole::Assistant => CanonicalMessageRole::Assistant,
            };
            let parts = match content {
                MessageContent::Text(t) => vec![CanonicalContentPart::Text(t.clone())],
                MessageContent::Parts(parts) => {
                    let mut mapped = Vec::with_capacity(parts.len());
                    for part in parts {
                        mapped.push(match part {
                            ContentPart::OutputText { text, .. } => {
                                CanonicalContentPart::OutputText(text.clone())
                            }
                            ContentPart::InputText { text } => {
                                CanonicalContentPart::Text(text.clone())
                            }
                            ContentPart::InputImage { image_url } => {
                                CanonicalContentPart::ImageUrl(
                                    image_url.url().to_string(),
                                    image_url.detail().map(String::from),
                                )
                            }
                            ContentPart::Refusal { .. } => {
                                return Err(
                                    ApiError::unsupported_content_part("refusal").to_string()
                                );
                            }
                            ContentPart::UnknownContentPart => {
                                return Err(
                                    ApiError::unsupported_content_part("unknown").to_string()
                                );
                            }
                        });
                    }
                    mapped
                }
            };
            Ok(CanonicalItem::Message(CanonicalMessage {
                role,
                content: parts,
            }))
        }
        InputItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => Ok(CanonicalItem::FunctionCall(CanonicalFunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        })),
        InputItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => Ok(CanonicalItem::FunctionCall(CanonicalFunctionCall {
            call_id: call_id.clone(),
            name: format!("__custom__{name}"),
            arguments: serde_json::to_string(input).unwrap_or_default(),
        })),
        InputItem::LocalShellCall {
            call_id,
            command,
            cwd,
            timeout_ms,
            ..
        } => Ok(CanonicalItem::FunctionCall(CanonicalFunctionCall {
            call_id: call_id.clone(),
            name: "__codex_local_shell".into(),
            arguments: serde_json::to_string(&serde_json::json!({
                "command": command,
                "cwd": cwd,
                "timeout_ms": timeout_ms,
            }))
            .unwrap_or_default(),
        })),
        InputItem::ApplyPatchCall {
            call_id,
            patch,
            diff,
            path,
            ..
        } => {
            let args = serde_json::json!({
                "patch": patch,
                "diff": diff,
                "path": path,
            });
            Ok(CanonicalItem::FunctionCall(CanonicalFunctionCall {
                call_id: call_id.clone(),
                name: "__codex_apply_patch".into(),
                arguments: serde_json::to_string(&args).unwrap_or_default(),
            }))
        }
        InputItem::FunctionCallOutput {
            call_id, output, ..
        } => Ok(CanonicalItem::FunctionCallOutput(
            CanonicalFunctionCallOutput {
                call_id: call_id.clone(),
                output: value_to_string(output),
            },
        )),
        InputItem::CustomToolCallOutput {
            call_id, output, ..
        } => Ok(CanonicalItem::FunctionCallOutput(
            CanonicalFunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            },
        )),
        InputItem::LocalShellCallOutput {
            call_id, output, ..
        } => Ok(CanonicalItem::FunctionCallOutput(
            CanonicalFunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            },
        )),
        InputItem::ApplyPatchCallOutput {
            call_id, output, ..
        } => Ok(CanonicalItem::FunctionCallOutput(
            CanonicalFunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            },
        )),
        InputItem::Reasoning {
            content, summary, ..
        } => {
            let text = extract_reasoning_text(content.as_ref(), summary.as_ref());
            Ok(CanonicalItem::Reasoning(CanonicalReasoningItem { text }))
        }
        InputItem::McpCall { .. } => Err(ApiError::unsupported_input_item("mcp_call").to_string()),
        InputItem::WebSearchCall { .. } => {
            Err(ApiError::unsupported_input_item("web_search_call").to_string())
        }
        InputItem::FileSearchCall { .. } => {
            Err(ApiError::unsupported_input_item("file_search_call").to_string())
        }
        InputItem::CodeInterpreterCall { .. } => {
            Err(ApiError::unsupported_input_item("code_interpreter_call").to_string())
        }
        InputItem::InputFile { .. } => Err(ApiError::file_input_not_supported().to_string()),
        InputItem::Unknown => Err(ApiError::unknown_input_item(None).to_string()),
    }
}

fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let texts: Vec<String> = items
                .iter()
                .filter_map(|item| {
                    item.get("type")
                        .and_then(|t| t.as_str())
                        .filter(|&t| t == "input_text")
                        .and_then(|_| item.get("text"))
                        .and_then(|t| t.as_str())
                        .map(String::from)
                })
                .collect();
            if texts.is_empty() {
                val.to_string()
            } else {
                texts.join("\n")
            }
        }
        _ => val.to_string(),
    }
}

fn extract_reasoning_text(
    content: Option<&Vec<ContentPart>>,
    summary: Option<&Vec<crate::responses::SummaryPart>>,
) -> String {
    if let Some(parts) = content {
        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::OutputText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }
    if let Some(summary) = summary {
        let text: String = summary
            .iter()
            .map(|p| match p {
                crate::responses::SummaryPart::SummaryText { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// Flatten namespace tools into their constituent function tools.
/// Codex sends MCP/connector tools wrapped in `{"type": "namespace", ...}` envelopes.
/// The upstream provider expects flat `{"type": "function", ...}` tools.
pub fn flatten_response_tools(tools: &[ResponseTool]) -> Vec<ResponseTool> {
    let mut flattened = Vec::new();
    for tool in tools {
        match tool {
            ResponseTool::Namespace {
                tools: inner_tools, ..
            } => {
                for inner in inner_tools {
                    match inner {
                        NamespaceTool::Function {
                            name,
                            description,
                            parameters,
                            strict,
                        } => {
                            flattened.push(ResponseTool::Function {
                                name: name.clone(),
                                description: description.clone(),
                                parameters: parameters.clone(),
                                strict: *strict,
                            });
                        }
                    }
                }
            }
            other => flattened.push(other.clone()),
        }
    }
    flattened
}

fn parse_tools(
    tools: &[ResponseTool],
    warnings: &mut Vec<String>,
) -> Result<Vec<CanonicalTool>, String> {
    let mut parsed = Vec::new();
    for tool in &flatten_response_tools(tools) {
        match tool {
            ResponseTool::Function {
                name,
                description,
                parameters,
                strict,
            } => {
                parsed.push(CanonicalTool {
                    name: name.clone(),
                    description: description.clone(),
                    parameters: parameters.clone(),
                    strict: *strict,
                });
            }
            ResponseTool::WebSearchPreview { .. } => {
                warnings.push("hosted tool 'web_search' was requested but is not supported by Chat Completions. Use a native Responses provider with supports_hosted_web_search=true to enable it.".into());
            }
            ResponseTool::FileSearch { .. } => {
                warnings.push("hosted tool 'file_search' was requested but is not supported by Chat Completions".into());
            }
            ResponseTool::CodeInterpreter { .. } => {
                warnings.push(
                    "hosted tool 'code_interpreter' was requested but is not supported".into(),
                );
            }
            ResponseTool::ComputerUse { .. } => {
                warnings
                    .push("hosted tool 'computer_use' was requested but is not supported".into());
            }
            ResponseTool::Mcp { .. } => {
                warnings.push("hosted tool 'mcp' was requested but is not supported".into());
            }
            ResponseTool::UnknownTool => {
                return Err(ApiError::unsupported_tool_type("unknown", 0).to_string());
            }
            // Namespace tools are already flattened by flatten_response_tools
            ResponseTool::Namespace { .. } => unreachable!(),
        }
    }
    Ok(parsed)
}

fn parse_tool_choice(tc: &ResponsesToolChoice) -> ToolChoice {
    match tc {
        ResponsesToolChoice::Auto(s) => match s.as_str() {
            "none" => ToolChoice::None,
            "required" => ToolChoice::Required,
            _ => ToolChoice::Auto,
        },
        ResponsesToolChoice::Specific { name, .. } => ToolChoice::Specific(name.clone()),
    }
}

fn parse_text_format(tf: &TextFormat) -> CanonicalTextFormat {
    match tf {
        TextFormat::Text => CanonicalTextFormat::Text,
        TextFormat::JsonObject => CanonicalTextFormat::JsonObject,
        TextFormat::JsonSchema {
            name,
            description,
            schema_,
            strict,
        } => CanonicalTextFormat::JsonSchema {
            name: name.clone(),
            description: description.clone(),
            schema: schema_.clone(),
            strict: *strict,
        },
    }
}

fn role_str(role: &CanonicalMessageRole) -> &'static str {
    match role {
        CanonicalMessageRole::User => "user",
        CanonicalMessageRole::System => "system",
        CanonicalMessageRole::Developer => "developer",
        CanonicalMessageRole::Assistant => "assistant",
    }
}

/// Enforce the OpenAI/DeepSeek API constraint that tool messages must
/// immediately follow the assistant message that contains the corresponding
/// tool_calls. Any non-Tool messages found between a tool_calls assistant
/// and its tool results are moved to before the assistant message, preserving
/// relative order.
pub fn enforce_tool_call_adjacency(messages: &mut Vec<ChatMessage>) {
    let mut i = 0;
    while i < messages.len() {
        // Collect tool_call_ids from an assistant message
        let call_ids: Vec<String> = match &messages[i] {
            ChatMessage::Assistant {
                tool_calls: Some(tcs),
                ..
            } => tcs.iter().map(|tc| tc.id.clone()).collect(),
            _ => {
                i += 1;
                continue;
            }
        };

        if call_ids.is_empty() {
            i += 1;
            continue;
        }

        // Scan forward to find tool results for these calls.
        // Track any non-Tool messages that intervene.
        let mut j = i + 1;
        let mut found: usize = 0;
        let mut intervening_indices: Vec<usize> = Vec::new();

        while j < messages.len() && found < call_ids.len() {
            let is_tool_match = match &messages[j] {
                ChatMessage::Tool { tool_call_id, .. } => {
                    call_ids.iter().any(|id| id == tool_call_id)
                }
                _ => false,
            };
            if is_tool_match {
                found += 1;
            } else {
                intervening_indices.push(j);
            }
            j += 1;
        }

        if !intervening_indices.is_empty() {
            // Extract all intervening messages and insert them
            // before the assistant message at position i.
            // Remove in reverse order to preserve indices.
            let mut moved: Vec<ChatMessage> = Vec::new();
            for &idx in intervening_indices.iter().rev() {
                moved.push(messages.remove(idx));
            }
            // Insert in original order before the assistant message
            for msg in moved.into_iter().rev() {
                messages.insert(i, msg);
                i += 1;
            }
        }

        // Advance past the assistant + all tool results
        i += 1 + found;
    }

    split_completed_parallel_tool_call_messages(messages);
}

fn split_completed_parallel_tool_call_messages(messages: &mut Vec<ChatMessage>) {
    let mut i = 0;
    while i < messages.len() {
        let (content, name, reasoning_content, tool_calls) = match &messages[i] {
            ChatMessage::Assistant {
                content,
                name,
                tool_calls: Some(tool_calls),
                reasoning_content,
            } if tool_calls.len() > 1 => (
                content.clone(),
                name.clone(),
                reasoning_content.clone(),
                tool_calls.clone(),
            ),
            _ => {
                i += 1;
                continue;
            }
        };

        if i + tool_calls.len() >= messages.len() {
            i += 1;
            continue;
        }

        let tool_segment = &messages[i + 1..i + 1 + tool_calls.len()];
        let mut tool_results = Vec::with_capacity(tool_calls.len());
        let mut all_results_present = true;
        for tool_call in &tool_calls {
            let Some(tool_result) = tool_segment.iter().find(|msg| {
                matches!(
                    msg,
                    ChatMessage::Tool { tool_call_id, .. } if tool_call_id == &tool_call.id
                )
            }) else {
                all_results_present = false;
                break;
            };
            tool_results.push(tool_result.clone());
        }

        if !all_results_present {
            i += 1;
            continue;
        }

        let mut replacement = Vec::with_capacity(tool_calls.len() * 2);
        let replace_end = i + 1 + tool_results.len();
        for (idx, tool_call) in tool_calls.into_iter().enumerate() {
            replacement.push(ChatMessage::Assistant {
                content: if idx == 0 { content.clone() } else { None },
                name: if idx == 0 { name.clone() } else { None },
                tool_calls: Some(vec![tool_call]),
                reasoning_content: if idx == 0 {
                    reasoning_content.clone()
                } else {
                    None
                },
            });
            replacement.push(tool_results[idx].clone());
        }

        messages.splice(i..replace_end, replacement);
        i += 1;
    }
}

fn canonical_message_to_chat(
    msg: &CanonicalMessage,
    pending_reasoning: &mut Option<String>,
) -> ChatMessage {
    let mut has_text = false;
    let mut has_image = false;
    let mut text_buf = String::new();
    let mut image_urls: Vec<ChatImageUrl> = Vec::new();

    for part in &msg.content {
        match part {
            CanonicalContentPart::Text(t) | CanonicalContentPart::OutputText(t) => {
                text_buf.push_str(t);
                has_text = true;
            }
            CanonicalContentPart::ImageUrl(url, detail) => {
                image_urls.push(ChatImageUrl {
                    url: url.clone(),
                    detail: detail.clone(),
                });
                has_image = true;
            }
        }
    }

    if has_image {
        let mut parts: Vec<crate::chat::ChatContentPart> = Vec::new();
        if has_text {
            parts.push(crate::chat::ChatContentPart::Text { text: text_buf });
        }
        for img in image_urls {
            parts.push(crate::chat::ChatContentPart::ImageUrl { image_url: img });
        }
        let content = ChatContent::Parts(parts);
        match msg.role {
            CanonicalMessageRole::User => ChatMessage::User {
                content,
                name: None,
            },
            CanonicalMessageRole::System | CanonicalMessageRole::Developer => ChatMessage::System {
                content,
                name: None,
            },
            CanonicalMessageRole::Assistant => ChatMessage::Assistant {
                content: Some(content),
                name: None,
                tool_calls: None,
                reasoning_content: pending_reasoning.take(),
            },
        }
    } else {
        let content = ChatContent::Text(text_buf);
        match msg.role {
            CanonicalMessageRole::User => ChatMessage::User {
                content,
                name: None,
            },
            CanonicalMessageRole::System | CanonicalMessageRole::Developer => ChatMessage::System {
                content,
                name: None,
            },
            CanonicalMessageRole::Assistant => ChatMessage::Assistant {
                content: Some(content),
                name: None,
                tool_calls: None,
                reasoning_content: pending_reasoning.take(),
            },
        }
    }
}

#[cfg(test)]
mod adjacency_tests {
    use super::*;
    use crate::chat::{ChatContent, ChatFunctionCall, ChatMessage, ChatToolCall};

    fn assistant_tool_call(id: &str, name: &str, args: &str) -> ChatMessage {
        assistant_tool_calls(vec![(id, name, args)])
    }

    fn assistant_tool_calls(calls: Vec<(&str, &str, &str)>) -> ChatMessage {
        ChatMessage::Assistant {
            content: None,
            name: None,
            tool_calls: Some(
                calls
                    .into_iter()
                    .map(|(id, name, args)| ChatToolCall {
                        id: id.to_string(),
                        call_type: "function".into(),
                        function: ChatFunctionCall {
                            name: Some(name.to_string()),
                            arguments: args.to_string(),
                        },
                    })
                    .collect(),
            ),
            reasoning_content: None,
        }
    }

    fn tool_result(id: &str, output: &str) -> ChatMessage {
        ChatMessage::Tool {
            content: ChatContent::Text(output.to_string()),
            tool_call_id: id.to_string(),
        }
    }

    fn system_msg(text: &str) -> ChatMessage {
        ChatMessage::System {
            content: ChatContent::Text(text.to_string()),
            name: None,
        }
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage::User {
            content: ChatContent::Text(text.to_string()),
            name: None,
        }
    }

    #[test]
    fn moves_intervening_system_before_tool_call() {
        let mut msgs = vec![
            user_msg("hello"),
            assistant_tool_call("call_1", "exec_command", "{}"),
            system_msg("Approved command prefix saved:\n- [some prefix]"),
            tool_result("call_1", "output"),
        ];
        enforce_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs.len(), 4);
        assert!(matches!(&msgs[0], ChatMessage::User { .. }));
        assert!(matches!(&msgs[1], ChatMessage::System { .. }));
        assert!(matches!(
            &msgs[2],
            ChatMessage::Assistant {
                tool_calls: Some(_),
                ..
            }
        ));
        assert!(matches!(&msgs[3], ChatMessage::Tool { .. }));
    }

    #[test]
    fn leaves_adjacent_tool_calls_untouched() {
        let mut msgs = vec![
            user_msg("hello"),
            assistant_tool_call("call_1", "exec_command", "{}"),
            tool_result("call_1", "output"),
        ];
        let expected = msgs.clone();
        enforce_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs, expected);
    }

    #[test]
    fn splits_completed_parallel_tool_calls_into_single_call_pairs() {
        let mut msgs = vec![
            user_msg("hello"),
            assistant_tool_calls(vec![
                ("call_1", "exec_command", r#"{"cmd":"one"}"#),
                ("call_2", "exec_command", r#"{"cmd":"two"}"#),
            ]),
            tool_result("call_2", "two"),
            tool_result("call_1", "one"),
            user_msg("next"),
        ];

        enforce_tool_call_adjacency(&mut msgs);

        assert_eq!(msgs.len(), 6);
        assert!(matches!(&msgs[0], ChatMessage::User { .. }));
        match &msgs[1] {
            ChatMessage::Assistant {
                tool_calls: Some(calls),
                ..
            } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
            }
            other => panic!("expected first assistant tool call, got {other:?}"),
        }
        assert!(matches!(
            &msgs[2],
            ChatMessage::Tool { tool_call_id, .. } if tool_call_id == "call_1"
        ));
        match &msgs[3] {
            ChatMessage::Assistant {
                tool_calls: Some(calls),
                ..
            } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_2");
            }
            other => panic!("expected second assistant tool call, got {other:?}"),
        }
        assert!(matches!(
            &msgs[4],
            ChatMessage::Tool { tool_call_id, .. } if tool_call_id == "call_2"
        ));
        assert!(matches!(&msgs[5], ChatMessage::User { .. }));
    }

    #[test]
    fn leaves_incomplete_parallel_tool_calls_grouped() {
        let mut msgs = vec![
            user_msg("hello"),
            assistant_tool_calls(vec![
                ("call_1", "exec_command", r#"{"cmd":"one"}"#),
                ("call_2", "exec_command", r#"{"cmd":"two"}"#),
            ]),
            tool_result("call_1", "one"),
        ];
        let expected = msgs.clone();

        enforce_tool_call_adjacency(&mut msgs);

        assert_eq!(msgs, expected);
    }

    #[test]
    fn no_tool_calls_leaves_messages_untouched() {
        let mut msgs = vec![
            user_msg("hello"),
            system_msg("instructions"),
            user_msg("world"),
        ];
        let expected = msgs.clone();
        enforce_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs, expected);
    }
}
