use std::collections::BTreeMap;

use protocol::common::{ContentPart, Usage};
use protocol::error::ApiError;
use protocol::responses::{ResponseOutputItem, SummaryPart};
use protocol::sse::{ResponseSseEvent, SseResponseShell};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct ResponsesCanonicalizer {
    response_id: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,
    created_sent: bool,
    in_progress_sent: bool,
    completed: bool,
    next_output_index: u32,
    items: BTreeMap<String, ItemState>,
    ignored_events: Vec<String>,
}

#[derive(Debug, Default)]
pub struct CanonicalizerOutcome {
    pub events: Vec<ResponseSseEvent>,
    pub completed_response: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemKind {
    Message,
    Reasoning,
    FunctionCall,
    CustomToolCall,
    Other(String),
}

impl Default for ItemKind {
    fn default() -> Self {
        Self::Other(String::new())
    }
}

#[derive(Debug, Default)]
struct ItemState {
    key: String,
    id: Option<String>,
    output_index: Option<u32>,
    order: u32,
    kind: ItemKind,
    role: Option<String>,
    text: String,
    text_seen: bool,
    reasoning_text: String,
    reasoning_deltas: Vec<String>,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    arguments_seen: bool,
    custom_input: String,
    custom_input_seen: bool,
    done_item: Option<Value>,
}

impl ResponsesCanonicalizer {
    pub fn new(response_id: String, model: String, created_at: i64) -> Self {
        Self {
            response_id: Some(response_id),
            model: Some(model),
            created_at: Some(created_at),
            ..Self::default()
        }
    }

    pub fn process_event(&mut self, event: &Value) -> Result<CanonicalizerOutcome, ApiError> {
        let mut outcome = CanonicalizerOutcome::default();
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            self.ignored_events.push("<missing type>".to_string());
            return Ok(outcome);
        };

