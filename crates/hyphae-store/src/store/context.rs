use std::collections::HashSet;

use hyphae_core::MemoirStore;

use super::SqliteStore;

// ---------------------------------------------------------------------------
// Code-related heuristic
// ---------------------------------------------------------------------------

/// File extensions that indicate a code-related query.
const CODE_EXTENSIONS: &[&str] = &[
    ".rs", ".ts", ".py", ".js", ".go", ".java", ".rb", ".cpp", ".c", ".h",
];

/// Returns `true` when the query looks like it references code symbols, file
/// paths, or module names.
///
/// Heuristics applied (any match → true):
/// - Contains a CamelCase word (uppercase letter followed by lowercase then uppercase)
/// - Contains a snake_case word (lowercase word with underscore and another lowercase word)
/// - Contains a recognised source-file extension
/// - Contains a path separator (`/`)
pub fn is_code_related(query: &str) -> bool {
    if query.contains('/') {
        return true;
    }

    for ext in CODE_EXTENSIONS {
        if query.contains(ext) {
            return true;
        }
    }

    if has_camel_case(query) {
        return true;
    }

    if has_snake_case(query) {
        return true;
    }

    false
}

/// Returns `true` if the string contains a CamelCase word: an uppercase letter
/// followed by one or more lowercase letters followed by another uppercase letter
/// (pattern `[A-Z][a-z]+[A-Z]`).
fn has_camel_case(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len < 3 {
        return false;
    }
    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_uppercase() {
            // Scan forward through lowercase letters
            let mut j = i + 1;
            while j < len && chars[j].is_ascii_lowercase() {
                j += 1;
            }
            // Pattern matches when we consumed at least one lowercase and hit
            // another uppercase (j > i+1 ensures at least one lowercase was seen).
            if j > i + 1 && j < len && chars[j].is_ascii_uppercase() {
                return true;
            }
            // Skip to j to avoid re-scanning already-seen chars
            i = if j > i { j } else { i + 1 };
        } else {
            i += 1;
        }
    }
    false
}

/// Returns `true` if the string contains a snake_case pattern: a run of
/// lowercase ASCII letters, an underscore, then another lowercase ASCII letter
/// (pattern `[a-z]+_[a-z]`).
fn has_snake_case(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len < 3 {
        return false;
    }
    for i in 1..len.saturating_sub(1) {
        if chars[i] == '_' && chars[i - 1].is_ascii_lowercase() && chars[i + 1].is_ascii_lowercase()
        {
            return true;
        }
    }
    false
}

const MAX_CODE_TERMS: usize = 8;

fn push_code_term(
    term: &str,
    allow_plain: bool,
    seen: &mut HashSet<String>,
    terms: &mut Vec<String>,
) {
    if terms.len() >= MAX_CODE_TERMS {
        return;
    }

    let trimmed = term.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
    if trimmed.is_empty() || trimmed.len() > 64 {
        return;
    }

    if !allow_plain && !has_camel_case(trimmed) && !has_snake_case(trimmed) {
        return;
    }

    if seen.insert(trimmed.to_string()) {
        terms.push(trimmed.to_string());
    }
}

fn collect_code_terms(
    fragment: &str,
    allow_plain: bool,
    seen: &mut HashSet<String>,
    terms: &mut Vec<String>,
) {
    if terms.len() >= MAX_CODE_TERMS {
        return;
    }

    let trimmed = fragment.trim_matches(|c: char| {
        !c.is_ascii_alphanumeric() && c != '_' && c != '/' && c != '.' && c != ':'
    });
    if trimmed.is_empty() {
        return;
    }

    if trimmed.contains('/') || trimmed.contains(':') {
        for segment in trimmed
            .split(['/', ':'])
            .filter(|segment| !segment.is_empty())
        {
            collect_code_terms(segment, true, seen, terms);
            if terms.len() >= MAX_CODE_TERMS {
                return;
            }
        }
        return;
    }

    if let Some((stem, ext)) = trimmed.rsplit_once('.') {
        if !stem.is_empty() && !ext.is_empty() {
            let stem = stem.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
            if !stem.is_empty() {
                push_code_term(stem, true, seen, terms);
            }
        }
    }

    push_code_term(trimmed, allow_plain, seen, terms);
}

fn extract_code_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = HashSet::new();

    for fragment in query.split_whitespace() {
        collect_code_terms(fragment, false, &mut seen, &mut terms);
        if terms.len() >= MAX_CODE_TERMS {
            break;
        }
    }

    terms
}

// ---------------------------------------------------------------------------
// Code-context expansion
// ---------------------------------------------------------------------------

