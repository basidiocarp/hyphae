use anyhow::{Context, Result};
use chrono::DateTime;
use hyphae_core::{
    Importance, Memoir, MemoirId, MemoirStore, Memory, MemoryId, MemoryStore,
};
use hyphae_store::{
    ArchiveMemoirRecord, ArchiveMemoryRecord, ArchiveSessionRecord, HyphaeArchive, SqliteStore,
};
use std::fs;
use std::path::PathBuf;

/// Conflict resolution strategy for existing records.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ConflictStrategy {
    /// Leave existing records unchanged and skip the imported record.
    Skip,
    /// Overwrite the existing record with the imported record.
    Overwrite,
    /// Merge the imported record into the existing record
    /// (union keywords, take max weight, keep earliest created_at).
    Merge,
}

pub(crate) fn cmd_import(
    store: &SqliteStore,
    input: PathBuf,
    on_conflict: ConflictStrategy,
    dry_run: bool,
) -> Result<()> {
    let raw = fs::read_to_string(&input)
        .with_context(|| format!("failed to read archive from {}", input.display()))?;

    let archive: HyphaeArchive =
        serde_json::from_str(&raw).context("failed to deserialize archive JSON")?;

    let mut memories_imported: usize = 0;
    let mut memories_skipped: usize = 0;
    let mut memoirs_imported: usize = 0;
    let mut memoirs_skipped: usize = 0;
    let mut sessions_imported: usize = 0;
    let mut sessions_skipped: usize = 0;

    // ── Memories ──────────────────────────────────────────────────────────────

    for rec in &archive.memories {
        let id: MemoryId = rec.id.clone().into();
        let existing = store
            .get(&id)
            .with_context(|| format!("failed to look up memory {}", rec.id))?;

        match (&on_conflict, existing) {
            (ConflictStrategy::Skip, Some(_)) => {
                if dry_run {
                    eprintln!("[dry-run] skip memory {}", rec.id);
                }
                memories_skipped += 1;
            }

            (ConflictStrategy::Overwrite, Some(_existing)) => {
                if dry_run {
                    eprintln!("[dry-run] overwrite memory {}", rec.id);
                    memories_imported += 1;
                } else {
                    let mem = archive_memory_to_domain(rec)?;
                    // Use replace_memory so that created_at from the archive is preserved.
                    store
                        .replace_memory(mem)
                        .with_context(|| format!("failed to overwrite memory {}", rec.id))?;
                    memories_imported += 1;
                }
            }

            (ConflictStrategy::Merge, Some(mut existing)) => {
                if dry_run {
                    eprintln!("[dry-run] merge memory {}", rec.id);
                    memories_imported += 1;
                } else {
                    // Union keywords
                    let incoming_keywords: Vec<String> = rec
                        .keywords
                        .as_deref()
                        .map(|kw| kw.split(',').map(str::trim).map(String::from).collect())
                        .unwrap_or_default();
                    let mut kw_set: std::collections::HashSet<String> =
                        existing.keywords.iter().cloned().collect();
                    kw_set.extend(incoming_keywords);
                    existing.keywords = kw_set.into_iter().collect();
                    existing.keywords.sort();

                    // Take max weight
                    let incoming_weight = rec.weight.unwrap_or(0.0);
                    if incoming_weight > existing.weight.value() {
                        existing.weight = hyphae_core::Weight::new_clamped(incoming_weight);
                    }

                    // Keep earlier created_at
                    if let Ok(incoming_created) =
                        DateTime::parse_from_rfc3339(&rec.created_at)
                    {
                        let incoming_created = incoming_created.with_timezone(&chrono::Utc);
                        if incoming_created < existing.created_at {
                            existing.created_at = incoming_created;
                        }
                    }

                    // Use replace_memory so that updated created_at is persisted
                    // (the standard update() method does not write created_at).
                    store
                        .replace_memory(existing)
                        .with_context(|| format!("failed to merge memory {}", rec.id))?;
                    memories_imported += 1;
                }
            }

            // No existing record — insert unconditionally for any strategy.
            (_, None) => {
                if dry_run {
                    eprintln!("[dry-run] import memory {}", rec.id);
                    memories_imported += 1;
                } else {
                    let mem = archive_memory_to_domain(rec)?;
                    store
                        .store(mem)
                        .with_context(|| format!("failed to insert memory {}", rec.id))?;
                    memories_imported += 1;
                }
            }
        }
    }

    // ── Memoirs ───────────────────────────────────────────────────────────────

    for rec in &archive.memoirs {
        let id: MemoirId = rec.id.clone().into();
        let existing = store
            .get_memoir(&id)
            .with_context(|| format!("failed to look up memoir {}", rec.id))?;

        match (&on_conflict, existing) {
            (ConflictStrategy::Skip | ConflictStrategy::Merge, Some(_)) => {
                // Memoir merge is deferred; treat as skip for now.
                if dry_run {
                    eprintln!("[dry-run] skip memoir {}", rec.id);
                }
                memoirs_skipped += 1;
            }

            (ConflictStrategy::Overwrite, Some(_)) => {
                if dry_run {
                    eprintln!("[dry-run] overwrite memoir {}", rec.id);
                    memoirs_imported += 1;
                } else {
                    let memoir = archive_memoir_to_domain(rec)?;
                    store
                        .update_memoir(&memoir)
                        .with_context(|| format!("failed to overwrite memoir {}", rec.id))?;
                    memoirs_imported += 1;
                }
            }

            (_, None) => {
                if dry_run {
                    eprintln!("[dry-run] import memoir {}", rec.id);
                    memoirs_imported += 1;
                } else {
                    let memoir = archive_memoir_to_domain(rec)?;
                    store
                        .create_memoir(memoir)
                        .with_context(|| format!("failed to insert memoir {}", rec.id))?;
                    memoirs_imported += 1;
                }
            }
        }
    }

    // ── Sessions ──────────────────────────────────────────────────────────────
    // Sessions are historical records; always skip on conflict.

    for rec in &archive.sessions {
        let exists = store
            .session_exists(&rec.id)
            .with_context(|| format!("failed to look up session {}", rec.id))?;

        if exists {
            if dry_run {
                eprintln!("[dry-run] skip session {} (conflict, sessions always skip)", rec.id);
            }
            sessions_skipped += 1;
        } else if dry_run {
            eprintln!("[dry-run] import session {}", rec.id);
            sessions_imported += 1;
        } else {
            import_session(store, rec)
                .with_context(|| format!("failed to import session {}", rec.id))?;
            sessions_imported += 1;
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────────

    let dry_label = if dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "Import complete{dry_label}: {} memories, {} memoirs, {} sessions imported; {} memories, {} memoirs, {} sessions skipped",
        memories_imported,
        memoirs_imported,
        sessions_imported,
        memories_skipped,
        memoirs_skipped,
        sessions_skipped,
    );

    Ok(())
}

// ── Domain conversion helpers ─────────────────────────────────────────────────

fn archive_memory_to_domain(rec: &ArchiveMemoryRecord) -> Result<Memory> {
    let importance: Importance = rec
        .importance
        .parse()
        .unwrap_or(Importance::Medium);

    let keywords: Vec<String> = rec
        .keywords
        .as_deref()
        .map(|kw| kw.split(',').map(str::trim).map(String::from).collect())
        .unwrap_or_default();

    let created_at = DateTime::parse_from_rfc3339(&rec.created_at)
        .with_context(|| format!("invalid created_at for memory {}", rec.id))?
        .with_timezone(&chrono::Utc);

    let updated_at = DateTime::parse_from_rfc3339(&rec.updated_at)
        .with_context(|| format!("invalid updated_at for memory {}", rec.id))?
        .with_timezone(&chrono::Utc);

    let mut builder = Memory::builder(rec.topic.clone(), rec.content.clone(), importance)
        .keywords(keywords);

    if let Some(project) = &rec.project {
        builder = builder.project(project.clone());
    }

    if let Some(w) = rec.weight {
        builder = builder.weight(w);
    }

    let mut mem = builder.build();

    // Preserve the original ID from the archive.
    mem.id = rec.id.clone().into();
    mem.created_at = created_at;
    mem.updated_at = updated_at;

    Ok(mem)
}

fn archive_memoir_to_domain(rec: &ArchiveMemoirRecord) -> Result<Memoir> {
    let created_at = DateTime::parse_from_rfc3339(&rec.created_at)
        .with_context(|| format!("invalid created_at for memoir {}", rec.id))?
        .with_timezone(&chrono::Utc);

    let updated_at = DateTime::parse_from_rfc3339(&rec.updated_at)
        .with_context(|| format!("invalid updated_at for memoir {}", rec.id))?
        .with_timezone(&chrono::Utc);

    let id: hyphae_core::MemoirId = rec.id.clone().into();

    Ok(Memoir {
        id,
        name: rec.name.clone(),
        description: rec.description.clone(),
        created_at,
        updated_at,
        consolidation_threshold: 50,
    })
}

fn import_session(store: &SqliteStore, rec: &ArchiveSessionRecord) -> Result<()> {
    let files_modified = rec.files_modified.as_ref().map(|v| v.join(","));
    let errors = rec.errors.as_ref().map(|v| v.join(","));

    store
        .import_session_record(
            &rec.id,
            &rec.project,
            rec.project_root.as_deref(),
            rec.worktree_id.as_deref(),
            rec.task.as_deref(),
            &rec.started_at,
            rec.ended_at.as_deref(),
            rec.summary.as_deref(),
            files_modified.as_deref(),
            errors.as_deref(),
            &rec.status,
        )
        .with_context(|| format!("failed to import session {}", rec.id))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::{ArchiveFilter, ArchiveIdentity};
    use std::fs;
    use tempfile::TempDir;

    fn make_store(dir: &TempDir) -> SqliteStore {
        let db_path = dir.path().join("test.db");
        SqliteStore::new(&db_path).expect("should create store")
    }

    fn write_archive(dir: &TempDir, archive: &HyphaeArchive) -> PathBuf {
        let path = dir.path().join("archive.json");
        let json = serde_json::to_string_pretty(archive).expect("serialize archive");
        fs::write(&path, json).expect("write archive");
        path
    }

    fn minimal_archive() -> HyphaeArchive {
        HyphaeArchive {
            schema_version: "1.0".to_string(),
            exported_at: chrono::Utc::now().to_rfc3339(),
            identity: ArchiveIdentity {
                project: None,
                project_root: None,
                hyphae_version: None,
            },
            filter: ArchiveFilter {
                topic: None,
                since: None,
                until: None,
                importance_minimum: None,
            },
            memories: vec![],
            memoirs: vec![],
            sessions: vec![],
        }
    }

    fn sample_memory_record(id: &str) -> ArchiveMemoryRecord {
        ArchiveMemoryRecord {
            id: id.to_string(),
            topic: "decisions/test".to_string(),
            content: "test content".to_string(),
            importance: "medium".to_string(),
            keywords: Some("rust,test".to_string()),
            project: Some("hyphae".to_string()),
            weight: Some(0.7),
            created_at: "2026-04-01T00:00:00Z".to_string(),
            updated_at: "2026-04-01T01:00:00Z".to_string(),
        }
    }

    fn sample_session_record(id: &str) -> ArchiveSessionRecord {
        ArchiveSessionRecord {
            id: id.to_string(),
            project: "hyphae".to_string(),
            project_root: None,
            worktree_id: None,
            task: Some("test task".to_string()),
            started_at: "2026-04-01T00:00:00Z".to_string(),
            ended_at: Some("2026-04-01T01:00:00Z".to_string()),
            summary: Some("completed".to_string()),
            files_modified: None,
            errors: None,
            status: "completed".to_string(),
        }
    }

    // ── skip strategy ─────────────────────────────────────────────────────────

    #[test]
    fn test_import_skip_inserts_new_memory() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut archive = minimal_archive();
        archive.memories.push(sample_memory_record("MEM_SKIP_NEW_01"));

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Skip, false).expect("import should succeed");

        let id: MemoryId = "MEM_SKIP_NEW_01".into();
        let mem = store.get(&id).expect("get should succeed");
        assert!(mem.is_some(), "new memory should have been inserted");
    }

    #[test]
    fn test_import_skip_leaves_existing_memory_unchanged() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        // Pre-insert a memory with the same ID but different content.
        let mut existing = Memory::new(
            "decisions/test".to_string(),
            "original content".to_string(),
            Importance::High,
        );
        existing.id = "MEM_SKIP_EXIST_01".into();
        store.store(existing).expect("pre-insert");

        let mut archive = minimal_archive();
        let mut rec = sample_memory_record("MEM_SKIP_EXIST_01");
        rec.content = "imported content".to_string();
        archive.memories.push(rec);

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Skip, false).expect("import should succeed");

        let id: MemoryId = "MEM_SKIP_EXIST_01".into();
        let mem = store
            .get(&id)
            .expect("get should succeed")
            .expect("memory should still exist");
        assert_eq!(mem.summary, "original content", "skip must not overwrite");
    }

    // ── overwrite strategy ────────────────────────────────────────────────────

    #[test]
    fn test_import_overwrite_replaces_existing_memory() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut existing = Memory::new(
            "decisions/test".to_string(),
            "original content".to_string(),
            Importance::High,
        );
        existing.id = "MEM_OW_01".into();
        store.store(existing).expect("pre-insert");

        let mut archive = minimal_archive();
        let mut rec = sample_memory_record("MEM_OW_01");
        rec.content = "overwritten content".to_string();
        archive.memories.push(rec);

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Overwrite, false).expect("import should succeed");

        let id: MemoryId = "MEM_OW_01".into();
        let mem = store
            .get(&id)
            .expect("get should succeed")
            .expect("memory should exist");
        assert_eq!(
            mem.summary, "overwritten content",
            "overwrite should replace content"
        );
    }

    // ── merge strategy ────────────────────────────────────────────────────────

    #[test]
    fn test_import_merge_unions_keywords() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut existing = Memory::builder(
            "decisions/test".to_string(),
            "original content".to_string(),
            Importance::Medium,
        )
        .keywords(vec!["existing".to_string(), "keyword".to_string()])
        .build();
        existing.id = "MEM_MG_KW_01".into();
        store.store(existing).expect("pre-insert");

        let mut archive = minimal_archive();
        let mut rec = sample_memory_record("MEM_MG_KW_01");
        rec.keywords = Some("rust,merge".to_string());
        archive.memories.push(rec);

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Merge, false).expect("import should succeed");

        let id: MemoryId = "MEM_MG_KW_01".into();
        let mem = store
            .get(&id)
            .expect("get should succeed")
            .expect("memory should exist");

        // After merge, keywords should be the union.
        let kws = &mem.keywords;
        assert!(kws.contains(&"existing".to_string()), "existing keyword preserved");
        assert!(kws.contains(&"keyword".to_string()), "existing keyword preserved");
        assert!(kws.contains(&"rust".to_string()), "imported keyword merged");
        assert!(kws.contains(&"merge".to_string()), "imported keyword merged");
    }

    #[test]
    fn test_import_merge_takes_max_weight() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut existing = Memory::builder(
            "decisions/test".to_string(),
            "original content".to_string(),
            Importance::Medium,
        )
        .weight(0.5)
        .build();
        existing.id = "MEM_MG_WT_01".into();
        store.store(existing).expect("pre-insert");

        let mut archive = minimal_archive();
        let mut rec = sample_memory_record("MEM_MG_WT_01");
        rec.weight = Some(0.9);
        archive.memories.push(rec);

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Merge, false).expect("import should succeed");

        let id: MemoryId = "MEM_MG_WT_01".into();
        let mem = store
            .get(&id)
            .expect("get should succeed")
            .expect("memory should exist");
        assert!(
            (mem.weight.value() - 0.9).abs() < 0.01,
            "merge should take max weight"
        );
    }

    #[test]
    fn test_import_merge_keeps_earlier_created_at() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut existing = Memory::new(
            "decisions/test".to_string(),
            "original content".to_string(),
            Importance::Medium,
        );
        existing.id = "MEM_MG_CA_01".into();
        // Force a newer created_at on the existing record.
        existing.created_at = DateTime::parse_from_rfc3339("2026-04-10T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        store.store(existing).expect("pre-insert");

        let mut archive = minimal_archive();
        let mut rec = sample_memory_record("MEM_MG_CA_01");
        rec.created_at = "2026-04-01T00:00:00Z".to_string(); // earlier
        archive.memories.push(rec);

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Merge, false).expect("import should succeed");

        let id: MemoryId = "MEM_MG_CA_01".into();
        let mem = store
            .get(&id)
            .expect("get should succeed")
            .expect("memory should exist");
        assert_eq!(
            mem.created_at.to_rfc3339(),
            "2026-04-01T00:00:00+00:00",
            "merge should keep the earlier created_at"
        );
    }

    // ── dry-run ───────────────────────────────────────────────────────────────

    #[test]
    fn test_import_dry_run_does_not_write() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut archive = minimal_archive();
        archive.memories.push(sample_memory_record("MEM_DRY_01"));

        let path = write_archive(&dir, &archive);
        cmd_import(&store, path, ConflictStrategy::Skip, true).expect("dry-run should succeed");

        let id: MemoryId = "MEM_DRY_01".into();
        let mem = store.get(&id).expect("get should succeed");
        assert!(mem.is_none(), "dry-run must not write to the store");
    }

    // ── sessions always skip ──────────────────────────────────────────────────

    #[test]
    fn test_import_sessions_skip_on_conflict() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let mut archive = minimal_archive();
        archive.sessions.push(sample_session_record("ses_IMPORT_SKIP_01"));

        // First import inserts the session.
        let path = write_archive(&dir, &archive);
        cmd_import(&store, path.clone(), ConflictStrategy::Overwrite, false)
            .expect("first import should succeed");

        // Second import should skip (sessions always skip on conflict).
        cmd_import(&store, path, ConflictStrategy::Overwrite, false)
            .expect("second import should succeed without error");

        // Only one session should exist.
        let exists = store
            .session_exists("ses_IMPORT_SKIP_01")
            .expect("session_exists should succeed");
        assert!(exists, "session should still exist after second import");
    }

    // ── invalid archive ───────────────────────────────────────────────────────

    #[test]
    fn test_import_fails_on_invalid_json() {
        let dir = TempDir::new().unwrap();
        let store = make_store(&dir);

        let bad_path = dir.path().join("bad.json");
        fs::write(&bad_path, "not valid json").unwrap();

        let result = cmd_import(&store, bad_path, ConflictStrategy::Skip, false);
        assert!(result.is_err(), "import of invalid JSON should fail");
    }
}
