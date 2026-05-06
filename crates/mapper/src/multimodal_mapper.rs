use protocol::chat::ChatContent;
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{MessageContent, ResponseInput};

/// Check if the input contains multimodal content (images).
/// Returns Ok if valid, Err if unsupported file types are found.
pub fn validate_multimodal_input(input: &ResponseInput) -> Result<bool, ApiError> {
    match input {
        ResponseInput::Text(_) | ResponseInput::Value(_) => Ok(false),
        ResponseInput::Items(items) => {
            let mut has_image = false;
            for item in items {
                if let protocol::responses::InputItem::Message { content, .. } = item
                    && let MessageContent::Parts(parts) = content
                {
                    for part in parts {
                        if let ContentPart::InputImage { .. } = part {
                            has_image = true;
                        }
                    }
                }
            }
            Ok(has_image)
        }
    }
}

/// Map a Responses message's content parts to a Chat content, handling images.
pub fn map_response_content_to_chat(parts: &[ContentPart]) -> ChatContent {
    if parts.len() == 1 {
        match &parts[0] {
            ContentPart::OutputText { text, .. } => {
                return ChatContent::Text(text.clone());
            }
            ContentPart::InputImage { image_url } => {
                return ChatContent::Parts(vec![protocol::chat::ChatContentPart::ImageUrl {
                    image_url: protocol::chat::ChatImageUrl {
                        url: image_url.url().to_string(),
                        detail: image_url.detail().map(String::from),
                    },
                }]);
            }
            _ => {}
        }
    }

    let chat_parts: Vec<protocol::chat::ChatContentPart> = parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::OutputText { text, .. } => {
                Some(protocol::chat::ChatContentPart::Text { text: text.clone() })
            }
            ContentPart::InputImage { image_url } => {
                Some(protocol::chat::ChatContentPart::ImageUrl {
                    image_url: protocol::chat::ChatImageUrl {
                        url: image_url.url().to_string(),
                        detail: image_url.detail().map(String::from),
                    },
                })
            }
            _ => None,
        })
        .collect();

    if chat_parts.is_empty() {
        ChatContent::Text(String::new())
    } else {
        ChatContent::Parts(chat_parts)
    }
}
