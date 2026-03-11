use std::collections::HashMap;

use hyphae_core::{Chunk, ChunkStore, HyphaeResult, Memory, MemoryStore};

use super::SqliteStore;

// ---------------------------------------------------------------------------
// Unified search result
// ---------------------------------------------------------------------------

/// A single result from cross-store search, tagged by origin.
#[derive(Debug, Clone)]
pub enum UnifiedSearchResult {
    Memory { memory: Memory, score: f32 },
    Chunk { chunk: Chunk, score: f32 },
}

impl UnifiedSearchResult {
    pub fn score(&self) -> f32 {
        match self {
            Self::Memory { score, .. } => *score,
            Self::Chunk { score, .. } => *score,
        }
    }
}

// ---------------------------------------------------------------------------
// Reciprocal Rank Fusion search
// ---------------------------------------------------------------------------

impl SqliteStore {
    /// Search across memories and document chunks using Reciprocal Rank Fusion.
    ///
    /// Each store is searched independently and results are ranked with RRF:
    /// `score = 1 / (k + rank)` where k = 60 (standard constant).
    /// If a result appears in both sets the scores are summed.
    pub fn search_all(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        include_docs: bool,
    ) -> HyphaeResult<Vec<UnifiedSearchResult>> {
        const K: f32 = 60.0;
        let pool = limit * 3; // fetch more than needed for better fusion

        // --- memory results ---
        let mem_results: Vec<(Memory, f32)> = if let Some(emb) = embedding {
            self.search_hybrid(query, emb, pool)?
        } else {
            // FTS returns Vec<Memory> without scores — assign rank-based scores
            self.search_fts(query, pool)?
                .into_iter()
                .enumerate()
                .map(|(i, m)| (m, 1.0 / (K + i as f32)))
                .collect()
        };

        // --- chunk results ---
        let chunk_results = if include_docs {
            if let Some(emb) = embedding {
                self.search_chunks_hybrid(query, emb, pool)?
            } else {
                self.search_chunks_fts(query, pool)?
            }
        } else {
            Vec::new()
        };

        // --- RRF scoring for memories ---
        let mut scores: HashMap<String, f32> = HashMap::new();
        let mut memory_map: HashMap<String, Memory> = HashMap::new();
        for (rank, (mem, _original_score)) in mem_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + rank as f32);
            let key = format!("mem:{}", mem.id);
            *scores.entry(key.clone()).or_default() += rrf;
            memory_map.insert(key, mem);
        }

        // --- RRF scoring for chunks ---
        let mut chunk_map: HashMap<String, Chunk> = HashMap::new();
        for (rank, csr) in chunk_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + rank as f32);
            let key = format!("chunk:{}", csr.chunk.id);
            *scores.entry(key.clone()).or_default() += rrf;
            chunk_map.insert(key, csr.chunk);
        }

        // --- merge and sort ---
        let mut ranked: Vec<(String, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        let results = ranked
            .into_iter()
            .filter_map(|(key, score)| {
                if let Some(mem) = memory_map.remove(&key) {
                    Some(UnifiedSearchResult::Memory { memory: mem, score })
                } else {
                    chunk_map
                        .remove(&key)
                        .map(|chunk| UnifiedSearchResult::Chunk { chunk, score })
                }
            })
            .collect();

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// FTS sanitisation
// ---------------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Unified search tests
    // -----------------------------------------------------------------------
    use hyphae_core::ids::{ChunkId, DocumentId};
    use hyphae_core::{
        Chunk, ChunkMetadata, ChunkStore, Document, Memory, MemoryStore, SourceType,
    };

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_doc(path: &str) -> Document {
        Document {
            id: DocumentId::new(),
            source_path: path.to_string(),
            source_type: SourceType::Text,
            chunk_count: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn make_chunk(doc_id: &DocumentId, content: &str, source_path: &str) -> Chunk {
        Chunk {
            id: ChunkId::new(),
            document_id: doc_id.clone(),
            chunk_index: 0,
            content: content.to_string(),
            metadata: ChunkMetadata {
                source_path: source_path.to_string(),
                source_type: SourceType::Text,
                language: None,
                heading: None,
                line_start: Some(1),
                line_end: Some(10),
            },
            embedding: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_search_all_memories_only() {
        let store = test_store();
        store
            .store(Memory::new(
                "rust".to_string(),
                "Rust is a systems programming language".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();
        store
            .store(Memory::new(
                "python".to_string(),
                "Python is an interpreted language".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        let results = store
            .search_all("Rust systems programming", None, 10, false)
            .unwrap();

        assert!(!results.is_empty(), "should find at least one memory");
        assert!(
            results
                .iter()
                .all(|r| matches!(r, UnifiedSearchResult::Memory { .. })),
            "all results should be memories when include_docs=false"
        );
        // First result should match our Rust memory
        if let UnifiedSearchResult::Memory { memory, .. } = &results[0] {
            assert!(
                memory.summary.contains("Rust"),
                "first result should be about Rust"
            );
        }
    }

    #[test]
    fn test_search_all_docs_only() {
        let store = test_store();
        let doc = make_doc("project/readme.md");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![make_chunk(
                &doc_id,
                "SQLite database engine overview",
                "project/readme.md",
            )])
            .unwrap();

        // Search with no memories present
        let results = store.search_all("SQLite database", None, 10, true).unwrap();

        assert!(!results.is_empty(), "should find the chunk");
        assert!(
            results
                .iter()
                .all(|r| matches!(r, UnifiedSearchResult::Chunk { .. })),
            "all results should be chunks when no memories match"
        );
        if let UnifiedSearchResult::Chunk { chunk, .. } = &results[0] {
            assert!(chunk.content.contains("SQLite"));
        }
    }

    #[test]
    fn test_search_all_mixed() {
        let store = test_store();

        // Store a memory
        store
            .store(Memory::new(
                "architecture".to_string(),
                "The system uses SQLite for persistent storage".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        // Store a document chunk
        let doc = make_doc("docs/design.md");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![make_chunk(
                &doc_id,
                "SQLite is configured with WAL mode for concurrency",
                "docs/design.md",
            )])
            .unwrap();

        let results = store.search_all("SQLite", None, 10, true).unwrap();

        assert!(results.len() >= 2, "should find results from both stores");

        let has_memory = results
            .iter()
            .any(|r| matches!(r, UnifiedSearchResult::Memory { .. }));
        let has_chunk = results
            .iter()
            .any(|r| matches!(r, UnifiedSearchResult::Chunk { .. }));
        assert!(has_memory, "should include memory results");
        assert!(has_chunk, "should include chunk results");

        // All scores should be positive
        for r in &results {
            assert!(r.score() > 0.0, "all RRF scores should be positive");
        }
        // Results should be sorted by descending score
        for w in results.windows(2) {
            assert!(
                w[0].score() >= w[1].score(),
                "results should be sorted by descending score"
            );
        }
    }

    #[test]
    fn test_search_all_include_docs_false() {
        let store = test_store();

        store
            .store(Memory::new(
                "test".to_string(),
                "Unit testing with cargo test".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        let doc = make_doc("tests/readme.md");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![make_chunk(
                &doc_id,
                "Testing framework documentation for cargo test",
                "tests/readme.md",
            )])
            .unwrap();

        let results = store.search_all("cargo test", None, 10, false).unwrap();

        assert!(
            results
                .iter()
                .all(|r| matches!(r, UnifiedSearchResult::Memory { .. })),
            "should only return memories when include_docs=false"
        );
    }

    #[test]
    fn test_search_all_empty_store() {
        let store = test_store();
        let results = store.search_all("anything", None, 10, true).unwrap();
        assert!(results.is_empty(), "empty store should return no results");
    }

    #[test]
    fn test_search_all_respects_limit() {
        let store = test_store();
        for i in 0..20 {
            store
                .store(Memory::new(
                    "topic".to_string(),
                    format!("Memory number {i} about testing"),
                    hyphae_core::Importance::Medium,
                ))
                .unwrap();
        }

        let results = store.search_all("testing", None, 5, false).unwrap();

        assert!(results.len() <= 5, "should respect the limit parameter");
    }
}
