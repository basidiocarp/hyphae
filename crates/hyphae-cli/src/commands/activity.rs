use anyhow::Result;
use hyphae_store::SqliteStore;
use serde_json::json;

const ACTIVITY_SCHEMA_VERSION: &str = "1.0";

pub(crate) fn cmd_activity(store: &SqliteStore, project: Option<String>) -> Result<()> {
    let payload = store.activity_snapshot(project.as_deref())?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schema_version": ACTIVITY_SCHEMA_VERSION,
            "snapshot": payload,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_payload_is_versioned() {
        let store = SqliteStore::in_memory().unwrap();
        let payload = store.activity_snapshot(None).unwrap();
        let json = json!({
            "schema_version": ACTIVITY_SCHEMA_VERSION,
            "snapshot": payload,
        });

        assert_eq!(json["schema_version"].as_str(), Some("1.0"));
        assert!(json["snapshot"].is_object());
    }
}
