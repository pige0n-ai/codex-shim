use protocol::chat::ChatCompletionChunk;
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{ResponseOutputItem, SummaryPart};
use protocol::sse::{ResponseSseEvent, SseResponseShell};
use uuid::Uuid;

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
    pub reasoning_content: String,
    final_usage: Option<protocol::common::Usage>,
    pub finish_reason: Option<String>,
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
            reasoning_content: String::new(),
            final_usage: None,
            finish_reason: None,
        }
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
                // Start tracking if this is a new tool call
                if !self.tool_call_active {
                    self.tool_call_active = true;
                    self.tool_call_index = tc.index;
                    if let Some(id) = &tc.id {
                        self.tool_call_id = Some(id.clone());
                    }
                }

                if let Some(func) = &tc.function {
                    if let Some(name) = &func.name {
                        self.tool_call_name = Some(name.clone());
                    }
                    if let Some(args) = &func.arguments {
                        self.tool_call_arguments.push_str(args);
                        events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDelta {
                            item_id: self.output_item_id.clone(),
                            output_index: self.output_index,
                            delta: args.clone(),
                        });
                    }
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
        if self.tool_call_active {
            // Finish the tool call
            events.push(ResponseSseEvent::ResponseFunctionCallArgumentsDone {
                item_id: self.output_item_id.clone(),
                output_index: self.output_index,
                arguments: self.tool_call_arguments.clone(),
                name: self.tool_call_name.clone().unwrap_or_default(),
            });
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
            ResponseOutputItem::FunctionCall {
                id: self.output_item_id.clone(),
                status: "completed".into(),
                call_id: self.tool_call_id.clone().unwrap_or_default(),
                name: self.tool_call_name.clone().unwrap_or_default(),
                arguments: self.tool_call_arguments.clone(),
            }
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
            output_index: self.output_index,
            item,
        });

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
            final_output.push(ResponseOutputItem::FunctionCall {
                id: self.output_item_id.clone(),
                status: "completed".into(),
                call_id: self.tool_call_id.clone().unwrap_or_default(),
                name: self.tool_call_name.clone().unwrap_or_default(),
                arguments: self.tool_call_arguments.clone(),
            });
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
    use protocol::chat::{ChatChunkChoice, ChatCompletionChunk, ChatDelta};
    use protocol::common::Usage;
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
}
