use anyhow::{Context, Result, bail};
use chrono::Utc;
use hyphae_core::MemoirStore;
use hyphae_store::{
    ArchiveFilter, ArchiveIdentity, ArchiveMemoirConceptRecord, ArchiveMemoirLinkRecord,
    ArchiveMemoirRecord, ArchiveMemoryRecord, ArchiveSessionRecord, HyphaeArchive, SqliteStore,
};
use std::fs;
use std::path::PathBuf;

pub(crate) fn cmd_export(
    store: &SqliteStore,
    output: PathBuf,
    project: Option<String>,
    topic: Option<String>,
    since: Option<String>,
    until: Option<String>,
    include_memoirs: bool,
    include_sessions: bool,
    min_weight: Option<f32>,
    pretty: bool,
    overwrite: bool,
) -> Result<()> {
    // Check overwrite guard
    if output.exists() && !overwrite {
        bail!(
            "File already exists at {} (use --overwrite to replace)",
            output.display()
        );
    }

    // Create parent directory if needed
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    // Query memories
    let memories = store
        .export_memories_for_archive(
            project.as_deref(),
            topic.as_deref(),
            since.as_deref(),
            until.as_deref(),
            min_weight,
        )
        .context("failed to query memories for archive")?;

    // Convert memories to archive records
    let memory_records: Vec<ArchiveMemoryRecord> = memories
        .iter()
        .map(|mem| {
            let keywords = if mem.keywords.is_empty() {
                None
            } else {
                Some(mem.keywords.join(","))
            };

            ArchiveMemoryRecord {
                id: mem.id.to_string(),
                topic: mem.topic.clone(),
                content: mem.summary.clone(),
                importance: mem.importance.to_string(),
                keywords,
                project: mem.project.clone(),
                weight: Some(mem.weight.value()),
                created_at: mem.created_at.to_rfc3339(),
                updated_at: Some(mem.updated_at.to_rfc3339()),
            }
        })
        .collect();

    // Query memoirs if requested
    let mut memoir_records = Vec::new();
    if include_memoirs {
        let memoirs = store
            .list_memoirs()
            .context("failed to query memoirs")?;

        for memoir in memoirs {
            let concepts = store
                .list_concepts(&memoir.id)
                .context("failed to query concepts")?;

            let concept_records: Vec<ArchiveMemoirConceptRecord> = concepts
                .iter()
                .map(|concept| ArchiveMemoirConceptRecord {
                    id: concept.id.to_string(),
                    name: concept.name.clone(),
                    definition: concept.definition.clone(),
                })
                .collect();

            // Get all links from each concept
            let mut link_records = Vec::new();
            for concept in &concepts {
                let outgoing = store
                    .get_links_from(&concept.id)
                    .context("failed to query links")?;

                for link in outgoing {
                    link_records.push(ArchiveMemoirLinkRecord {
                        from_id: link.source_id.to_string(),
                        to_id: link.target_id.to_string(),
                        relationship: link.relation.to_string(),
                    });
                }
            }

            memoir_records.push(ArchiveMemoirRecord {
                id: memoir.id.to_string(),
                name: memoir.name.clone(),
                description: memoir.description.clone(),
                created_at: memoir.created_at.to_rfc3339(),
                updated_at: memoir.updated_at.to_rfc3339(),
                concepts: concept_records,
                links: link_records,
            });
        }
    }

    // Query sessions if requested
    let mut session_records = Vec::new();
    if include_sessions {
        let sessions = store
            .export_sessions_for_archive(
                project.as_deref(),
                since.as_deref(),
                until.as_deref(),
            )
            .context("failed to query sessions")?;

        session_records = sessions
            .iter()
            .map(|session| {
                let files_modified = session
                    .files_modified
                    .as_ref()
                    .and_then(|fm| {
                        if fm.is_empty() {
                            None
                        } else {
                            Some(fm.split(',').map(|s| s.to_string()).collect())
                        }
                    });

                let errors = session
                    .errors
                    .as_ref()
                    .and_then(|e| {
                        if e.is_empty() {
                            None
                        } else {
                            Some(e.split(',').map(|s| s.to_string()).collect())
                        }
                    });

                ArchiveSessionRecord {
                    id: session.id.clone(),
                    project: session.project.clone(),
                    project_root: session.project_root.clone(),
                    worktree_id: session.worktree_id.clone(),
                    task: session.task.clone(),
                    started_at: session.started_at.clone(),
                    ended_at: session.ended_at.clone(),
                    summary: session.summary.clone(),
                    files_modified,
                    errors,
                    status: session.status.clone(),
                }
            })
            .collect();
    }

    // Build filter description
    let filter = ArchiveFilter {
        topic: topic.clone(),
        since: since.clone(),
        importance_minimum: None,
    };

    // Build identity
    let identity = ArchiveIdentity {
        project: project.clone(),
        project_root: None,
        hyphae_version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };

    // Build archive
    let archive = HyphaeArchive {
        schema_version: "1.0".to_string(),
        exported_at: Utc::now().to_rfc3339(),
        identity,
        filter,
        memories: memory_records,
        memoirs: memoir_records,
        sessions: session_records,
    };

    // Serialize
    let serialized = if pretty {
        serde_json::to_string_pretty(&archive).context("failed to serialize archive")?
    } else {
        serde_json::to_string(&archive).context("failed to serialize archive")?
    };

    // Write to file
    fs::write(&output, serialized)
        .with_context(|| format!("failed to write archive to {}", output.display()))?;

    // Print summary to stderr
    eprintln!("Archive exported to {}", output.display());
    eprintln!("  Memories: {}", archive.memories.len());
    eprintln!("  Memoirs: {}", archive.memoirs.len());
    eprintln!("  Sessions: {}", archive.sessions.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::SqliteStore;
    use tempfile::TempDir;

    #[test]
    fn test_cmd_export_refuses_overwrite_without_flag() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let output_path = dir.path().join("archive.json");

        // Create output file
        fs::write(&output_path, "{}").unwrap();

        let store = SqliteStore::new(&db_path).expect("should create store");
        let result = cmd_export(
            &store,
            output_path,
            None,
            None,
            None,
            None,
            false,
            false,
            None,
            false,
            false,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_cmd_export_creates_valid_archive_structure() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let output_path = dir.path().join("archive.json");

        let store = SqliteStore::new(&db_path).expect("should create store");

        let result = cmd_export(
            &store,
            output_path.clone(),
            None,
            None,
            None,
            None,
            false,
            false,
            None,
            false,
            false,
        );

        assert!(result.is_ok(), "export should succeed");
        assert!(output_path.exists(), "output file should exist");

        let content = fs::read_to_string(&output_path).unwrap();
        let obj: serde_json::Value = serde_json::from_str(&content)
            .expect("output should be valid JSON");

        assert_eq!(obj["schema_version"], "1.0");
        assert!(obj["exported_at"].is_string());
        assert!(obj["memories"].is_array());
        assert!(obj["memoirs"].is_array());
        assert!(obj["sessions"].is_array());
    }

    #[test]
    fn test_cmd_export_with_pretty_formatting() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let output_path = dir.path().join("archive.json");

        let store = SqliteStore::new(&db_path).expect("should create store");

        let result = cmd_export(
            &store,
            output_path.clone(),
            None,
            None,
            None,
            None,
            false,
            false,
            None,
            true,
            false,
        );

        assert!(result.is_ok(), "export should succeed");
        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains('\n'), "pretty format should have newlines");
    }
}
