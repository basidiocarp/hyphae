/// Sanitize a query string for FTS5 MATCH.
///
/// FTS5 treats characters like `-`, `*`, `"`, `:`, `^`, `+`, `~` as operators.
/// A query like `"sqlite-vec"` makes FTS5 interpret `-` as NOT and `vec` as a
/// column name, causing "no such column: vec".
///
/// This function strips special chars and wraps each token in double quotes.
pub(crate) fn sanitize_fts_query(query: &str) -> String {
    // Replace FTS5 operator chars with spaces, then quote each resulting token.
    // FTS5 tokenizer (unicode61) splits on `-` too, so we must keep tokens separate.
    let cleaned: String = query
        .chars()
        .map(|c| {
            if matches!(
                c,
                '-' | '*' | '"' | '(' | ')' | '{' | '}' | ':' | '^' | '+' | '~' | '\\'
            ) {
                ' '
            } else {
                c
            }
        })
        .collect();

    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    tokens.join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_plain_text() {
        let result = sanitize_fts_query("hello world");
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_sanitize_fts_operators_and() {
        let result = sanitize_fts_query("test AND query");
        // AND is not a special character, just whitespace is there
        assert_eq!(result, "\"test\" \"AND\" \"query\"");
    }

    #[test]
    fn test_sanitize_fts_operators_or() {
        let result = sanitize_fts_query("foo OR bar");
        // OR is not a special character
        assert_eq!(result, "\"foo\" \"OR\" \"bar\"");
    }

    #[test]
    fn test_sanitize_fts_operators_not() {
        let result = sanitize_fts_query("-exclude");
        assert_eq!(result, "\"exclude\"");
    }

    #[test]
    fn test_sanitize_special_chars_hyphen() {
        let result = sanitize_fts_query("sqlite-vec");
        assert_eq!(result, "\"sqlite\" \"vec\"");
    }

    #[test]
    fn test_sanitize_special_chars_asterisk() {
        let result = sanitize_fts_query("test*query");
        assert_eq!(result, "\"test\" \"query\"");
    }

    #[test]
    fn test_sanitize_special_chars_quotes() {
        let result = sanitize_fts_query("\"quoted\"");
        assert_eq!(result, "\"quoted\"");
    }

    #[test]
    fn test_sanitize_special_chars_parentheses() {
        let result = sanitize_fts_query("(test)");
        assert_eq!(result, "\"test\"");
    }

    #[test]
    fn test_sanitize_special_chars_braces() {
        let result = sanitize_fts_query("{test}");
        assert_eq!(result, "\"test\"");
    }

    #[test]
    fn test_sanitize_special_chars_colon() {
        let result = sanitize_fts_query("column:value");
        assert_eq!(result, "\"column\" \"value\"");
    }

    #[test]
    fn test_sanitize_special_chars_caret() {
        let result = sanitize_fts_query("test^10");
        assert_eq!(result, "\"test\" \"10\"");
    }

    #[test]
    fn test_sanitize_special_chars_plus() {
        let result = sanitize_fts_query("test+query");
        assert_eq!(result, "\"test\" \"query\"");
    }

    #[test]
    fn test_sanitize_special_chars_tilde() {
        let result = sanitize_fts_query("test~0.8");
        // ~ is replaced with space, . is not a special char
        assert_eq!(result, "\"test\" \"0.8\"");
    }

    #[test]
    fn test_sanitize_special_chars_backslash() {
        let result = sanitize_fts_query("test\\escape");
        assert_eq!(result, "\"test\" \"escape\"");
    }

    #[test]
    fn test_sanitize_empty_string() {
        let result = sanitize_fts_query("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_only_whitespace() {
        let result = sanitize_fts_query("   ");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_only_special_chars() {
        let result = sanitize_fts_query("---***");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_mixed_special_chars() {
        let result = sanitize_fts_query("api-docs:search*");
        assert_eq!(result, "\"api\" \"docs\" \"search\"");
    }

    #[test]
    fn test_sanitize_preserves_alphanumeric() {
        let result = sanitize_fts_query("test123abc");
        assert_eq!(result, "\"test123abc\"");
    }

    #[test]
    fn test_sanitize_multiple_spaces() {
        let result = sanitize_fts_query("hello     world");
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_sanitize_leading_trailing_spaces() {
        let result = sanitize_fts_query("  hello world  ");
        assert_eq!(result, "\"hello\" \"world\"");
    }
}
