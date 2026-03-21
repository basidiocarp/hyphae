use chrono::DateTime;
use std::io::{self, Write};

use hyphae_store::SqliteStore;

// ─────────────────────────────────────────────────────────────────────────────
// Purge Statistics
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct PurgeStats {
    memories: usize,
    sessions: usize,
    chunks: usize,
    documents: usize,
}

impl PurgeStats {
    fn total(&self) -> usize {
        self.memories + self.sessions + self.chunks + self.documents
    }

    fn print_summary(&self, prefix: &str) {
        println!("{}", prefix);
        println!("  {} memories", self.memories);
        println!("  {} sessions", self.sessions);
        println!("  {} chunks", self.chunks);
        println!("  {} documents", self.documents);
        println!("  ─────────────────────");
        println!("  {} total items", self.total());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Confirmation Prompt
// ─────────────────────────────────────────────────────────────────────────────

fn prompt_confirmation(stats: &PurgeStats, force: bool) -> anyhow::Result<bool> {
    if force {
        return Ok(true);
    }

    stats.print_summary("This will delete:");
    print!("\nContinue? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().eq_ignore_ascii_case("y"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Purge by Project
// ─────────────────────────────────────────────────────────────────────────────

pub fn cmd_purge(
    store: &SqliteStore,
    project: Option<String>,
    before: Option<String>,
    dry_run: bool,
    force: bool,
    _resolved_project: Option<String>,
) -> anyhow::Result<()> {
    if project.is_none() && before.is_none() {
        anyhow::bail!("must specify either --project or --before");
    }

    if let Some(proj) = project {
        purge_by_project(store, &proj, dry_run, force)?;
    } else if let Some(before_str) = before {
        purge_before_date(store, &before_str, dry_run, force)?;
    }

    Ok(())
}

fn purge_by_project(
    store: &SqliteStore,
    project: &str,
    dry_run: bool,
    force: bool,
) -> anyhow::Result<()> {
    // Count items to delete
    let memories_count = store.count_memories_by_project(project)?;
    let sessions_count = store.count_sessions_by_project(project)?;
    let chunks_count = store.count_chunks_by_project(project)?;
    let documents_count = store.count_documents_by_project(project)?;

    let stats = PurgeStats {
        memories: memories_count,
        sessions: sessions_count,
        chunks: chunks_count,
        documents: documents_count,
    };

    if dry_run {
        println!("DRY RUN: Deletion by project '{}'", project);
        stats.print_summary("Would delete:");
        println!("\nTo confirm, run without --dry-run");
        return Ok(());
    }

    if !prompt_confirmation(&stats, force)? {
        println!("Purge cancelled.");
        return Ok(());
    }

    // Perform deletion
    let (mem_del, ses_del, chk_del, doc_del) = store.purge_project(project)?;

    let result_stats = PurgeStats {
        memories: mem_del,
        sessions: ses_del,
        chunks: chk_del,
        documents: doc_del,
    };

    result_stats.print_summary(&format!("Deleted from project '{}':", project));
    Ok(())
}

fn purge_before_date(
    store: &SqliteStore,
    before_str: &str,
    dry_run: bool,
    force: bool,
) -> anyhow::Result<()> {
    // Parse date
    let before_dt = DateTime::parse_from_rfc3339(&format!("{before_str}T00:00:00+00:00"))
        .or_else(|_| {
            let with_time = format!("{before_str}T00:00:00+00:00");
            DateTime::parse_from_rfc3339(&with_time)
        })
        .map_err(|_| {
            anyhow::anyhow!("invalid date format: {before_str}, use YYYY-MM-DD or ISO 8601")
        })?
        .with_timezone(&chrono::Utc);

    let before_rfc3339 = before_dt.to_rfc3339();

    // Count items to delete
    let memories_count = store.count_memories_before_date(&before_rfc3339)?;
    let sessions_count = store.count_sessions_before_date(&before_rfc3339)?;
    let chunks_count = store.count_chunks_before_date(&before_rfc3339)?;
    let documents_count = store.count_documents_before_date(&before_rfc3339)?;

    let stats = PurgeStats {
        memories: memories_count,
        sessions: sessions_count,
        chunks: chunks_count,
        documents: documents_count,
    };

    if dry_run {
        println!("DRY RUN: Deletion before date '{}'", before_str);
        stats.print_summary("Would delete:");
        println!("\nTo confirm, run without --dry-run");
        return Ok(());
    }

    if !prompt_confirmation(&stats, force)? {
        println!("Purge cancelled.");
        return Ok(());
    }

    // Perform deletion
    let (mem_del, ses_del, chk_del, doc_del) = store.purge_before_date(&before_rfc3339)?;

    let result_stats = PurgeStats {
        memories: mem_del,
        sessions: ses_del,
        chunks: chk_del,
        documents: doc_del,
    };

    result_stats.print_summary(&format!("Deleted items created before '{}':", before_str));
    Ok(())
}
