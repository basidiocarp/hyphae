//! Export operations for creating portable archives.

use rusqlite::params;
use serde::Serialize;

use hyphae_core::{HyphaeError, HyphaeResult, Memory};

use super::session::Session;
use super::SqliteStore;

/// Top-level archive payload structure.
#[derive(Debug, Clone, Serialize)]
pub struct HyphaeArchive {
    pub schema_version: String,
    pub exported_at: String,
    pub identity: ArchiveIdentity,
    pub filter: ArchiveFilter,
    pub memories: Vec<ArchiveMemoryRecord>,
    pub memoirs: Vec<ArchiveMemoirRecord>,
    pub sessions: Vec<ArchiveSessionRecord>,
}

/// Identity of the source instance.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveIdentity {
    pub project: Option<String>,
    pub project_root: Option<String>,
    pub hyphae_version: Option<String>,
}

/// Filters applied during export.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveFilter {
    pub topic: Option<String>,
    pub since: Option<String>,
    pub importance_minimum: Option<String>,
}

/// Memory record in archive format.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMemoryRecord {
    pub id: String,
    pub topic: String,
    pub content: String,
    pub importance: String,
    pub keywords: Option<String>,
    pub project: Option<String>,
    pub weight: Option<f32>,
    pub created_at: String,
    pub updated_at: String,
}

/// Memoir concept in archive format.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMemoirConceptRecord {
    pub id: String,
    pub name: String,
    pub definition: String,
}

/// Memoir link in archive format.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMemoirLinkRecord {
    pub from_id: String,
    pub to_id: String,
    pub relationship: String,
}

/// Memoir record in archive format.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMemoirRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub concepts: Vec<ArchiveMemoirConceptRecord>,
    pub links: Vec<ArchiveMemoirLinkRecord>,
}

/// Session record in archive format.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveSessionRecord {
    pub id: String,
    pub project: String,
    pub project_root: Option<String>,
    pub worktree_id: Option<String>,
    pub task: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub files_modified: Option<Vec<String>>,
    pub errors: Option<Vec<String>>,
    pub status: String,
}

impl SqliteStore {
    /// Export memories for archive with optional filters.
    pub fn export_memories_for_archive(
        &self,
        project: Option<&str>,
        topic: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        min_weight: Option<f32>,
    ) -> HyphaeResult<Vec<Memory>> {
        use super::helpers::SELECT_COLS;

        // Static SQL with positional (col = ? OR ? IS NULL) pattern.
        // All 10 params are always bound — no ordering drift risk.
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE invalidated_at IS NULL
               AND (project = ? OR ? IS NULL)
               AND (topic = ? OR ? IS NULL)
               AND (created_at >= ? OR ? IS NULL)
               AND (created_at <= ? OR ? IS NULL)
               AND (weight >= ? OR ? IS NULL)
             ORDER BY created_at DESC"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![
                    project, project,
                    topic, topic,
                    since, since,
                    until, until,
                    min_weight, min_weight,
                ],
                |row| {
                    use super::helpers::row_to_memory;
                    row_to_memory(row)
                },
            )
            .map_err(|e| HyphaeError::Database(format!("query_map failed: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            match row {
                Ok(mem) => results.push(mem),
                Err(e) => {
                    return Err(HyphaeError::Database(format!("memory row error: {}", e)));
                }
            }
        }

        Ok(results)
    }

    /// Export sessions for archive with optional filters.
    pub fn export_sessions_for_archive(
        &self,
        project: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> HyphaeResult<Vec<Session>> {
        let sql = "SELECT id, project, project_root, worktree_id, scope, runtime_session_id, task,
                          started_at, ended_at, summary, files_modified, errors, status
                   FROM sessions
                   WHERE (project = ? OR ? IS NULL)
                     AND (started_at >= ? OR ? IS NULL)
                     AND (started_at <= ? OR ? IS NULL)
                   ORDER BY started_at DESC";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project, project, since, since, until, until], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    project_root: row.get(2)?,
                    worktree_id: row.get(3)?,
                    scope: row.get(4)?,
                    runtime_session_id: row.get(5)?,
                    task: row.get(6)?,
                    started_at: row.get(7)?,
                    ended_at: row.get(8)?,
                    summary: row.get(9)?,
                    files_modified: row.get(10)?,
                    errors: row.get(11)?,
                    status: row.get(12)?,
                })
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e: rusqlite::Error| HyphaeError::Database(e.to_string()))?);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory};

    #[test]
    fn test_archive_memory_record_serializes() {
        let record = ArchiveMemoryRecord {
            id: "test-id".to_string(),
            topic: "test".to_string(),
            content: "test content".to_string(),
            importance: "high".to_string(),
            keywords: Some("test,example".to_string()),
            project: Some("myproject".to_string()),
            weight: Some(0.85),
            created_at: "2026-04-14T00:00:00Z".to_string(),
            updated_at: "2026-04-14T01:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&record).expect("should serialize");
        assert!(json.contains("test-id"));
        assert!(json.contains("test content"));
    }

    #[test]
    fn test_archive_session_record_serializes() {
        let record = ArchiveSessionRecord {
            id: "session-1".to_string(),
            project: "myproject".to_string(),
            project_root: Some("/home/user/project".to_string()),
            worktree_id: Some("main".to_string()),
            task: Some("test task".to_string()),
            started_at: "2026-04-14T00:00:00Z".to_string(),
            ended_at: Some("2026-04-14T01:00:00Z".to_string()),
            summary: Some("test summary".to_string()),
            files_modified: Some(vec!["file1.rs".to_string()]),
            errors: None,
            status: "completed".to_string(),
        };
        let json = serde_json::to_string(&record).expect("should serialize");
        assert!(json.contains("session-1"));
        assert!(json.contains("completed"));
    }

    #[test]
    fn test_archive_identity_serializes() {
        let identity = ArchiveIdentity {
            project: Some("basidiocarp".to_string()),
            project_root: Some("/Users/dev/projects/basidiocarp".to_string()),
            hyphae_version: Some("0.10.9".to_string()),
        };
        let json = serde_json::to_string(&identity).expect("should serialize");
        assert!(json.contains("basidiocarp"));
        assert!(json.contains("0.10.9"));
    }
}
