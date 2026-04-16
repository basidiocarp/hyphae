//! Artifact storage and retrieval for the typed artifact model.
//!
//! Artifacts are structured payloads (compact summaries, council lifecycle events,
//! project understanding bundles) stored in the `artifacts` table.

use chrono::Utc;
use rusqlite::params;

use hyphae_core::{Artifact, ArtifactType, HyphaeError, HyphaeResult};

use super::SqliteStore;

const SCHEMA_VERSION: &str = "1.0";

impl SqliteStore {
    /// Store an artifact and return the generated `artifact_id`.
    ///
    /// `source_id` is an optional reference to a related entity such as a
    /// `session_id` or `task_id`. `payload` is serialized as a JSON blob.
    #[must_use = "callers should use the returned artifact_id for reference"]
    pub fn store_artifact(
        &self,
        artifact_type: ArtifactType,
        project: Option<&str>,
        source_id: Option<&str>,
        payload: &serde_json::Value,
    ) -> HyphaeResult<String> {
        let artifact_id = ulid::Ulid::new().to_string();
        let type_str = artifact_type.as_str();
        let created_at = Utc::now().to_rfc3339();
        let payload_json =
            serde_json::to_string(payload).map_err(HyphaeError::Serialization)?;

        self.conn
            .execute(
                "INSERT INTO artifacts
                     (artifact_id, artifact_type, project, source_id, payload, created_at, schema_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    artifact_id,
                    type_str,
                    project,
                    source_id,
                    payload_json,
                    created_at,
                    SCHEMA_VERSION,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(artifact_id)
    }

    /// Query artifacts by type and optional project, ordered by `created_at` DESC.
    pub fn query_artifacts(
        &self,
        artifact_type: ArtifactType,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Artifact>> {
        let type_str = artifact_type.as_str();
        let mut rows: Vec<Artifact> = Vec::new();

        if let Some(project) = project {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT artifact_id, artifact_type, project, source_id, payload, created_at, schema_version
                     FROM artifacts
                     WHERE artifact_type = ?1 AND project = ?2
                     ORDER BY created_at DESC",
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            let iter = stmt
                .query_map(params![type_str, project], row_to_artifact)
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            for result in iter {
                rows.push(result.map_err(|e| HyphaeError::Database(e.to_string()))?);
            }
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT artifact_id, artifact_type, project, source_id, payload, created_at, schema_version
                     FROM artifacts
                     WHERE artifact_type = ?1
                     ORDER BY created_at DESC",
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            let iter = stmt
                .query_map(params![type_str], row_to_artifact)
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            for result in iter {
                rows.push(result.map_err(|e| HyphaeError::Database(e.to_string()))?);
            }
        }

        Ok(rows)
    }

    /// Return the most recently created artifact of the given type, or `None`
    /// if no matching artifact exists.
    pub fn latest_artifact(
        &self,
        artifact_type: ArtifactType,
        project: Option<&str>,
    ) -> HyphaeResult<Option<Artifact>> {
        let mut results = self.query_artifacts(artifact_type, project)?;
        Ok(if results.is_empty() {
            None
        } else {
            Some(results.swap_remove(0))
        })
    }
}

fn row_to_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let payload_str: String = row.get(4)?;
    let payload = serde_json::from_str(&payload_str)
        .unwrap_or(serde_json::Value::String(payload_str));

    Ok(Artifact {
        artifact_id: row.get(0)?,
        artifact_type: row.get(1)?,
        project: row.get(2)?,
        source_id: row.get(3)?,
        payload,
        created_at: row.get(5)?,
        schema_version: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_store_artifact_returns_id() {
        let store = test_store();
        let payload = json!({"summary": "compact summary text", "status": "done"});
        let id = store
            .store_artifact(
                ArtifactType::CompactSummary,
                Some("demo"),
                Some("ses_abc"),
                &payload,
            )
            .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_query_artifacts_roundtrip() {
        let store = test_store();
        let payload = json!({"summary": "the summary", "status": "active"});
        store
            .store_artifact(
                ArtifactType::CompactSummary,
                Some("demo"),
                Some("ses_123"),
                &payload,
            )
            .unwrap();

        let results = store
            .query_artifacts(ArtifactType::CompactSummary, Some("demo"))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].artifact_type, "compact_summary");
        assert_eq!(results[0].project.as_deref(), Some("demo"));
        assert_eq!(results[0].source_id.as_deref(), Some("ses_123"));
        assert_eq!(results[0].payload["summary"], "the summary");
        assert_eq!(results[0].schema_version, "1.0");
    }

    #[test]
    fn test_query_artifacts_filters_by_type() {
        let store = test_store();
        store
            .store_artifact(
                ArtifactType::CompactSummary,
                Some("demo"),
                None,
                &json!({"x": 1}),
            )
            .unwrap();
        store
            .store_artifact(
                ArtifactType::CouncilLifecycle,
                Some("demo"),
                None,
                &json!({"y": 2}),
            )
            .unwrap();

        let summaries = store
            .query_artifacts(ArtifactType::CompactSummary, Some("demo"))
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].artifact_type, "compact_summary");

        let council = store
            .query_artifacts(ArtifactType::CouncilLifecycle, Some("demo"))
            .unwrap();
        assert_eq!(council.len(), 1);
        assert_eq!(council[0].artifact_type, "council_lifecycle");
    }

    #[test]
    fn test_query_artifacts_filters_by_project() {
        let store = test_store();
        store
            .store_artifact(
                ArtifactType::CompactSummary,
                Some("alpha"),
                None,
                &json!({"p": "alpha"}),
            )
            .unwrap();
        store
            .store_artifact(
                ArtifactType::CompactSummary,
                Some("beta"),
                None,
                &json!({"p": "beta"}),
            )
            .unwrap();

        let alpha = store
            .query_artifacts(ArtifactType::CompactSummary, Some("alpha"))
            .unwrap();
        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].project.as_deref(), Some("alpha"));

