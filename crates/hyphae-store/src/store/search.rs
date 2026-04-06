use std::collections::{HashMap, HashSet};

use hyphae_core::{Chunk, ChunkStore, HyphaeResult, Memory, MemoryStore};

use super::{SHARED_PROJECT, SqliteStore, context};

// ---------------------------------------------------------------------------
// Unified search result
// ---------------------------------------------------------------------------

/// A single result from cross-store search, tagged by origin.
#[derive(Debug, Clone)]
pub enum UnifiedSearchResult {
    Memory { memory: Box<Memory>, score: f32 },
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
    ///
    /// When `code_expand_project` is `Some(project)`, the query is expanded with
    /// code symbols from the `code:{project}` memoir (if it exists and the query
    /// looks code-related). Expanded FTS results are merged in with 0.5× weight.
    ///
    /// Optimization: Fetches `limit + offset` per source (not 3x), avoids materializing
    /// full HashMaps, uses dedup with HashSet for efficiency.
    #[allow(
        clippy::too_many_arguments,
        reason = "parameters mirror the MemoryStore search API"
    )]
    pub fn search_all(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        offset: usize,
        include_docs: bool,
        project: Option<&str>,
        code_expand_project: Option<&str>,
    ) -> HyphaeResult<Vec<UnifiedSearchResult>> {
        self.search_all_impl(
            query,
            embedding,
            limit,
            offset,
            include_docs,
            project,
            None,
            false,
            code_expand_project,
        )
    }

    /// Search across memories and document chunks using Reciprocal Rank Fusion.
    ///
    /// This variant scopes memory results to a specific worktree when `worktree`
    /// is supplied. Document chunks remain project-scoped because the chunk store
    /// does not track worktree identity yet.
    #[allow(
        clippy::too_many_arguments,
        reason = "parameters mirror the MemoryStore search API"
    )]
    pub fn search_all_scoped(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        offset: usize,
        include_docs: bool,
        project: Option<&str>,
        worktree: Option<&str>,
        code_expand_project: Option<&str>,
    ) -> HyphaeResult<Vec<UnifiedSearchResult>> {
        self.search_all_impl(
            query,
            embedding,
            limit,
            offset,
            include_docs,
            project,
            worktree,
            true,
            code_expand_project,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "shared internal implementation for the public search helpers"
    )]
    fn search_all_impl(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        offset: usize,
        include_docs: bool,
        project: Option<&str>,
        worktree: Option<&str>,
        merge_shared_memories: bool,
        code_expand_project: Option<&str>,
    ) -> HyphaeResult<Vec<UnifiedSearchResult>> {
        const K: f32 = 60.0;
        const EXPANDED_WEIGHT: f32 = 0.5;
        let pool = limit + offset;

        let mem_results: Vec<(Memory, f32)> = if let Some(emb) = embedding {
            if let Some(worktree) = worktree {
                self.search_hybrid_scoped(query, emb, pool, 0, project, Some(worktree))?
            } else {
                self.search_hybrid(query, emb, pool, 0, project)?
            }
        } else {
            if let Some(worktree) = worktree {
                self.search_fts_scoped(query, pool, 0, project, Some(worktree))?
            } else {
                self.search_fts(query, pool, 0, project)?
            }
            .into_iter()
            .enumerate()
            .map(|(i, m)| (m, 1.0 / (K + i as f32)))
            .collect()
        };

        let mut combined_mem_results: Vec<(Memory, f32)> = mem_results;
        let mut seen_mem_ids: HashSet<String> = combined_mem_results
            .iter()
            .map(|(mem, _)| mem.id.to_string())
            .collect();
        let should_merge_shared =
            merge_shared_memories && worktree.is_some() && project != Some(SHARED_PROJECT);
        if should_merge_shared {
            let shared_mem_results: Vec<(Memory, f32)> = if let Some(emb) = embedding {
                self.search_hybrid(query, emb, pool, 0, Some(SHARED_PROJECT))?
            } else {
                self.search_fts(query, pool, 0, Some(SHARED_PROJECT))?
                    .into_iter()
                    .enumerate()
                    .map(|(i, m)| (m, 1.0 / (K + i as f32)))
                    .collect()
            };
            for (mem, score) in shared_mem_results {
                let id = mem.id.to_string();
                if seen_mem_ids.insert(id) {
                    combined_mem_results.push((mem, score));
                }
            }
        }

        // Fetch chunk results once, keep them for later use
        let chunk_search_results = if include_docs {
            if let Some(emb) = embedding {
                self.search_chunks_hybrid(query, emb, pool, 0, project)?
            } else {
                self.search_chunks_fts(query, pool, 0, project)?
            }
        } else {
            Vec::new()
        };

        // ─────────────────────────────────────────────────────────────────────────
        // RRF scoring and deduplication
        // ─────────────────────────────────────────────────────────────────────────

        let mut scores: HashMap<String, f32> = HashMap::new();
        let mut memory_map: HashMap<String, Memory> = HashMap::new();

        // Score memories from primary search
        for (rank, (mem, _original_score)) in combined_mem_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + rank as f32);
            let key = format!("mem:{}", mem.id);
            *scores.entry(key.clone()).or_default() += rrf;
            memory_map.insert(key, mem);
        }

        // Score chunks from primary search
        for (rank, csr) in chunk_search_results.iter().enumerate() {
            let rrf = 1.0 / (K + rank as f32);
            let key = format!("chunk:{}", csr.chunk.id);
            *scores.entry(key).or_default() += rrf;
        }

        // ─────────────────────────────────────────────────────────────────────────
        // Optional code-context expansion with deduplication
        // ─────────────────────────────────────────────────────────────────────────

        if let Some(expand_project) = code_expand_project {
            if context::is_code_related(query) {
                let extra_terms = context::expand_with_code_context(self, query, expand_project);
                if !extra_terms.is_empty() {
                    let expanded_query = extra_terms
                        .iter()
                        .map(|t| sanitize_fts_query(t))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" OR ");

                    if !expanded_query.is_empty() {
                        let expanded_mems = if let Some(worktree) = worktree {
                            self.search_fts_scoped(
                                &expanded_query,
                                pool,
                                0,
                                project,
                                Some(worktree),
                            )
                            .unwrap_or_default()
                        } else {
                            self.search_fts(&expanded_query, pool, 0, project)
                                .unwrap_or_default()
                        };

                        for (rank, mem) in expanded_mems.into_iter().enumerate() {
                            let id = mem.id.to_string();
                            // Only score if not already in primary results
                            if !seen_mem_ids.contains(&id) {
                                let rrf = EXPANDED_WEIGHT / (K + rank as f32);
                                let key = format!("mem:{id}");
                                *scores.entry(key.clone()).or_default() += rrf;
                                memory_map.insert(key, mem);
                            }
                        }
                    }
                }
            }
        }

        // ─────────────────────────────────────────────────────────────────────────
        // Sort, apply offset/limit, and collect results
        // ─────────────────────────────────────────────────────────────────────────

        let mut ranked: Vec<(String, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Build chunk map for result lookup only
        let mut chunk_map: HashMap<String, Chunk> = HashMap::new();
        for csr in chunk_search_results {
            let key = format!("chunk:{}", csr.chunk.id);
            chunk_map.insert(key, csr.chunk);
        }

        let results = ranked
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(key, score)| {
                if let Some(mem) = memory_map.remove(&key) {
                    Some(UnifiedSearchResult::Memory {
                        memory: Box::new(mem),
                        score,
                    })
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
            project: None,
            runtime_session_id: None,
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
            .search_all("Rust systems programming", None, 10, 0, false, None, None)
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
        let results = store
            .search_all("SQLite database", None, 10, 0, true, None, None)
            .unwrap();

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

        let results = store
            .search_all("SQLite", None, 10, 0, true, None, None)
            .unwrap();

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
    fn test_search_all_scoped_keeps_chunks_project_scoped_and_merges_shared_memories() {
        let store = test_store();

        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Alpha scoped target".to_string(),
                    hyphae_core::Importance::Medium,
                )
                .project("demo".to_string())
                .worktree("/repo/demo".to_string())
                .build(),
            )
            .unwrap();
        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Beta other worktree target".to_string(),
                    hyphae_core::Importance::Medium,
                )
                .project("demo".to_string())
                .worktree("/repo/other".to_string())
                .build(),
            )
            .unwrap();
        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Shared scoped target".to_string(),
                    hyphae_core::Importance::Medium,
                )
                .project(crate::SHARED_PROJECT.to_string())
                .build(),
            )
            .unwrap();

        let mut doc = make_doc("docs/scoped.md");
        doc.project = Some("demo".to_string());
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![make_chunk(
                &doc_id,
                "Scoped target document chunk",
                "docs/scoped.md",
            )])
            .unwrap();

        let results = store
            .search_all_scoped(
                "target",
                None,
                10,
                0,
                true,
                Some("demo"),
                Some("/repo/demo"),
                None,
            )
            .unwrap();

        let has_alpha = results.iter().any(|r| match r {
            UnifiedSearchResult::Memory { memory, .. } => memory.summary.contains("Alpha scoped"),
            _ => false,
        });
        let has_shared = results.iter().any(|r| match r {
            UnifiedSearchResult::Memory { memory, .. } => memory.summary.contains("Shared scoped"),
            _ => false,
        });
        let has_beta = results.iter().any(|r| match r {
            UnifiedSearchResult::Memory { memory, .. } => memory.summary.contains("Beta other"),
            _ => false,
        });
        let has_chunk = results
            .iter()
            .any(|r| matches!(r, UnifiedSearchResult::Chunk { .. }));

        assert!(has_alpha, "worktree-scoped memory should be included");
        assert!(has_shared, "_shared memories should still be included");
        assert!(has_chunk, "document chunks should remain project-scoped");
        assert!(
            !has_beta,
            "other worktrees should not leak into scoped memory results"
        );
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

        let results = store
            .search_all("cargo test", None, 10, 0, false, None, None)
            .unwrap();

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
        let results = store
            .search_all("anything", None, 10, 0, true, None, None)
            .unwrap();
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

        let results = store
            .search_all("testing", None, 5, 0, false, None, None)
            .unwrap();

        assert!(results.len() <= 5, "should respect the limit parameter");
    }

    #[test]
    fn test_search_fts_offset_pagination() {
        let store = test_store();
        // Store 10 memories with distinct summaries
        for i in 0..10 {
            store
                .store(Memory::new(
                    "pagination".to_string(),
                    format!("Pagination test memory number {i}"),
                    hyphae_core::Importance::Medium,
                ))
                .unwrap();
        }

        // First page: offset=0, limit=5
        let page1 = store
            .search_fts("pagination test memory", 5, 0, None)
            .unwrap();
        assert_eq!(page1.len(), 5, "first page should have 5 results");

        // Second page: offset=5, limit=5
        let page2 = store
            .search_fts("pagination test memory", 5, 5, None)
            .unwrap();
        assert_eq!(page2.len(), 5, "second page should have 5 results");

        // Verify no overlap: collect IDs from both pages
        let page1_ids: Vec<String> = page1.iter().map(|m| m.id.to_string()).collect();
        let page2_ids: Vec<String> = page2.iter().map(|m| m.id.to_string()).collect();
        for id in &page1_ids {
            assert!(
                !page2_ids.contains(id),
                "pages should not overlap: {id} found in both"
            );
        }
    }

    #[test]
    fn test_search_all_offset_pagination() {
        let store = test_store();
        for i in 0..10 {
            store
                .store(Memory::new(
                    "offset_test".to_string(),
                    format!("Offset pagination memory {i}"),
                    hyphae_core::Importance::Medium,
                ))
                .unwrap();
        }

        let page1 = store
            .search_all("offset pagination", None, 5, 0, false, None, None)
            .unwrap();
        let page2 = store
            .search_all("offset pagination", None, 5, 5, false, None, None)
            .unwrap();

        assert_eq!(page1.len(), 5, "first page should have 5 results");
        assert_eq!(page2.len(), 5, "second page should have 5 results");

        // Extract IDs and verify no overlap
        let get_id = |r: &UnifiedSearchResult| match r {
            UnifiedSearchResult::Memory { memory, .. } => memory.id.to_string(),
            UnifiedSearchResult::Chunk { chunk, .. } => chunk.id.to_string(),
        };
        let ids1: Vec<String> = page1.iter().map(get_id).collect();
        let ids2: Vec<String> = page2.iter().map(get_id).collect();
        for id in &ids1 {
            assert!(
                !ids2.contains(id),
                "pages should not overlap: {id} found in both"
            );
        }
    }

    #[test]
    fn test_search_all_code_context_expansion() {
        use hyphae_core::memoir::{Concept, Memoir};
        use hyphae_core::{MemoirStore, ids::MemoirId};

        let store = test_store();

        // Store a memory that mentions verify_token explicitly so expanded FTS can find it
        store
            .store(Memory::new(
                "refactoring".to_string(),
                "Refactored verify_token to improve performance in the auth pipeline".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        // Store an unrelated memory to verify selectivity
        store
            .store(Memory::new(
                "other".to_string(),
                "Database migration completed successfully".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        // Create a code memoir with a verify_token concept
        let memoir = Memoir::new(
            "code:myapp".to_string(),
            "Code symbols for myapp".to_string(),
        );
        let memoir_id: MemoirId = store.create_memoir(memoir).unwrap();
        let concept = Concept::new(
            memoir_id,
            "verify_token".to_string(),
            "Validates JWT tokens for the auth pipeline".to_string(),
        );
        store.add_concept(concept).unwrap();

        // Query using snake_case (is_code_related = true), expansion finds verify_token concept
        // which in turn finds the memory containing "verify_token"
        let results = store
            .search_all(
                "auth_pipeline performance",
                None,
                10,
                0,
                false,
                None,
                Some("myapp"),
            )
            .unwrap();

        // The expanded FTS for "verify_token" should surface the memory about verify_token
        assert!(!results.is_empty(), "should find at least one memory");
        let found = results.iter().any(|r| {
            if let UnifiedSearchResult::Memory { memory, .. } = r {
                memory.summary.contains("verify_token")
            } else {
                false
            }
        });
        assert!(
            found,
            "should find the verify_token memory via expanded code context; got: {results:?}"
        );
    }

    #[test]
    fn test_search_all_code_context_none_no_expansion() {
        let store = test_store();

        store
            .store(Memory::new(
                "test".to_string(),
                "Some test memory".to_string(),
                hyphae_core::Importance::Medium,
            ))
            .unwrap();

        // With code_expand_project = None, should behave the same as before
        let results = store
            .search_all("test memory", None, 10, 0, false, None, None)
            .unwrap();

        assert!(
            !results.is_empty(),
            "should still find memories without code expansion"
        );
    }
}
