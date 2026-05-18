use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::StreamExt;

use protocol::sse::ResponseSseEvent;

/// Convert a stream of ResponseSseEvent into an axum SSE response.
pub fn sse_response(
    event_stream: impl Stream<Item = Result<ResponseSseEvent, Infallible>> + Send + 'static,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + Send> {
    let sse_stream = event_stream.map(|result| {
        let event = match result {
            Ok(event) => event,
            Err(_infallible) => unreachable!(),
        };

        let event_type = sse_event_type(&event);
        let data = serde_json::to_string(&event).unwrap_or_default();

        Ok(Event::default().event(event_type).data(data))
    });

    Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn sse_event_type(event: &ResponseSseEvent) -> &str {
    match event {
        ResponseSseEvent::ResponseCreated { .. } => "response.created",
        ResponseSseEvent::ResponseInProgress { .. } => "response.in_progress",
        ResponseSseEvent::ResponseOutputItemAdded { .. } => "response.output_item.added",
        ResponseSseEvent::ResponseContentPartAdded { .. } => "response.content_part.added",
        ResponseSseEvent::ResponseOutputTextDelta { .. } => "response.output_text.delta",
        ResponseSseEvent::ResponseOutputTextDone { .. } => "response.output_text.done",
        ResponseSseEvent::ResponseContentPartDone { .. } => "response.content_part.done",
        ResponseSseEvent::ResponseOutputItemDone { .. } => "response.output_item.done",
        ResponseSseEvent::ResponseFunctionCallArgumentsDelta { .. } => {
            "response.function_call_arguments.delta"
        }
        ResponseSseEvent::ResponseFunctionCallArgumentsDone { .. } => {
            "response.function_call_arguments.done"
        }
        ResponseSseEvent::ResponseCustomToolCallInputDelta { .. } => {
            "response.custom_tool_call_input.delta"
        }
        ResponseSseEvent::ResponseCompleted { .. } => "response.completed",
        ResponseSseEvent::ResponseFailed { .. } => "response.failed",
        ResponseSseEvent::Error { .. } => "error",
    }
}
