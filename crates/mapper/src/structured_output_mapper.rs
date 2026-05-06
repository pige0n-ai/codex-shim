use protocol::responses::{TextConfig, TextFormat};

/// Map Responses `text.format` → Chat `response_format`.
pub fn map_text_format(text: Option<&TextConfig>) -> Option<serde_json::Value> {
    let text = text?;
    match &text.format {
        TextFormat::Text => None,
        TextFormat::JsonObject => Some(serde_json::json!({"type": "json_object"})),
        TextFormat::JsonSchema {
            name,
            description,
            schema_,
            strict,
        } => {
            let mut fmt = serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": name,
                    "schema": schema_,
                }
            });
            if let Some(desc) = description {
                fmt["json_schema"]["description"] = serde_json::Value::String(desc.clone());
            }
            if let Some(s) = strict {
                fmt["json_schema"]["strict"] = serde_json::Value::Bool(*s);
            }
            Some(fmt)
        }
    }
}
