use anyhow::Result;
use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

fn parse_importance(s: &str) -> hyphae_core::Importance {
    match s.parse() {
        Ok(importance) => importance,
        Err(_) => {
            tracing::warn!("unrecognized importance level: {s}, defaulting to medium");
            hyphae_core::Importance::Medium
        }
    }
}

pub(crate) fn cmd_store(
    store: &SqliteStore,
    topic: String,
    content: String,
    importance: &str,
    project: Option<String>,
) -> Result<()> {
    let mut mem = hyphae_core::Memory::new(topic, content, parse_importance(importance));
    mem.project = project;
    store.store(mem)?;
    println!("Memory stored");
    Ok(())
}

pub(crate) fn cmd_search(
    store: &SqliteStore,
    query: String,
    limit: usize,
    project: Option<String>,
) -> Result<()> {
    let results = store.search_fts(&query, limit, project.as_deref())?;
    for mem in &results {
        crate::display::print_memory(mem, None);
    }
    Ok(())
}

pub(crate) fn cmd_stats(store: &SqliteStore, project: Option<String>) -> Result<()> {
    let stats = store.stats(project.as_deref())?;
    println!("Database Statistics:");
    println!("  Total memories: {}", stats.total_memories);
    Ok(())
}
