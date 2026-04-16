//! MCP tools for the typed artifact storage model.
//!
//! Provides `hyphae_artifact_store` and `hyphae_artifact_query`.

use serde_json::Value;

use hyphae_core::ArtifactType;
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{ToolTraceContext, get_str, validate_required_string};

/// `hyphae_artifact_store` — store a typed artifact payload.
pub(crate) fn tool_artifact_store(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    _trace: &ToolTraceContext,
) -> ToolResult {
    let artifact_type_str = match validate_required_string(args, "artifact_type") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let artifact_type = match artifact_type_str.parse::<ArtifactType>() {
        Ok(t) => t,
        Err(_) => {
            return ToolResult::error(format!(
                "unknown artifact_type '{artifact_type_str}'; \
                 expected one of: compact_summary, council_lifecycle, project_understanding"
            ));
        }
    };

    let payload = match args.get("payload") {
        Some(p) => p,
        None => return ToolResult::error("missing required field: payload".into()),
    };

    let project = get_str(args, "project").or(project);
    let source_id = get_str(args, "source_id");

    match store.store_artifact(artifact_type, project, source_id, payload) {
        Ok(id) => ToolResult::text(format!("Stored artifact {id} (type={artifact_type_str})")),
        Err(e) => ToolResult::error(format!("failed to store artifact: {e}")),
    }
}

/// `hyphae_artifact_query` — query artifacts by type and optional project.
pub(crate) fn tool_artifact_query(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    _trace: &ToolTraceContext,
) -> ToolResult {
    let artifact_type_str = match validate_required_string(args, "artifact_type") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let artifact_type = match artifact_type_str.parse::<ArtifactType>() {
        Ok(t) => t,
        Err(_) => {
            return ToolResult::error(format!(
                "unknown artifact_type '{artifact_type_str}'; \
                 expected one of: compact_summary, council_lifecycle, project_understanding"
            ));
        }
    };

    let project = get_str(args, "project").or(project);

    match store.query_artifacts(artifact_type, project) {
        Ok(artifacts) if artifacts.is_empty() => {
            ToolResult::text("No artifacts found.".into())
        }
        Ok(artifacts) => {
            match serde_json::to_string_pretty(&artifacts) {
                Ok(json) => ToolResult::text(json),
                Err(e) => ToolResult::error(format!("serialization error: {e}")),
            }
        }
        Err(e) => ToolResult::error(format!("failed to query artifacts: {e}")),
    }
}