/// Look up the `code:{project}` memoir and search its concepts using extracted
/// code-shaped terms from `query`. Returns up to 5 concept names to use as
/// additional search terms when expanding a recall query.
///
/// Returns an empty `Vec` when:
/// - No `code:{project}` memoir exists (graceful degradation)
/// - No code-shaped terms are extracted from the query
/// - The exact-match and FTS lookups return no matching concepts
pub fn expand_with_code_context(store: &SqliteStore, query: &str, project: &str) -> Vec<String> {
    let memoir_name = format!("code:{project}");

    let memoir = match store.get_memoir_by_name(&memoir_name) {
        Ok(Some(m)) => m,
        Ok(None) => return Vec::new(),
        Err(_) => return Vec::new(),
    };

    let terms = extract_code_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }

    let mut concepts = Vec::new();
    let mut seen = HashSet::new();

    for term in &terms {
        if concepts.len() >= 5 {
            break;
        }
        if let Ok(Some(concept)) = store.get_concept_by_name(&memoir.id, term) {
            if seen.insert(concept.name.clone()) {
                concepts.push(concept);
            }
        }
    }

    for term in &terms {
        if concepts.len() >= 5 {
            break;
        }
        let matches = match store.search_concepts_fts(&memoir.id, term, 5) {
            Ok(c) => c,
            Err(_) => break,
        };
        for concept in matches {
            if concepts.len() >= 5 {
                break;
            }
            if seen.insert(concept.name.clone()) {
                concepts.push(concept);
            }
        }
    }

    concepts.into_iter().map(|c| c.name).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::memoir::{Concept, Memoir};
    use hyphae_core::{MemoirStore, ids::MemoirId};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memoir(store: &SqliteStore, name: &str) -> MemoirId {
        let memoir = Memoir::new(name.to_string(), format!("Test memoir: {name}"));
        store.create_memoir(memoir).unwrap()
    }

    fn add_concept(store: &SqliteStore, memoir_id: &MemoirId, name: &str, definition: &str) {
        let concept = Concept::new(memoir_id.clone(), name.to_string(), definition.to_string());
        store.add_concept(concept).unwrap();
    }

    fn extract_terms(query: &str) -> Vec<String> {
        extract_code_terms(query)
    }

    // --- is_code_related ---

    #[test]
    fn test_is_code_related_camel_case() {
        assert!(is_code_related("AuthMiddleware handles requests"));
        assert!(is_code_related("the TokenValidator struct"));
    }

    #[test]
    fn test_is_code_related_snake_case() {
        assert!(is_code_related("verify_token function"));
        assert!(is_code_related("session_store lookup"));
    }

    #[test]
    fn test_is_code_related_file_extension() {
        assert!(is_code_related("middleware.rs file"));
        assert!(is_code_related("index.ts handler"));
        assert!(is_code_related("auth.py module"));
        assert!(is_code_related("utils.js helper"));
        assert!(is_code_related("server.go endpoint"));
        assert!(is_code_related("Main.java class"));
        assert!(is_code_related("model.rb ActiveRecord"));
        assert!(is_code_related("parser.cpp implementation"));
        assert!(is_code_related("config.c binding"));
        assert!(is_code_related("header.h include"));
    }

    #[test]
    fn test_is_code_related_path_separator() {
        assert!(is_code_related("src/auth/middleware"));
        assert!(is_code_related("how to use src/utils"));
    }

    #[test]
    fn test_is_code_related_natural_language_false() {
        assert!(!is_code_related("how to deploy the application"));
        assert!(!is_code_related("what is the best approach"));
        assert!(!is_code_related("tell me about the authentication flow"));
    }

    // --- expand_with_code_context ---

    #[test]
    fn test_expand_returns_empty_when_no_memoir() {
        let store = make_store();
        let terms = expand_with_code_context(&store, "auth middleware", "myproject");
        assert!(
            terms.is_empty(),
            "should return empty vec when no memoir exists"
        );
    }

    #[test]
    fn test_expand_returns_matching_concept_names() {
        let store = make_store();
        let memoir_id = make_memoir(&store, "code:myproject");

        add_concept(
            &store,
            &memoir_id,
            "AuthMiddleware",
            "Handles auth middleware pipeline for HTTP request validation",
        );
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token path validates auth tokens",
        );
        add_concept(
            &store,
            &memoir_id,
            "DatabasePool",
            "Manages database connection pooling",
        );

        let terms = expand_with_code_context(&store, "previous verify_token failure", "myproject");

        assert!(
            !terms.is_empty(),
            "should return expanded terms for a mixed prose/code query"
        );
        assert!(
            terms.contains(&"TokenValidator".to_string()),
            "should include TokenValidator in expanded terms, got: {terms:?}"
        );
    }

    #[test]
    fn test_expand_limits_to_five_concepts() {
        let store = make_store();
        let memoir_id = make_memoir(&store, "code:bigproject");

        for i in 0..10 {
            add_concept(
                &store,
                &memoir_id,
                &format!("AuthHandler{i}"),
                &format!("Auth handler number {i} for authentication"),
            );
        }

        let terms = expand_with_code_context(&store, "auth handler authentication", "bigproject");
        assert!(
            terms.len() <= 5,
            "should return at most 5 concepts, got {}",
            terms.len()
        );
    }

    #[test]
    fn test_expand_wrong_project_returns_empty() {
        let store = make_store();
        let memoir_id = make_memoir(&store, "code:projectA");

        add_concept(
            &store,
            &memoir_id,
            "AuthMiddleware",
            "Auth middleware for projectA",
        );

        let terms = expand_with_code_context(&store, "auth middleware", "projectB");
        assert!(
            terms.is_empty(),
            "should return empty for wrong project name"
        );
    }

    #[test]
    fn test_expand_returns_empty_when_query_has_no_code_terms() {
        let store = make_store();
        let memoir_id = make_memoir(&store, "code:myproject");

        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "validates auth tokens",
        );

        let terms =
            expand_with_code_context(&store, "previous failure in the workflow", "myproject");

        assert!(
            terms.is_empty(),
            "prose-only queries should not produce code expansion terms"
        );
    }

    #[test]
    fn test_extract_code_terms_returns_empty_for_prose() {
        assert!(extract_terms("previous failure in the workflow").is_empty());
    }

    #[test]
    fn test_extract_code_terms_dedupes_and_caps_results() {
        let terms = extract_terms(
            "TokenValidator verify_token TokenValidator src/auth/middleware.rs AuthMiddleware another_term",
        );

        assert!(terms.len() <= MAX_CODE_TERMS);
        assert_eq!(
            terms
                .iter()
                .filter(|term| *term == "TokenValidator")
                .count(),
            1
        );
        assert_eq!(
            terms.iter().filter(|term| *term == "verify_token").count(),
            1
        );
        assert!(terms.iter().any(|term| term == "middleware"));
    }
}
