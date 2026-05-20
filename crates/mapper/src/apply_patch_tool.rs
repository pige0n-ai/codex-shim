use protocol::error::ApiError;
use serde::Deserialize;
use serde_json::Value;

pub const APPLY_PATCH_TOOL_NAME: &str = "apply_patch";
pub const APPLY_PATCH_UPSTREAM_FREEFORM: &str = "freeform";
pub const APPLY_PATCH_UPSTREAM_STRUCTURED: &str = "structured";

pub fn structured_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "hunks": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "oneOf": [
                        { "$ref": "#/$defs/add_hunk" },
                        { "$ref": "#/$defs/delete_hunk" },
                        { "$ref": "#/$defs/update_hunk" }
                    ]
                }
            },
            "raw_patch": {
                "type": "string",
                "description": "Optional escape hatch. If present, it must be a complete Codex apply_patch payload including Begin Patch and End Patch markers. Prefer hunks."
            }
        },
        "additionalProperties": false,
        "$defs": {
            "add_hunk": {
                "type": "object",
                "properties": {
                    "kind": { "const": "add" },
                    "path": { "type": "string", "minLength": 1 },
                    "lines": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string" }
                    }
                },
                "required": ["kind", "path", "lines"],
                "additionalProperties": false
            },
            "delete_hunk": {
                "type": "object",
                "properties": {
                    "kind": { "const": "delete" },
                    "path": { "type": "string", "minLength": 1 }
                },
                "required": ["kind", "path"],
                "additionalProperties": false
            },
            "update_hunk": {
                "type": "object",
                "properties": {
                    "kind": { "const": "update" },
                    "path": { "type": "string", "minLength": 1 },
                    "move_to": { "type": ["string", "null"] },
                    "changes": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/change" }
                    }
                },
                "required": ["kind", "path", "changes"],
                "additionalProperties": false
            },
            "change": {
                "type": "object",
                "properties": {
                    "anchor": {
                        "type": ["string", "null"],
                        "description": "Literal text after @@. Use null for bare @@. This is not a line range."
                    },
                    "lines": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/change_line" }
                    },
                    "end_of_file": {
                        "type": "boolean",
                        "description": "Only true when this change must match at the physical end of the file."
                    }
                },
                "required": ["anchor", "lines"],
                "additionalProperties": false
            },
            "change_line": {
                "type": "object",
                "properties": {
                    "op": { "enum": ["context", "add", "remove"] },
                    "text": { "type": "string" }
                },
                "required": ["op", "text"],
                "additionalProperties": false
            }
        }
    })
}

pub fn structured_description(original_description: &str) -> String {
    format!(
        "{original_description}\n\n\
Chat adapter contract: this upstream tool uses structured JSON. The shim will compile the JSON AST into the raw Codex apply_patch payload before returning it to Codex.\n\n\
Use `hunks` for normal edits. Do not include `*** Begin Patch`, `*** End Patch`, line-prefix characters, or unified diff headers in the JSON fields; the shim writes those markers. \
For update changes, each line object uses `op`: `context`, `remove`, or `add`. `anchor` is literal text after `@@`, not a line range. Set `end_of_file` only when the change must match the physical end of the file."
    )
}

pub fn structured_arguments_from_patch_input(input: &str) -> String {
    match parse_patch_to_ast(input) {
        Ok(value) => value.to_string(),
        Err(_) => serde_json::json!({ "raw_patch": input }).to_string(),
    }
}

pub fn structured_arguments_to_patch(arguments: &str) -> Result<String, ApiError> {
    let value: Value = serde_json::from_str(arguments).map_err(|error| {
        ApiError::upstream_error(format!(
            "apply_patch returned invalid structured arguments: {error}"
        ))
    })?;
    if let Some(raw_patch) = value.get("raw_patch").and_then(Value::as_str) {
        if value.get("hunks").is_some() {
            return Err(ApiError::upstream_error(
                "apply_patch structured arguments must not include both 'hunks' and 'raw_patch'",
            ));
        }
        return Ok(raw_patch.to_string());
    }
    let patch: StructuredPatch = serde_json::from_value(value).map_err(|error| {
        ApiError::upstream_error(format!(
            "apply_patch returned invalid structured arguments: {error}"
        ))
    })?;
    compile_structured_patch(&patch)
}

