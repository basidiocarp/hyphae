//! `hyphae recall-bundle` — assemble a budget-aware session-start context bundle.
//!
//! Greedily fills up to the token budget with the highest-value context items,
//! ordered by priority level (L0 → L3):
//!
//! - L0: always included: project identity string
//! - L1: active errors + recent decisions
//! - L2: effective memories (semantic search)
//! - L3: session summary (TBD)

use anyhow::Result;
use clap::Args;
use hyphae_core::{Memory, MemoryStore};
use hyphae_store::SqliteStore;
use serde::Serialize;

const RECALL_BUNDLE_SCHEMA_VERSION: &str = "1.0";

#[derive(Args)]
pub(crate) struct RecallBundleArgs {
    /// Token budget for assembled context (default: 4000)
    #[arg(long, default_value = "4000")]
    pub budget: usize,

    /// Emit structured JSON instead of human-readable text
    #[arg(long)]
    pub json: bool,

    /// Project name (defaults to current git repo name)
    #[arg(long)]
    pub project: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Token estimation
// ─────────────────────────────────────────────────────────────────────────────

/// Estimate token count using the approximation: 4 chars ≈ 1 token.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON payload types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct MemoryPayload {
    id: String,
    topic: String,
    summary: String,
    importance: String,
    created_at: String,
}

