use serde_json::{Value, json};
use std::str::FromStr;

use hyphae_core::{ApplicabilityRule, Authority, InputSpec, KnowledgeDomain};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{ToolTraceContext, get_str};

pub(crate) fn tool_domain_upsert(
    store: &SqliteStore,
    args: &Value,
    _trace: &ToolTraceContext,
) -> ToolResult {
    let id = match get_str(args, "id") {
        Some(s) => s,
        None => return ToolResult::error("id required".to_string()),
    };
    let description = get_str(args, "description").unwrap_or("").to_owned();
    let authority_str = get_str(args, "authority").unwrap_or("primary");
    let authority = Authority::from_str(authority_str).unwrap_or_default();

    let applies_when: Vec<ApplicabilityRule> = args
        .get("applies_when")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let required_inputs: Vec<InputSpec> = args
        .get("required_inputs")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let domain = KnowledgeDomain {
        id: id.to_owned(),
        description,
        applies_when,
        required_inputs,
        query_template: get_str(args, "query_template").map(ToOwned::to_owned),
        authority,
        freshness_ttl_secs: args.get("freshness_ttl_secs").and_then(|v| v.as_u64()),
        boundary_note: get_str(args, "boundary_note").map(ToOwned::to_owned),
    };

    match store.upsert_knowledge_domain(&domain) {
        Ok(()) => {
            let result = json!({ "id": domain.id, "status": "upserted" });
            match serde_json::to_string_pretty(&result) {
                Ok(json) => ToolResult::text(json),
                Err(e) => ToolResult::error(format!("serialization error: {e}")),
            }
        }
        Err(e) => ToolResult::error(format!("failed to upsert domain: {e}")),
    }
}

pub(crate) fn tool_domain_list(
    store: &SqliteStore,
    _args: &Value,
    _trace: &ToolTraceContext,
) -> ToolResult {
    match store.list_knowledge_domains() {
        Ok(domains) => {
            let result = json!({ "domains": domains });
            match serde_json::to_string_pretty(&result) {
                Ok(json) => ToolResult::text(json),
                Err(e) => ToolResult::error(format!("serialization error: {e}")),
            }
        }
        Err(e) => ToolResult::error(format!("failed to list domains: {e}")),
    }
}
