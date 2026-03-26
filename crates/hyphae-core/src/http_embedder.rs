// ─────────────────────────────────────────────────────────────────────────────
// HTTP Embedder — calls OpenAI-compatible or Ollama embedding APIs
// ─────────────────────────────────────────────────────────────────────────────

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::embedder::Embedder;
use crate::error::{HyphaeError, HyphaeResult};

/// An embedder that calls an HTTP endpoint for vector embeddings.
///
/// Supports two API formats:
/// - **OpenAI-compatible** (`/v1/embeddings`): used by most providers
/// - **Ollama** (`/api/embed`): auto-detected from URL path or port 11434
///
/// Configure via environment variables:
/// - `HYPHAE_EMBEDDING_URL`: base URL (e.g. `http://localhost:11434`)
/// - `HYPHAE_EMBEDDING_MODEL`: model name (e.g. `nomic-embed-text`)
#[derive(Debug)]
pub struct HttpEmbedder {
    url: String,
    model: String,
    dims: Arc<AtomicUsize>,
    api_format: ApiFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiFormat {
    /// POST /v1/embeddings with OpenAI-compatible JSON
    OpenAi,
    /// POST /api/embed with Ollama JSON (batch endpoint)
    Ollama,
}

impl HttpEmbedder {
    /// Create from environment variables.
    ///
    /// Returns `None` if `HYPHAE_EMBEDDING_URL` is not set.
    /// Returns `Err` if the URL is set but the model is missing.
    pub fn from_env() -> HyphaeResult<Option<Self>> {
        let url = match std::env::var("HYPHAE_EMBEDDING_URL") {
            Ok(u) if !u.is_empty() => u,
            _ => return Ok(None),
        };

        let model = std::env::var("HYPHAE_EMBEDDING_MODEL").unwrap_or_default();
        if model.is_empty() {
            return Err(HyphaeError::Config(
                "HYPHAE_EMBEDDING_URL is set but HYPHAE_EMBEDDING_MODEL is missing".into(),
            ));
        }

        Ok(Some(Self::new(url, model)))
    }

    /// Create with explicit URL and model.
    pub fn new(url: String, model: String) -> Self {
        let api_format = if url.contains("/api/embed") || url.contains(":11434") {
            ApiFormat::Ollama
        } else {
            ApiFormat::OpenAi
        };

        Self {
            url,
            model,
            dims: Arc::new(AtomicUsize::new(0)), // discovered on first call
            api_format,
        }
    }

    /// Build the full endpoint URL.
    fn endpoint(&self) -> String {
        let base = self.url.trim_end_matches('/');
        match self.api_format {
            ApiFormat::OpenAi => {
                if base.ends_with("/v1/embeddings") {
                    base.to_string()
                } else if base.ends_with("/v1") {
                    format!("{base}/embeddings")
                } else {
                    format!("{base}/v1/embeddings")
                }
            }
            ApiFormat::Ollama => {
                if base.ends_with("/api/embed") {
                    base.to_string()
                } else if base.ends_with("/api") {
                    format!("{base}/embed")
                } else {
                    format!("{base}/api/embed")
                }
            }
        }
    }

    /// Send a POST request with JSON body and parse the response.
    fn post_json(&self, body: &Value) -> HyphaeResult<Value> {
        let endpoint = self.endpoint();
        let resp = ureq::post(&endpoint)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| {
                HyphaeError::Embedding(format!("HTTP request to {endpoint} failed: {e}"))
            })?;

        let json: Value = serde_json::from_reader(resp.into_body().as_reader())
            .map_err(|e| HyphaeError::Embedding(format!("failed to parse response JSON: {e}")))?;

        Ok(json)
    }

    /// Parse a JSON array of numbers into `Vec<f32>`.
    fn parse_float_array(arr: &[Value]) -> HyphaeResult<Vec<f32>> {
        arr.iter()
            .map(|v: &Value| {
                v.as_f64()
                    .map(|f| f as f32)
                    .ok_or_else(|| HyphaeError::Embedding("non-numeric embedding value".into()))
            })
            .collect()
    }

    /// Call OpenAI-compatible /v1/embeddings endpoint.
    fn embed_openai(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>> {
        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self.post_json(&body)?;

        let data: &Vec<Value> = resp.get("data").and_then(Value::as_array).ok_or_else(|| {
            HyphaeError::Embedding(format!(
                "unexpected response format: missing 'data' array: {resp}"
            ))
        })?;

        let mut results = Vec::with_capacity(data.len());
        for item in data {
            let embedding: &Vec<Value> = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    HyphaeError::Embedding("missing 'embedding' field in response".into())
                })?;

            results.push(Self::parse_float_array(embedding)?);
        }

        Ok(results)
    }

    /// Call Ollama /api/embed endpoint (batch).
    fn embed_ollama(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>> {
        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self.post_json(&body)?;

        let embeddings: &Vec<Value> = resp
            .get("embeddings")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                HyphaeError::Embedding(format!(
                    "unexpected Ollama response: missing 'embeddings' array: {resp}"
                ))
            })?;

        let mut results = Vec::with_capacity(embeddings.len());
        for emb in embeddings {
            let arr: &Vec<Value> = emb
                .as_array()
                .ok_or_else(|| HyphaeError::Embedding("embedding entry is not an array".into()))?;

            results.push(Self::parse_float_array(arr)?);
        }

        Ok(results)
    }
}

