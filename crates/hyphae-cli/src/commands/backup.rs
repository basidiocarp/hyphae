use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// ─────────────────────────────────────────────────────────────────────────
/// Create a Backup of the Database
/// ─────────────────────────────────────────────────────────────────────────
pub(crate) fn cmd_backup(output: Option<PathBuf>) -> Result<()> {
    let db_path = get_db_path();

    if !db_path.exists() {
        return Err(anyhow::anyhow!(
            "database not found at {}",
            db_path.display()
        ));
    }

    let backup_path = if let Some(p) = output {
        p
    } else {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let backup_name = format!("hyphae-backup-{}.db", timestamp);
        PathBuf::from(backup_name)
    };

    fs::copy(&db_path, &backup_path)
        .with_context(|| format!("failed to backup database to {}", backup_path.display()))?;

    let size = fs::metadata(&backup_path).map(|m| m.len()).unwrap_or(0);

    println!("Backup created: {}", backup_path.display());
    println!("Size: {} bytes", size);

    Ok(())
}

/// ─────────────────────────────────────────────────────────────────────────
/// Restore Database from a Backup
/// ─────────────────────────────────────────────────────────────────────────
pub(crate) fn cmd_restore(path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup file not found at {}",
            path.display()
        ));
    }

    // Verify it's a SQLite file by checking magic bytes
    fs::read(&path)
        .context("failed to read backup file")?
        .get(0..16)
        .and_then(|bytes| {
            if bytes.starts_with(b"SQLite format 3") {
                Some(())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("backup file is not a valid SQLite database"))?;

    let db_path = get_db_path();

    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::copy(&path, &db_path)
        .with_context(|| format!("failed to restore database from {}", path.display()))?;

    println!("Database restored from {}", path.display());
    println!("Location: {}", db_path.display());

    Ok(())
}

/// ─────────────────────────────────────────────────────────────────────────
/// Get Database Path
/// ─────────────────────────────────────────────────────────────────────────
fn get_db_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "hyphae")
        .map(|d| d.data_dir().join("hyphae.db"))
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local/share/hyphae/hyphae.db"))
                .unwrap_or_else(|| PathBuf::from(".local/share/hyphae/hyphae.db"))
        })
}
