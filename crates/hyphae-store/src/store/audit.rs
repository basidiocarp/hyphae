use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use hyphae_core::{HyphaeError, HyphaeResult, Memory, MemoryId, MemoryStore};

use super::SqliteStore;

// ---------------------------------------------------------------------------
// Audit log types
// ---------------------------------------------------------------------------

/// The kind of mutation recorded in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    Store,
    Update,
    Delete,
    Invalidate,
    Decay,
    Prune,
    PruneExpired,
    Consolidate,
}

impl AuditOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Invalidate => "invalidate",
            Self::Decay => "decay",
            Self::Prune => "prune",
            Self::PruneExpired => "prune_expired",
            Self::Consolidate => "consolidate",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "store" => Some(Self::Store),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "invalidate" => Some(Self::Invalidate),
            "decay" => Some(Self::Decay),
            "prune" => Some(Self::Prune),
            "prune_expired" => Some(Self::PruneExpired),
            "consolidate" => Some(Self::Consolidate),
            _ => None,
        }
    }
}

impl std::fmt::Display for AuditOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single entry in the append-only audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub operation: AuditOperation,
    pub memory_id: String,
    pub topic: Option<String>,
    pub content_hash: Option<String>,
    pub metadata_json: Option<String>,
}

/// Stable FNV-1a hash for content fingerprinting in audit entries.
///
/// Unlike `DefaultHasher`, FNV-1a produces the same output across Rust
/// versions, which matters for audit log consistency.
fn content_hash(text: &str) -> u64 {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// ---------------------------------------------------------------------------
// SqliteStore audit methods
// ---------------------------------------------------------------------------

impl SqliteStore {
    /// Write an audit record. Called BEFORE the actual mutation to survive crashes.
    pub(crate) fn write_audit(
        &self,
        operation: AuditOperation,
        memory_id: &str,
        topic: Option<&str>,
        content_hash: Option<&str>,
        metadata_json: Option<&str>,
    ) -> HyphaeResult<String> {
        let id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO audit_log (id, timestamp, operation, memory_id, topic, content_hash, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    now,
                    operation.as_str(),
                    memory_id,
                    topic,
                    content_hash,
                    metadata_json,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(id)
    }

    /// Write an audit record for a memory struct, computing a content hash from its summary.
    pub(crate) fn audit_memory(
        &self,
        operation: AuditOperation,
        memory: &Memory,
    ) -> HyphaeResult<String> {
        let content_hash = format!("{:016x}", content_hash(&memory.summary));

        let meta = serde_json::json!({
            "importance": memory.importance.to_string(),
            "project": memory.project,
            "weight": memory.weight.value(),
        });

        self.write_audit(
            operation,
            memory.id.as_ref(),
            Some(&memory.topic),
            Some(&content_hash),
            Some(&meta.to_string()),
        )
    }

    /// List audit entries with optional filters.
    pub fn audit_list(
        &self,
        since: Option<&str>,
        operation: Option<AuditOperation>,
        limit: usize,
    ) -> HyphaeResult<Vec<AuditEntry>> {
        let sql = "SELECT id, timestamp, operation, memory_id, topic, content_hash, metadata_json
             FROM audit_log
             WHERE (?1 IS NULL OR timestamp >= ?1)
               AND (?2 IS NULL OR operation = ?2)
             ORDER BY timestamp DESC
             LIMIT ?3";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let since_param = since.map(|s| s.to_string());
        let op_param = operation.map(|o| o.as_str().to_string());

        let rows = stmt
            .query_map(params![since_param, op_param, limit as i64], |row| {
                let ts_str: String = row.get(1)?;
                let op_str: String = row.get(2)?;
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: parse_audit_timestamp(&ts_str)?,
                    operation: parse_audit_operation(&op_str)?,
                    memory_id: row.get(3)?,
                    topic: row.get(4)?,
                    content_hash: row.get(5)?,
                    metadata_json: row.get(6)?,
                })
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Get a single audit entry by id.
    pub fn audit_get(&self, audit_id: &str) -> HyphaeResult<Option<AuditEntry>> {
        let result = self
            .conn
            .query_row(
                "SELECT id, timestamp, operation, memory_id, topic, content_hash, metadata_json
                 FROM audit_log WHERE id = ?1",
                params![audit_id],
                |row| {
                    let ts_str: String = row.get(1)?;
                    let op_str: String = row.get(2)?;
                    Ok(AuditEntry {
                        id: row.get(0)?,
                        timestamp: parse_audit_timestamp(&ts_str)?,
                        operation: parse_audit_operation(&op_str)?,
                        memory_id: row.get(3)?,
                        topic: row.get(4)?,
                        content_hash: row.get(5)?,
                        metadata_json: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(result)
    }

    /// Rollback a single mutation by audit id.
    ///
    /// - For `store`: deletes the stored memory
    /// - For `invalidate`: clears the invalidation fields
    /// - For `update`/`delete`/`decay`/`prune`/`consolidate`: not reversible,
    ///   returns an error explaining why
    pub fn audit_rollback(&self, audit_id: &str) -> HyphaeResult<String> {
        let entry = self
            .audit_get(audit_id)?
            .ok_or_else(|| HyphaeError::NotFound(format!("audit entry: {audit_id}")))?;

        match entry.operation {
            AuditOperation::Store => {
                // Refuse rollback if the memory was updated after creation
                let updated_after: bool = self
                    .conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM audit_log WHERE memory_id = ?1 AND operation = 'update' AND timestamp > ?2)",
                        params![entry.memory_id, entry.timestamp.to_rfc3339()],
                        |row| row.get(0),
                    )
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
                if updated_after {
                    return Err(HyphaeError::Validation(format!(
                        "cannot rollback store for '{}': memory was updated after creation",
                        entry.memory_id
                    )));
                }
                let mem_id = MemoryId::from(entry.memory_id.clone());
                self.delete(&mem_id)?;
                Ok(format!(
                    "Rolled back store: deleted memory {}",
                    entry.memory_id
                ))
            }
            AuditOperation::Invalidate => {
                self.conn
                    .execute(
                        "UPDATE memories
                         SET invalidated_at = NULL,
                             invalidation_reason = NULL,
                             superseded_by = NULL,
                             updated_at = ?2
                         WHERE id = ?1",
                        params![entry.memory_id, Utc::now().to_rfc3339()],
                    )
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
                Ok(format!(
                    "Rolled back invalidate: restored memory {}",
                    entry.memory_id
                ))
            }
            other => Err(HyphaeError::Validation(format!(
                "cannot rollback '{}' operations: the previous state is not recorded",
                other
            ))),
        }
    }
}

use rusqlite::OptionalExtension;

/// Parse an RFC 3339 timestamp from an audit row, returning a `rusqlite::Error`
/// on failure so it can propagate through `query_row` / `query_map` closures.
fn parse_audit_timestamp(ts_str: &str) -> Result<DateTime<Utc>, rusqlite::Error> {
    chrono::DateTime::parse_from_rfc3339(ts_str)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid audit timestamp: {e}"),
                )),
            )
        })
}

