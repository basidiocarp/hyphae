const PREAMBLE_VERSION: &str = "1";

/// Applies an idempotent attribute-extraction instruction preamble to a messages vector.
/// The preamble is guarded by a sentinel `[hyphae-attr-extraction-v{version}]` to prevent
/// double-insertion if called multiple times.
fn apply_attribute_extraction_preamble(messages: &mut [serde_json::Value], version: &str) {
    let sentinel = format!("[hyphae-attr-extraction-v{version}]");

    // Check if preamble already exists in any message.
    for msg in messages.iter() {
        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            if content.contains(&sentinel) {
                return; // Already applied, idempotent no-op.
            }
        }
    }

    // If the first message is a user message, prepend the sentinel and preamble.
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("user") {
            // Take an owned copy of the existing content from an immutable borrow so
            // the borrow ends before we mutate, then assign through a single get_mut.
            if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                let new_content = format!("{sentinel}\n{content}");
                if let Some(slot) = first.get_mut("content") {
                    *slot = serde_json::json!(new_content);
                }
            }
        }
    }
}

/// Calls an OpenAI-compatible `/v1/chat/completions` endpoint to consolidate a
/// concept definition. Returns `None` if the LLM is unavailable or not configured.
///
/// Configure via:
///   `HYPHAE_LLM_URL`   — base URL, e.g. <http://localhost:11434> or <https://api.openai.com>
///   `HYPHAE_LLM_MODEL` — model name, e.g. gpt-4o-mini or llama3
///   `HYPHAE_LLM_API_KEY` (optional) — sent as Bearer token
#[must_use]
pub fn consolidate_via_llm(name: &str, definition: &str) -> Option<String> {
    let base_url = std::env::var("HYPHAE_LLM_URL")
        .ok()
        .filter(|s| !s.is_empty())?;
    let model = std::env::var("HYPHAE_LLM_MODEL")
        .ok()
        .filter(|s| !s.is_empty())?;
    let api_key = std::env::var("HYPHAE_LLM_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());

    let timeout_secs: u64 = std::env::var("HYPHAE_LLM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let endpoint = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let prompt = format!(
        "You are summarizing accumulated knowledge about '{name}' in a software system memoir.\n\
         Consolidate the following description fragments into a single coherent definition.\n\
         Preserve all important technical details. Remove redundancy.\n\
         ---\n{definition}\n---\nConsolidated definition:"
    );

    let mut messages = vec![serde_json::json!({"role": "user", "content": prompt})];

    // Apply attribute-extraction preamble before building the request.
    apply_attribute_extraction_preamble(&mut messages, PREAMBLE_VERSION);

    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": 1024
    });

    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut req = agent
        .post(&endpoint)
        .header("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }

    let resp = req.send_json(&body).ok()?;

    let json: serde_json::Value = serde_json::from_reader(resp.into_body().as_reader()).ok()?;

    json.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_attribute_extraction_preamble_inserts_once() {
        let mut messages = vec![serde_json::json!({"role": "user", "content": "test prompt"})];

        apply_attribute_extraction_preamble(&mut messages, "1");

        let content = messages[0].get("content").and_then(|c| c.as_str()).unwrap();
        assert!(content.contains("[hyphae-attr-extraction-v1]"));
        assert!(content.starts_with("[hyphae-attr-extraction-v1]\n"));
        assert!(content.contains("test prompt"));
    }

    #[test]
    fn test_apply_attribute_extraction_preamble_idempotent() {
        let mut messages = vec![serde_json::json!({"role": "user", "content": "test prompt"})];

        apply_attribute_extraction_preamble(&mut messages, "1");
        let first_call = messages[0]
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap()
            .to_string();

        apply_attribute_extraction_preamble(&mut messages, "1");
        let second_call = messages[0]
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap()
            .to_string();

        assert_eq!(
            first_call, second_call,
            "preamble should not be inserted twice"
        );
        assert_eq!(first_call.matches("[hyphae-attr-extraction-v1]").count(), 1);
    }

    #[test]
    fn test_apply_attribute_extraction_preamble_empty_messages_noop() {
        // Empty slice must not panic and must insert nothing.
        let mut messages: Vec<serde_json::Value> = Vec::new();

        apply_attribute_extraction_preamble(&mut messages, PREAMBLE_VERSION);

        assert!(messages.is_empty(), "no message should be inserted");
    }

    #[test]
    fn test_apply_attribute_extraction_preamble_skips_non_user_first() {
        // A first message whose role is not "user" must be left unchanged.
        let mut messages = vec![serde_json::json!({"role": "assistant", "content": "prior reply"})];

        apply_attribute_extraction_preamble(&mut messages, PREAMBLE_VERSION);

        let content = messages[0].get("content").and_then(|c| c.as_str()).unwrap();
        assert_eq!(
            content, "prior reply",
            "non-user first message should be untouched"
        );
        assert!(!content.contains("[hyphae-attr-extraction-v"));
    }

    #[test]
    fn test_apply_attribute_extraction_preamble_sentinel_present() {
        let mut messages = vec![serde_json::json!({"role": "user", "content": "definition here"})];

        apply_attribute_extraction_preamble(&mut messages, "1");

        let content = messages[0].get("content").and_then(|c| c.as_str()).unwrap();
        assert!(content.contains("[hyphae-attr-extraction-v1]"));
    }
}