#[derive(Debug, Deserialize)]
struct StructuredPatch {
    hunks: Vec<Hunk>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum Hunk {
    #[serde(rename = "add")]
    Add { path: String, lines: Vec<String> },
    #[serde(rename = "delete")]
    Delete { path: String },
    #[serde(rename = "update")]
    Update {
        path: String,
        #[serde(default)]
        move_to: Option<String>,
        changes: Vec<Change>,
    },
}

#[derive(Debug, Deserialize)]
struct Change {
    anchor: Option<String>,
    lines: Vec<ChangeLine>,
    #[serde(default)]
    end_of_file: bool,
}

#[derive(Debug, Deserialize)]
struct ChangeLine {
    op: ChangeOp,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChangeOp {
    Context,
    Add,
    Remove,
}

fn compile_structured_patch(patch: &StructuredPatch) -> Result<String, ApiError> {
    if patch.hunks.is_empty() {
        return Err(ApiError::upstream_error(
            "apply_patch structured arguments require at least one hunk",
        ));
    }
    let mut out = String::from("*** Begin Patch\n");
    for hunk in &patch.hunks {
        match hunk {
            Hunk::Add { path, lines } => {
                validate_path(path)?;
                if lines.is_empty() {
                    return Err(ApiError::upstream_error(
                        "apply_patch add hunk requires at least one line",
                    ));
                }
                out.push_str(&format!("*** Add File: {path}\n"));
                for line in lines {
                    out.push('+');
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Hunk::Delete { path } => {
                validate_path(path)?;
                out.push_str(&format!("*** Delete File: {path}\n"));
            }
            Hunk::Update {
                path,
                move_to,
                changes,
            } => {
                validate_path(path)?;
                if changes.is_empty() {
                    return Err(ApiError::upstream_error(
                        "apply_patch update hunk requires at least one change",
                    ));
                }
                out.push_str(&format!("*** Update File: {path}\n"));
                if let Some(move_to) = move_to {
                    validate_path(move_to)?;
                    out.push_str(&format!("*** Move to: {move_to}\n"));
                }
                for change in changes {
                    if change.lines.is_empty() {
                        return Err(ApiError::upstream_error(
                            "apply_patch update change requires at least one line",
                        ));
                    }
                    if let Some(anchor) = &change.anchor {
                        out.push_str("@@ ");
                        out.push_str(anchor);
                        out.push('\n');
                    } else {
                        out.push_str("@@\n");
                    }
                    for line in &change.lines {
                        match line.op {
                            ChangeOp::Context => out.push(' '),
                            ChangeOp::Add => out.push('+'),
                            ChangeOp::Remove => out.push('-'),
                        }
                        out.push_str(&line.text);
                        out.push('\n');
                    }
                    if change.end_of_file {
                        out.push_str("*** End of File\n");
                    }
                }
            }
        }
    }
    out.push_str("*** End Patch");
    Ok(out)
}

fn validate_path(path: &str) -> Result<(), ApiError> {
    if path.trim().is_empty() || path.contains('\n') {
        return Err(ApiError::upstream_error(
            "apply_patch paths must be non-empty single-line strings",
        ));
    }
    Ok(())
}

fn parse_patch_to_ast(input: &str) -> Result<Value, ()> {
    let mut lines: Vec<&str> = input.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err(());
    }
    let mut i = 1usize;
    let end = lines.len() - 1;
    let mut hunks = Vec::new();
    while i < end {
        let line = lines[i];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut add_lines = Vec::new();
            while i < end && !lines[i].starts_with("*** ") {
                let Some(text) = lines[i].strip_prefix('+') else {
                    return Err(());
                };
                add_lines.push(Value::String(text.to_string()));
                i += 1;
            }
            hunks.push(serde_json::json!({"kind": "add", "path": path, "lines": add_lines}));
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            hunks.push(serde_json::json!({"kind": "delete", "path": path}));
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = Value::Null;
            if i < end
                && let Some(dest) = lines[i].strip_prefix("*** Move to: ")
            {
                move_to = Value::String(dest.to_string());
                i += 1;
            }
            let mut changes = Vec::new();
            while i < end && !lines[i].starts_with("*** ") {
                let anchor = if lines[i] == "@@" {
                    Value::Null
                } else if let Some(anchor) = lines[i].strip_prefix("@@ ") {
                    Value::String(anchor.to_string())
                } else {
                    return Err(());
                };
                i += 1;
                let mut change_lines = Vec::new();
                let mut end_of_file = false;
                while i < end
                    && !lines[i].starts_with("@@")
                    && !lines[i].starts_with("*** Add File: ")
                    && !lines[i].starts_with("*** Delete File: ")
                    && !lines[i].starts_with("*** Update File: ")
                {
                    if lines[i] == "*** End of File" {
                        end_of_file = true;
                        i += 1;
                        break;
                    }
                    let (op, text) = match lines[i].chars().next() {
                        Some(' ') => ("context", &lines[i][1..]),
                        Some('+') => ("add", &lines[i][1..]),
                        Some('-') => ("remove", &lines[i][1..]),
                        _ => return Err(()),
                    };
                    change_lines.push(serde_json::json!({"op": op, "text": text}));
                    i += 1;
                }
                changes.push(serde_json::json!({
                    "anchor": anchor,
                    "lines": change_lines,
                    "end_of_file": end_of_file
                }));
            }
            hunks.push(serde_json::json!({
                "kind": "update",
                "path": path,
                "move_to": move_to,
                "changes": changes
            }));
        } else {
            return Err(());
        }
    }
    Ok(serde_json::json!({ "hunks": hunks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_structured_patch_to_raw_patch() {
        let args = serde_json::json!({
            "hunks": [
                {"kind": "add", "path": "a.txt", "lines": ["one", "two"]},
                {"kind": "update", "path": "b.txt", "move_to": null, "changes": [{
                    "anchor": null,
                    "lines": [
                        {"op": "context", "text": "fn main() {"},
                        {"op": "remove", "text": "    old();"},
                        {"op": "add", "text": "    new();"}
                    ],
                    "end_of_file": false
                }]}
            ]
        });
        let patch = structured_arguments_to_patch(&args.to_string()).unwrap();
        assert!(patch.contains("*** Begin Patch\n*** Add File: a.txt\n+one\n+two\n"));
        assert!(
            patch.contains("*** Update File: b.txt\n@@\n fn main() {\n-    old();\n+    new();\n")
        );
        assert!(patch.ends_with("*** End Patch"));
    }

    #[test]
    fn converts_raw_patch_history_to_structured_arguments() {
        let raw = "*** Begin Patch\n*** Update File: a.txt\n@@ anchor\n old\n-new\n+newer\n*** End of File\n*** End Patch";
        let value: Value = serde_json::from_str(&structured_arguments_from_patch_input(raw))
            .expect("structured json");
        assert_eq!(value["hunks"][0]["kind"], "update");
        assert_eq!(value["hunks"][0]["changes"][0]["anchor"], "anchor");
        assert_eq!(value["hunks"][0]["changes"][0]["end_of_file"], true);
    }
}
