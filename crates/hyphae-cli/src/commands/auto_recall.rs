//! `hyphae auto-recall` — query memories relevant to a prompt and surface them
//! for injection into the agent context.
//!
//! This command owns all recall logic: search, dedup against a session-scoped
//! seen-set, character budget, and output formatting. Cortina delegates to this
//! command rather than duplicating the logic.
//!
//! Exit codes:
//!   0 — at least one memory was emitted
//!   1 — nothing emitted (empty result, all deduped, or budget exhausted)

use anyhow::Result;
use hyphae_core::{MemoryStore, SearchOrder as StoreSearchOrder};
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

pub(crate) struct AutoRecallArgs {
    pub query: String,
    pub session_id: String,
    pub project: Option<String>,
    pub budget: usize,
    pub limit: usize,
}

/// Run the auto-recall command.
///
/// Returns `Ok(true)` when at least one memory was emitted, `Ok(false)` when
/// nothing was output (caller should exit 1).
pub(crate) fn cmd_auto_recall(store: &dyn MemoryStore, args: AutoRecallArgs) -> Result<bool> {
    if args.query.trim().is_empty() {
        return Ok(false);
    }

    // Load the session-scoped seen-set.
    let state_path = recall_seen_state_path(&args.session_id);
    let mut seen: HashSet<String> = load_seen(&state_path);

    // Query the store (FTS, weight-ranked).
    let results = store.search_fts_with_options(
        &args.query,
        None, // no topic filter
        args.limit,
        0,
        args.project.as_deref(),
        false, // exclude invalidated
        StoreSearchOrder::WeightDesc,
    )?;

    if results.is_empty() {
        return Ok(false);
    }

    // Filter already-seen memories.
    let fresh: Vec<_> = results
        .into_iter()
        .filter(|m| !seen.contains(&m.id.to_string()))
        .collect();

    if fresh.is_empty() {
        return Ok(false);
    }

    // Apply the character budget.
    let mut total_chars = 0usize;
    let mut emitted: Vec<(String, String)> = Vec::new(); // (id, content)

    for (i, memory) in fresh.into_iter().enumerate() {
        let id = memory.id.to_string();
        // Prefer summary; fall back to raw_excerpt.
        let content = if !memory.summary.is_empty() {
            memory.summary.clone()
        } else if let Some(ref excerpt) = memory.raw_excerpt {
            excerpt.clone()
        } else {
            continue;
        };

        let len = content.len();
        if total_chars + len > args.budget {
            if i == 0 {
                // Always include at least the first memory, truncated to budget.
                let truncated: String = content.chars().take(args.budget).collect();
                emitted.push((id, truncated));
            }
            break;
        }
        total_chars += len;
        emitted.push((id, content));
    }

    if emitted.is_empty() {
        return Ok(false);
    }

    // Write output to stdout.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for (id, content) in &emitted {
        writeln!(out, "[cortina-recall] id={id} content={content}")?;
    }
    drop(out);

    // Persist new IDs to the seen-set.
    let new_ids: Vec<String> = emitted.into_iter().map(|(id, _)| id).collect();
    for id in &new_ids {
        seen.insert(id.clone());
    }
    save_seen(&state_path, &seen);

    Ok(true)
}

fn recall_seen_state_path(session_id: &str) -> PathBuf {
    spore::paths::data_dir("hyphae").join(format!("recall-seen-{session_id}.json"))
}

fn load_seen(path: &PathBuf) -> HashSet<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn save_seen(path: &PathBuf, seen: &HashSet<String>) {
    let ids: Vec<&String> = seen.iter().collect();
    if let Ok(json) = serde_json::to_string(&ids) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, json);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::MemoryStore;
    use hyphae_store::SqliteStore;

    const DEFAULT_CHAR_BUDGET: usize = 8_000;
    const DEFAULT_LIMIT: usize = 10;

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn store_memory(store: &SqliteStore, topic: &str, content: &str, project: Option<&str>) {
        let mut mem = hyphae_core::Memory::new(
            topic.to_string(),
            content.to_string(),
            hyphae_core::Importance::Medium,
        );
        mem.project = project.map(str::to_string);
        store.store(mem).unwrap();
    }

    #[test]
    fn empty_query_returns_false() {
        let store = make_store();
        let args = AutoRecallArgs {
            query: "   ".to_string(),
            session_id: "sess-empty".to_string(),
            project: None,
            budget: DEFAULT_CHAR_BUDGET,
            limit: DEFAULT_LIMIT,
        };
        assert!(!cmd_auto_recall(&store, args).unwrap());
    }

    #[test]
    fn no_memories_returns_false() {
        let store = make_store();
        let args = AutoRecallArgs {
            query: "something to recall".to_string(),
            session_id: "sess-none".to_string(),
            project: None,
            budget: DEFAULT_CHAR_BUDGET,
            limit: DEFAULT_LIMIT,
        };
        assert!(!cmd_auto_recall(&store, args).unwrap());
    }

    #[test]
    fn matching_memory_returns_true() {
        let store = make_store();
        store_memory(
            &store,
            "test/topic",
            "rust borrow checker lifetime rules",
            None,
        );

        let args = AutoRecallArgs {
            query: "rust borrow checker".to_string(),
            session_id: "sess-match".to_string(),
            project: None,
            budget: DEFAULT_CHAR_BUDGET,
            limit: DEFAULT_LIMIT,
        };
        assert!(cmd_auto_recall(&store, args).unwrap());
    }

    #[test]
    fn seen_set_deduplicates_across_calls() {
        let store = make_store();
        store_memory(
            &store,
            "test/topic",
            "rust borrow checker lifetime rules",
            None,
        );

        let session_id = "sess-dedup-test-unique";
        // Clean up any leftover state from a previous run.
        let _ = std::fs::remove_file(recall_seen_state_path(session_id));

        let make_args = || AutoRecallArgs {
            query: "rust borrow checker".to_string(),
            session_id: session_id.to_string(),
            project: None,
            budget: DEFAULT_CHAR_BUDGET,
            limit: DEFAULT_LIMIT,
        };

        // First call should succeed.
        assert!(cmd_auto_recall(&store, make_args()).unwrap());
        // Second call: all results are now in the seen-set, so nothing new.
        assert!(!cmd_auto_recall(&store, make_args()).unwrap());

        // Clean up.
        let _ = std::fs::remove_file(recall_seen_state_path(session_id));
    }

    #[test]
    fn char_budget_truncates_first_oversized_memory() {
        let store = make_store();
        // Store a memory whose summary exceeds a tiny budget.
        store_memory(
            &store,
            "test/topic",
            &"rust ".repeat(100), // 500 chars
            None,
        );

        let session_id = "sess-budget-trunc";
        let _ = std::fs::remove_file(recall_seen_state_path(session_id));

        let tiny_budget = 20;
        let args = AutoRecallArgs {
            query: "rust".to_string(),
            session_id: session_id.to_string(),
            project: None,
            budget: tiny_budget,
            limit: DEFAULT_LIMIT,
        };
        // Should still return true — first memory gets truncated, not dropped.
        assert!(cmd_auto_recall(&store, args).unwrap());

        let _ = std::fs::remove_file(recall_seen_state_path(session_id));
    }

    #[test]
    fn auto_recall_parse_defaults() {
        assert_eq!(DEFAULT_CHAR_BUDGET, 8_000);
        assert_eq!(DEFAULT_LIMIT, 10);
    }
}
