use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use hyphae_core::{BackupExportManifest, ScopedIdentity};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::paths;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackupEntry {
    path: PathBuf,
    size: u64,
    modified: Option<DateTime<Utc>>,
}

pub(crate) fn cmd_backup(output: Option<PathBuf>, db_path: PathBuf) -> Result<()> {
    let backup_path = create_backup(&db_path, output)?;
    let size = fs::metadata(&backup_path).map(|m| m.len()).unwrap_or(0);
    let manifest_path = backup_manifest_path(&backup_path);

    println!("Backup created: {}", backup_path.display());
    println!("Manifest: {}", manifest_path.display());
    println!("Size: {} bytes", size);

    Ok(())
}

pub(crate) fn auto_backup(db_path: &Path) -> Result<PathBuf> {
    create_backup(db_path, None)
}

pub(crate) fn cmd_backup_list() -> Result<()> {
    let backup_dir = paths::backup_dir();
    let backups = collect_backups(&backup_dir)?;

    if backups.is_empty() {
        println!("No backups found in {}", backup_dir.display());
        return Ok(());
    }

    println!("Backups in {}:", backup_dir.display());
    for backup in backups {
        let modified = backup
            .modified
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "  {}  {} bytes  {}",
            backup.path.display(),
            backup.size,
            modified
        );
    }

    Ok(())
}

pub(crate) fn cmd_restore(path: PathBuf, db_path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("backup file not found at {}", path.display()));
    }

    validate_sqlite_backup(&path)?;

    if !prompt_restore_confirmation(&path, &db_path)? {
        println!("Restore cancelled.");
        return Ok(());
    }

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

pub(crate) fn create_backup(db_path: &Path, output: Option<PathBuf>) -> Result<PathBuf> {
    if !db_path.exists() {
        return Err(anyhow!("database not found at {}", db_path.display()));
    }

    let backup_path = if let Some(path) = output {
        path
    } else {
        paths::backup_dir().join(format!(
            "hyphae-backup-{}.db",
            Utc::now().format("%Y%m%d-%H%M%S-%3f")
        ))
    };

    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::copy(db_path, &backup_path)
        .with_context(|| format!("failed to backup database to {}", backup_path.display()))?;

    validate_sqlite_backup(&backup_path)?;

    let size = fs::metadata(&backup_path).map(|m| m.len()).unwrap_or(0);
    write_backup_manifest(&backup_path, size, None)?;

    Ok(backup_path)
}

fn backup_manifest_path(backup_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.manifest.json", backup_path.display()))
}

fn write_backup_manifest(
    backup_path: &Path,
    size: u64,
    scoped_identity: Option<ScopedIdentity>,
) -> Result<PathBuf> {
    let manifest_path = backup_manifest_path(backup_path);
    let manifest = BackupExportManifest::new(
        &Utc::now().to_rfc3339(),
        &backup_path.display().to_string(),
        size,
        scoped_identity,
    );
    let serialized =
        serde_json::to_string_pretty(&manifest).context("failed to serialize backup manifest")?;
    fs::write(&manifest_path, serialized).with_context(|| {
        format!(
            "failed to write backup manifest {}",
            manifest_path.display()
        )
    })?;
    Ok(manifest_path)
}

fn collect_backups(backup_dir: &Path) -> Result<Vec<BackupEntry>> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }

    let mut backups = Vec::new();
    for entry in fs::read_dir(backup_dir)
        .with_context(|| format!("failed to read {}", backup_dir.display()))?
    {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".manifest.json"))
        {
            continue;
        }

        let modified = meta.modified().ok().map(DateTime::<Utc>::from);

        backups.push(BackupEntry {
            path: entry.path(),
            size: meta.len(),
            modified,
        });
    }

    backups.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.path.cmp(&a.path))
    });

    Ok(backups)
}

fn validate_sqlite_backup(path: &Path) -> Result<()> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open backup file {}", path.display()))?;

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("failed to run SQLite integrity check")?;

    if integrity.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        bail!("backup file failed SQLite integrity check: {integrity}");
    }
}

fn prompt_restore_confirmation(path: &Path, db_path: &Path) -> Result<bool> {
    println!(
        "This will replace the current database at {} with {}.",
        db_path.display(),
        path.display()
    );
    print!("Continue? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn test_create_backup_copies_file_to_explicit_path() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("hyphae.db");
        let backup_path = dir.path().join("nested").join("backup.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();

        let created = create_backup(&db_path, Some(backup_path.clone())).unwrap();
        assert_eq!(created, backup_path);
        validate_sqlite_backup(&created).unwrap();
        let manifest_path = backup_manifest_path(&created);
        let manifest: BackupExportManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.export_kind, "sqlite_backup");
        assert_eq!(manifest.artifact_path, created.display().to_string());
        assert_eq!(manifest.sqlite_integrity, "ok");
    }

    #[test]
    fn test_collect_backups_sorts_newest_first() {
        let dir = TempDir::new().unwrap();
        let older = dir.path().join("hyphae-backup-20260409-010101-000.db");
        let newer = dir.path().join("hyphae-backup-20260409-020202-000.db");

        fs::write(&older, b"older").unwrap();
        fs::write(&newer, b"newer").unwrap();

        let backups = collect_backups(dir.path()).unwrap();
        assert_eq!(backups.len(), 2);
        assert_eq!(backups[0].path, newer);
        assert_eq!(backups[1].path, older);
    }

    #[test]
    fn test_collect_backups_ignores_manifest_sidecars() {
        let dir = TempDir::new().unwrap();
        let backup = dir.path().join("hyphae-backup-20260409-010101-000.db");
        let manifest = backup_manifest_path(&backup);

        fs::write(&backup, b"sqlite-placeholder").unwrap();
        fs::write(&manifest, b"{\"schema_version\":\"1.0\"}").unwrap();

        let backups = collect_backups(dir.path()).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].path, backup);
    }

    #[test]
    fn test_validate_sqlite_backup_accepts_real_database() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("backup.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();

        validate_sqlite_backup(&db_path).unwrap();
    }

    #[test]
    fn test_validate_sqlite_backup_rejects_plain_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("not-sqlite.db");
        fs::write(&path, b"plain text").unwrap();

        assert!(validate_sqlite_backup(&path).is_err());
    }
}
