use protocol::chat::{ChatCompletionChunk, ChatFunctionCall, ChatToolCall};
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{ResponseOutputItem, SummaryPart};
use protocol::sse::{ResponseSseEvent, SseResponseShell};
use uuid::Uuid;

use crate::chat_tool_context::ChatToolContext;
use crate::custom_tools::custom_tool_input_from_arguments;
use crate::tool_call_normalizer::split_concatenated_json_objects;

#[derive(Debug, Default)]
struct ReasoningItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Default)]
struct TextItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Clone)]
struct ToolCallState {
    chat_index: u32,
    output_index: Option<u32>,
    item_id: String,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    added: bool,
    done: bool,
}

impl ToolCallState {
    fn new(chat_index: u32) -> Self {
        Self {
            chat_index,
            output_index: None,
            item_id: format!("fc_{}", Uuid::new_v4()),
            call_id: None,
            name: None,
            arguments: String::new(),
            added: false,
            done: false,
        }
    }

    fn require_call_id(&self) -> Result<String, ApiError> {
        self.call_id
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| {
                ApiError::upstream_error(format!(
                    "streamed tool call at index {} missing required call_id",
                    self.chat_index
                ))
            })
    }

    fn require_name(&self) -> Result<String, ApiError> {
        self.name
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| {
                ApiError::upstream_error(format!(
                    "streamed tool call at index {} missing required function.name",
                    self.chat_index
                ))
            })
    }
}

/// Mutable state for tracking SSE event emission across a Chat Completions stream.
#[derive(Debug)]
pub struct StreamState {
    pub response_id: String,
    pub model: String,
    pub created_at: i64,
    lifecycle_sent: bool,
    completed_sent: bool,
    next_output_index: u32,
    reasoning: ReasoningItemState,
    text: TextItemState,
    tools: Vec<ToolCallState>,
    completed_items: Vec<(u32, ResponseOutputItem)>,
    pub accumulated_text: String,
    pub tool_call_id: Option<String>,
    pub tool_call_name: Option<String>,
    pub tool_call_arguments: String,
    pub tool_call_index: u32,
    pub tool_call_active: bool,
    pub reasoning_content: String,
    final_usage: Option<protocol::common::Usage>,
    pub finish_reason: Option<String>,
    tool_context: ChatToolContext,
}

impl StreamState {
    pub fn new(
        response_id: String,
        model: String,
        created_at: i64,
        output_item_id: String,
        tool_context: ChatToolContext,
    ) -> Self {
        Self {
            response_id,
            model,
            created_at,
            lifecycle_sent: false,
            completed_sent: false,
            next_output_index: 0,
            reasoning: ReasoningItemState::default(),
            text: TextItemState {
                item_id: output_item_id,
                ..TextItemState::default()
            },
            tools: Vec::new(),
            completed_items: Vec::new(),
            accumulated_text: String::new(),
            tool_call_id: None,
            tool_call_name: None,
            tool_call_arguments: String::new(),
            tool_call_index: 0,
            tool_call_active: false,
            reasoning_content: String::new(),
            final_usage: None,
            finish_reason: None,
            tool_context,
        }
    }

    pub fn chat_tool_calls(&self) -> Result<Vec<ChatToolCall>, ApiError> {
        self.normalized_tool_calls()
            .into_iter()
            .map(|tc| {
                Ok(ChatToolCall {
                    id: tc.require_call_id()?,
                    call_type: "function".into(),
                    function: ChatFunctionCall {
                        name: Some(tc.require_name()?),
                        arguments: tc.arguments,
                    },
                })
            })
            .collect()
    }

    pub fn final_usage(&self) -> Option<&protocol::common::Usage> {
        self.final_usage.as_ref()
    }

