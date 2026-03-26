//! `hyphae ingest-sessions` - import Claude Code and Codex session transcripts.

use anyhow::Result;
use hyphae_store::SqliteStore;
use sha2::{Digest, Sha256};
use spore::editors::{self, Editor as SharedEditor};
use std::path::{Path, PathBuf};

pub fn run(
    store: &SqliteStore,
    path: Option<PathBuf>,
    since: Option<String>,
    dry_run: bool,
    project: Option<&str>,
) -> Result<()> {
    let sessions = discover_sessions(path.as_deref(), since.as_deref());

    if sessions.is_empty() {
        println!("No session transcripts found.");
        return Ok(());
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for session_path in &sessions {
        let summary = match hyphae_ingest::transcript::parse_transcript(session_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Error parsing {}: {e}", session_path.display());
                errors += 1;
                continue;
            }
        };

        let dedupe_hash = session_hash(&summary.session_id, session_path);
        match store.memory_exists_with_keyword(&dedupe_hash) {
            Ok(true) => {
                if dry_run {
                    println!(
                        "[dry-run] Would skip (already ingested): {}",
                        summary.session_id
                    );
                }
                skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(_) => {}
        }

        let text = hyphae_ingest::transcript::summary_to_text(&summary);
        let resolved_project = project.map(String::from).or_else(|| {
            if summary.project.is_empty() {
                None
            } else {
                Some(summary.project.clone())
            }
        });
        let project_name = resolved_project.as_deref().unwrap_or("unknown");

        if dry_run {
            println!(
                "[dry-run] Would ingest {} session: {} ({} messages, {} files, {} errors) -> session/{}",
                summary.runtime,
                summary.session_id,
                summary.message_count,
                summary.files_modified.len(),
                summary.errors.len(),
                project_name,
            );
            imported += 1;
            continue;
        }

        let topic = format!("session/{}", project_name);
        use hyphae_core::memory::{Importance, Memory};
        use hyphae_core::store::MemoryStore;
        let mut builder = Memory::builder(topic, text, Importance::Medium).keywords(vec![
            format!("hash:{dedupe_hash}"),
            format!("session_id:{}", summary.session_id),
        ]);
        if let Some(ref proj) = resolved_project {
            builder = builder.project(proj.clone());
        }
        let memory = builder
            .source(session_source(
                &summary.runtime,
                &summary.session_id,
                session_path,
            ))
            .build();

        match store.store(memory) {
            Ok(_) => {
                imported += 1;
                println!(
                    "  Ingested {} session: {} ({} messages)",
                    summary.runtime, summary.session_id, summary.message_count
                );
            }
            Err(e) => {
                eprintln!("  Error storing {}: {e}", summary.session_id);
                errors += 1;
            }
        }
    }

    println!();
    println!("Ingested: {imported}, Skipped: {skipped} (already ingested), Errors: {errors}");

    Ok(())
}

fn session_hash(session_id: &str, path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(path.to_string_lossy().as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex[..12].to_string()
}

fn session_source(
    runtime: &hyphae_ingest::transcript::SessionRuntime,
    session_id: &str,
    session_path: &Path,
) -> hyphae_core::memory::MemorySource {
    match runtime {
        hyphae_ingest::transcript::SessionRuntime::ClaudeCode => {
            hyphae_core::memory::MemorySource::agent_session(
                hyphae_core::memory::SessionHost::ClaudeCode,
                session_id,
                Some(session_path.display().to_string()),
            )
        }
        hyphae_ingest::transcript::SessionRuntime::Codex => {
            hyphae_core::memory::MemorySource::agent_session(
                hyphae_core::memory::SessionHost::Codex,
                session_id,
                Some(session_path.display().to_string()),
            )
        }
    }
}

/// Discover session transcript files from Claude Code and Codex directories.
fn discover_sessions(path: Option<&Path>, since: Option<&str>) -> Vec<PathBuf> {
    let since_ts = since.and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc().timestamp())
    });

    let mut sessions = Vec::new();
    if let Some(p) = path {
        collect_jsonl_files(p, &mut sessions);
    } else {
        for root in default_session_roots() {
            collect_jsonl_files(&root, &mut sessions);
        }
    }

    sessions.retain(|path| {
        if let Some(ts) = since_ts {
            if let Ok(meta) = path.metadata() {
                if let Ok(modified) = meta.modified() {
                    let file_ts = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    return file_ts >= ts;
                }
            }
        }
        true
    });

    sessions.sort();
    sessions.dedup();
    sessions
}

fn default_session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(claude_dir) = editors::claude_dir() {
        roots.push(claude_dir.join("projects"));
    }

    if let Ok(codex_config) = editors::config_path(SharedEditor::CodexCli) {
        if let Some(codex_dir) = codex_config.parent() {
            roots.push(codex_dir.join("history.jsonl"));
            roots.push(codex_dir.join("sessions"));
        }
    }

    roots
}

fn collect_jsonl_files(path: &Path, sessions: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            sessions.push(path.to_path_buf());
        }
        return;
    }

    if !path.is_dir() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_jsonl_files(&entry_path, sessions);
        } else if entry_path.extension().is_some_and(|ext| ext == "jsonl") {
            sessions.push(entry_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_sessions_includes_codex_history_file() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history.jsonl");
        std::fs::write(&history, r#"{"session_id":"sess-1","ts":1,"text":"hello"}"#).unwrap();

        let sessions = discover_sessions(Some(dir.path()), None);
        assert_eq!(sessions, vec![history]);
    }

    #[test]
    fn test_codex_history_ingests_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"session_id":"sess-1","ts":1,"text":"//help"}"#,
                "\n",
                r#"{"session_id":"sess-1","ts":2,"text":"Please review the repo"}"#,
            ),
        )
        .unwrap();

        let store = SqliteStore::in_memory().unwrap();
        run(&store, Some(path.clone()), None, false, Some("demo")).unwrap();
        assert!(
            store
                .memory_exists_with_keyword(&session_hash("sess-1", path.as_path()))
                .unwrap()
        );
    }
}
