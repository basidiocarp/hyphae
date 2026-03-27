//! Feedback loop persistence for recall events and session outcomes.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{HyphaeError, HyphaeResult};

use super::SqliteStore;

impl SqliteStore {
    fn resolve_feedback_project(
        &self,
        session_id: Option<&str>,
        project: Option<&str>,
    ) -> HyphaeResult<Option<String>> {
        let Some(session_id) = session_id else {
            return Ok(project.map(ToOwned::to_owned));
        };

        let session_project: String = self
            .conn
            .query_row(
                "SELECT project FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to validate session: {e}")))?
            .ok_or_else(|| HyphaeError::NotFound(format!("session '{session_id}'")))?;

        if let Some(project_name) = project
            && project_name != session_project
        {
            return Err(HyphaeError::Validation(format!(
                "session '{session_id}' belongs to project '{session_project}', not '{project_name}'"
            )));
        }

        Ok(Some(session_project))
    }

    pub fn active_session_id(&self, project: &str) -> HyphaeResult<Option<String>> {
        self.conn
            .query_row(
                "SELECT id
                 FROM sessions
                 WHERE project = ?1 AND status = 'active'
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![project],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query active session: {e}")))
    }

    pub fn log_recall_event(
        &self,
        session_id: Option<&str>,
        query: &str,
        memory_ids: &[String],
        project: Option<&str>,
    ) -> HyphaeResult<String> {
        let event_id = format!("rec_{}", ulid::Ulid::new());
        let recalled_at = Utc::now().to_rfc3339();
        let resolved_project = self.resolve_feedback_project(session_id, project)?;
        let memory_ids_json = serde_json::to_string(memory_ids)
            .map_err(|e| HyphaeError::Database(format!("failed to encode recall event: {e}")))?;

        self.conn
            .execute(
                "INSERT INTO recall_events
                    (id, session_id, query, recalled_at, memory_ids, memory_count, project)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    event_id,
                    session_id,
                    query,
                    recalled_at,
                    memory_ids_json,
                    i64::try_from(memory_ids.len()).unwrap_or(i64::MAX),
                    resolved_project,
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to log recall event: {e}")))?;

        Ok(event_id)
    }

    pub fn log_outcome_signal(
        &self,
        session_id: Option<&str>,
        signal_type: &str,
        signal_value: i64,
        source: Option<&str>,
        project: Option<&str>,
    ) -> HyphaeResult<String> {
        let signal_id = format!("sig_{}", ulid::Ulid::new());
        let occurred_at = Utc::now().to_rfc3339();
        let resolved_project = self.resolve_feedback_project(session_id, project)?;

        self.conn
            .execute(
                "INSERT INTO outcome_signals
                    (id, session_id, signal_type, signal_value, occurred_at, source, project)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    signal_id,
                    session_id,
                    signal_type,
                    signal_value,
                    occurred_at,
                    source,
                    resolved_project,
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to log outcome signal: {e}")))?;

        Ok(signal_id)
    }

    pub fn count_outcome_signals(
        &self,
        session_id: Option<&str>,
        signal_type: Option<&str>,
        signal_value: Option<i64>,
    ) -> HyphaeResult<i64> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM outcome_signals
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR signal_type = ?2)
                   AND (?3 IS NULL OR signal_value = ?3)",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare outcome signal count: {e}"))
            })?;

        stmt.query_row(params![session_id, signal_type, signal_value], |row| {
            row.get(0)
        })
        .map_err(|e| HyphaeError::Database(format!("failed to count outcome signals: {e}")))
    }

    pub fn count_recall_events(
        &self,
        session_id: Option<&str>,
        project: Option<&str>,
        memory_count: Option<i64>,
    ) -> HyphaeResult<i64> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM recall_events
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR project = ?2)
                   AND (?3 IS NULL OR memory_count = ?3)",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare recall event count: {e}"))
            })?;

        stmt.query_row(params![session_id, project, memory_count], |row| row.get(0))
            .map_err(|e| HyphaeError::Database(format!("failed to count recall events: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct RecallEventRow {
        id: String,
        session_id: Option<String>,
        query: String,
        recalled_at: String,
        memory_ids: Vec<String>,
        project: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct OutcomeSignalRow {
        id: String,
        session_id: Option<String>,
        signal_type: String,
        signal_value: i64,
        occurred_at: String,
        source: Option<String>,
        project: Option<String>,
    }

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_active_session_id_returns_latest_active_session() {
        let store = test_store();
        let (first, _) = store.session_start("demo", Some("first")).unwrap();
        let (second, _) = store.session_start("demo", Some("second")).unwrap();

        let active = store.active_session_id("demo").unwrap();
        assert_eq!(active.as_deref(), Some(first.as_str()));
        assert_eq!(first, second);
    }

    #[test]
    fn test_log_recall_event_persists_payload() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();
        let memory_ids = vec!["mem_1".to_string(), "mem_2".to_string()];

        let event_id = store
            .log_recall_event(
                Some(&session_id),
                "ownership borrow",
                &memory_ids,
                Some("demo"),
            )
            .unwrap();

        let row: RecallEventRow = store
            .conn
            .query_row(
                "SELECT id, session_id, query, recalled_at, memory_ids, project
                 FROM recall_events
                 WHERE id = ?1",
                params![event_id],
                |row| {
                    let ids_json: String = row.get(4)?;
                    let ids = serde_json::from_str(&ids_json).unwrap_or_default();
                    Ok(RecallEventRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        query: row.get(2)?,
                        recalled_at: row.get(3)?,
                        memory_ids: ids,
                        project: row.get(5)?,
                    })
                },
            )
            .unwrap();

        assert_eq!(row.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(row.query, "ownership borrow");
        assert_eq!(row.memory_ids, memory_ids);
        assert_eq!(row.project.as_deref(), Some("demo"));
        assert!(!row.id.is_empty());
        assert!(!row.recalled_at.is_empty());
    }

    #[test]
    fn test_log_outcome_signal_persists_payload() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let signal_id = store
            .log_outcome_signal(
                Some(&session_id),
                "session_success",
                2,
                Some("hyphae.session_end"),
                Some("demo"),
            )
            .unwrap();

        let row: OutcomeSignalRow = store
            .conn
            .query_row(
                "SELECT id, session_id, signal_type, signal_value, occurred_at, source, project
                 FROM outcome_signals
                 WHERE id = ?1",
                params![signal_id],
                |row| {
                    Ok(OutcomeSignalRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        signal_type: row.get(2)?,
                        signal_value: row.get(3)?,
                        occurred_at: row.get(4)?,
                        source: row.get(5)?,
                        project: row.get(6)?,
                    })
                },
            )
            .unwrap();

        assert_eq!(row.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(row.signal_type, "session_success");
        assert_eq!(row.signal_value, 2);
        assert_eq!(row.source.as_deref(), Some("hyphae.session_end"));
        assert_eq!(row.project.as_deref(), Some("demo"));
        assert!(!row.id.is_empty());
        assert!(!row.occurred_at.is_empty());
    }

    #[test]
    fn test_log_outcome_signal_rejects_unknown_session() {
        let store = test_store();
        let result = store.log_outcome_signal(
            Some("ses_missing"),
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            None,
        );
        assert!(matches!(result, Err(HyphaeError::NotFound(_))));
    }

    #[test]
    fn test_log_outcome_signal_rejects_project_mismatch() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();
        let result = store.log_outcome_signal(
            Some(&session_id),
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            Some("other-project"),
        );
        assert!(matches!(result, Err(HyphaeError::Validation(_))));
    }

    #[test]
    fn test_log_outcome_signal_backfills_project_from_session() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "session_success",
                2,
                Some("hyphae.session_end"),
                None,
            )
            .unwrap();

        let count = store
            .count_outcome_signals(Some(&session_id), Some("session_success"), Some(2))
            .unwrap();
        assert_eq!(count, 1);
    }
}