    pub fn process_chunk(
        &mut self,
        chunk: &ChatCompletionChunk,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let mut events = Vec::new();
        self.ensure_lifecycle_started(&mut events);

        if let Some(usage) = &chunk.usage {
            self.final_usage = Some(usage.clone());
        }

        let choices = chunk.choices.as_deref().unwrap_or(&[]);
        if choices.is_empty() {
            return Ok(events);
        }

        let choice = &choices[0];
        if let Some(finish_reason) = &choice.finish_reason {
            self.finish_reason = Some(finish_reason.clone());
        }

        let delta = &choice.delta;
        if let Some(reasoning) = &delta.reasoning_content
            && !reasoning.is_empty()
        {
            events.extend(self.push_reasoning_delta(reasoning));
        }

        if let Some(text) = &delta.content
            && !text.is_empty()
        {
            events.extend(self.finalize_reasoning()?);
            events.extend(self.push_text_delta(text));
        }

        if let Some(tool_calls) = &delta.tool_calls {
            events.extend(self.finalize_reasoning()?);
            for tool_call in tool_calls {
                events.extend(self.push_tool_call_delta(tool_call)?);
            }
        }

        Ok(events)
    }

    pub fn complete(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        if self.completed_sent {
            return Ok(Vec::new());
        }
        self.completed_sent = true;

        let mut events = Vec::new();
        self.ensure_lifecycle_started(&mut events);
        events.extend(self.finalize_reasoning()?);
        if self.text.added || !self.tool_call_active {
            events.extend(self.finalize_text()?);
        }
        events.extend(self.finalize_tools()?);

        let mut output = self.completed_items.clone();
        output.sort_by_key(|(output_index, _)| *output_index);
        let output = output.into_iter().map(|(_, item)| item).collect::<Vec<_>>();

        let mut shell = SseResponseShell::minimal(
            self.response_id.clone(),
            self.model.clone(),
            self.created_at,
        );
        shell.status = "completed".into();
        shell.output = Some(output);
        shell.output_text = Some(self.accumulated_text.clone());
        shell.usage = Some(self.final_usage.clone().unwrap_or_default());
        events.push(ResponseSseEvent::ResponseCompleted { response: shell });
        Ok(events)
    }

    fn ensure_lifecycle_started(&mut self, events: &mut Vec<ResponseSseEvent>) {
        if self.lifecycle_sent {
            return;
        }
        let response = SseResponseShell::minimal(
            self.response_id.clone(),
            self.model.clone(),
            self.created_at,
        );
        events.push(ResponseSseEvent::ResponseCreated {
            response: response.clone(),
        });
        events.push(ResponseSseEvent::ResponseInProgress { response });
        self.lifecycle_sent = true;
    }

    fn next_output_index(&mut self) -> u32 {
        let output_index = self.next_output_index;
        self.next_output_index += 1;
        output_index
    }

    fn push_reasoning_delta(&mut self, delta: &str) -> Vec<ResponseSseEvent> {
        let mut events = Vec::new();
        if !self.reasoning.added {
            let output_index = self.next_output_index();
            let item_id = format!("rs_{}", Uuid::new_v4());
            self.reasoning.output_index = Some(output_index);
            self.reasoning.item_id = item_id.clone();
            self.reasoning.added = true;
            events.push(ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::Reasoning {
                    id: item_id.clone(),
                    content: None,
                    summary: Some(vec![]),
                    status: Some("in_progress".into()),
                },
            });
            events.push(ResponseSseEvent::ResponseReasoningSummaryPartAdded {
                item_id,
                output_index,
                summary_index: 0,
                part: SummaryPart::SummaryText {
                    text: String::new(),
                },
            });
        }

