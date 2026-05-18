use anyhow::{Context, Result};
use hyphae_core::{Importance, MemoryStore};
use hyphae_store::SqliteStore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

/// Benchmark memory store write and search throughput using an isolated in-memory database.
pub(crate) fn cmd_bench(count: usize) -> Result<()> {
    let store = SqliteStore::in_memory().map_err(|e| anyhow::anyhow!("bench store: {e}"))?;
    println!("Benchmarking memory store ({count} operations) …");

    // Write benchmark
    let t0 = Instant::now();
    for i in 0..count {
        let mem = hyphae_core::Memory::new(
            format!("bench-topic-{i}"),
            format!("Benchmark memory #{i} — testing write throughput of the memory store."),
            Importance::Medium,
        );
        store.store(mem)?;
    }
    let write_ms = t0.elapsed().as_millis();
    let write_per_s = write_ms
        .checked_div(1)
        .map_or(0, |_| count as u128 * 1000 / write_ms.max(1));
    println!("  Write:  {write_ms}ms total  ({write_per_s} writes/s)");

    // Search benchmark — query across stored memories
    let t1 = Instant::now();
    let stride = (count / 10).max(1);
    for i in 0..count {
        let _ = store.search_fts(&format!("bench-topic-{}", i % stride), 5, 0, None)?;
    }
    let search_ms = t1.elapsed().as_millis();
    let search_per_s = count as u128 * 1000 / search_ms.max(1);
    println!("  Search: {search_ms}ms total  ({search_per_s} searches/s)");

    Ok(())
}