/// Parse an `AuditOperation` from a stored string, returning a `rusqlite::Error`
/// when the value does not match any known variant.
fn parse_audit_operation(op_str: &str) -> Result<AuditOperation, rusqlite::Error> {
    AuditOperation::parse(op_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown audit operation: {op_str}"),
            )),
        )
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_write_and_list_audit() {
        let store = test_store();

        store
            .write_audit(AuditOperation::Store, "mem-1", Some("test"), None, None)
            .unwrap();
        store
            .write_audit(AuditOperation::Update, "mem-1", Some("test"), None, None)
            .unwrap();

        let entries = store.audit_list(None, None, 10).unwrap();
        assert_eq!(entries.len(), 2);
        // Most recent first
        assert_eq!(entries[0].operation, AuditOperation::Update);
        assert_eq!(entries[1].operation, AuditOperation::Store);
    }

    #[test]
    fn test_audit_filter_by_operation() {
        let store = test_store();

        store
            .write_audit(AuditOperation::Store, "mem-1", Some("a"), None, None)
            .unwrap();
        store
            .write_audit(AuditOperation::Delete, "mem-2", Some("b"), None, None)
            .unwrap();
        store
            .write_audit(AuditOperation::Store, "mem-3", Some("c"), None, None)
            .unwrap();

        let entries = store
            .audit_list(None, Some(AuditOperation::Store), 10)
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.operation == AuditOperation::Store));
    }

    #[test]
    fn test_audit_get() {
        let store = test_store();

        let id = store
            .write_audit(AuditOperation::Store, "mem-1", Some("topic"), None, None)
            .unwrap();

        let entry = store.audit_get(&id).unwrap().unwrap();
        assert_eq!(entry.memory_id, "mem-1");
        assert_eq!(entry.operation, AuditOperation::Store);

        assert!(store.audit_get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_audit_rollback_store() {
        let store = test_store();
        let mem = Memory::new(
            "rollback-test".to_string(),
            "Should be deleted".to_string(),
            Importance::Medium,
        );
        let mem_id = mem.id.clone();
        store.store(mem).unwrap();

        // Find the audit entry (written by the audit-wrapped store)
        let entries = store
            .audit_list(None, Some(AuditOperation::Store), 10)
            .unwrap();
        assert!(!entries.is_empty());
        let audit_id = &entries[0].id;

        let msg = store.audit_rollback(audit_id).unwrap();
        assert!(msg.contains("Rolled back store"));

        // Memory should be gone
        assert!(store.get(&mem_id).unwrap().is_none());
    }

    #[test]
    fn test_audit_rollback_invalidate() {
        let store = test_store();
        let mem = Memory::new(
            "rollback-inval".to_string(),
            "Should be restored".to_string(),
            Importance::Medium,
        );
        let mem_id = mem.id.clone();
        store.store(mem).unwrap();
        store.invalidate(&mem_id, Some("test"), None).unwrap();

        let entries = store
            .audit_list(None, Some(AuditOperation::Invalidate), 10)
            .unwrap();
        assert!(!entries.is_empty());
        let audit_id = &entries[0].id;

        let msg = store.audit_rollback(audit_id).unwrap();
        assert!(msg.contains("Rolled back invalidate"));

        // Memory should be active again
        let restored = store.get(&mem_id).unwrap().unwrap();
        assert!(restored.invalidated_at.is_none());
    }

    #[test]
    fn test_audit_rollback_non_reversible() {
        let store = test_store();
        let id = store
            .write_audit(AuditOperation::Decay, "mem-1", None, None, None)
            .unwrap();

        let result = store.audit_rollback(&id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot rollback"));
    }

    #[test]
    fn test_audit_memory_helper() {
        let store = test_store();
        let mem = Memory::new(
            "audit-helper".to_string(),
            "Test audit helper".to_string(),
            Importance::High,
        );

        let id = store.audit_memory(AuditOperation::Store, &mem).unwrap();
        let entry = store.audit_get(&id).unwrap().unwrap();
        assert_eq!(entry.topic.as_deref(), Some("audit-helper"));
        assert!(entry.content_hash.is_some());
        assert!(entry.metadata_json.is_some());
    }

    #[test]
    fn test_audit_operation_roundtrip() {
        let ops = [
            AuditOperation::Store,
            AuditOperation::Update,
            AuditOperation::Delete,
            AuditOperation::Invalidate,
            AuditOperation::Decay,
            AuditOperation::Prune,
            AuditOperation::PruneExpired,
            AuditOperation::Consolidate,
        ];
        for op in ops {
            assert_eq!(AuditOperation::parse(op.as_str()), Some(op));
        }
        assert_eq!(AuditOperation::parse("unknown"), None);
    }
}
