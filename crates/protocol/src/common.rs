use serde::{Deserialize, Serialize};

/// Token usage statistics.
///
/// Supports both Chat Completions naming (prompt_tokens/completion_tokens)
/// and Responses naming (input_tokens/output_tokens) via serde aliases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(alias = "prompt_tokens")]
    pub input_tokens: u32,
    #[serde(alias = "completion_tokens")]
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "output_text")]
    OutputText {
        text: String,
        #[serde(default)]
        annotations: Vec<Annotation>,
    },
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
    #[serde(rename = "input_image")]
    InputImage { image_url: ImageUrl },
    /// Catch-all for unknown content part types.
    #[serde(other)]
    UnknownContentPart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageUrl {
    /// Codex format: "image_url": "https://..."
    Plain(String),
    /// OpenAI format: "image_url": {"url": "https://...", "detail": "high"} (Codex never produces this; kept for non-Codex client compatibility)
    Detailed {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl ImageUrl {
    pub fn url(&self) -> &str {
        match self {
            ImageUrl::Plain(s) => s,
            ImageUrl::Detailed { url, .. } => url,
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            ImageUrl::Plain(_) => None,
            ImageUrl::Detailed { detail, .. } => detail.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Annotation {
    #[serde(rename = "file_citation")]
    FileCitation {
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        file_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
    #[serde(rename = "url_citation")]
    UrlCitation {
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "file_path")]
    FilePath {
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        file_id: String,
    },
}