        match event_type {
            "response.created" => {
                self.capture_response(event.get("response"));
                outcome.events.extend(self.ensure_response_started()?);
            }
            "response.in_progress" => {
                self.capture_response(event.get("response"));
                outcome.events.extend(self.ensure_response_started()?);
            }
            "response.output_item.added" => {
                self.capture_item_event(event, event.get("item"));
            }
            "response.content_part.added" => {
                let state = self.item_for_event(event, Some(ItemKind::Message));
                if state.role.is_none() {
                    state.role = Some("assistant".to_string());
                }
            }
            "response.output_text.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let state = self.item_for_event(event, Some(ItemKind::Message));
                state.kind = ItemKind::Message;
                state.role.get_or_insert_with(|| "assistant".to_string());
                state.text.push_str(delta);
                state.text_seen = true;
            }
            "response.output_text.done" => {
                let state = self.item_for_event(event, Some(ItemKind::Message));
                state.kind = ItemKind::Message;
                state.role.get_or_insert_with(|| "assistant".to_string());
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    state.text = text.to_string();
                    state.text_seen = true;
                }
            }
            "response.content_part.done" => {
                let state = self.item_for_event(event, Some(ItemKind::Message));
                if let Some(part) = event.get("part")
                    && let Some(text) = content_part_text(part)
                {
                    state.text = text;
                    state.text_seen = true;
                }
            }
            "response.reasoning_summary_part.added" => {
                let state = self.item_for_event(event, Some(ItemKind::Reasoning));
                state.kind = ItemKind::Reasoning;
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let state = self.item_for_event(event, Some(ItemKind::Reasoning));
                state.kind = ItemKind::Reasoning;
                state.reasoning_text.push_str(delta);
                state.reasoning_deltas.push(delta.to_string());
            }
            "response.reasoning_summary_text.done" => {
                let state = self.item_for_event(event, Some(ItemKind::Reasoning));
                state.kind = ItemKind::Reasoning;
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    state.reasoning_text = text.to_string();
                    if state.reasoning_deltas.is_empty() && !text.is_empty() {
                        state.reasoning_deltas.push(text.to_string());
                    }
                }
            }
            "response.reasoning_summary_part.done" => {
                let state = self.item_for_event(event, Some(ItemKind::Reasoning));
                state.kind = ItemKind::Reasoning;
                if let Some(part) = event.get("part")
                    && let Some(text) = summary_part_text(part)
                {
                    state.reasoning_text = text;
                }
            }
            "response.function_call_arguments.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let state = self.item_for_event(event, Some(ItemKind::FunctionCall));
                state.kind = ItemKind::FunctionCall;
                state.arguments.push_str(delta);
                state.arguments_seen = true;
            }
            "response.function_call_arguments.done" => {
                let state = self.item_for_event(event, Some(ItemKind::FunctionCall));
                state.kind = ItemKind::FunctionCall;
                if let Some(arguments) = event.get("arguments").and_then(Value::as_str) {
                    state.arguments = arguments.to_string();
                    state.arguments_seen = true;
                }
                if let Some(name) = event.get("name").and_then(Value::as_str) {
                    state.name = Some(name.to_string());
                }
            }
            "response.custom_tool_call_input.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let state = self.item_for_event(event, Some(ItemKind::CustomToolCall));
                state.kind = ItemKind::CustomToolCall;
                state.custom_input.push_str(delta);
                state.custom_input_seen = true;
            }
            "response.custom_tool_call_input.done" => {
                let state = self.item_for_event(event, Some(ItemKind::CustomToolCall));
                state.kind = ItemKind::CustomToolCall;
                if let Some(input) = event.get("input").and_then(Value::as_str) {
                    state.custom_input = input.to_string();
                    state.custom_input_seen = true;
                }
            }
            "response.output_item.done" => {
                self.capture_item_event(event, event.get("item"));
                let state = self.item_for_event(event, None);
                state.done_item = event.get("item").cloned();
                apply_done_item(state)?;
            }
            "response.completed" => {
                self.capture_response(event.get("response"));
                outcome.events.extend(self.finalize(event.get("response"))?);
                outcome.completed_response = event.get("response").cloned();
            }
            "response.failed" => {
                self.completed = true;
                outcome.events.extend(self.ensure_response_started()?);
                let response = self.response_shell_from_value(event.get("response"), "failed")?;
                outcome
                    .events
                    .push(ResponseSseEvent::ResponseFailed { response });
                outcome.completed_response = event.get("response").cloned();
            }
            other => self.ignored_events.push(other.to_string()),
        }

        Ok(outcome)
    }

    pub fn finish_stream(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        if self.completed {
            return Ok(Vec::new());
        }
        Err(ApiError::stream_interrupted_with_details(
            "native Responses stream ended before completion event",
        ))
    }

    pub fn ignored_events(&self) -> &[String] {
        &self.ignored_events
    }

    fn capture_response(&mut self, response: Option<&Value>) {
        let Some(response) = response else {
            return;
        };
        if let Some(id) = response.get("id").and_then(Value::as_str) {
            self.response_id = Some(id.to_string());
        }
        if let Some(model) = response.get("model").and_then(Value::as_str) {
            self.model = Some(model.to_string());
        }
        if let Some(created_at) = response.get("created_at").and_then(Value::as_i64) {
            self.created_at = Some(created_at);
        }
    }

    fn capture_item_event(&mut self, event: &Value, item: Option<&Value>) {
        let Some(item) = item else {
            return;
        };
        let kind = item_kind(item);
        let state = self.item_for_event(event, kind.clone());
        if let Some(kind) = kind {
            state.kind = kind;
        }
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            state.id = Some(id.to_string());
        }
        if let Some(role) = item.get("role").and_then(Value::as_str) {
            state.role = Some(role.to_string());
        }
        if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
            state.call_id = Some(call_id.to_string());
        }
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            state.name = Some(name.to_string());
        }
        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
            state.arguments = arguments.to_string();
            state.arguments_seen = true;
        }
        if let Some(input) = item.get("input").and_then(Value::as_str) {
            state.custom_input = input.to_string();
            state.custom_input_seen = true;
        }
        if let Some(text) = message_item_text(item) {
            state.text = text;
            state.text_seen = true;
        }
        if let Some(text) = reasoning_item_text(item) {
            state.reasoning_text = text.clone();
            if state.reasoning_deltas.is_empty() && !text.is_empty() {
                state.reasoning_deltas.push(text);
            }
        }
    }

    fn item_for_event(&mut self, event: &Value, kind: Option<ItemKind>) -> &mut ItemState {
        let key = item_key(event, kind.as_ref());
        if self.items.contains_key(&key) {
            return self.items.get_mut(&key).expect("item exists");
        }
        let output_index = event
            .get("output_index")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());
        if let Some(output_index) = output_index {
            self.next_output_index = self.next_output_index.max(output_index.saturating_add(1));
        }
        let item_id = event
            .get("item_id")
            .and_then(Value::as_str)
            .or_else(|| {
                event
                    .get("item")
                    .and_then(|item| item.get("id"))
                    .and_then(Value::as_str)
            })
            .map(ToString::to_string);
        let order = output_index.unwrap_or_else(|| self.allocate_output_index());
        self.items.insert(
            key.clone(),
            ItemState {
                key: key.clone(),
                id: item_id,
                output_index: Some(order),
                order,
                kind: kind.unwrap_or_default(),
                ..ItemState::default()
            },
        );
        self.items.get_mut(&key).expect("inserted item exists")
    }

    fn allocate_output_index(&mut self) -> u32 {
        let index = self.next_output_index;
        self.next_output_index = self.next_output_index.saturating_add(1);
        index
    }

    fn ensure_response_started(&mut self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let mut events = Vec::new();
        if !self.created_sent {
            self.created_sent = true;
            events.push(ResponseSseEvent::ResponseCreated {
                response: self.minimal_response("in_progress")?,
            });
        }
        if !self.in_progress_sent {
            self.in_progress_sent = true;
            events.push(ResponseSseEvent::ResponseInProgress {
                response: self.minimal_response("in_progress")?,
            });
        }
        Ok(events)
    }

    fn finalize(
        &mut self,
        completed_response: Option<&Value>,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        if self.completed {
            return Ok(Vec::new());
        }
        self.completed = true;

        let mut events = self.ensure_response_started()?;
        let mut outputs = Vec::new();
        let states = self.ordered_item_keys();
        let has_tool = states.iter().any(|key| {
            self.items.get(key).is_some_and(|state| {
                matches!(
                    state.kind,
                    ItemKind::FunctionCall | ItemKind::CustomToolCall
                )
            })
        });
        let nonblank_non_tool = states.iter().any(|key| {
            self.items.get(key).is_some_and(|state| match state.kind {
                ItemKind::Message => !state.text.trim().is_empty(),
                ItemKind::Reasoning => !state.reasoning_text.trim().is_empty(),
                _ => false,
            })
        });

        for key in states {
            let Some(state) = self.items.get(&key) else {
                continue;
            };
            match state.kind {
                ItemKind::Reasoning => {
                    if state.reasoning_text.trim().is_empty() {
                        continue;
                    }
                    let item = state.reasoning_output_item();
                    events.extend(state.reasoning_events()?);
                    outputs.push(item);
                }
                ItemKind::Message => {
                    let is_blank = state.text.trim().is_empty();
                    if is_blank && (has_tool || nonblank_non_tool) {
                        continue;
                    }
                    let item = state.message_output_item();
                    events.extend(state.message_events()?);
                    outputs.push(item);
                }
                ItemKind::FunctionCall => {
                    let item = state.function_output_item()?;
                    events.extend(state.function_events(&item)?);
                    outputs.push(item);
                }
                ItemKind::CustomToolCall => {
                    let item = state.custom_tool_output_item()?;
                    events.extend(state.custom_tool_events(&item)?);
                    outputs.push(item);
                }
                ItemKind::Other(_) => {}
            }
        }

        let mut completed = self.response_shell_from_value(completed_response, "completed")?;
        completed.output_text = Some(output_text_from_items(&outputs));
        completed.output = Some(outputs);
        events.push(ResponseSseEvent::ResponseCompleted {
            response: completed,
        });

        Ok(events)
    }

    fn ordered_item_keys(&self) -> Vec<String> {
        let mut keyed = self
            .items
            .values()
            .map(|state| (state.order, state.key.clone()))
            .collect::<Vec<_>>();
        keyed.sort_by_key(|(order, _)| *order);
        keyed.into_iter().map(|(_, key)| key).collect()
    }

    fn response_shell_from_value(
        &self,
        response: Option<&Value>,
        default_status: &str,
    ) -> Result<SseResponseShell, ApiError> {
        let mut shell = self.minimal_response(default_status)?;
        let Some(response) = response else {
            return Ok(shell);
        };
        if let Some(status) = response.get("status").and_then(Value::as_str) {
            shell.status = status.to_string();
        }
        if let Some(output_text) = response.get("output_text").and_then(Value::as_str) {
            shell.output_text = Some(output_text.to_string());
        }
        if let Some(usage) = response.get("usage") {
            shell.usage = serde_json::from_value::<Usage>(usage.clone()).ok();
        }
        if let Some(previous_response_id) =
            response.get("previous_response_id").and_then(Value::as_str)
        {
            shell.previous_response_id = Some(previous_response_id.to_string());
        }
        Ok(shell)
    }

    fn minimal_response(&self, status: &str) -> Result<SseResponseShell, ApiError> {
        let mut response = SseResponseShell::minimal(
            self.response_id
                .clone()
                .ok_or_else(|| ApiError::stream_interrupted_with_details("missing response id"))?,
            self.model.clone().ok_or_else(|| {
                ApiError::stream_interrupted_with_details("missing response model")
            })?,
            self.created_at
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        );
        response.status = status.to_string();
        Ok(response)
    }
}