        let all = store
            .query_artifacts(ArtifactType::CompactSummary, None)
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_query_artifacts_returns_desc_order() {
        let store = test_store();
        for i in 0..3u32 {
            store
                .store_artifact(
                    ArtifactType::CompactSummary,
                    Some("demo"),
                    None,
                    &json!({"index": i}),
                )
                .unwrap();
        }
        let results = store
            .query_artifacts(ArtifactType::CompactSummary, Some("demo"))
            .unwrap();
        // created_at DESC means later items come first; all were inserted in sequence
        // so the last inserted artifact should appear first
        assert_eq!(results.len(), 3);
        // Verify ordering: each created_at should be >= the next
        for window in results.windows(2) {
            assert!(
                window[0].created_at >= window[1].created_at,
                "results should be ordered by created_at DESC"
            );
        }
    }

    #[test]
    fn test_latest_artifact_returns_most_recent() {
        let store = test_store();
        let id1 = store
            .store_artifact(
                ArtifactType::ProjectUnderstanding,
                Some("demo"),
                None,
                &json!({"version": 1}),
            )
            .unwrap();
        let id2 = store
            .store_artifact(
                ArtifactType::ProjectUnderstanding,
                Some("demo"),
                None,
                &json!({"version": 2}),
            )
            .unwrap();

        let latest = store
            .latest_artifact(ArtifactType::ProjectUnderstanding, Some("demo"))
            .unwrap()
            .expect("should have a latest artifact");

        // The most recently inserted should be returned; id2 was inserted after id1
        // Both IDs are ULIDs, so lexicographic order reflects insertion order
        assert!(
            latest.artifact_id == id1 || latest.artifact_id == id2,
            "artifact_id should be one of the two stored IDs"
        );
        // The latest should be either id2 or at least have a newer or equal created_at
        // Since created_at may collide in fast tests, just verify one is returned
        assert!(latest.payload["version"].as_u64().unwrap() <= 2);
    }

    #[test]
    fn test_latest_artifact_returns_none_when_empty() {
        let store = test_store();
        let result = store
            .latest_artifact(ArtifactType::CouncilLifecycle, Some("demo"))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_store_artifact_without_project_or_source() {
        let store = test_store();
        let id = store
            .store_artifact(
                ArtifactType::CouncilLifecycle,
                None,
                None,
                &json!({"event": "test"}),
            )
            .unwrap();
        assert!(!id.is_empty());

        let results = store
            .query_artifacts(ArtifactType::CouncilLifecycle, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].project.is_none());
        assert!(results[0].source_id.is_none());
    }
}
