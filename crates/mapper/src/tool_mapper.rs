use protocol::chat::{ChatFunctionDef, ChatTool};
use protocol::responses::{ResponseInput, ResponseTool, ToolChoice};

use crate::MappingConfig;
use crate::apply_patch_tool;
use crate::chat_tool_context::flatten_namespace_tool_name;
use crate::custom_tools::{custom_tool_description, custom_tool_schema};

/// Map Responses tool definitions → Chat Completions tools.
pub fn map_response_tools(tools: &[ResponseTool], config: &MappingConfig) -> Vec<ChatTool> {
    let mut mapped = Vec::new();
    for t in tools {
        match t {
            ResponseTool::Function {
                name,
                description,
                parameters,
                strict,
            } => mapped.push(ChatTool {
                tool_type: "function".into(),
                function: ChatFunctionDef {
                    name: name.clone(),
                    description: description.clone(),
                    parameters: parameters.clone(),
                    strict: *strict,
                },
            }),
            // Custom Codex tools are function tools to the upstream model
            ResponseTool::CodeInterpreter { .. } => {}
            ResponseTool::ComputerUse { .. } => {}
            ResponseTool::FileSearch { .. } => {}
            ResponseTool::WebSearchPreview { .. } => {}
            ResponseTool::ToolSearch { description } => mapped.push(ChatTool {
                tool_type: "function".into(),
                function: ChatFunctionDef {
                    name: "tool_search".into(),
                    description: description.clone().or_else(|| {
                        Some("Search and load Codex tools for the current task.".into())
                    }),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "limit": { "type": "integer" }
                        },
                        "required": ["query"]
                    })),
                    strict: Some(false),
                },
            }),
            ResponseTool::Custom {
                name,
                description,
                format,
            } => {
                if name == apply_patch_tool::APPLY_PATCH_TOOL_NAME
                    && config.apply_patch_upstream_tool_type
                        == apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                {
                    mapped.push(ChatTool {
                        tool_type: "function".into(),
                        function: ChatFunctionDef {
                            name: name.clone(),
                            description: Some(apply_patch_tool::structured_description(
                                description,
                            )),
                            parameters: Some(apply_patch_tool::structured_schema()),
                            strict: Some(config.apply_patch_upstream_strict),
                        },
                    })
                } else {
                    mapped.push(ChatTool {
                        tool_type: "function".into(),
                        function: ChatFunctionDef {
                            name: name.clone(),
                            description: Some(custom_tool_description(description, format)),
                            parameters: Some(custom_tool_schema()),
                            strict: Some(false),
                        },
                    })
                }
            }
            ResponseTool::Namespace { name, tools, .. } => {
                for tool in tools {
                    match tool {
                        protocol::responses::NamespaceTool::Function {
                            name: child_name,
                            description,
                            parameters,
                            strict,
                        } => mapped.push(ChatTool {
                            tool_type: "function".into(),
                            function: ChatFunctionDef {
                                name: flatten_namespace_tool_name(name, child_name),
                                description: description.clone(),
                                parameters: parameters.clone(),
                                strict: *strict,
                            },
                        }),
                    }
                }
            }
            ResponseTool::Mcp { .. } | ResponseTool::UnknownTool => {}
        }
    }
    mapped
}

pub fn apply_chat_tool_mapping_overrides(
    chat_req: &mut protocol::chat::ChatCompletionRequest,
    config: &MappingConfig,
) {
    if config.apply_patch_upstream_tool_type != apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED {
        return;
    }
    if let Some(tools) = &mut chat_req.tools {
        for tool in tools {
            if tool.function.name == apply_patch_tool::APPLY_PATCH_TOOL_NAME {
                let original = tool
                    .function
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .split("\n\nChat adapter contract:")
                    .next()
                    .unwrap_or("");
                tool.function.description =
                    Some(apply_patch_tool::structured_description(original));
                tool.function.parameters = Some(apply_patch_tool::structured_schema());
                tool.function.strict = Some(config.apply_patch_upstream_strict);
            }
        }
    }
}

/// Map Responses `tool_choice` → Chat `tool_choice` value.
pub fn map_tool_choice(tc: &ToolChoice) -> serde_json::Value {
    match tc {
        ToolChoice::Auto(s) => match s.as_str() {
            "none" => serde_json::Value::String("none".into()),
            "required" => serde_json::Value::String("required".into()),
            _ => serde_json::Value::String("auto".into()),
        },
        ToolChoice::Specific { name, .. } => serde_json::json!({
            "type": "function",
            "function": {"name": name}
        }),
    }
}

/// Check whether the input contains any tool-output items that need to be threaded.
pub fn has_tool_outputs(input: &ResponseInput) -> bool {
    if let Some(items) = get_items(input) {
        items.iter().any(|item| {
            matches!(
                item,
                protocol::responses::InputItem::FunctionCallOutput { .. }
                    | protocol::responses::InputItem::CustomToolCallOutput { .. }
                    | protocol::responses::InputItem::LocalShellCallOutput { .. }
                    | protocol::responses::InputItem::ApplyPatchCallOutput { .. }
            )
        })
    } else {
        false
    }
}

/// Extract a Vec of InputItems from any ResponseInput variant.
fn get_items(input: &ResponseInput) -> Option<Vec<protocol::responses::InputItem>> {
    match input {
        ResponseInput::Text(_) => None,
        ResponseInput::Items(items) => Some(items.clone()),
        ResponseInput::Value(val) => match val {
            serde_json::Value::Array(arr) => Some(
                arr.iter()
                    .filter_map(|v| {
                        serde_json::from_value::<protocol::responses::InputItem>(v.clone()).ok()
                    })
                    .collect(),
            ),
            _ => None,
        },
    }
}
