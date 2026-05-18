// ─────────────────────────────────────────────────────────────────────────────
// Secrets Detection Patterns
//
// Keep in sync with cortina/src/hooks/post_tool_use/secret_redaction.rs.
// Cortina uses hand-rolled string matching; hyphae uses regex. When adding
// or removing a pattern category, update both files.
// ─────────────────────────────────────────────────────────────────────────────

use regex::Regex;
use std::sync::OnceLock;

const SECRET_PATTERNS: &[(&str, &str)] = &[
    (r"(?i)(api[_-]?key|apikey)\s*[:=]\s*\S{10,}", "API key"),
    (
        r"(?i)(secret|password|passwd|pwd)\s*[:=]\s*\S{8,}",
        "password/secret",
    ),
    (r"sk-[a-zA-Z0-9_-]{20,}", "OpenAI API key"),
    (r"ghp_[a-zA-Z0-9]{36,}", "GitHub personal access token"),
    (r"(?i)bearer\s+[a-zA-Z0-9._-]{20,}", "Bearer token"),
    (r"AKIA[0-9A-Z]{16}", "AWS access key"),
    (
        r"(?i)(token|auth)\s*[:=]\s*[a-zA-Z0-9._-]{20,}",
        "auth token",
    ),
    (r"-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----", "private key"),
    // Stripe-style live/test secret keys — synced from cortina (SK_LIVE, SK_TEST).
    (
        r"(?i)sk[_-](live|test)\s*[:=]\s*\S{8,}",
        "Stripe secret key",
    ),
    // Authorization header (non-Bearer form, e.g. Basic auth) — synced from cortina.
    (r"(?i)^authorization\s*:.*$", "Authorization header"),
    // URL-embedded credentials (https://user:pass@host) — synced from cortina.
    (r"https?://[^@\s]+:[^@\s]+@", "URL credentials"),
];

fn compiled_secret_patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        SECRET_PATTERNS
            .iter()
            .filter_map(|(pattern, label)| Regex::new(pattern).ok().map(|r| (r, *label)))
            .collect()
    })
}

/// Detect common secret patterns in content.
///
/// Returns a vector of detected secret types. If secrets are found, the caller
/// should warn or block storage depending on configuration.
pub fn detect_secrets(content: &str) -> Vec<String> {
    compiled_secret_patterns()
        .iter()
        .filter_map(|(regex, label)| {
            if regex.is_match(content) {
                Some((*label).to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_api_key() {
        let content = "api_key: sk1234567890abcdefghij";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("API")));
    }

    #[test]
    fn test_detect_github_token() {
        let content = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("GitHub")));
    }

    #[test]
    fn test_detect_openai_key() {
        let content = "sk-proj-1234567890abcdefghijklmnopqrst";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("OpenAI")));
    }

    #[test]
    fn test_detect_aws_key() {
        let content = "AKIAIOSFODNN7EXAMPLE";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("AWS")));
    }

    #[test]
    fn test_detect_bearer_token() {
        let content = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9abc123xyz";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("Bearer")));
    }

    #[test]
    fn test_detect_private_key() {
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("private key")));
    }

    #[test]
    fn test_detect_password() {
        let content = "password = superSecurePassword123";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("password")));
    }

    #[test]
    fn test_detect_stripe_live_key() {
        let content = "SK_LIVE=sk_live_abc123defghijklmnop";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("Stripe")));
    }

    #[test]
    fn test_detect_stripe_test_key() {
        let content = "sk_test: sk_test_xyz987uvwrstuvwxyz";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("Stripe")));
    }

    #[test]
    fn test_detect_authorization_header() {
        let content = "Authorization: Basic dXNlcm5hbWU6cGFzc3dvcmQ=";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("Authorization")));
    }

    #[test]
    fn test_detect_url_credentials() {
        let content = "Cloning from https://user:mypassword@github.com/org/repo.git";
        let secrets = detect_secrets(content);
        assert!(!secrets.is_empty());
        assert!(secrets.iter().any(|s| s.contains("URL")));
    }

    #[test]
    fn test_no_secrets_in_normal_text() {
        let content = "This is a normal memory about how to debug issues";
        let secrets = detect_secrets(content);
        assert!(secrets.is_empty());
    }

    #[test]
    fn test_multiple_secrets() {
        let content = r#"
            api_key = "sk1234567890abcdefghij"
            ghp_token = "ghp_1234567890abcdefghijklmnopqrstuvwxyz"
        "#;
        let secrets = detect_secrets(content);
        assert!(secrets.len() >= 2);
    }
}
