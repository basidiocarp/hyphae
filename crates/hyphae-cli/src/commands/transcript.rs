//! `hyphae ingest-sessions` — import Claude Code session transcripts.

use anyhow::Result;
use hyphae_store::SqliteStore;
use std::path::PathBuf;

pub fn run(
    store: &SqliteStore,
    path: Option<PathBuf>,
    since: Option<String>,
    dry_run: bool,
    project: Option<&str>,
) -> Result<()> {
    let sessions = discover_sessions(path.as_deref(), since.as_deref())?;

    if sessions.is_empty() {
        println!("No session transcripts found.");
        return Ok(());
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for session_path in &sessions {
        // Parse transcript
        let summary = match hyphae_ingest::transcript::parse_transcript(session_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Error parsing {}: {e}", session_path.display());
                errors += 1;
                continue;
            }
        };

        // Check dedup via session_id keyword
        let keyword = format!("session_id:{}", summary.session_id);
        match store.memory_exists_with_keyword(&keyword) {
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
            Err(_) => {} // Proceed if check fails
        }

        let text = hyphae_ingest::transcript::summary_to_text(&summary);
        let resolved_project = project.map(String::from).or_else(|| {
            if summary.project.is_empty() {
                None
            } else {
                Some(summary.project.clone())
            }
        });

        if dry_run {
            println!(
                "[dry-run] Would ingest: {} ({} messages, {} files, {} errors) → session/{}",
                summary.session_id,
                summary.message_count,
                summary.files_modified.len(),
                summary.errors.len(),
                resolved_project.as_deref().unwrap_or("unknown"),
            );
            imported += 1;
            continue;
        }

        // Store as memory
        let topic = format!(
            "session/{}",
            resolved_project.as_deref().unwrap_or("unknown")
        );
        use hyphae_core::memory::{Importance, Memory};
        use hyphae_core::store::MemoryStore;
        let mut builder =
            Memory::builder(topic, text, Importance::Medium).keywords(vec![keyword.clone()]);
        if let Some(ref proj) = resolved_project {
            builder = builder.project(proj.clone());
        }
        let memory = builder.build();

        match store.store(memory) {
            Ok(_) => {
                imported += 1;
                println!(
                    "  Ingested: {} ({} messages)",
                    summary.session_id, summary.message_count
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

/// Discover session transcript files from Claude Code directories.
fn discover_sessions(path: Option<&std::path::Path>, since: Option<&str>) -> Result<Vec<PathBuf>> {
    let mut sessions = Vec::new();

    let since_ts = since.and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
    });

    let dirs_to_scan = if let Some(p) = path {
        vec![p.to_path_buf()]
    } else {
        // Scan all Claude Code project session directories
        let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
        else {
            return Ok(sessions);
        };
        let claude_projects = home.join(".claude/projects");
        if !claude_projects.exists() {
            return Ok(sessions);
        }
        let mut dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&claude_projects) {
            for entry in entries.flatten() {
                let sessions_dir = entry.path().join("sessions");
                if sessions_dir.is_dir() {
                    dirs.push(sessions_dir);
                }
            }
        }
        dirs
    };

    for dir in &dirs_to_scan {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    // Check since filter
                    if let Some(ts) = since_ts {
                        if let Ok(meta) = path.metadata() {
                            if let Ok(modified) = meta.modified() {
                                let file_ts = modified
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs() as i64)
                                    .unwrap_or(0);
                                if file_ts < ts {
                                    continue;
                                }
                            }
                        }
                    }
                    sessions.push(path);
                }
            }
        }
    }

    sessions.sort();
    Ok(sessions)
}