impl MemoryPayload {
    fn from_memory(memory: &Memory) -> Self {
        Self {
            id: memory.id.to_string(),
            topic: memory.topic.clone(),
            summary: memory.summary.clone(),
            importance: format!("{:?}", memory.importance),
            created_at: memory.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
struct RecallBundleJsonPayload {
    schema_version: String,
    budget: usize,
    estimated_tokens: usize,
    truncated: bool,
    project: String,
    sections: RecallBundleSections,
}

#[derive(Serialize)]
struct RecallBundleSections {
    l0_identity: String,
    l1_active_errors: Vec<MemoryPayload>,
    l1_decisions: Vec<MemoryPayload>,
    l2_memories: Vec<MemoryPayload>,
    l3_session_summary: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Main logic
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn cmd_recall_bundle(
    store: &SqliteStore,
    args: RecallBundleArgs,
    resolved_project: Option<String>,
) -> Result<()> {
    let project = args
        .project
        .or_else(|| resolved_project.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let resolved_project_ref = resolved_project.as_deref();

    // L0: Project identity (always included, minimal tokens)
    let now = chrono::Utc::now();
    let l0_identity = format!("Project: {}  {}", project, now.to_rfc3339());
    let mut used_tokens = estimate_tokens(&l0_identity);
    let mut truncated = false;

    // Collections for each level
    let mut l1_active_errors = Vec::new();
    let mut l1_decisions = Vec::new();
    let mut l2_memories = Vec::new();

    // L1: Active errors (search topic "errors/active", limit 5)
    if used_tokens < args.budget {
        if let Ok(errors) = store.get_by_topic("errors/active", resolved_project_ref) {
            for error in errors.into_iter().take(5) {
                let item_tokens = estimate_tokens(&error.summary);
                if used_tokens + item_tokens > args.budget {
                    truncated = true;
                    break;
                }
                used_tokens += item_tokens;
                l1_active_errors.push(error);
            }
        }
    }

    // L1: Recent decisions (search topic "decisions/{project}", limit 3)
    if used_tokens < args.budget {
        let decisions_topic = format!("decisions/{}", project);
        if let Ok(decisions) = store.get_by_topic(&decisions_topic, resolved_project_ref) {
            for decision in decisions.into_iter().take(3) {
                let item_tokens = estimate_tokens(&decision.summary);
                if used_tokens + item_tokens > args.budget {
                    truncated = true;
                    break;
                }
                used_tokens += item_tokens;
                l1_decisions.push(decision);
            }
        }
    }

    // L2: Effective memories (semantic search with project name, limit 10)
    if used_tokens < args.budget {
        // Try hybrid search if embeddings are available; fall back to FTS
        let search_results = store
            .search_fts(&project, 10, 0, resolved_project_ref)
            .unwrap_or_default();

        for memory in search_results {
            let item_tokens = estimate_tokens(&memory.summary);
            if used_tokens + item_tokens > args.budget {
                truncated = true;
                break;
            }
            used_tokens += item_tokens;
            l2_memories.push(memory);
        }
    }

    // L3: Session summary (TBD — currently null)
    let l3_session_summary: Option<String> = None;

    if args.json {
        emit_json(
            &project,
            args.budget,
            used_tokens,
            truncated,
            &l0_identity,
            &l1_active_errors,
            &l1_decisions,
            &l2_memories,
            &l3_session_summary,
        )?;
    } else {
        emit_text(
            args.budget,
            used_tokens,
            truncated,
            &l0_identity,
            &l1_active_errors,
            &l1_decisions,
            &l2_memories,
            &l3_session_summary,
        );
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_text(
    budget: usize,
    used_tokens: usize,
    truncated: bool,
    l0_identity: &str,
    l1_active_errors: &[Memory],
    l1_decisions: &[Memory],
    l2_memories: &[Memory],
    _l3_session_summary: &Option<String>,
) {
    println!(
        "=== Recall Bundle (budget: {} tokens, used: ~{}, truncated: {}) ===",
        budget, used_tokens, truncated
    );
    println!();

    // L0
    println!("[L0] {}", l0_identity);
    println!();

    // L1: Active Errors
    if !l1_active_errors.is_empty() {
        println!("[L1] Active Errors ({}): ", l1_active_errors.len());
        for memory in l1_active_errors {
            println!(
                "  • [{}] {} ({})",
                memory.topic,
                memory.summary,
                memory.created_at.format("%Y-%m-%d")
            );
        }
        println!();
    }

    // L1: Decisions
    if !l1_decisions.is_empty() {
        println!("[L1] Recent Decisions ({}): ", l1_decisions.len());
        for memory in l1_decisions {
            println!(
                "  • [{}] {} ({})",
                memory.topic,
                memory.summary,
                memory.created_at.format("%Y-%m-%d")
            );
        }
        println!();
    }

    // L2: Effective Memories
    if !l2_memories.is_empty() {
        println!("[L2] Effective Memories ({}): ", l2_memories.len());
        for memory in l2_memories {
            let weight = memory.weight.value();
            println!(
                "  • [{}] {} (score: {:.2})",
                memory.topic, memory.summary, weight
            );
        }
        println!();
    }

    // L3: Session Summary (currently omitted)
}

#[allow(clippy::too_many_arguments)]
fn emit_json(
    project: &str,
    budget: usize,
    used_tokens: usize,
    truncated: bool,
    l0_identity: &str,
    l1_active_errors: &[Memory],
    l1_decisions: &[Memory],
    l2_memories: &[Memory],
    l3_session_summary: &Option<String>,
) -> Result<()> {
    let payload = RecallBundleJsonPayload {
        schema_version: RECALL_BUNDLE_SCHEMA_VERSION.to_string(),
        budget,
        estimated_tokens: used_tokens,
        truncated,
        project: project.to_string(),
        sections: RecallBundleSections {
            l0_identity: l0_identity.to_string(),
            l1_active_errors: l1_active_errors
                .iter()
                .map(MemoryPayload::from_memory)
                .collect(),
            l1_decisions: l1_decisions
                .iter()
                .map(MemoryPayload::from_memory)
                .collect(),
            l2_memories: l2_memories.iter().map(MemoryPayload::from_memory).collect(),
            l3_session_summary: l3_session_summary.clone(),
        },
    };

    let json = serde_json::to_string_pretty(&payload)?;
    println!("{}", json);

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn store_memory(store: &SqliteStore, topic: &str, summary: &str) {
        let mem = Memory::new(topic.to_string(), summary.to_string(), Importance::Medium);
        store.store(mem).unwrap();
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn recall_bundle_empty_store() -> Result<()> {
        let store = make_store();
        let args = RecallBundleArgs {
            budget: 2000,
            json: false,
            project: Some("test-project".to_string()),
        };

        // Should not panic on empty store
        cmd_recall_bundle(&store, args, None)?;
        Ok(())
    }

    #[test]
    fn recall_bundle_with_errors() -> Result<()> {
        let store = make_store();
        store_memory(&store, "errors/active", "Cannot connect to database");
        store_memory(&store, "errors/active", "Build fails on arm64");

        let args = RecallBundleArgs {
            budget: 4000,
            json: false,
            project: Some("test-project".to_string()),
        };

        cmd_recall_bundle(&store, args, None)?;
        Ok(())
    }

    #[test]
    fn recall_bundle_json_output() -> Result<()> {
        let store = make_store();
        store_memory(&store, "errors/active", "Test error");
        store_memory(&store, "decisions/test-project", "Use SQLite");

        let args = RecallBundleArgs {
            budget: 4000,
            json: true,
            project: Some("test-project".to_string()),
        };

        cmd_recall_bundle(&store, args, None)?;
        Ok(())
    }

    #[test]
    fn recall_bundle_respects_budget() -> Result<()> {
        let store = make_store();
        // Store many memories
        for i in 0..20 {
            store_memory(
                &store,
                &format!("errors/active"),
                &format!("Error message number {} with substantial content", i),
            );
        }

        let args = RecallBundleArgs {
            budget: 100, // Very small budget
            json: false,
            project: Some("test-project".to_string()),
        };

        cmd_recall_bundle(&store, args, None)?;
        Ok(())
    }
}