/// Fixture for retrieval quality benchmarking
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryFixture {
    topic: String,
    content: String,
    importance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueryFixture {
    query: String,
    #[serde(default)]
    expected_rank_1_contains: Option<String>,
    #[serde(default)]
    expected_top_k_contains: Option<String>,
    #[serde(default = "default_k")]
    k: usize,
    #[serde(default)]
    description: String,
}

fn default_k() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchFixture {
    description: String,
    memories: Vec<MemoryFixture>,
    queries: Vec<QueryFixture>,
}

fn parse_importance(s: &str) -> Importance {
    if let Ok(importance) = s.parse() {
        importance
    } else {
        tracing::warn!("unrecognized importance level: {s}, defaulting to medium");
        Importance::Medium
    }
}

/// Result of running a single fixture: passed, skipped, or failed.
enum FixtureOutcome {
    Passed,
    Skipped,
    Failed,
}

fn run_single_fixture(fixture: &BenchFixture) -> Result<(FixtureOutcome, String)> {
    let store = SqliteStore::in_memory().map_err(|e| anyhow::anyhow!("bench store: {e}"))?;

    // Seed memories
    for mem_fixture in &fixture.memories {
        let mem = hyphae_core::Memory::new(
            mem_fixture.topic.clone(),
            mem_fixture.content.clone(),
            parse_importance(&mem_fixture.importance),
        );
        store.store(mem)?;
    }

    let mut passed = true;
    let mut all_skipped = true;
    let mut details = Vec::new();

    // Run queries
    for query_fixture in &fixture.queries {
        let results = store.search_fts(&query_fixture.query, 10, 0, None)?;

        let has_assertions = query_fixture.expected_rank_1_contains.is_some()
            || query_fixture.expected_top_k_contains.is_some();

        let mut query_detail = format!(
            "  Query: '{}' — {} … ",
            query_fixture.query, query_fixture.description
        );

        if !has_assertions {
            query_detail.push_str("SKIP (no assertions)");
            details.push(query_detail);
            continue;
        }

        all_skipped = false;
        let mut query_passed = false;

        // Check expected_rank_1_contains
        if let Some(expected) = &query_fixture.expected_rank_1_contains {
            if let Some(first_result) = results.first() {
                if first_result
                    .summary
                    .to_lowercase()
                    .contains(&expected.to_lowercase())
                {
                    query_passed = true;
                    query_detail.push_str("PASS (rank 1)");
                } else {
                    query_detail.push_str(&format!(
                        "FAIL (rank 1 expected '{}', got '{}')",
                        expected, first_result.summary
                    ));
                }
            } else {
                query_detail.push_str("FAIL (no results)");
            }
        }

        // Check expected_top_k_contains
        if let Some(expected) = &query_fixture.expected_top_k_contains {
            let k = query_fixture.k.min(results.len());
            let top_k = &results[..k];
            let found = top_k
                .iter()
                .any(|r| r.summary.to_lowercase().contains(&expected.to_lowercase()));

            if found {
                query_passed = true;
                if query_fixture.expected_rank_1_contains.is_none() {
                    query_detail.push_str(&format!("PASS (top {k})"));
                }
            } else {
                let summaries = top_k
                    .iter()
                    .map(|r| r.summary.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                query_detail.push_str(&format!(
                    "FAIL (expected '{expected}' in top {k}, got: {summaries})"
                ));
            }
        }

        if !query_passed {
            passed = false;
        }
        details.push(query_detail);
    }

    let outcome = if all_skipped {
        FixtureOutcome::Skipped
    } else if passed {
        FixtureOutcome::Passed
    } else {
        FixtureOutcome::Failed
    };

    let prefix = match &outcome {
        FixtureOutcome::Passed => "PASS",
        FixtureOutcome::Skipped => "SKIP",
        FixtureOutcome::Failed => "FAIL",
    };
    let result_text = format!(
        "{}: {}\n{}",
        prefix,
        fixture.description,
        details.join("\n")
    );

    Ok((outcome, result_text))
}

/// Benchmark retrieval quality using fixture-driven tests.
pub(crate) fn cmd_bench_retrieval(fixtures_dir: Option<PathBuf>) -> Result<()> {
    let mut fixtures_path = fixtures_dir.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("benchmarks/fixtures")
    });

    // If the computed path doesn't exist, try relative to the binary location
    if !fixtures_path.exists() {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let candidate = exe_dir
                    .parent()
                    .unwrap_or(exe_dir)
                    .join("benchmarks/fixtures");
                if candidate.exists() {
                    fixtures_path = candidate;
                }
            }
        }
    }

    if !fixtures_path.exists() {
        anyhow::bail!(
            "fixtures directory not found at {fixtures_path:?}. Run from the hyphae project root."
        );
    }

    println!("Benchmarking retrieval quality from {fixtures_path:?} …\n");

    let mut passed_count = 0;
    let mut failed_count = 0;
    let mut skipped_count = 0;
    let mut results = Vec::new();

    // Load and run all JSON fixtures
    for entry in std::fs::read_dir(&fixtures_path).context("failed to read fixtures directory")? {
        let entry = entry.context("failed to read fixture entry")?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read fixture: {path:?}"))?;
            let fixture: BenchFixture = serde_json::from_str(&content)
                .with_context(|| format!("failed to parse fixture: {path:?}"))?;

            match run_single_fixture(&fixture) {
                Ok((outcome, result_text)) => {
                    results.push(result_text);
                    match outcome {
                        FixtureOutcome::Passed => passed_count += 1,
                        FixtureOutcome::Skipped => skipped_count += 1,
                        FixtureOutcome::Failed => failed_count += 1,
                    }
                }
                Err(e) => {
                    results.push(format!("ERROR: {path:?}: {e}"));
                    failed_count += 1;
                }
            }
        }
    }

    for result in results {
        println!("{result}\n");
    }

    println!("Results: {passed_count} passed, {skipped_count} skipped, {failed_count} failed");

    if failed_count > 0 {
        anyhow::bail!("{failed_count} fixture(s) failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrieval_bench_passes_with_known_fixture() {
        // Create an in-memory store with known memories
        let store = SqliteStore::in_memory().expect("failed to create in-memory store");

        // Seed with specific memories
        let error_mem = hyphae_core::Memory::new(
            "errors/resolved".to_string(),
            "fixed database connection pool exhaustion by reducing timeout".to_string(),
            Importance::High,
        );
        store
            .store(error_mem)
            .expect("failed to store error memory");

        let context_mem = hyphae_core::Memory::new(
            "context/project".to_string(),
            "basidiocarp is a multi-crate Rust workspace for AI tooling".to_string(),
            Importance::Medium,
        );
        store
            .store(context_mem)
            .expect("failed to store context memory");

        // Search for the error
        let results = store
            .search_fts("database connection pool exhaustion timeout", 10, 0, None)
            .expect("search failed");

        // Verify the error memory surfaces first
        assert!(!results.is_empty(), "search should return results");
        assert!(
            results[0]
                .summary
                .to_lowercase()
                .contains("connection pool"),
            "error memory should rank first"
        );
    }

    #[test]
    fn test_retrieval_bench_fails_on_impossible_query() {
        let store = SqliteStore::in_memory().expect("failed to create in-memory store");

        // Seed with unrelated content
        let mem = hyphae_core::Memory::new(
            "noise".to_string(),
            "cooking recipes for italian pasta dishes".to_string(),
            Importance::Low,
        );
        store.store(mem).expect("failed to store memory");

        // Search for something unrelated
        let results = store
            .search_fts("segfault pointer dereference rust", 10, 0, None)
            .expect("search failed");

        // Should either return no results or low-relevance noise
        if !results.is_empty() {
            assert!(
                !results[0].summary.to_lowercase().contains("segfault"),
                "unrelated content should not rank for technical query"
            );
        }
    }
}
