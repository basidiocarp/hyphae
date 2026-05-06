use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::HyphaeResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedContextEntry {
    pub entry_id: String, // ULID
    pub session_id: String,
    pub agent_id: String,
    pub key: String,
    pub value: serde_json::Value,
    pub written_at: DateTime<Utc>,
}

pub trait SharedContextStore {
    /// Write or overwrite a key. Returns the new entry_id.
    fn put_context(
        &self,
        session_id: &str,
        agent_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> HyphaeResult<String>;

    /// Read the most recent value for a key. Returns None if never written.
    fn get_context(&self, session_id: &str, key: &str) -> HyphaeResult<Option<SharedContextEntry>>;

    /// List all distinct keys written in this session, most-recently-written first.
    fn list_context_keys(&self, session_id: &str) -> HyphaeResult<Vec<String>>;
}
