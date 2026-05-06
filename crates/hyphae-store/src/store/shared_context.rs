use chrono::Utc;
use hyphae_core::{HyphaeError, HyphaeResult, SharedContextEntry, SharedContextStore};
use rusqlite::params;

use crate::SqliteStore;

impl SharedContextStore for SqliteStore {
    fn put_context(
        &self,
        session_id: &str,
        agent_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> HyphaeResult<String> {
        let entry_id = ulid::Ulid::new().to_string();
        let written_at = Utc::now().to_rfc3339();
        let value_str = serde_json::to_string(&value)?;

        self.conn
            .execute(
                "INSERT INTO shared_context (entry_id, session_id, agent_id, key, value, written_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![&entry_id, session_id, agent_id, key, &value_str, &written_at],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(entry_id)
    }

    fn get_context(&self, session_id: &str, key: &str) -> HyphaeResult<Option<SharedContextEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_id, session_id, agent_id, key, value, written_at
             FROM shared_context
             WHERE session_id = ?1 AND key = ?2
             ORDER BY written_at DESC
             LIMIT 1",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let result = stmt.query_row(params![session_id, key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        });

        match result {
            Ok((entry_id, session_id_col, agent_id, key_col, value_str, written_at_str)) => {
                let value: serde_json::Value = serde_json::from_str(&value_str)?;
                let written_at = written_at_str
                    .parse::<chrono::DateTime<Utc>>()
                    .map_err(|e| HyphaeError::Validation(format!("invalid timestamp: {e}")))?;
                Ok(Some(SharedContextEntry {
                    entry_id,
                    session_id: session_id_col,
                    agent_id,
                    key: key_col,
                    value,
                    written_at,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(HyphaeError::Database(e.to_string())),
        }
    }

    fn list_context_keys(&self, session_id: &str) -> HyphaeResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT key FROM shared_context
             WHERE session_id = ?1
             GROUP BY key
             ORDER BY MAX(written_at) DESC",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let keys = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (SqliteStore, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let db_path = dir.path().join("test.db");
        let store = SqliteStore::new(&db_path).expect("failed to create store");
        (store, dir)
    }

    #[test]
    fn test_put_and_get_context() {
        let (store, _dir) = setup_store();
        let session_id = "session-123";
        let agent_id = "agent-456";
        let key = "config";
        let value = serde_json::json!({"enabled": true, "timeout": 30});

        let entry_id = store
            .put_context(session_id, agent_id, key, value.clone())
            .expect("put_context failed");

        assert!(!entry_id.is_empty());

        let retrieved = store
            .get_context(session_id, key)
            .expect("get_context failed")
            .expect("context not found");

        assert_eq!(retrieved.entry_id, entry_id);
        assert_eq!(retrieved.session_id, session_id);
        assert_eq!(retrieved.agent_id, agent_id);
        assert_eq!(retrieved.key, key);
        assert_eq!(retrieved.value, value);
    }

    #[test]
    fn test_get_context_not_found() {
        let (store, _dir) = setup_store();
        let session_id = "session-123";
        let key = "nonexistent";

        let result = store
            .get_context(session_id, key)
            .expect("get_context failed");

        assert_eq!(result, None);
    }

    #[test]
    fn test_overwrite_context_returns_newest() {
        let (store, _dir) = setup_store();
        let session_id = "session-123";
        let agent_id = "agent-456";
        let key = "state";

        let value1 = serde_json::json!({"version": 1});
        let _entry_id_1 = store
            .put_context(session_id, agent_id, key, value1)
            .expect("first put_context failed");

        // Small delay to ensure written_at differs
        std::thread::sleep(std::time::Duration::from_millis(10));

        let value2 = serde_json::json!({"version": 2});
        let entry_id_2 = store
            .put_context(session_id, agent_id, key, value2.clone())
            .expect("second put_context failed");

        let retrieved = store
            .get_context(session_id, key)
            .expect("get_context failed")
            .expect("context not found");

        assert_eq!(retrieved.entry_id, entry_id_2);
        assert_eq!(retrieved.value, value2);
    }

    #[test]
    fn test_list_context_keys() {
        let (store, _dir) = setup_store();
        let session_id = "session-123";
        let agent_id = "agent-456";

        store
            .put_context(session_id, agent_id, "key1", serde_json::json!({"a": 1}))
            .expect("put_context failed");

        std::thread::sleep(std::time::Duration::from_millis(10));

        store
            .put_context(session_id, agent_id, "key2", serde_json::json!({"b": 2}))
            .expect("put_context failed");

        let keys = store
            .list_context_keys(session_id)
            .expect("list_context_keys failed");

        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "key2"); // Most recent first
        assert_eq!(keys[1], "key1");
    }

    #[test]
    fn test_list_context_keys_empty_session() {
        let (store, _dir) = setup_store();
        let session_id = "nonexistent-session";

        let keys = store
            .list_context_keys(session_id)
            .expect("list_context_keys failed");

        assert!(keys.is_empty());
    }

    #[test]
    fn test_multiple_sessions_isolated() {
        let (store, _dir) = setup_store();
        let session_id_1 = "session-1";
        let session_id_2 = "session-2";
        let agent_id = "agent-456";
        let key = "shared_key";

        let value1 = serde_json::json!({"session": 1});
        store
            .put_context(session_id_1, agent_id, key, value1.clone())
            .expect("put_context failed");

        let value2 = serde_json::json!({"session": 2});
        store
            .put_context(session_id_2, agent_id, key, value2.clone())
            .expect("put_context failed");

        let retrieved_1 = store
            .get_context(session_id_1, key)
            .expect("get_context failed")
            .expect("context not found");

        let retrieved_2 = store
            .get_context(session_id_2, key)
            .expect("get_context failed")
            .expect("context not found");

        assert_eq!(retrieved_1.value, value1);
        assert_eq!(retrieved_2.value, value2);
    }
}