impl ItemState {
    fn id(&self, prefix: &str) -> String {
        self.id
            .clone()
            .unwrap_or_else(|| format!("{prefix}_{}", self.output_index.unwrap_or(self.order)))
    }

    fn output_index(&self) -> u32 {
        self.output_index.unwrap_or(self.order)
    }

    fn message_output_item(&self) -> ResponseOutputItem {
        ResponseOutputItem::Message {
            id: self.id("msg"),
            status: "completed".to_string(),
            role: self.role.clone().unwrap_or_else(|| "assistant".to_string()),
            content: vec![ContentPart::OutputText {
                text: self.text.clone(),
                annotations: Vec::new(),
            }],
        }
    }

    fn reasoning_output_item(&self) -> ResponseOutputItem {
        ResponseOutputItem::Reasoning {
            id: self.id("rs"),
            content: None,
            summary: Some(vec![SummaryPart::SummaryText {
                text: self.reasoning_text.clone(),
            }]),
            status: None,
        }
    }

    fn function_output_item(&self) -> Result<ResponseOutputItem, ApiError> {
        Ok(ResponseOutputItem::FunctionCall {
            id: self.id("fc"),
            status: "completed".to_string(),
            call_id: self.required_call_id()?,
            name: self.required_name()?,
            arguments: self.required_arguments()?,
        })
    }

