use std::collections::BTreeSet;

use protocol::error::ApiError;
use protocol::responses::{CustomToolFormat, ResponseTool};
use serde_json::Value;

use crate::apply_patch_tool;

pub const CUSTOM_TOOL_INPUT_PROPERTY: &str = "input";

pub fn custom_tool_names(tools: &[ResponseTool]) -> BTreeSet<String> {
    tools
        .iter()
        .filter_map(|tool| match tool {
            ResponseTool::Custom { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

pub fn custom_tool_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            CUSTOM_TOOL_INPUT_PROPERTY: {
                "type": "string",
                "description": "Raw freeform tool input. Do not wrap or transform the tool payload inside this string."
            }
        },
        "required": [CUSTOM_TOOL_INPUT_PROPERTY],
        "additionalProperties": false
    })
}

pub fn custom_tool_description(description: &str, format: &CustomToolFormat) -> String {
    format!(
        "{description}\n\n\
Chat adapter contract: call this function with a JSON object containing exactly one string field named `{CUSTOM_TOOL_INPUT_PROPERTY}`. \
The `{CUSTOM_TOOL_INPUT_PROPERTY}` value must be the raw freeform tool input; do not wrap the freeform payload in JSON inside that string.\n\n\
Freeform format:\n\
- type: {format_type}\n\
- syntax: {syntax}\n\
- definition:\n{definition}",
        format_type = format.format_type,
        syntax = format.syntax,
        definition = format.definition
    )
}

pub fn custom_tool_input_from_arguments(name: &str, arguments: &str) -> Result<String, ApiError> {
    if name == apply_patch_tool::APPLY_PATCH_TOOL_NAME {
        return Ok(apply_patch_tool::native_input_from_chat_arguments(
            arguments,
        ));
    }

    let value: Value = serde_json::from_str(arguments).map_err(|error| {
        ApiError::upstream_error(format!(
            "custom tool '{name}' returned invalid arguments: expected JSON object with string field '{CUSTOM_TOOL_INPUT_PROPERTY}': {error}"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        ApiError::upstream_error(format!(
            "custom tool '{name}' returned invalid arguments: expected JSON object with string field '{CUSTOM_TOOL_INPUT_PROPERTY}'"
        ))
    })?;
    if object.len() != 1 || !object.contains_key(CUSTOM_TOOL_INPUT_PROPERTY) {
        return Err(ApiError::upstream_error(format!(
            "custom tool '{name}' returned invalid arguments: expected exactly one string field '{CUSTOM_TOOL_INPUT_PROPERTY}'"
        )));
    }
    object
        .get(CUSTOM_TOOL_INPUT_PROPERTY)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::upstream_error(format!(
                "custom tool '{name}' returned invalid arguments: field '{CUSTOM_TOOL_INPUT_PROPERTY}' must be a string"
            ))
        })
}

pub fn custom_tool_arguments(input: &str) -> String {
    serde_json::json!({ CUSTOM_TOOL_INPUT_PROPERTY: input }).to_string()
}

pub fn custom_tool_arguments_for_upstream(
    name: &str,
    input: &str,
    config: &crate::MappingConfig,
) -> String {
    if name == apply_patch_tool::APPLY_PATCH_TOOL_NAME
        && config.apply_patch_upstream_tool_type
            == apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
    {
        apply_patch_tool::structured_arguments_from_patch_input(input)
    } else {
        custom_tool_arguments(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_patch_accepts_valid_input_field() {
        assert_eq!(
            custom_tool_input_from_arguments("apply_patch", r#"{"input":"patch"}"#).unwrap(),
            "patch"
        );
        assert_eq!(
            custom_tool_input_from_arguments("apply_patch", r#"{"input":"patch","extra":1}"#)
                .unwrap(),
            r#"{"input":"patch","extra":1}"#
        );
        assert!(
            custom_tool_input_from_arguments("apply_patch", r#"{"input":1}"#)
                .unwrap()
                .contains(r#""input":1"#)
        );
    }

    #[test]
    fn apply_patch_preserves_invalid_arguments_as_native_input() {
        assert_eq!(
            custom_tool_input_from_arguments("apply_patch", "[not json").unwrap(),
            "[not json"
        );
        assert_eq!(
            custom_tool_input_from_arguments("apply_patch", r#"{"input":"patch","extra":1}"#)
                .unwrap(),
            r#"{"input":"patch","extra":1}"#
        );
        assert_eq!(
            custom_tool_input_from_arguments("apply_patch", r#"{"hunks":[{"path":"a"}]}"#).unwrap(),
            r#"{"hunks":[{"path":"a"}]}"#
        );
    }

    #[test]
    fn non_apply_patch_custom_tools_remain_strict() {
        assert!(
            custom_tool_input_from_arguments("custom_editor", r#"{"input":1}"#)
                .unwrap_err()
                .to_string()
                .contains("must be a string")
        );
    }
}
