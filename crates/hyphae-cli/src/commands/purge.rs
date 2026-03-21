use chrono::DateTime;

use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

pub fn cmd_purge(
    store: &SqliteStore,
    project: Option<String>,
    before: Option<String>,
    dry_run: bool,
    _resolved_project: Option<String>,
) -> anyhow::Result<()> {
    if project.is_none() && before.is_none() {
        anyhow::bail!("must specify either --project or --before");
    }

    if let Some(proj) = project {
        // Delete all memories for a project by getting all topics and deleting from each
        let topics = store.list_topics(Some(&proj))?;

        let mut to_delete = Vec::new();
        for (topic_name, _) in topics {
            let memories = store.get_by_topic(&topic_name, Some(&proj))?;
            for memory in memories {
                to_delete.push(memory.id.clone());
            }
        }

        let count = to_delete.len();

        if !dry_run {
            for id in to_delete {
                store.delete(&id)?;
            }
        }

        if dry_run {
            println!(
                "DRY RUN: Would delete {} memories from project '{}'",
                count, proj
            );
        } else {
            println!("Deleted {} memories from project '{}'", count, proj);
        }

        println!("  (Document deletion by project not yet implemented)");
    } else if let Some(before_str) = before {
        // Delete memories created before the given date
        let before_dt = DateTime::parse_from_rfc3339(&format!("{before_str}T00:00:00+00:00"))
            .or_else(|_| {
                // Try YYYY-MM-DD format
                let with_time = format!("{before_str}T00:00:00+00:00");
                DateTime::parse_from_rfc3339(&with_time)
            })
            .map_err(|_| anyhow::anyhow!("invalid date format: {before_str}, use YYYY-MM-DD"))?;

        // Get all topics and all memories
        let topics = store.list_topics(None)?;
        let mut to_delete = Vec::new();

        for (topic_name, _) in topics {
            let memories = store.get_by_topic(&topic_name, None)?;
            for memory in memories {
                if memory.created_at < before_dt.with_timezone(&chrono::Utc) {
                    to_delete.push(memory.id.clone());
                }
            }
        }

        let count = to_delete.len();

        if !dry_run {
            for id in to_delete {
                store.delete(&id)?;
            }
        }

        if dry_run {
            println!(
                "DRY RUN: Would delete {} memories created before {}",
                count, before_str
            );
        } else {
            println!("Deleted {} memories created before {}", count, before_str);
        }
    }

    if dry_run {
        println!("\nTo confirm, run without --dry-run");
    }

    Ok(())
}