        self.reasoning.text.push_str(delta);
        self.reasoning_content.push_str(delta);
        events.push(ResponseSseEvent::ResponseReasoningSummaryTextDelta {
            item_id: self.reasoning.item_id.clone(),
            output_index: self.reasoning.output_index.unwrap_or(0),
            summary_index: 0,
            delta: delta.to_string(),
        });
        events
    }

    fn finalize_reasoning(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        if !self.reasoning.added || self.reasoning.done {
            return Ok(Vec::new());
        }
        let output_index = self.reasoning.output_index.unwrap_or(0);
        let item_id = self.reasoning.item_id.clone();
        let text = self.reasoning.text.clone();
        let summary = SummaryPart::SummaryText { text: text.clone() };
        let item = ResponseOutputItem::Reasoning {
            id: item_id.clone(),
            content: None,
            summary: Some(vec![summary.clone()]),
            status: Some("completed".into()),
        };
        self.completed_items.push((output_index, item.clone()));
        self.reasoning.done = true;
        Ok(vec![
            ResponseSseEvent::ResponseReasoningSummaryTextDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                text: text.clone(),
            },
            ResponseSseEvent::ResponseReasoningSummaryPartDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                part: summary,
            },
            ResponseSseEvent::ResponseOutputItemDone { output_index, item },
        ])
    }

    fn push_text_delta(&mut self, delta: &str) -> Vec<ResponseSseEvent> {
        let mut events = Vec::new();
        if !self.text.added {
            let output_index = self.next_output_index();
            self.text.output_index = Some(output_index);
            self.text.added = true;
            events.push(ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::Message {
                    id: self.text.item_id.clone(),
                    status: "in_progress".into(),
                    role: "assistant".into(),
                    content: vec![],
                },
            });
            events.push(ResponseSseEvent::ResponseContentPartAdded {
                item_id: self.text.item_id.clone(),
                output_index,
                content_index: 0,
                part: ContentPart::OutputText {
                    text: String::new(),
                    annotations: vec![],
                },
            });
        }

        self.text.text.push_str(delta);
        self.accumulated_text.push_str(delta);
        events.push(ResponseSseEvent::ResponseOutputTextDelta {
            item_id: self.text.item_id.clone(),
            output_index: self.text.output_index.unwrap_or(0),
            content_index: 0,
            delta: delta.to_string(),
        });
        events
    }

    fn finalize_text(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        if self.text.done {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        if !self.text.added {
            let output_index = self.next_output_index();
            self.text.output_index = Some(output_index);
            self.text.added = true;
            events.push(ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::Message {
                    id: self.text.item_id.clone(),
                    status: "in_progress".into(),
                    role: "assistant".into(),
                    content: vec![],
                },
            });
            events.push(ResponseSseEvent::ResponseContentPartAdded {
                item_id: self.text.item_id.clone(),
                output_index,
                content_index: 0,
                part: ContentPart::OutputText {
                    text: String::new(),
                    annotations: vec![],
                },
            });
        }
        let output_index = self.text.output_index.unwrap_or_else(|| {
            let output_index = self.next_output_index;
            self.next_output_index += 1;
            self.text.output_index = Some(output_index);
            output_index
        });
        let item = ResponseOutputItem::Message {
            id: self.text.item_id.clone(),
            status: "completed".into(),
            role: "assistant".into(),
            content: vec![ContentPart::OutputText {
                text: self.text.text.clone(),
                annotations: vec![],
            }],
        };
        self.completed_items.push((output_index, item.clone()));
        self.text.done = true;
        events.extend([
            ResponseSseEvent::ResponseOutputTextDone {
                item_id: self.text.item_id.clone(),
                output_index,
                content_index: 0,
                text: self.text.text.clone(),
            },
            ResponseSseEvent::ResponseContentPartDone {
                item_id: self.text.item_id.clone(),
                output_index,
                content_index: 0,
                part: ContentPart::OutputText {
                    text: self.text.text.clone(),
                    annotations: vec![],
                },
            },
            ResponseSseEvent::ResponseOutputItemDone { output_index, item },
        ]);
        Ok(events)
    }

    fn push_tool_call_delta(
        &mut self,
        tool_call: &protocol::chat::ChatToolCallDelta,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let mut events = Vec::new();
        let pos = match self
            .tools
            .iter()
            .position(|existing| existing.chat_index == tool_call.index)
        {
            Some(pos) => pos,
            None => {
                self.tool_call_active = true;
                self.tools.push(ToolCallState::new(tool_call.index));
                self.tools.len() - 1
            }
        };

        if let Some(id) = &tool_call.id {
            let chat_index = self.tools[pos].chat_index;
            set_once(&mut self.tools[pos].call_id, id, "call_id", chat_index)?;
        }
        if let Some(function) = &tool_call.function {
            if let Some(name) = &function.name {
                let chat_index = self.tools[pos].chat_index;
                set_once(&mut self.tools[pos].name, name, "function.name", chat_index)?;
            }
            if let Some(arguments) = &function.arguments {
                self.tools[pos].arguments.push_str(arguments);
            }
        }

        if !self.tools[pos].added
            && self.tools[pos]
                .call_id
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && self.tools[pos]
                .name
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && !self.is_tool_search(&self.tools[pos])
        {
            let output_index = self.next_output_index();
            self.tools[pos].output_index = Some(output_index);
            self.tools[pos].added = true;
            let item = self.tool_item(&self.tools[pos], "in_progress")?;
            events.push(ResponseSseEvent::ResponseOutputItemAdded { output_index, item });
            if self.emits_function_arguments_events(&self.tools[pos])
                && !self.tools[pos].arguments.is_empty()
            {
                events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDelta {
                    item_id: self.tools[pos].item_id.clone(),
                    output_index,
                    delta: self.tools[pos].arguments.clone(),
                });
            }
        } else if self.tools[pos].added
            && self.emits_function_arguments_events(&self.tools[pos])
            && let Some(arguments) = tool_call
                .function
                .as_ref()
                .and_then(|function| function.arguments.as_ref())
            && !arguments.is_empty()
        {
            events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDelta {
                item_id: self.tools[pos].item_id.clone(),
                output_index: self.tools[pos].output_index.unwrap_or(0),
                delta: arguments.clone(),
            });
        }

        if let Some(first) = self.tools.first() {
            self.tool_call_index = first.chat_index;
            self.tool_call_id = first.call_id.clone();
            self.tool_call_name = first.name.clone();
            self.tool_call_arguments = first.arguments.clone();
        }

        Ok(events)
    }

    fn finalize_tools(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let normalized = self.normalized_tool_calls();
        let mut events = Vec::new();
        for mut tool in normalized {
            if tool.done {
                continue;
            }
            let output_index = match tool.output_index {
                Some(output_index) => output_index,
                None => {
                    let output_index = self.next_output_index();
                    tool.output_index = Some(output_index);
                    output_index
                }
            };
            if !tool.added {
                let item = self.tool_item(&tool, "in_progress")?;
                events.push(ResponseSseEvent::ResponseOutputItemAdded { output_index, item });
            }

            let name = tool.require_name()?;
            if self.is_custom_tool(&tool) {
                let input = custom_tool_input_from_arguments(&name, &tool.arguments)?;
                if !input.is_empty() {
                    events.push(ResponseSseEvent::ResponseCustomToolCallInputDelta {
                        item_id: tool.item_id.clone(),
                        output_index,
                        delta: input.clone(),
                    });
                }
                events.push(ResponseSseEvent::ResponseCustomToolCallInputDone {
                    item_id: tool.item_id.clone(),
                    output_index,
                    input,
                });
            } else if self.emits_function_arguments_events(&tool) {
                events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDone {
                    item_id: tool.item_id.clone(),
                    output_index,
                    arguments: tool.arguments.clone(),
                    name: Some(name),
                });
            }

            let item = self.tool_item(&tool, "completed")?;
            self.completed_items.push((output_index, item.clone()));
            events.push(ResponseSseEvent::ResponseOutputItemDone { output_index, item });
        }
        Ok(events)
    }

    fn tool_item(
        &self,
        tool: &ToolCallState,
        status: &str,
    ) -> Result<ResponseOutputItem, ApiError> {
        self.tool_context.output_item(
            tool.item_id.clone(),
            status.into(),
            tool.require_call_id()?,
            tool.require_name()?,
            tool.arguments.clone(),
        )
    }

    fn is_custom_tool(&self, tool: &ToolCallState) -> bool {
        tool.name
            .as_ref()
            .is_some_and(|name| self.tool_context.is_custom_tool_name(name))
    }

    fn is_tool_search(&self, tool: &ToolCallState) -> bool {
        tool.name
            .as_ref()
            .is_some_and(|name| self.tool_context.is_tool_search_name(name))
    }

    fn emits_function_arguments_events(&self, tool: &ToolCallState) -> bool {
        !self.is_custom_tool(tool) && !self.is_tool_search(tool)
    }

    fn normalized_tool_calls(&self) -> Vec<ToolCallState> {
        let mut normalized = Vec::new();
        for tool_call in &self.tools {
            if let Some(arguments) = split_concatenated_json_objects(&tool_call.arguments) {
                for (idx, arguments) in arguments.into_iter().enumerate() {
                    let mut split = tool_call.clone();
                    split.chat_index = normalized.len() as u32;
                    split.output_index = if idx == 0 { split.output_index } else { None };
                    split.item_id = if idx == 0 {
                        split.item_id
                    } else {
                        format!("fc_{}", Uuid::new_v4())
                    };
                    split.call_id = split.call_id.map(|call_id| {
                        if idx == 0 {
                            call_id
                        } else {
                            format!("{call_id}_{idx}")
                        }
                    });
                    split.arguments = arguments;
                    split.added = idx == 0 && split.added;
                    split.done = false;
                    normalized.push(split);
                }
            } else {
                let mut tool_call = tool_call.clone();
                tool_call.chat_index = normalized.len() as u32;
                normalized.push(tool_call);
            }
        }
        normalized
    }
}

