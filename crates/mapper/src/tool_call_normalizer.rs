use protocol::chat::{ChatFunctionCall, ChatToolCall};
use serde_json::Value;

pub fn normalize_chat_tool_calls(tool_calls: &[ChatToolCall]) -> Vec<ChatToolCall> {
    let mut normalized = Vec::new();
    for tool_call in tool_calls {
        if let Some(arguments) = split_concatenated_json_objects(&tool_call.function.arguments) {
            for (idx, arguments) in arguments.into_iter().enumerate() {
                normalized.push(ChatToolCall {
                    id: suffixed_call_id(&tool_call.id, idx),
                    call_type: tool_call.call_type.clone(),
                    function: ChatFunctionCall {
                        name: tool_call.function.name.clone(),
                        arguments,
                    },
                });
            }
        } else {
            normalized.push(tool_call.clone());
        }
    }
    normalized
}

pub fn split_concatenated_json_objects(arguments: &str) -> Option<Vec<String>> {
    if serde_json::from_str::<Value>(arguments).is_ok() {
        return None;
    }

    let stream = serde_json::Deserializer::from_str(arguments).into_iter::<Value>();
    let mut values = Vec::new();
    for value in stream {
        let value = value.ok()?;
        if !value.is_object() {
            return None;
        }
        values.push(value);
    }

    if values.len() < 2 {
        return None;
    }

    values
        .into_iter()
        .map(|value| serde_json::to_string(&value).ok())
        .collect()
}

fn suffixed_call_id(call_id: &str, idx: usize) -> String {
    if idx == 0 {
        call_id.to_string()
    } else {
        format!("{call_id}_{idx}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_concatenated_json_object_arguments() {
        let parts =
            split_concatenated_json_objects(r#"{"cmd":"a"}{"cmd":"b"}"#).expect("split args");
        assert_eq!(parts, vec![r#"{"cmd":"a"}"#, r#"{"cmd":"b"}"#]);
    }

    #[test]
    fn leaves_valid_json_arguments_untouched() {
        assert!(split_concatenated_json_objects(r#"{"cmd":"a"}"#).is_none());
    }

    #[test]
    fn rejects_non_object_json_sequences() {
        assert!(split_concatenated_json_objects(r#"{"cmd":"a"}["b"]"#).is_none());
    }

    #[test]
    fn normalizes_single_bad_tool_call_into_multiple_calls() {
        let tool_calls = vec![ChatToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: ChatFunctionCall {
                name: Some("exec_command".into()),
                arguments: r#"{"cmd":"a"}{"cmd":"b"}"#.into(),
            },
        }];

        let normalized = normalize_chat_tool_calls(&tool_calls);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].id, "call_1");
        assert_eq!(normalized[1].id, "call_1_1");
        assert_eq!(normalized[0].function.arguments, r#"{"cmd":"a"}"#);
        assert_eq!(normalized[1].function.arguments, r#"{"cmd":"b"}"#);
    }
}
