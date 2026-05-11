use anyhow::Result;
use hyphae_core::{Importance, Memory, MemoryId, MemoryStore};
use hyphae_store::SqliteStore;

/// List constitution memories, optionally filtered to a project.
pub fn cmd_list(store: &SqliteStore, project: Option<&str>) -> Result<()> {
    // Constitution memories live in topics that start with "constitution"
    // or carry `Importance::Constitution` regardless of topic. We query all
    // active memories and filter by importance.
    let topics = store.list_topics(project)?;

    let mut found = false;
    for (topic, _) in &topics {
        let memories = store.get_by_topic(topic, project)?;
        for mem in memories {
            if mem.importance == Importance::Constitution {
                if !found {
                    println!("Constitution memories:");
                    found = true;
                }
                println!(
                    "  [{}] topic={} | {}",
                    mem.id, mem.topic, mem.summary
                );
                if let Some(p) = &mem.project {
                    println!("    project: {p}");
                }
            }
        }
    }

    if !found {
        println!("No constitution memories found.");
    }

    Ok(())
}

/// Store a new constitution memory.
pub fn cmd_add(
    store: &SqliteStore,
    content: &str,
    topic: Option<&str>,
    project: Option<&str>,
) -> Result<()> {
    let resolved_topic = topic
        .map(str::to_owned)
        .unwrap_or_else(|| match project {
            Some(p) => format!("constitution/{p}"),
            None => "constitution".to_string(),
        });

    let mut builder =
        Memory::builder(resolved_topic.clone(), content.to_string(), Importance::Constitution);

    if let Some(p) = project {
        builder = builder.project(p.to_string());
    }

    let memory = builder.build();
    let id = store.store(memory)?;

    println!("Stored constitution policy: {id}");
    println!("  topic: {resolved_topic}");
    println!("  This memory will never decay and is excluded from consolidation.");

    Ok(())
}

/// Remove a constitution memory by ID.
///
/// Allows permanent deletion of a governance policy that is no longer
/// applicable. This is a destructive action; use with care.
pub fn cmd_remove(store: &SqliteStore, id: &str) -> Result<()> {
    // Verify the memory exists and is actually a constitution memory before
    // deleting, so callers cannot accidentally remove non-constitution memories
    // via this path.
    let memory_id = MemoryId::from(id);
    match store.get(&memory_id)? {
        None => anyhow::bail!("memory not found: {id}"),
        Some(mem) if mem.importance != Importance::Constitution => {
            anyhow::bail!(
                "memory {id} is not a constitution memory (importance: {}); \
                 use `hyphae memory forget` to delete regular memories",
                mem.importance
            )
        }
        Some(_) => {}
    }

    store.delete(&memory_id)?;
    println!("Removed constitution memory: {id}");
    Ok(())
}