    fn custom_tool_output_item(&self) -> Result<ResponseOutputItem, ApiError> {
        Ok(ResponseOutputItem::CustomToolCall {
            id: self.id("ctc"),
            status: "completed".to_string(),
            call_id: self.required_call_id()?,
            name: self.required_name()?,
            input: self.required_custom_input()?,
        })
    }

    fn message_events(&self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let item_id = self.id("msg");
        let output_index = self.output_index();
        Ok(vec![
            ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::Message {
                    id: item_id.clone(),
                    status: "in_progress".to_string(),
                    role: self.role.clone().unwrap_or_else(|| "assistant".to_string()),
                    content: Vec::new(),
                },
            },
            ResponseSseEvent::ResponseContentPartAdded {
                item_id: item_id.clone(),
                output_index,
                content_index: 0,
                part: ContentPart::OutputText {
                    text: String::new(),
                    annotations: Vec::new(),
                },
            },
            ResponseSseEvent::ResponseOutputTextDelta {
                item_id: item_id.clone(),
                output_index,
                content_index: 0,
                delta: self.text.clone(),
            },
            ResponseSseEvent::ResponseOutputTextDone {
                item_id: item_id.clone(),
                output_index,
                content_index: 0,
                text: self.text.clone(),
            },
            ResponseSseEvent::ResponseContentPartDone {
                item_id: item_id.clone(),
                output_index,
                content_index: 0,
                part: ContentPart::OutputText {
                    text: self.text.clone(),
                    annotations: Vec::new(),
                },
            },
            ResponseSseEvent::ResponseOutputItemDone {
                output_index,
                item: self.message_output_item(),
            },
        ])
    }

    fn reasoning_events(&self) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let item_id = self.id("rs");
        let output_index = self.output_index();
        let summary = SummaryPart::SummaryText {
            text: self.reasoning_text.clone(),
        };
        let mut events = vec![
            ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::Reasoning {
                    id: item_id.clone(),
                    content: None,
                    summary: Some(Vec::new()),
                    status: Some("in_progress".to_string()),
                },
            },
            ResponseSseEvent::ResponseReasoningSummaryPartAdded {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                part: SummaryPart::SummaryText {
                    text: String::new(),
                },
            },
        ];
        let deltas = if self.reasoning_deltas.is_empty() {
            vec![self.reasoning_text.clone()]
        } else {
            self.reasoning_deltas.clone()
        };
        for delta in deltas {
            if !delta.is_empty() {
                events.push(ResponseSseEvent::ResponseReasoningSummaryTextDelta {
                    item_id: item_id.clone(),
                    output_index,
                    summary_index: 0,
                    delta,
                });
            }
        }
        events.extend([
            ResponseSseEvent::ResponseReasoningSummaryTextDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                text: self.reasoning_text.clone(),
            },
            ResponseSseEvent::ResponseReasoningSummaryPartDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                part: summary,
            },
            ResponseSseEvent::ResponseOutputItemDone {
                output_index,
                item: self.reasoning_output_item(),
            },
        ]);
        Ok(events)
    }

    fn function_events(
        &self,
        item: &ResponseOutputItem,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let item_id = self.id("fc");
        let output_index = self.output_index();
        Ok(vec![
            ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::FunctionCall {
                    id: item_id.clone(),
                    status: "in_progress".to_string(),
                    call_id: self.required_call_id()?,
                    name: self.required_name()?,
                    arguments: String::new(),
                },
            },
            ResponseSseEvent::ResponseFunctionCallArgumentsDelta {
                item_id: item_id.clone(),
                output_index,
                delta: self.required_arguments()?,
            },
            ResponseSseEvent::ResponseFunctionCallArgumentsDone {
                item_id: item_id.clone(),
                output_index,
                arguments: self.required_arguments()?,
                name: self.name.clone(),
            },
            ResponseSseEvent::ResponseOutputItemDone {
                output_index,
                item: item.clone(),
            },
        ])
    }

    fn custom_tool_events(
        &self,
        item: &ResponseOutputItem,
    ) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let item_id = self.id("ctc");
        let output_index = self.output_index();
        let input = self.required_custom_input()?;
        Ok(vec![
            ResponseSseEvent::ResponseOutputItemAdded {
                output_index,
                item: ResponseOutputItem::CustomToolCall {
                    id: item_id.clone(),
                    status: "in_progress".to_string(),
                    call_id: self.required_call_id()?,
                    name: self.required_name()?,
                    input: String::new(),
                },
            },
            ResponseSseEvent::ResponseCustomToolCallInputDelta {
                item_id: item_id.clone(),
                output_index,
                delta: input.clone(),
            },
            ResponseSseEvent::ResponseCustomToolCallInputDone {
                item_id: item_id.clone(),
                output_index,
                input,
            },
            ResponseSseEvent::ResponseOutputItemDone {
                output_index,
                item: item.clone(),
            },
        ])
    }

    fn required_call_id(&self) -> Result<String, ApiError> {
        self.call_id.clone().ok_or_else(|| {
            ApiError::stream_interrupted_with_details(format!(
                "missing call_id for Responses output item {}",
                self.key
            ))
        })
    }

    fn required_name(&self) -> Result<String, ApiError> {
        self.name.clone().ok_or_else(|| {
            ApiError::stream_interrupted_with_details(format!(
                "missing name for Responses output item {}",
                self.key
            ))
        })
    }

    fn required_arguments(&self) -> Result<String, ApiError> {
        if self.arguments_seen {
            Ok(self.arguments.clone())
        } else {
            Err(ApiError::stream_interrupted_with_details(format!(
                "missing arguments for Responses function call {}",
                self.key
            )))
        }
    }

    fn required_custom_input(&self) -> Result<String, ApiError> {
        if self.custom_input_seen {
            Ok(self.custom_input.clone())
        } else {
            Err(ApiError::stream_interrupted_with_details(format!(
                "missing input for Responses custom tool call {}",
                self.key
            )))
        }
    }
}

