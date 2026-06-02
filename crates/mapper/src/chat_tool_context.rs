use std::collections::{BTreeMap, BTreeSet};

use protocol::chat::ChatCompletionRequest;
use protocol::error::ApiError;
use protocol::responses::{NamespaceTool, ResponseOutputItem, ResponseTool};
use serde_json::Value;

use crate::custom_tools::custom_tool_input_from_arguments;

const TOOL_SEARCH_PROXY_NAME: &str = "tool_search";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatToolKind {
    Function,
    Custom,
    Namespace { namespace: String, name: String },
    ToolSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatToolSpec {
    pub kind: ChatToolKind,
}

#[derive(Debug, Clone, Default)]
pub struct ChatToolContext {
    specs_by_chat_name: BTreeMap<String, ChatToolSpec>,
    namespace_name_to_chat_name: BTreeMap<(String, String), String>,
}

impl ChatToolContext {
    pub fn from_response_tools(tools: &[ResponseTool]) -> Self {
        let mut context = Self::default();
        for tool in tools {
            context.add_response_tool(tool);
        }
        context
    }

    pub fn custom_tool_names(&self) -> BTreeSet<String> {
        self.specs_by_chat_name
            .iter()
            .filter(|(_, spec)| matches!(spec.kind, ChatToolKind::Custom))
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn apply_to_chat_request(&self, request: &mut ChatCompletionRequest) {
        if let Some(tools) = &mut request.tools {
            for tool in tools {
                let original = tool.function.name.clone();
                if let Some(chat_name) = self.chat_name_for_namespace_child(&original) {
                    tool.function.name = chat_name;
                }
            }
        }

        if let Some(Value::Object(choice)) = &mut request.tool_choice
            && choice.get("type").and_then(Value::as_str) == Some("function")
            && let Some(function) = choice.get_mut("function").and_then(Value::as_object_mut)
            && let Some(name) = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
            && let Some(chat_name) = self.chat_name_for_namespace_child(&name)
        {
            function.insert("name".into(), Value::String(chat_name));
        }
    }

    pub fn output_item(
        &self,
        id: String,
        status: String,
        call_id: String,
        chat_name: String,
        arguments: String,
    ) -> Result<ResponseOutputItem, ApiError> {
        match self
            .specs_by_chat_name
            .get(&chat_name)
            .map(|spec| &spec.kind)
        {
            Some(ChatToolKind::Custom) if status == "completed" => {
                Ok(ResponseOutputItem::CustomToolCall {
                    id,
                    status,
                    call_id,
                    name: chat_name.clone(),
                    input: custom_tool_input_from_arguments(&chat_name, &arguments)?,
                })
            }
            Some(ChatToolKind::Custom) => Ok(ResponseOutputItem::CustomToolCall {
                id,
                status,
                call_id,
                name: chat_name,
                input: String::new(),
            }),
            Some(ChatToolKind::Namespace { namespace, name }) => {
                Ok(ResponseOutputItem::FunctionCall {
                    id,
                    status,
                    call_id,
                    name: name.clone(),
                    namespace: Some(namespace.clone()),
                    arguments,
                })
            }
            Some(ChatToolKind::ToolSearch) => Ok(ResponseOutputItem::ToolSearchCall {
                id,
                status,
                call_id,
                execution: "client".into(),
                arguments: parse_tool_arguments_object(&arguments)?,
            }),
            Some(ChatToolKind::Function) | None => Ok(ResponseOutputItem::FunctionCall {
                id,
                status,
                call_id,
                name: chat_name,
                namespace: None,
                arguments,
            }),
        }
    }

    fn add_response_tool(&mut self, tool: &ResponseTool) {
        match tool {
            ResponseTool::Function { name, .. } => {
                self.specs_by_chat_name.insert(
                    name.clone(),
                    ChatToolSpec {
                        kind: ChatToolKind::Function,
                    },
                );
            }
            ResponseTool::Custom { name, .. } => {
                self.specs_by_chat_name.insert(
                    name.clone(),
                    ChatToolSpec {
                        kind: ChatToolKind::Custom,
                    },
                );
            }
            ResponseTool::Namespace { name, tools, .. } => {
                for tool in tools {
                    match tool {
                        NamespaceTool::Function {
                            name: child_name, ..
                        } => {
                            let chat_name = flatten_namespace_tool_name(name, child_name);
                            self.namespace_name_to_chat_name
                                .insert((name.clone(), child_name.clone()), chat_name.clone());
                            self.specs_by_chat_name.insert(
                                chat_name,
                                ChatToolSpec {
                                    kind: ChatToolKind::Namespace {
                                        namespace: name.clone(),
                                        name: child_name.clone(),
                                    },
                                },
                            );
                        }
                    }
                }
            }
            ResponseTool::ToolSearch { .. } => {
                self.specs_by_chat_name.insert(
                    TOOL_SEARCH_PROXY_NAME.into(),
                    ChatToolSpec {
                        kind: ChatToolKind::ToolSearch,
                    },
                );
            }
            ResponseTool::WebSearchPreview { .. }
            | ResponseTool::FileSearch { .. }
            | ResponseTool::ComputerUse { .. }
            | ResponseTool::CodeInterpreter { .. }
            | ResponseTool::Mcp { .. }
            | ResponseTool::UnknownTool => {}
        }
    }

    fn chat_name_for_namespace_child(&self, child_name: &str) -> Option<String> {
        let mut matches = self
            .namespace_name_to_chat_name
            .iter()
            .filter(|((_, name), _)| name == child_name)
            .map(|(_, chat_name)| chat_name.clone());
        let first = matches.next()?;
        if matches.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
}

pub fn flatten_namespace_tool_name(namespace: &str, name: &str) -> String {
    format!("{namespace}___{name}")
}

pub fn parse_tool_arguments_object(arguments: &str) -> Result<Value, ApiError> {
    serde_json::from_str::<Value>(arguments)
        .map_err(|error| {
            ApiError::upstream_error(format!(
                "tool call returned invalid JSON object arguments: {error}"
            ))
        })
        .and_then(|value| {
            if value.is_object() {
                Ok(value)
            } else {
                Err(ApiError::upstream_error(
                    "tool call returned invalid arguments: expected JSON object",
                ))
            }
        })
}

impl From<&[ResponseTool]> for ChatToolContext {
    fn from(value: &[ResponseTool]) -> Self {
        Self::from_response_tools(value)
    }
}
