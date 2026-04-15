//! Query sanitization to strip system-prompt contamination from search queries.
//!
//! When an LLM constructs a search query it may include conversation framing,
//! XML tags from its system prompt, or other noise that hurts embedding quality.
//! This module strips that contamination before the query reaches the embedder.

use regex::Regex;
use std::sync::LazyLock;

/// Maximum query length in characters (approximate 512-token budget at ~4 chars/token).
const MAX_QUERY_CHARS: usize = 2048;

/// XML-style tags commonly injected by system prompts.
static XML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?[a-zA-Z_][a-zA-Z0-9_:.-]*(?:\s[^>]*)?>").unwrap());

/// Conversation framing prefixes that add no search value.
static FRAMING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?mi)^(?:(?:Human|Assistant|System|User|AI|Claude)\s*:\s*|(?:###|##|#)\s*(?:Instructions?|Context|Query|Search|System)\s*\n)",
    )
    .unwrap()
});

/// Markdown-style emphasis and code fences that are noise for search.
static MARKDOWN_NOISE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"```[a-z]*\n?|```|`{1,3}").unwrap());

/// Result of sanitizing a query, carrying transparency metadata.
#[derive(Debug, Clone)]
pub struct SanitizedQuery {
    /// The cleaned query text.
    pub text: String,
    /// Whether any sanitization was applied.
    pub was_sanitized: bool,
    /// Human-readable description of what was removed.
    pub removed: Vec<String>,
    /// Whether the query was truncated to fit the length budget.
    pub was_truncated: bool,
}

/// Sanitize a search query by stripping system-prompt contamination.
///
/// Returns a [`SanitizedQuery`] with the cleaned text and transparency metadata
/// about what was removed.
///
/// # Examples
///
/// ```
/// use hyphae_core::sanitize::sanitize_query;
///
/// let result = sanitize_query("<system>Find errors</system>");
/// assert_eq!(result.text, "Find errors");
/// assert!(result.was_sanitized);
/// ```
pub fn sanitize_query(query: &str) -> SanitizedQuery {
    let mut text = query.to_string();
    let mut removed = Vec::new();
    let original_len = text.len();

    // 1. Strip XML tags
    if XML_TAG_RE.is_match(&text) {
        let before = text.clone();
        text = XML_TAG_RE.replace_all(&text, " ").to_string();
        if text != before {
            removed.push("xml_tags".to_string());
        }
    }

    // 2. Remove conversation framing prefixes
    if FRAMING_RE.is_match(&text) {
        let before = text.clone();
        text = FRAMING_RE.replace_all(&text, "").to_string();
        if text != before {
            removed.push("conversation_framing".to_string());
        }
    }

    // 3. Strip markdown noise
    if MARKDOWN_NOISE_RE.is_match(&text) {
        let before = text.clone();
        text = MARKDOWN_NOISE_RE.replace_all(&text, " ").to_string();
        if text != before {
            removed.push("markdown_noise".to_string());
        }
    }

    // 4. Normalize whitespace
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() != text.trim().len() {
        // Only flag if non-trivial normalization happened beyond simple trim
        let text_trimmed = text.trim().to_string();
        if normalized != text_trimmed {
            removed.push("whitespace_normalized".to_string());
        }
    }
    text = normalized;

    // 5. Truncate to max length
    let was_truncated = text.len() > MAX_QUERY_CHARS;
    if was_truncated {
        // Truncate at a word boundary
        if let Some(boundary) = text[..MAX_QUERY_CHARS].rfind(' ') {
            text.truncate(boundary);
        } else {
            text.truncate(MAX_QUERY_CHARS);
        }
        removed.push("truncated".to_string());
    }

    let was_sanitized = !removed.is_empty() || text.len() != original_len;

    SanitizedQuery {
        text,
        was_sanitized,
        removed,
        was_truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_query_unchanged() {
        let result = sanitize_query("rust error handling patterns");
        assert_eq!(result.text, "rust error handling patterns");
        assert!(!result.was_sanitized);
        assert!(result.removed.is_empty());
    }

    #[test]
    fn test_strips_xml_tags() {
        let result = sanitize_query("<system>Find all errors</system>");
        assert_eq!(result.text, "Find all errors");
        assert!(result.was_sanitized);
        assert!(result.removed.contains(&"xml_tags".to_string()));
    }

    #[test]
    fn test_strips_nested_xml_tags() {
        let result = sanitize_query("<tool_use><query>search term</query></tool_use>");
        assert_eq!(result.text, "search term");
        assert!(result.removed.contains(&"xml_tags".to_string()));
    }

    #[test]
    fn test_strips_conversation_framing() {
        let result = sanitize_query("Human: What is the best approach?");
        assert_eq!(result.text, "What is the best approach?");
        assert!(result.removed.contains(&"conversation_framing".to_string()));
    }

    #[test]
    fn test_strips_assistant_prefix() {
        let result = sanitize_query("Assistant: Here are the results");
        assert_eq!(result.text, "Here are the results");
        assert!(result.removed.contains(&"conversation_framing".to_string()));
    }

    #[test]
    fn test_strips_system_prefix() {
        let result = sanitize_query("System: You are a helpful assistant");
        assert_eq!(result.text, "You are a helpful assistant");
    }

    #[test]
    fn test_strips_markdown_noise() {
        let result = sanitize_query("```rust\nfn main() {}\n```");
        assert!(result.removed.contains(&"markdown_noise".to_string()));
    }

    #[test]
    fn test_normalizes_whitespace() {
        let result = sanitize_query("  too   many    spaces  ");
        assert_eq!(result.text, "too many spaces");
    }

    #[test]
    fn test_truncates_long_query() {
        let long_query = "word ".repeat(1000);
        let result = sanitize_query(&long_query);
        assert!(result.was_truncated);
        assert!(result.text.len() <= MAX_QUERY_CHARS);
        assert!(result.removed.contains(&"truncated".to_string()));
    }

    #[test]
    fn test_combined_contamination() {
        let query = "Human: <system>Search for</system> errors in ```production``` environment";
        let result = sanitize_query(query);
        assert!(result.was_sanitized);
        assert!(result.removed.contains(&"xml_tags".to_string()));
        assert!(result.removed.contains(&"conversation_framing".to_string()));
        assert!(result.removed.contains(&"markdown_noise".to_string()));
        // The meaningful content should survive
        assert!(result.text.contains("errors"));
        assert!(result.text.contains("environment"));
    }

    #[test]
    fn test_empty_query() {
        let result = sanitize_query("");
        assert_eq!(result.text, "");
    }

    #[test]
    fn test_only_tags() {
        let result = sanitize_query("<system></system>");
        assert_eq!(result.text, "");
        assert!(result.was_sanitized);
    }

    #[test]
    fn test_xml_tags_with_attributes() {
        let result = sanitize_query("<tool_use type=\"search\">query text</tool_use>");
        assert_eq!(result.text, "query text");
    }

    #[test]
    fn test_heading_framing() {
        let result = sanitize_query("### Instructions\nFind the error handler");
        assert!(result.text.contains("Find the error handler"));
        assert!(result.removed.contains(&"conversation_framing".to_string()));
    }

    #[test]
    fn test_internal_double_space_normalized() {
        let result = sanitize_query("a  b");
        assert_eq!(result.text, "a b");
        assert!(result.was_sanitized);
        assert!(
            result
                .removed
                .contains(&"whitespace_normalized".to_string())
        );
    }
}
