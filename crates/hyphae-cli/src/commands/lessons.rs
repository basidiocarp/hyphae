use anyhow::Result;
use hyphae_store::SqliteStore;
use serde_json::json;

const LESSONS_SCHEMA_VERSION: &str = "1.0";

pub(crate) fn cmd_lessons(
    store: &SqliteStore,
    project: Option<String>,
    per_topic_limit: usize,
) -> Result<()> {
    let lessons = store.extract_lessons(project.as_deref(), per_topic_limit)?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schema_version": LESSONS_SCHEMA_VERSION,
            "lessons": lessons,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lessons_payload_is_versioned() {
        let store = SqliteStore::in_memory().unwrap();
        let lessons = store.extract_lessons(None, 50).unwrap();
        let json = json!({
            "schema_version": LESSONS_SCHEMA_VERSION,
            "lessons": lessons,
        });

        assert_eq!(json["schema_version"].as_str(), Some("1.0"));
        assert!(json["lessons"].is_array());
    }
}
