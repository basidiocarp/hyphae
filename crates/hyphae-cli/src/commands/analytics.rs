use anyhow::Result;
use hyphae_store::SqliteStore;
use serde_json::json;

const ANALYTICS_SCHEMA_VERSION: &str = "1.0";

pub(crate) fn cmd_analytics(store: &SqliteStore, project: Option<String>) -> Result<()> {
    let payload = store.analytics_snapshot(project.as_deref())?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schema_version": ANALYTICS_SCHEMA_VERSION,
            "analytics": payload,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_payload_is_versioned() {
        let store = SqliteStore::in_memory().unwrap();
        let payload = store.analytics_snapshot(None).unwrap();
        let json = json!({
            "schema_version": ANALYTICS_SCHEMA_VERSION,
            "analytics": payload,
        });

        assert_eq!(json["schema_version"].as_str(), Some("1.0"));
        assert!(json["analytics"].is_object());
    }
}
