/// Calls an OpenAI-compatible /v1/chat/completions endpoint to consolidate a
/// concept definition. Returns None if the LLM is unavailable or not configured.
///
/// Configure via:
///   HYPHAE_LLM_URL   — base URL, e.g. http://localhost:11434 or https://api.openai.com
///   HYPHAE_LLM_MODEL — model name, e.g. gpt-4o-mini or llama3
///   HYPHAE_LLM_API_KEY (optional) — sent as Bearer token
pub fn consolidate_via_llm(name: &str, definition: &str) -> Option<String> {
    let base_url = std::env::var("HYPHAE_LLM_URL").ok().filter(|s| !s.is_empty())?;
    let model = std::env::var("HYPHAE_LLM_MODEL").ok().filter(|s| !s.is_empty())?;
    let api_key = std::env::var("HYPHAE_LLM_API_KEY").ok().filter(|s| !s.is_empty());

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

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "max_tokens": 1024
    });

    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut req = agent.post(&endpoint).header("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }

    let resp = req.send_json(&body).ok()?;

    let json: serde_json::Value =
        serde_json::from_reader(resp.into_body().as_reader()).ok()?;

    json.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
