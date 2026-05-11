use protocol::chat::{ChatFunctionDef, ChatTool};
use protocol::responses::{ResponseInput, ResponseTool, ToolChoice};

/// Map Responses tool definitions → Chat Completions tools.
pub fn map_response_tools(tools: &[ResponseTool]) -> Vec<ChatTool> {
    tools
        .iter()
        .filter_map(|t| match t {
            ResponseTool::Function {
                name,
                description,
                parameters,
                strict,
            } => Some(ChatTool {
                tool_type: "function".into(),
                function: ChatFunctionDef {
                    name: name.clone(),
                    description: description.clone(),
                    parameters: parameters.clone(),
                    strict: *strict,
                },
            }),
            // Custom Codex tools are function tools to the upstream model
            ResponseTool::CodeInterpreter { .. } => None,
            ResponseTool::ComputerUse { .. } => None,
            ResponseTool::FileSearch { .. } => None,
            ResponseTool::WebSearchPreview { .. } => None,
            ResponseTool::Mcp { .. }
            | ResponseTool::UnknownTool
            | ResponseTool::Namespace { .. } => None,
        })
        .collect()
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