fn set_once(
    slot: &mut Option<String>,
    value: &str,
    field: &str,
    chat_index: u32,
) -> Result<(), ApiError> {
    match slot {
        Some(existing) if existing != value => Err(ApiError::upstream_error(format!(
            "streamed tool call at index {chat_index} changed {field} from '{existing}' to '{value}'"
        ))),
        Some(_) => Ok(()),
        None => {
            *slot = Some(value.to_string());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::chat::{ChatChunkChoice, ChatDelta, ChatFunctionDelta, ChatToolCallDelta};
    use protocol::common::Usage;
    use protocol::responses::{CustomToolFormat, ResponseTool};

    fn chunk(
        reasoning: Option<&str>,
        content: Option<&str>,
        tool_calls: Option<Vec<ChatToolCallDelta>>,
        finish_reason: Option<&str>,
        usage: Option<Usage>,
        empty_choices: bool,
    ) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chatcmpl_test".into(),
            object: "chat.completion.chunk".into(),
            created: 1,
            model: "test-model".into(),
            choices: if empty_choices {
                Some(vec![])
            } else {
                Some(vec![ChatChunkChoice {
                    index: 0,
                    delta: ChatDelta {
                        content: content.map(str::to_string),
                        reasoning_content: reasoning.map(str::to_string),
                        tool_calls,
                        ..ChatDelta::default()
                    },
                    finish_reason: finish_reason.map(str::to_string),
                }])
            },
            usage,
        }
    }

    fn state() -> StreamState {
        StreamState::new(
            "resp_test".into(),
            "test-model".into(),
            1,
            "msg_test".into(),
            ChatToolContext::default(),
        )
    }

    fn tool_delta(
        index: u32,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) -> ChatToolCallDelta {
        ChatToolCallDelta {
            index,
            id: id.map(str::to_string),
            function: Some(ChatFunctionDelta {
                name: name.map(str::to_string),
                arguments: args.map(str::to_string),
            }),
        }
    }

    #[test]
    fn streamed_reasoning_emits_summary_lifecycle_before_message() {
        let mut state = state();
        let mut events = state
            .process_chunk(&chunk(Some("think"), None, None, None, None, false))
            .unwrap();
        events.extend(
            state
                .process_chunk(&chunk(None, Some("answer"), None, None, None, false))
                .unwrap(),
        );

        let reasoning_done = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ResponseSseEvent::ResponseOutputItemDone {
                        item: ResponseOutputItem::Reasoning { .. },
                        ..
                    }
                )
            })
            .unwrap();
        let message_added = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ResponseSseEvent::ResponseOutputItemAdded {
                        item: ResponseOutputItem::Message { .. },
                        ..
                    }
                )
            })
            .unwrap();
        assert!(reasoning_done < message_added);
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseReasoningSummaryTextDelta { delta, .. } if delta == "think"
        )));
    }

    #[test]
    fn text_preamble_then_tool_call_keeps_message_done_before_tool_done() {
        let mut state = state();
        let mut events = state
            .process_chunk(&chunk(None, Some("checking"), None, None, None, false))
            .unwrap();
        events.extend(
            state
                .process_chunk(&chunk(
                    None,
                    None,
                    Some(vec![tool_delta(
                        0,
                        Some("call_1"),
                        Some("lookup"),
                        Some("{\"q\":\"x\"}"),
                    )]),
                    Some("tool_calls"),
                    None,
                    false,
                ))
                .unwrap(),
        );
        events.extend(state.complete().unwrap());

        let message_done = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ResponseSseEvent::ResponseOutputItemDone {
                        item: ResponseOutputItem::Message { .. },
                        ..
                    }
                )
            })
            .unwrap();
        let tool_done = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ResponseSseEvent::ResponseOutputItemDone {
                        item: ResponseOutputItem::FunctionCall { .. },
                        ..
                    }
                )
            })
            .unwrap();
        assert!(message_done < tool_done);
    }

    #[test]
    fn reasoning_text_and_tool_call_finish_in_codex_order() {
        let mut state = state();
        let mut events = state
            .process_chunk(&chunk(Some("think "), None, None, None, None, false))
            .unwrap();
        events.extend(
            state
                .process_chunk(&chunk(None, Some("checking"), None, None, None, false))
                .unwrap(),
        );
        events.extend(
            state
                .process_chunk(&chunk(
                    None,
                    None,
                    Some(vec![tool_delta(
                        0,
                        Some("call_1"),
                        Some("lookup"),
                        Some("{\"q\":\"x\"}"),
                    )]),
                    Some("tool_calls"),
                    None,
                    false,
                ))
                .unwrap(),
        );
        events.extend(state.complete().unwrap());

        let reasoning_done = done_position(&events, "reasoning");
        let message_added = added_position(&events, "message");
        let message_done = done_position(&events, "message");
        let tool_done = done_position(&events, "function_call");
        let completed = events
            .iter()
            .position(|event| matches!(event, ResponseSseEvent::ResponseCompleted { .. }))
            .unwrap();

        assert!(reasoning_done < message_added);
        assert!(message_added < message_done);
        assert!(message_done < tool_done);
        assert!(tool_done < completed);
    }

    #[test]
    fn pure_tool_call_does_not_emit_empty_message() {
        let mut state = state();
        let mut events = state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![tool_delta(
                    0,
                    Some("call_1"),
                    Some("lookup"),
                    Some("{\"q\":\"x\"}"),
                )]),
                Some("tool_calls"),
                None,
                false,
            ))
            .unwrap();
        events.extend(state.complete().unwrap());

        assert!(!events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseOutputItemDone {
                item: ResponseOutputItem::Message { .. },
                ..
            }
        )));
    }

    #[test]
    fn parallel_tool_calls_have_stable_output_order() {
        let mut state = state();
        let mut events = state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![
                    tool_delta(0, Some("call_a"), Some("lookup_a"), Some("{\"q\":\"a\"}")),
                    tool_delta(1, Some("call_b"), Some("lookup_b"), Some("{\"q\":\"b\"}")),
                ]),
                Some("tool_calls"),
                None,
                false,
            ))
            .unwrap();
        events.extend(state.complete().unwrap());

        let added_indexes = events
            .iter()
            .filter_map(|event| match event {
                ResponseSseEvent::ResponseOutputItemAdded {
                    output_index,
                    item: ResponseOutputItem::FunctionCall { .. },
                } => Some(*output_index),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(added_indexes, vec![0, 1]);

        let completed_output = events
            .iter()
            .find_map(|event| match event {
                ResponseSseEvent::ResponseCompleted { response } => response.output.as_ref(),
                _ => None,
            })
            .unwrap();
        let names = completed_output
            .iter()
            .map(|item| match item {
                ResponseOutputItem::FunctionCall { name, .. } => name.as_str(),
                other => panic!("expected function call output item, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["lookup_a", "lookup_b"]);
    }

    #[test]
    fn missing_tool_name_fails_closed() {
        let mut state = state();
        state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![tool_delta(0, Some("call_1"), None, Some("{}"))]),
                Some("tool_calls"),
                None,
                false,
            ))
            .unwrap();
        let err = state.complete().unwrap_err();
        assert!(err.to_string().contains("missing required function.name"));
    }

    #[test]
    fn missing_tool_call_id_fails_closed() {
        let mut state = state();
        state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![tool_delta(0, None, Some("lookup"), Some("{}"))]),
                Some("tool_calls"),
                None,
                false,
            ))
            .unwrap();
        let err = state.complete().unwrap_err();
        assert!(err.to_string().contains("missing required call_id"));
    }

    #[test]
    fn usage_chunk_with_empty_choices_is_preserved() {
        let mut state = state();
        state
            .process_chunk(&chunk(None, Some("ok"), None, None, None, false))
            .unwrap();
        state
            .process_chunk(&chunk(
                None,
                None,
                None,
                None,
                Some(Usage {
                    input_tokens: 2,
                    output_tokens: 1,
                    total_tokens: 3,
                    ..Usage::default()
                }),
                true,
            ))
            .unwrap();
        let events = state.complete().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseCompleted {
                response: SseResponseShell {
                    usage: Some(Usage {
                        total_tokens: 3,
                        ..
                    }),
                    ..
                }
            }
        )));
    }

    #[test]
    fn custom_tool_restores_input() {
        let context = ChatToolContext::from_response_tools(&[ResponseTool::Custom {
            name: "apply_patch".into(),
            description: "Apply a patch".into(),
            format: CustomToolFormat {
                format_type: "grammar".into(),
                syntax: "lark".into(),
                definition: "start: /.+/".into(),
            },
        }]);
        let mut state = StreamState::new(
            "resp_test".into(),
            "test-model".into(),
            1,
            "msg_test".into(),
            context,
        );
        state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![tool_delta(
                    0,
                    Some("call_1"),
                    Some("apply_patch"),
                    Some("{\"input\":\"patch\"}"),
                )]),
                Some("tool_calls"),
                None,
                false,
            ))
            .unwrap();
        let events = state.complete().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseCustomToolCallInputDone { input, .. } if input == "patch"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseOutputItemDone {
                item: ResponseOutputItem::CustomToolCall { input, .. },
                ..
            } if input == "patch"
        )));
    }

    #[test]
    fn tool_search_waits_for_complete_streamed_arguments() {
        let context =
            ChatToolContext::from_response_tools(&[ResponseTool::ToolSearch { description: None }]);
        let mut state = StreamState::new(
            "resp_test".into(),
            "test-model".into(),
            1,
            "msg_test".into(),
            context,
        );

        let first_events = state
            .process_chunk(&chunk(
                None,
                None,
                Some(vec![tool_delta(
                    0,
                    Some("call_1"),
                    Some("tool_search"),
                    Some("{\"qu"),
                )]),
                None,
                None,
                false,
            ))
            .unwrap();
        assert!(!first_events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseOutputItemAdded {
                item: ResponseOutputItem::ToolSearchCall { .. },
                ..
            }
        )));

        let mut events = first_events;
        events.extend(
            state
                .process_chunk(&chunk(
                    None,
                    None,
                    Some(vec![tool_delta(0, None, None, Some("ery\":\"rg\"}"))]),
                    Some("tool_calls"),
                    None,
                    false,
                ))
                .unwrap(),
        );
        events.extend(state.complete().unwrap());

        assert!(!events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseFunctionCallArgumentsDelta { .. }
                | ResponseSseEvent::ResponseFunctionCallArgumentsDone { .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseOutputItemDone {
                item: ResponseOutputItem::ToolSearchCall { arguments, .. },
                ..
            } if arguments["query"] == "rg"
        )));
    }

    fn added_position(events: &[ResponseSseEvent], item_type: &str) -> usize {
        events
            .iter()
            .position(|event| {
                matches!(
                    (item_type, event),
                    (
                        "message",
                        ResponseSseEvent::ResponseOutputItemAdded {
                            item: ResponseOutputItem::Message { .. },
                            ..
                        },
                    ) | (
                        "function_call",
                        ResponseSseEvent::ResponseOutputItemAdded {
                            item: ResponseOutputItem::FunctionCall { .. },
                            ..
                        },
                    )
                )
            })
            .unwrap()
    }

    fn done_position(events: &[ResponseSseEvent], item_type: &str) -> usize {
        events
            .iter()
            .position(|event| {
                matches!(
                    (item_type, event),
                    (
                        "reasoning",
                        ResponseSseEvent::ResponseOutputItemDone {
                            item: ResponseOutputItem::Reasoning { .. },
                            ..
                        },
                    ) | (
                        "message",
                        ResponseSseEvent::ResponseOutputItemDone {
                            item: ResponseOutputItem::Message { .. },
                            ..
                        },
                    ) | (
                        "function_call",
                        ResponseSseEvent::ResponseOutputItemDone {
                            item: ResponseOutputItem::FunctionCall { .. },
                            ..
                        },
                    )
                )
            })
            .unwrap()
    }
}