fn item_key(event: &Value, kind: Option<&ItemKind>) -> String {
    if let Some(id) = event
        .get("item")
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .or_else(|| event.get("item_id").and_then(Value::as_str))
    {
        return id.to_string();
    }
    if let Some(output_index) = event.get("output_index").and_then(Value::as_u64) {
        return format!("out:{output_index}");
    }
    let prefix = match kind {
        Some(ItemKind::Message) => "message",
        Some(ItemKind::Reasoning) => "reasoning",
        Some(ItemKind::FunctionCall) => "function",
        Some(ItemKind::CustomToolCall) => "custom",
        Some(ItemKind::Other(value)) if !value.is_empty() => value.as_str(),
        _ => "item",
    };
    format!("{prefix}:implicit")
}

fn item_kind(item: &Value) -> Option<ItemKind> {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => Some(ItemKind::Message),
        Some("reasoning") => Some(ItemKind::Reasoning),
        Some("function_call") => Some(ItemKind::FunctionCall),
        Some("custom_tool_call") => Some(ItemKind::CustomToolCall),
        Some(other) => Some(ItemKind::Other(other.to_string())),
        None => None,
    }
}

fn apply_done_item(state: &mut ItemState) -> Result<(), ApiError> {
    let Some(item) = state.done_item.clone() else {
        return Ok(());
    };
    if let Some(kind) = item_kind(&item) {
        state.kind = kind;
    }
    if let Some(id) = item.get("id").and_then(Value::as_str) {
        state.id = Some(id.to_string());
    }
    if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
        state.call_id = Some(call_id.to_string());
    }
    if let Some(name) = item.get("name").and_then(Value::as_str) {
        state.name = Some(name.to_string());
    }
    if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
        state.arguments = arguments.to_string();
        state.arguments_seen = true;
    }
    if let Some(input) = item.get("input").and_then(Value::as_str) {
        state.custom_input = input.to_string();
        state.custom_input_seen = true;
    }
    if let Some(text) = message_item_text(&item) {
        state.text = text;
        state.text_seen = true;
    }
    if let Some(text) = reasoning_item_text(&item) {
        state.reasoning_text = text.clone();
        if state.reasoning_deltas.is_empty() && !text.is_empty() {
            state.reasoning_deltas.push(text);
        }
    }
    Ok(())
}

