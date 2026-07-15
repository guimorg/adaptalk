//! Adapt's protocol-independent display contract and its protocol translator.

use anyhow::Result;
use rmcp::model::RawContent;

use crate::adapt_client::QueryResponse;

/// One textual part of an Adapt response, ready for terminal display.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    pub text: String,
}

/// The application-owned representation of a remote response.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptResponse {
    pub text_blocks: Vec<TextBlock>,
    /// A deliberately opaque, already-redacted result for inline display.
    pub structured_result: Option<serde_json::Value>,
    pub remote_chat_id: Option<String>,
}

impl TranscriptResponse {
    pub fn display_value(&self) -> serde_json::Value {
        serde_json::json!({
            "text_blocks": self.text_blocks.iter().map(|block| &block.text).collect::<Vec<_>>(),
            "structured_result": self.structured_result,
            "remote_chat_id": self.remote_chat_id,
        })
    }
}

/// Preserve text for the terminal and, when present, one opaque structured result.
/// Protocol-specific fields such as citations deliberately do not escape this boundary.
pub fn from_query_response(response: QueryResponse) -> Result<TranscriptResponse> {
    let text_blocks = response
        .content
        .into_iter()
        .map(|content| match content.raw {
            RawContent::Text(text) => Ok(TextBlock { text: text.text }),
            _ => Ok(TextBlock {
                text: serde_json::to_string(&content)?,
            }),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TranscriptResponse {
        text_blocks,
        structured_result: response.structured_content,
        remote_chat_id: response.chat_id,
    })
}

#[cfg(test)]
mod tests {
    use super::from_query_response;
    use crate::adapt_client::QueryResponse;
    use rmcp::model::{Content, RawContent};

    #[test]
    fn keeps_one_opaque_structured_result_without_extracting_protocol_fields() {
        let transcript = from_query_response(QueryResponse {
            content: vec![Content::new(RawContent::text("hello"), None)],
            structured_content: Some(serde_json::json!({"citations": ["source"]})),
            chat_id: Some("chat-1".into()),
        })
        .unwrap();
        assert_eq!(transcript.text_blocks[0].text, "hello");
        assert_eq!(
            transcript.structured_result,
            Some(serde_json::json!({"citations": ["source"]}))
        );
        assert_eq!(transcript.remote_chat_id.as_deref(), Some("chat-1"));
    }
}
