use std::path::PathBuf;

use anyhow::Result;
use hyphae_store::SqliteStore;
use rusqlite::Connection;

pub fn run(db: Option<PathBuf>) -> Result<()> {
    let path = db.unwrap_or_else(crate::paths::default_db_path);

    if !path.exists() {
        println!("No database found at {}", path.display());
        println!("Run any hyphae write command first to create it.");
        return Ok(());
    }

    let version_before: i64 = {
        let conn = Connection::open(&path)?;
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))?
    };

    // Opening the store runs all pending schema migrations.
    SqliteStore::new(&path).map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;

    let version_after: i64 = {
        let conn = Connection::open(&path)?;
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))?
    };

    if version_before == version_after {
        println!("Database is already up to date (schema version {version_after}).");
    } else {
        println!(
            "Migrated database from schema version {version_before} to {version_after}."
        );
    }
    println!("Path: {}", path.display());
    Ok(())
}
