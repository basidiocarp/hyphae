use chrono::Utc;
use serde_json::{json, Value};
use spore::logging::workflow_span;

use hyphae_core::{MemoryId, ReflexionConfidence, ReflexionErrorType, ReflexionRecord, ReflexionStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::super::{ToolTraceContext, get_str, validate_required_string, workflow_span_context};

pub(crate) fn tool_reflexion_record(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let error_type_str = match validate_required_string(args, "error_type") {
        Ok(s) => s,
        Err(e) => return e,
    };

    let root_cause = match validate_required_string(args, "root_cause") {
        Ok(s) => s,
        Err(e) => return e,
    };

    let fix_applied = match validate_required_string(args, "fix_applied") {
        Ok(s) => s,
        Err(e) => return e,
    };

    let abstract_pattern = match validate_required_string(args, "abstract_pattern") {
        Ok(s) => s,
        Err(e) => return e,
    };

    let project = get_str(args, "project").map(String::from);
    let confidence_str = get_str(args, "confidence").unwrap_or("medium");

    let error_type = error_type_str
        .parse::<ReflexionErrorType>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                "unrecognized error_type {}, defaulting to Other",
                error_type_str
            );
            ReflexionErrorType::Other
        });

    let confidence = confidence_str
        .parse::<ReflexionConfidence>()
        .unwrap_or_else(|_| {
            tracing::warn!(
                "unrecognized confidence {}, defaulting to Medium",
                confidence_str
            );
            ReflexionConfidence::Medium
        });

    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("reflexion_record", &workflow_context).entered();

    let record = ReflexionRecord {
        id: format!("refl_{}", MemoryId::new()),
        error_type,
        root_cause: root_cause.to_string(),
        fix_applied: fix_applied.to_string(),
        abstract_pattern: abstract_pattern.to_string(),
        project,
        confidence,
        created_at: Utc::now(),
    };

    let record_id = record.id.clone();
    match store.store_reflexion(&record) {
        Ok(_) => ToolResult::text(json!({
            "id": record_id,
            "stored_at": Utc::now().to_rfc3339()
        }).to_string()),
        Err(e) => ToolResult::error(format!("failed to store reflexion record: {e}")),
    }
}

pub(crate) fn tool_reflexion_search(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let query = match validate_required_string(args, "query") {
        Ok(s) => s,
        Err(e) => return e,
    };

    let error_type_str = get_str(args, "error_type");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .map(|v| (v as usize).min(100))
        .unwrap_or(10);

    let error_type = error_type_str.and_then(|s| s.parse::<ReflexionErrorType>().ok());

    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("reflexion_search", &workflow_context).entered();

    match store.search_reflexions(query, error_type.as_ref(), limit) {
        Ok(records) => {
            let result = json!({
                "query": query,
                "limit": limit,
                "count": records.len(),
                "records": records
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "error_type": r.error_type.to_string(),
                            "root_cause": r.root_cause,
                            "fix_applied": r.fix_applied,
                            "abstract_pattern": r.abstract_pattern,
                            "project": r.project,
                            "confidence": r.confidence.to_string(),
                            "created_at": r.created_at.to_rfc3339()
                        })
                    })
                    .collect::<Vec<_>>()
            });
            ToolResult::text(result.to_string())
        }
        Err(e) => ToolResult::error(format!("reflexion search failed: {e}")),
    }
}