impl Embedder for HttpEmbedder {
    fn embed(&self, text: &str) -> HyphaeResult<Vec<f32>> {
        let results = self.embed_batch(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| HyphaeError::Embedding("empty embedding result".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        match self.api_format {
            ApiFormat::OpenAi => self.embed_openai(texts),
            ApiFormat::Ollama => self.embed_ollama(texts),
        }
    }

    fn dimensions(&self) -> usize {
        let cached = self.dims.load(Ordering::Relaxed);
        if cached > 0 {
            return cached;
        }
        // Probe dimensions by embedding a short text.
        // Fall back to common default if probe fails.
        let result = match self.embed("dimension probe") {
            Ok(vec) => vec.len(),
            Err(_) => 768,
        };
        // Store the probed dimensions for next time
        self.dims.store(result, Ordering::Relaxed);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_from_env_returns_none_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();

        // SAFETY: test-only, single-threaded env manipulation
        unsafe {
            std::env::remove_var("HYPHAE_EMBEDDING_URL");
            std::env::remove_var("HYPHAE_EMBEDDING_MODEL");
        }

        let result = HttpEmbedder::from_env().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_from_env_error_when_url_set_but_no_model() {
        let _guard = ENV_LOCK.lock().unwrap();

        // SAFETY: test-only, single-threaded env manipulation
        unsafe {
            std::env::set_var("HYPHAE_EMBEDDING_URL", "http://localhost:11434");
            std::env::remove_var("HYPHAE_EMBEDDING_MODEL");
        }

        let result = HttpEmbedder::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("HYPHAE_EMBEDDING_MODEL"));

        // SAFETY: cleanup
        unsafe {
            std::env::remove_var("HYPHAE_EMBEDDING_URL");
        }
    }

    #[test]
    fn test_api_format_detection_ollama() {
        let embedder =
            HttpEmbedder::new("http://localhost:11434".into(), "nomic-embed-text".into());
        assert_eq!(embedder.api_format, ApiFormat::Ollama);

        let embedder2 = HttpEmbedder::new(
            "http://localhost:11434/api/embed".into(),
            "nomic-embed-text".into(),
        );
        assert_eq!(embedder2.api_format, ApiFormat::Ollama);
    }

    #[test]
    fn test_api_format_detection_openai() {
        let embedder = HttpEmbedder::new(
            "http://localhost:8080".into(),
            "text-embedding-3-small".into(),
        );
        assert_eq!(embedder.api_format, ApiFormat::OpenAi);

        let embedder2 = HttpEmbedder::new(
            "https://api.openai.com/v1/embeddings".into(),
            "text-embedding-3-small".into(),
        );
        assert_eq!(embedder2.api_format, ApiFormat::OpenAi);
    }

    #[test]
    fn test_endpoint_construction_openai() {
        let e = HttpEmbedder::new("http://localhost:8080".into(), "m".into());
        assert_eq!(e.endpoint(), "http://localhost:8080/v1/embeddings");

        let e2 = HttpEmbedder::new("http://localhost:8080/v1".into(), "m".into());
        assert_eq!(e2.endpoint(), "http://localhost:8080/v1/embeddings");

        let e3 = HttpEmbedder::new("http://localhost:8080/v1/embeddings".into(), "m".into());
        assert_eq!(e3.endpoint(), "http://localhost:8080/v1/embeddings");
    }

    #[test]
    fn test_endpoint_construction_ollama() {
        let e = HttpEmbedder::new("http://localhost:11434".into(), "m".into());
        assert_eq!(e.endpoint(), "http://localhost:11434/api/embed");

        let e2 = HttpEmbedder::new("http://localhost:11434/api".into(), "m".into());
        assert_eq!(e2.endpoint(), "http://localhost:11434/api/embed");

        let e3 = HttpEmbedder::new("http://localhost:11434/api/embed".into(), "m".into());
        assert_eq!(e3.endpoint(), "http://localhost:11434/api/embed");
    }

    #[test]
    fn test_trailing_slash_stripped() {
        let e = HttpEmbedder::new("http://localhost:8080/".into(), "m".into());
        assert_eq!(e.endpoint(), "http://localhost:8080/v1/embeddings");
    }

    #[test]
    fn test_parse_float_array() {
        let arr = vec![
            serde_json::json!(1.0),
            serde_json::json!(2.5),
            serde_json::json!(3.0),
        ];
        let result = HttpEmbedder::parse_float_array(&arr).unwrap();
        assert_eq!(result, vec![1.0_f32, 2.5, 3.0]);
    }

    #[test]
    fn test_parse_float_array_invalid() {
        let arr = vec![serde_json::json!(1.0), serde_json::json!("not a number")];
        let result = HttpEmbedder::parse_float_array(&arr);
        assert!(result.is_err());
    }
}
