use protocol::chat::{ChatCompletionChunk, ChatFunctionCall, ChatToolCall};
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{ResponseOutputItem, SummaryPart};
use protocol::sse::{ResponseSseEvent, SseResponseShell};
use uuid::Uuid;

use crate::tool_call_normalizer::split_concatenated_json_objects;

/// Mutable state for tracking SSE event emission across a stream.
#[derive(Debug)]
pub struct StreamState {
    pub response_id: String,
    pub model: String,
    pub created_at: i64,
    pub output_item_id: String,
    pub output_index: u32,
    content_index: u32,
    lifecycle_sent: bool,
    completed_sent: bool, // guard against duplicate completion events
    pub accumulated_text: String,
    pub tool_call_id: Option<String>,
    pub tool_call_name: Option<String>,
    pub tool_call_arguments: String,
    pub tool_call_index: u32,
    pub tool_call_active: bool,
    tool_calls: Vec<StreamToolCall>,
    pub reasoning_content: String,
    final_usage: Option<protocol::common::Usage>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct StreamToolCall {
    index: u32,
    item_id: String,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl StreamToolCall {
    fn output_item(&self, status: &str) -> ResponseOutputItem {
        ResponseOutputItem::FunctionCall {
            id: self.item_id.clone(),
            status: status.into(),
            call_id: self.call_id.clone().unwrap_or_default(),
            name: self.name.clone().unwrap_or_default(),
            arguments: self.arguments.clone(),
        }
    }
}

impl StreamState {
    pub fn new(
        response_id: String,
        model: String,
        created_at: i64,
        output_item_id: String,
    ) -> Self {
        Self {
            response_id,
            model,
            created_at,
            output_item_id,
            output_index: 0,
            content_index: 0,
            lifecycle_sent: false,
            completed_sent: false,
            accumulated_text: String::new(),
            tool_call_id: None,
            tool_call_name: None,
            tool_call_arguments: String::new(),
            tool_call_index: 0,
            tool_call_active: false,
            tool_calls: Vec::new(),
            reasoning_content: String::new(),
            final_usage: None,
            finish_reason: None,
        }
    }

    pub fn chat_tool_calls(&self) -> Vec<ChatToolCall> {
        self.normalized_tool_calls()
            .iter()
            .map(|tc| ChatToolCall {
                id: tc.call_id.clone().unwrap_or_default(),
                call_type: "function".into(),
                function: ChatFunctionCall {
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                },
            })
            .collect()
    }

    fn normalized_tool_calls(&self) -> Vec<StreamToolCall> {
        let mut normalized = Vec::new();
        for tool_call in &self.tool_calls {
            if let Some(arguments) = split_concatenated_json_objects(&tool_call.arguments) {
                for (idx, arguments) in arguments.into_iter().enumerate() {
                    normalized.push(StreamToolCall {
                        index: normalized.len() as u32,
                        item_id: if idx == 0 {
                            tool_call.item_id.clone()
                        } else {
                            format!("fc_{}", Uuid::new_v4())
                        },
                        call_id: tool_call.call_id.as_ref().map(|call_id| {
                            if idx == 0 {
                                call_id.clone()
                            } else {
                                format!("{call_id}_{idx}")
                            }
                        }),
                        name: tool_call.name.clone(),
                        arguments,
                    });
                }
            } else {
                let mut tool_call = tool_call.clone();
                tool_call.index = normalized.len() as u32;
                normalized.push(tool_call);
            }
        }
        normalized
    }

    /// Process a single Chat SSE chunk. Returns zero or more Response SSE events.
    pub fn process_chunk(
        &mut self,
        chunk: &ChatCompletionChunk,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let mut events: Vec<ResponseSseEvent> = Vec::new();

        // Send lifecycle events on first chunk
        if !self.lifecycle_sent {
            events.push(ResponseSseEvent::ResponseCreated {
                response: SseResponseShell::minimal(
                    self.response_id.clone(),
                    self.model.clone(),
                    self.created_at,
                ),
            });
            // Pre-announce the output item (we'll fill in details at done)
            events.push(ResponseSseEvent::ResponseOutputItemAdded {
                output_index: self.output_index,
                item: ResponseOutputItem::Message {
                    id: self.output_item_id.clone(),
                    status: "in_progress".into(),
                    role: "assistant".into(),
                    content: vec![],
                },
            });
            events.push(ResponseSseEvent::ResponseContentPartAdded {
                item_id: self.output_item_id.clone(),
                output_index: self.output_index,
                content_index: self.content_index,
                part: ContentPart::OutputText {
                    text: String::new(),
                    annotations: vec![],
                },
            });
            self.lifecycle_sent = true;
        }

        // Save usage before checking empty choices
        // (OpenAI-compatible backends often send usage in a choices: [] final chunk)
        if let Some(usage) = &chunk.usage {
            self.final_usage = Some(usage.clone());
        }

        let choices = chunk.choices.as_deref().unwrap_or(&[]);
        if choices.is_empty() {
            return Ok(events);
        }

        let choice = &choices[0];
        let delta = &choice.delta;

        // Save finish reason
        if let Some(fr) = &choice.finish_reason {
            self.finish_reason = Some(fr.clone());
        }

        // Accumulate reasoning content (needed for DeepSeek tool call continuation)
        if let Some(rc) = &delta.reasoning_content
            && !rc.is_empty()
        {
            self.reasoning_content.push_str(rc);
        }

        // Handle text content
        if let Some(text) = &delta.content
            && !text.is_empty()
        {
            self.accumulated_text.push_str(text);
            events.push(ResponseSseEvent::ResponseOutputTextDelta {
                item_id: self.output_item_id.clone(),
                output_index: self.output_index,
                content_index: self.content_index,
                delta: text.clone(),
            });
        }

        // Handle tool calls
        if let Some(tool_calls) = &delta.tool_calls {
            for tc in tool_calls {
                let existing_pos = self
                    .tool_calls
                    .iter()
                    .position(|call| call.index == tc.index);
                let (pos, emit_added) = match existing_pos {
                    Some(pos) => (pos, false),
                    None => {
                        let is_first = self.tool_calls.is_empty();
                        let item_id = if is_first {
                            self.output_item_id.clone()
                        } else {
                            format!("fc_{}", Uuid::new_v4())
                        };
                        self.tool_call_active = true;
                        self.tool_calls.push(StreamToolCall {
                            index: tc.index,
                            item_id,
                            call_id: tc.id.clone(),
                            name: None,
                            arguments: String::new(),
                        });
                        let pos = self.tool_calls.len() - 1;
                        (pos, !is_first)
                    }
                };

                if let Some(id) = &tc.id {
                    self.tool_calls[pos].call_id = Some(id.clone());
                }

                if let Some(func) = &tc.function {
                    if let Some(name) = &func.name {
                        self.tool_calls[pos].name = Some(name.clone());
                    }
                    if emit_added {
                        events.push(ResponseSseEvent::ResponseOutputItemAdded {
                            output_index: tc.index,
                            item: self.tool_calls[pos].output_item("in_progress"),
                        });
                    }
                    if let Some(args) = &func.arguments {
                        self.tool_calls[pos].arguments.push_str(args);
                        events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDelta {
                            item_id: self.tool_calls[pos].item_id.clone(),
                            output_index: tc.index,
                            delta: args.clone(),
                        });
                    }
                }

                if pos == 0 {
                    self.tool_call_index = self.tool_calls[0].index;
                    self.tool_call_id = self.tool_calls[0].call_id.clone();
                    self.tool_call_name = self.tool_calls[0].name.clone();
                    self.tool_call_arguments = self.tool_calls[0].arguments.clone();
                }
            }
        }

        Ok(events)
    }

    pub fn final_usage(&self) -> Option<&protocol::common::Usage> {
        self.final_usage.as_ref()
    }

    /// Emit the final completion lifecycle events once the upstream stream ends.
    pub fn complete(&mut self) -> Vec<ResponseSseEvent> {
        if self.completed_sent {
            return Vec::new();
        }
        self.completed_sent = true;
        let mut events = Vec::new();
        self.emit_completion_events(&mut events);
        events
    }

    fn emit_completion_events(&self, events: &mut Vec<ResponseSseEvent>) {
        let tool_calls = self.normalized_tool_calls();

        if self.tool_call_active {
            for tool_call in tool_calls.iter().skip(1).filter(|tool_call| {
                !self
                    .tool_calls
                    .iter()
                    .any(|original| original.item_id == tool_call.item_id)
            }) {
                events.push(ResponseSseEvent::ResponseOutputItemAdded {
                    output_index: tool_call.index,
                    item: tool_call.output_item("in_progress"),
                });
            }

            // Finish the tool call
            for tool_call in &tool_calls {
                events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDone {
                    item_id: tool_call.item_id.clone(),
                    output_index: tool_call.index,
                    arguments: tool_call.arguments.clone(),
                    name: tool_call.name.clone().unwrap_or_default(),
                });
            }
        }

        if !self.accumulated_text.is_empty() || !self.tool_call_active {
            // Finish text output
            events.push(ResponseSseEvent::ResponseOutputTextDone {
                item_id: self.output_item_id.clone(),
                output_index: self.output_index,
                content_index: self.content_index,
                text: self.accumulated_text.clone(),
            });

            // Content part done
            events.push(ResponseSseEvent::ResponseContentPartDone {
                item_id: self.output_item_id.clone(),
                output_index: self.output_index,
                content_index: self.content_index,
                part: ContentPart::OutputText {
                    text: self.accumulated_text.clone(),
                    annotations: vec![],
                },
            });
        }

        // Output item done
        let item = if self.tool_call_active {
            tool_calls
                .first()
                .map(|tool_call| tool_call.output_item("completed"))
                .unwrap_or_else(|| ResponseOutputItem::FunctionCall {
                    id: self.output_item_id.clone(),
                    status: "completed".into(),
                    call_id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                })
        } else {
            ResponseOutputItem::Message {
                id: self.output_item_id.clone(),
                status: "completed".into(),
                role: "assistant".into(),
                content: vec![ContentPart::OutputText {
                    text: self.accumulated_text.clone(),
                    annotations: vec![],
                }],
            }
        };
        events.push(ResponseSseEvent::ResponseOutputItemDone {
            output_index: if self.tool_call_active {
                tool_calls
                    .first()
                    .map(|tool_call| tool_call.index)
                    .unwrap_or(self.output_index)
            } else {
                self.output_index
            },
            item,
        });
        if self.tool_call_active {
            for tool_call in tool_calls.iter().skip(1) {
                events.push(ResponseSseEvent::ResponseOutputItemDone {
                    output_index: tool_call.index,
                    item: tool_call.output_item("completed"),
                });
            }
        }

        // Response completed
        let mut shell = SseResponseShell::minimal(
            self.response_id.clone(),
            self.model.clone(),
            self.created_at,
        );
        let mut final_output: Vec<ResponseOutputItem> = Vec::new();

        // Include reasoning item if present.
        // Codex requires `summary` to be present (Vec<ReasoningItemReasoningSummary>),
        // and uses `reasoning_text`/`text` for content items. `output_text` is not
        // a recognized ReasoningItemContent variant, so we put reasoning text in
        // `summary` (as SummaryText) which Codex always parses.
        if !self.reasoning_content.is_empty() {
            final_output.push(ResponseOutputItem::Reasoning {
                id: format!("rs_{}", Uuid::new_v4()),
                content: None,
                summary: Some(vec![SummaryPart::SummaryText {
                    text: self.reasoning_content.clone(),
                }]),
                status: Some("completed".into()),
            });
        }

        if self.tool_call_active {
            final_output.extend(
                tool_calls
                    .iter()
                    .map(|tool_call| tool_call.output_item("completed")),
            );
        } else {
            final_output.push(ResponseOutputItem::Message {
                id: self.output_item_id.clone(),
                status: "completed".into(),
                role: "assistant".into(),
                content: vec![ContentPart::OutputText {
                    text: self.accumulated_text.clone(),
                    annotations: vec![],
                }],
            });
        }

        shell.status = "completed".into();
        shell.output = Some(final_output);
        shell.output_text = Some(self.accumulated_text.clone());
        shell.usage = Some(self.final_usage.clone().unwrap_or_default());

        events.push(ResponseSseEvent::ResponseCompleted { response: shell });
    }
}

