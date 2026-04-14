use anyhow::Result;
use hyphae_store::{AuditOperation, SqliteStore};

use crate::cli::AuditCommand;

pub(crate) fn dispatch(store: &SqliteStore, command: AuditCommand) -> Result<()> {
    match command {
        AuditCommand::List {
            since,
            operation,
            limit,
            json,
        } => cmd_audit_list(store, since, operation, limit, json),
        AuditCommand::Rollback { audit_id } => cmd_audit_rollback(store, audit_id),
    }
}

fn cmd_audit_list(
    store: &SqliteStore,
    since: Option<String>,
    operation: Option<String>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let op = operation
        .as_deref()
        .map(|s| {
            AuditOperation::parse(s)
                .ok_or_else(|| anyhow::anyhow!("unknown operation: {s}. Valid: store, update, delete, invalidate, decay, prune, prune_expired, consolidate"))
        })
        .transpose()?;

    let entries = store.audit_list(since.as_deref(), op, limit)?;

    if json {
        let payload = serde_json::json!({
            "entries": entries,
            "count": entries.len(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No audit entries found.");
        return Ok(());
    }

    for entry in &entries {
        println!(
            "{} | {:12} | {} | {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            entry.operation,
            entry.memory_id,
            entry.topic.as_deref().unwrap_or("-"),
        );
        if let Some(ref meta) = entry.metadata_json {
            println!("  id: {} | meta: {}", entry.id, meta);
        } else {
            println!("  id: {}", entry.id);
        }
    }
    println!("\n{} entries shown.", entries.len());

    Ok(())
}

fn cmd_audit_rollback(store: &SqliteStore, audit_id: String) -> Result<()> {
    let msg = store.audit_rollback(&audit_id)?;
    println!("{msg}");
    Ok(())
}