fn message_item_text(item: &Value) -> Option<String> {
    let content = item.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter_map(content_part_text)
        .collect::<Vec<_>>()
        .join("");
    Some(text)
}

fn reasoning_item_text(item: &Value) -> Option<String> {
    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
        let text = summary
            .iter()
            .filter_map(summary_part_text)
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return Some(text);
        }
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        let text = content
            .iter()
            .filter_map(content_part_text)
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

fn content_part_text(part: &Value) -> Option<String> {
    match part.get("type").and_then(Value::as_str) {
        Some("output_text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        Some("refusal") => part
            .get("refusal")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

fn summary_part_text(part: &Value) -> Option<String> {
    match part.get("type").and_then(Value::as_str) {
        Some("summary_text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

fn output_text_from_items(items: &[ResponseOutputItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseOutputItem::Message { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::OutputText { text, .. } => Some(text.as_str()),
                        ContentPart::Refusal { refusal } => Some(refusal.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events: Vec<Value>) -> Result<Vec<ResponseSseEvent>, ApiError> {
        let mut canonicalizer =
            ResponsesCanonicalizer::new("resp_test".into(), "test-model".into(), 1);
        let mut out = Vec::new();
        for event in events {
            out.extend(canonicalizer.process_event(&event)?.events);
        }
        out.extend(canonicalizer.finish_stream()?);
        Ok(out)
    }

    fn event_types(events: &[ResponseSseEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                ResponseSseEvent::ResponseCreated { .. } => "response.created",
                ResponseSseEvent::ResponseInProgress { .. } => "response.in_progress",
                ResponseSseEvent::ResponseOutputItemAdded { item, .. } => match item {
                    ResponseOutputItem::Reasoning { .. } => "added.reasoning",
                    ResponseOutputItem::Message { .. } => "added.message",
                    ResponseOutputItem::FunctionCall { .. } => "added.function",
                    ResponseOutputItem::CustomToolCall { .. } => "added.custom",
                    _ => "added.other",
                },
                ResponseSseEvent::ResponseReasoningSummaryTextDelta { .. } => "reasoning.delta",
                ResponseSseEvent::ResponseOutputTextDelta { .. } => "text.delta",
                ResponseSseEvent::ResponseFunctionCallArgumentsDone { .. } => "function.done",
                ResponseSseEvent::ResponseOutputItemDone { item, .. } => match item {
                    ResponseOutputItem::Reasoning { .. } => "done.reasoning",
                    ResponseOutputItem::Message { .. } => "done.message",
                    ResponseOutputItem::FunctionCall { .. } => "done.function",
                    ResponseOutputItem::CustomToolCall { .. } => "done.custom",
                    _ => "done.other",
                },
                ResponseSseEvent::ResponseCompleted { .. } => "response.completed",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn canonicalizes_interleaved_fireworks_like_stream() {
        let events = run(vec![
            json!({"type":"response.created","response":{"id":"resp_fw","object":"response","created_at":1,"status":"in_progress","model":"m"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","status":"in_progress","summary":[]}}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"summary_index":0,"delta":"Need files."}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":0,"delta":"\n\n"}),
            json!({"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"in_progress","call_id":"call_1","name":"exec_command","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":2,"delta":"{\"cmd\":\"ls\"}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"Need files."}]}}),
            json!({"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":2,"arguments":"{\"cmd\":\"ls\"}"}),
            json!({"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"completed","call_id":"call_1","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"}}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"\n\n","annotations":[]}]}}),
            json!({"type":"response.completed","response":{"id":"resp_fw","object":"response","created_at":1,"status":"completed","model":"m","usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
        ]).expect("canonicalized");

        let types = event_types(&events);
        let reasoning_done = types.iter().position(|t| *t == "done.reasoning").unwrap();
        let function_added = types.iter().position(|t| *t == "added.function").unwrap();
        assert!(reasoning_done < function_added);
        assert!(!types.contains(&"done.message"));
        assert!(types.contains(&"reasoning.delta"));
        assert_eq!(types.last().copied(), Some("response.completed"));
    }

    #[test]
    fn function_arguments_done_does_not_require_name() {
        let events = run(vec![
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"in_progress","call_id":"call_1","name":"tool","arguments":""}}),
            json!({"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":"{}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"completed","call_id":"call_1","name":"tool","arguments":"{}"}}),
            json!({"type":"response.completed","response":{"id":"resp","object":"response","created_at":1,"status":"completed","model":"m"}}),
        ])
        .expect("canonicalized");
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseSseEvent::ResponseFunctionCallArgumentsDone { name: Some(name), .. } if name == "tool"
        )));
    }

    #[test]
    fn fails_when_stream_ends_without_completed() {
        let err = run(vec![json!({"type":"response.created","response":{"id":"resp","object":"response","created_at":1,"status":"in_progress","model":"m"}})])
            .expect_err("missing completed should fail");
        assert_eq!(err.error.error_type, "stream_interrupted");
    }
}
