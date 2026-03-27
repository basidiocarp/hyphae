//! Session lifecycle persistence.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{HyphaeError, HyphaeResult};

use super::SqliteStore;

/// A session record.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub scope: Option<String>,
    pub task: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub files_modified: Option<String>,
    pub errors: Option<String>,
    pub status: String,
}

impl SqliteStore {
    /// Start a new session. Returns (session_id, started_at).
    pub fn session_start(
        &self,
        project: &str,
        task: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_scoped(project, task, None)
    }

    /// Start a new session scoped to a specific worker or runtime. Returns (session_id, started_at).
    pub fn session_start_scoped(
        &self,
        project: &str,
        task: Option<&str>,
        scope: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        if let Some(existing) = self
            .conn
            .query_row(
                "SELECT id, started_at
                 FROM sessions
                 WHERE project = ?1 AND scope IS ?2 AND status = 'active'
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![project, scope],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query active session: {e}")))?
        {
            return Ok(existing);
        }

        let session_id = format!("ses_{}", ulid::Ulid::new());
        let started_at = Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO sessions (id, project, scope, task, started_at, status) VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
                params![session_id, project, scope, task, started_at],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to insert session: {e}")))?;

        Ok((session_id, started_at))
    }

    /// End an active session. Returns (project, started_at, task, ended_at, duration_minutes).
    pub fn session_end(
        &self,
        session_id: &str,
        summary: Option<&str>,
        files_modified: Option<&str>,
        errors: Option<&str>,
    ) -> HyphaeResult<(String, String, Option<String>, String, i64)> {
        // Fetch the active session
        let row: (String, String, Option<String>) = self
            .conn
            .query_row(
                "SELECT project, started_at, task FROM sessions WHERE id = ?1 AND status = 'active'",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    HyphaeError::NotFound(format!("no active session with id '{session_id}'"))
                }
                other => HyphaeError::Database(format!("failed to query session: {other}")),
            })?;

        let (project, started_at, task) = row;
        let ended_at = Utc::now().to_rfc3339();

        let duration_minutes = chrono::DateTime::parse_from_rfc3339(&ended_at)
            .ok()
            .zip(chrono::DateTime::parse_from_rfc3339(&started_at).ok())
            .map(|(end, start)| (end - start).num_minutes())
            .unwrap_or(0);

        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1, summary = ?2, files_modified = ?3, errors = ?4, status = 'completed' WHERE id = ?5",
                params![ended_at, summary, files_modified, errors, session_id],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to update session: {e}")))?;

        let error_count = errors
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let signal_type = if error_count > 0 {
            "session_failure"
        } else {
            "session_success"
        };
        let signal_value = if error_count > 0 { -2 } else { 2 };
        if let Err(e) = self.log_outcome_signal(
            Some(session_id),
            signal_type,
            signal_value,
            Some("hyphae.session_end"),
            Some(&project),
        ) {
            tracing::warn!("failed to record session outcome signal: {e}");
        }

        Ok((project, started_at, task, ended_at, duration_minutes))
    }

    /// Get recent sessions for a project.
    pub fn session_context(&self, project: &str, limit: i64) -> HyphaeResult<Vec<Session>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project, scope, task, started_at, ended_at, summary, files_modified, errors, status
                 FROM sessions
                 WHERE project = ?1
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![project, limit], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    scope: row.get(2)?,
                    task: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    summary: row.get(6)?,
                    files_modified: row.get(7)?,
                    errors: row.get(8)?,
                    status: row.get(9)?,
                })
            })
            .map_err(|e| HyphaeError::Database(format!("failed to query sessions: {e}")))?;

        let sessions: Vec<Session> = rows.filter_map(Result::ok).collect();
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_session_lifecycle() {
        let store = test_store();

        // Start
        let (sid, started_at) = store
            .session_start("test-project", Some("implement feature"))
            .unwrap();
        assert!(sid.starts_with("ses_"));
        assert!(!started_at.is_empty());

        // Context shows active session
        let sessions = store.session_context("test-project", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[0].scope, None);

        // End
        let (project, _, task, _, duration) = store
            .session_end(&sid, Some("done"), Some("[\"file.rs\"]"), Some("0"))
            .unwrap();
        assert_eq!(project, "test-project");
        assert_eq!(task.as_deref(), Some("implement feature"));
        assert!(duration >= 0);

        // Context shows completed
        let sessions = store.session_context("test-project", 10).unwrap();
        assert_eq!(sessions[0].status, "completed");
    }

    #[test]
    fn test_session_end_invalid_id() {
        let store = test_store();
        let result = store.session_end("nonexistent", None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_context_empty() {
        let store = test_store();
        let sessions = store.session_context("no-such-project", 5).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_session_start_reuses_existing_active_session() {
        let store = test_store();
        let (first_id, first_started_at) = store.session_start("demo", Some("first")).unwrap();
        let (second_id, second_started_at) = store.session_start("demo", Some("second")).unwrap();

        assert_eq!(first_id, second_id);
        assert_eq!(first_started_at, second_started_at);

        let sessions = store.session_context("demo", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
    }

    #[test]
    fn test_session_start_scoped_separates_parallel_workers() {
        let store = test_store();
        let (worker_a, _) = store
            .session_start_scoped("demo", Some("first"), Some("worker-a"))
            .unwrap();
        let (worker_b, _) = store
            .session_start_scoped("demo", Some("second"), Some("worker-b"))
            .unwrap();
        let (worker_a_again, _) = store
            .session_start_scoped("demo", Some("first-again"), Some("worker-a"))
            .unwrap();

        assert_ne!(worker_a, worker_b);
        assert_eq!(worker_a, worker_a_again);

        let sessions = store.session_context("demo", 10).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[1].status, "active");
    }
}
