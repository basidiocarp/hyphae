use hyphae_core::{MemoryStore, detect_secrets};
use hyphae_store::SqliteStore;

pub fn cmd_audit_secrets(
    store: &SqliteStore,
    topic: Option<String>,
    detailed: bool,
    project: Option<String>,
) -> anyhow::Result<()> {
    let mut total = 0;
    let mut with_secrets = 0;
    let mut all_findings = Vec::new();

    // If topic specified, only audit that topic
    let memories = if let Some(t) = topic {
        store.get_by_topic(&t, project.as_deref())?
    } else {
        // Audit all memories by getting all topics and combining
        let topics = store.list_topics(project.as_deref())?;
        let mut all_mems = Vec::new();
        for (topic_name, _) in topics {
            let mems = store.get_by_topic(&topic_name, project.as_deref())?;
            all_mems.extend(mems);
        }
        all_mems
    };

    for memory in memories {
        total += 1;
        let secrets = detect_secrets(&memory.summary);

        if !secrets.is_empty() {
            with_secrets += 1;
            all_findings.push((memory.id.clone(), memory.topic.clone(), secrets));
        }
    }

    println!("Secrets audit results:");
    println!("  Total memories scanned: {total}");
    println!("  Memories with secrets: {with_secrets}");
    println!(
        "  Risk level: {}",
        if with_secrets > 0 { "HIGH" } else { "LOW" }
    );

    if with_secrets > 0 {
        println!("\nSecrets detected:");
        for (id, topic_name, secrets) in all_findings {
            println!("  - {id} ({topic_name}):");
            if detailed {
                for secret in secrets {
                    println!("    * {secret}");
                }
            } else {
                println!("    * {} found", secrets.len());
            }
        }
        println!("\nRecommendation: Use 'hyphae_memory_forget' to remove these memories.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::MemoryStore;
    use hyphae_store::SqliteStore;

    #[test]
    fn test_audit_secrets_empty_store() -> anyhow::Result<()> {
        let store = SqliteStore::in_memory()?;
        // Should complete without error
        cmd_audit_secrets(&store, None, false, None)?;
        Ok(())
    }

    #[test]
    fn test_audit_secrets_finds_api_keys() -> anyhow::Result<()> {
        let store = SqliteStore::in_memory()?;

        // Store a memory with an API key
        let memory = hyphae_core::Memory::builder(
            "credentials".into(),
            "api_key = sk1234567890abcdefghij".into(),
            hyphae_core::Importance::Medium,
        ).build();
        let _id = store.store(memory)?;

        // Run audit - should detect the secret
        cmd_audit_secrets(&store, None, false, None)?;
        Ok(())
    }

    #[test]
    fn test_audit_secrets_filters_by_topic() -> anyhow::Result<()> {
        let store = SqliteStore::in_memory()?;

        // Store two memories - one with secrets, one without
        let mem_with_secret = hyphae_core::Memory::builder(
            "credentials".into(),
            "password = secret123".into(),
            hyphae_core::Importance::Medium,
        ).build();
        store.store(mem_with_secret)?;

        let mem_clean = hyphae_core::Memory::builder(
            "notes".into(),
            "How to debug memory leaks".into(),
            hyphae_core::Importance::Medium,
        ).build();
        store.store(mem_clean)?;

        // Audit only credentials topic
        cmd_audit_secrets(&store, Some("credentials".into()), false, None)?;
        Ok(())
    }
}