#[cfg(test)]
mod tests {
    use super::StreamState;
    use protocol::chat::{
        ChatChunkChoice, ChatCompletionChunk, ChatDelta, ChatFunctionDelta, ChatToolCallDelta,
    };
    use protocol::common::Usage;
    use protocol::responses::ResponseOutputItem;
    use protocol::sse::ResponseSseEvent;

    fn chunk(
        content: Option<&str>,
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
                        ..ChatDelta::default()
                    },
                    finish_reason: finish_reason.map(str::to_string),
                }])
            },
            usage,
        }
    }

    fn completed_usage(events: &[ResponseSseEvent]) -> Usage {
        events
            .iter()
            .find_map(|event| match event {
                ResponseSseEvent::ResponseCompleted { response } => response.usage.clone(),
                _ => None,
            })
            .expect("response.completed usage")
    }

    fn tool_chunk(
        tool_calls: Vec<ChatToolCallDelta>,
        finish_reason: Option<&str>,
    ) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chatcmpl_test".into(),
            object: "chat.completion.chunk".into(),
            created: 1,
            model: "test-model".into(),
            choices: Some(vec![ChatChunkChoice {
                index: 0,
                delta: ChatDelta {
                    tool_calls: Some(tool_calls),
                    ..ChatDelta::default()
                },
                finish_reason: finish_reason.map(str::to_string),
            }]),
            usage: None,
        }
    }

    #[test]
    fn complete_keeps_usage_from_final_empty_chunk() {
        let mut state =
            StreamState::new("resp_test".into(), "test-model".into(), 1, "out_1".into());

        state
            .process_chunk(&chunk(Some("Hello"), None, None, false))
            .expect("text chunk");
        state
            .process_chunk(&chunk(None, Some("stop"), None, false))
            .expect("finish chunk");
        state
            .process_chunk(&chunk(
                None,
                None,
                Some(Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    total_tokens: 18,
                    input_tokens_details: None,
                    output_tokens_details: None,
                }),
                true,
            ))
            .expect("usage chunk");

        let events = state.complete();
        let usage = completed_usage(&events);
        assert_eq!(usage.total_tokens, 18);
    }

    #[test]
    fn complete_uses_latest_streamed_usage_totals() {
        let mut state =
            StreamState::new("resp_test".into(), "test-model".into(), 1, "out_1".into());

        state
            .process_chunk(&chunk(
                Some("Hi"),
                None,
                Some(Usage {
                    input_tokens: 10,
                    output_tokens: 1,
                    total_tokens: 11,
                    input_tokens_details: None,
                    output_tokens_details: None,
                }),
                false,
            ))
            .expect("first chunk");
        state
            .process_chunk(&chunk(
                None,
                Some("stop"),
                Some(Usage {
                    input_tokens: 10,
                    output_tokens: 3,
                    total_tokens: 13,
                    input_tokens_details: None,
                    output_tokens_details: None,
                }),
                false,
            ))
            .expect("final chunk");

        let events = state.complete();
        let usage = completed_usage(&events);
        assert_eq!(usage.total_tokens, 13);
    }

    #[test]
    fn complete_keeps_multiple_tool_call_arguments_separate() {
        let mut state =
            StreamState::new("resp_test".into(), "test-model".into(), 1, "out_1".into());

        let first_args = r#"{"cmd":"cat fixture.py"}"#;
        let second_args = r#"{"cmd":"cat fixture.json"}"#;

        state
            .process_chunk(&tool_chunk(
                vec![
                    ChatToolCallDelta {
                        index: 0,
                        id: Some("call_1".into()),
                        function: Some(ChatFunctionDelta {
                            name: Some("exec_command".into()),
                            arguments: Some(first_args.into()),
                        }),
                    },
                    ChatToolCallDelta {
                        index: 1,
                        id: Some("call_2".into()),
                        function: Some(ChatFunctionDelta {
                            name: Some("exec_command".into()),
                            arguments: Some(second_args.into()),
                        }),
                    },
                ],
                Some("tool_calls"),
            ))
            .expect("tool chunks");

        let events = state.complete();
        let completed = events
            .iter()
            .find_map(|event| match event {
                ResponseSseEvent::ResponseCompleted { response } => response.output.as_ref(),
                _ => None,
            })
            .expect("response.completed output");

        let args: Vec<&str> = completed
            .iter()
            .filter_map(|item| match item {
                ResponseOutputItem::FunctionCall { arguments, .. } => Some(arguments.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args, vec![first_args, second_args]);
        assert!(
            state
                .chat_tool_calls()
                .iter()
                .all(
                    |tc| serde_json::from_str::<serde_json::Value>(&tc.function.arguments).is_ok()
                )
        );
    }

    #[test]
    fn complete_splits_concatenated_single_tool_call_arguments() {
        let mut state =
            StreamState::new("resp_test".into(), "test-model".into(), 1, "out_1".into());

        state
            .process_chunk(&tool_chunk(
                vec![ChatToolCallDelta {
                    index: 0,
                    id: Some("call_1".into()),
                    function: Some(ChatFunctionDelta {
                        name: Some("exec_command".into()),
                        arguments: Some(r#"{"cmd":"a"}{"cmd":"b"}"#.into()),
                    }),
                }],
                Some("tool_calls"),
            ))
            .expect("tool chunk");

        let events = state.complete();
        let completed = events
            .iter()
            .find_map(|event| match event {
                ResponseSseEvent::ResponseCompleted { response } => response.output.as_ref(),
                _ => None,
            })
            .expect("response.completed output");

        let calls: Vec<(&str, &str)> = completed
            .iter()
            .filter_map(|item| match item {
                ResponseOutputItem::FunctionCall {
                    call_id, arguments, ..
                } => Some((call_id.as_str(), arguments.as_str())),
                _ => None,
            })
            .collect();

        assert_eq!(
            calls,
            vec![("call_1", r#"{"cmd":"a"}"#), ("call_1_1", r#"{"cmd":"b"}"#)]
        );
        assert!(
            state
                .chat_tool_calls()
                .iter()
                .all(
                    |tc| serde_json::from_str::<serde_json::Value>(&tc.function.arguments).is_ok()
                )
        );
    }
}
