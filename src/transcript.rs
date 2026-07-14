//! Translate Adapt's protocol response into the application's display contract.

use anyhow::Result;
use rmcp::model::RawContent;

use crate::{
    adapt_client::QueryResponse,
    session_history::{TextBlock, TranscriptResponse},
};

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
