use anyhow::Result;
use hyphae_core::{Importance, MemoryStore};
use hyphae_store::SqliteStore;
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
        .map(|_| count as u128 * 1000 / write_ms.max(1))
        .unwrap_or(0);
    println!("  Write:  {write_ms}ms total  ({write_per_s} writes/s)");

    // Search benchmark — query across stored memories
    let t1 = Instant::now();
    let stride = (count / 10).max(1);
    for i in 0..count {
        let _ = store.search_fts(&format!("bench-topic-{}", i % stride), 5, None)?;
    }
    let search_ms = t1.elapsed().as_millis();
    let search_per_s = count as u128 * 1000 / search_ms.max(1);
    println!("  Search: {search_ms}ms total  ({search_per_s} searches/s)");

    Ok(())
}
