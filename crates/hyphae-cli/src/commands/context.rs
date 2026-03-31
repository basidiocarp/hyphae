use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use hyphae_mcp::tools::call_tool;
use hyphae_store::SqliteStore;
use serde_json::{Value, json};

const GATHER_CONTEXT_SCHEMA_VERSION: &str = "1.0";

#[derive(Args)]
pub(crate) struct GatherContextArgs {
    /// Task description to gather context for
    #[arg(short, long)]
    pub(crate) task: String,
    /// Gather across all projects instead of using the resolved/default project
    #[arg(long)]
    pub(crate) all_projects: bool,
    /// Optional repository root for identity v1 lookup
    #[arg(long)]
    pub(crate) project_root: Option<String>,
    /// Optional worktree identifier for identity v1 lookup
    #[arg(long)]
    pub(crate) worktree_id: Option<String>,
    /// Optional worker or runtime scope filter
    #[arg(long)]
    pub(crate) scope: Option<String>,
    /// Maximum tokens to include in the result
    #[arg(long = "token-budget", default_value = "2000")]
    pub(crate) token_budget: i64,
    /// Include one or more sources: memories, errors, sessions, code
    #[arg(long = "include")]
    pub(crate) include: Vec<String>,
}

pub(crate) fn dispatch(
    store: &SqliteStore,
    args: GatherContextArgs,
    project: Option<&str>,
) -> Result<()> {
    println!(
        "{}",
        gather_context_envelope(store, &args, effective_project(&args, project))?
    );
    Ok(())
}

fn effective_project<'a>(
    args: &GatherContextArgs,
    resolved_project: Option<&'a str>,
) -> Option<&'a str> {
    if args.all_projects {
        None
    } else {
        resolved_project
    }
}

fn gather_context_payload(
    store: &SqliteStore,
    args: &GatherContextArgs,
    project: Option<&str>,
) -> Result<String> {
    let tool_args = json!({
        "task": args.task,
        "project_root": args.project_root,
        "worktree_id": args.worktree_id,
        "scope": args.scope,
        "token_budget": args.token_budget,
        "include": (!args.include.is_empty()).then_some(args.include.as_slice()),
    });

    let result = call_tool(
        store,
        None,
        "hyphae_gather_context",
        &tool_args,
        false,
        project,
        false,
    );

    if result.is_error {
        bail!(
            "{}",
            result
                .content
                .first()
                .map(|block| block.text.as_str())
                .unwrap_or("gather-context failed")
        );
    }

    result
        .content
        .first()
        .map(|block| block.text.clone())
        .ok_or_else(|| anyhow!("gather-context returned no content"))
}

fn gather_context_envelope(
    store: &SqliteStore,
    args: &GatherContextArgs,
    project: Option<&str>,
) -> Result<String> {
    let payload = gather_context_payload(store, args, project)?;
    let Value::Object(mut record) =
        serde_json::from_str::<Value>(&payload).context("gather-context returned invalid JSON")?
    else {
        bail!("gather-context returned a non-object payload");
    };
    record.insert(
        "schema_version".to_string(),
        Value::String(GATHER_CONTEXT_SCHEMA_VERSION.to_string()),
    );
    serde_json::to_string(&Value::Object(record))
        .context("failed to serialize gather-context envelope")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn gather_args(task: &str) -> GatherContextArgs {
        GatherContextArgs {
            task: task.to_string(),
            all_projects: false,
            project_root: None,
            worktree_id: None,
            scope: None,
            token_budget: 2000,
            include: Vec::new(),
        }
    }

    #[test]
    fn test_gather_context_payload_returns_json_shape() {
        let store = test_store();
        let memory = Memory::builder(
            "architecture".to_string(),
            "Auth middleware uses JWT with RS256".to_string(),
            Importance::High,
        )
        .project("demo".to_string())
        .build();
        store.store(memory).unwrap();

        let mut args = gather_args("auth middleware");
        args.include = vec!["memories".to_string()];

        let payload = gather_context_envelope(&store, &args, Some("demo")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(parsed["tokens_budget"].as_i64(), Some(2000));
        assert_eq!(parsed["sources_queried"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["sources_queried"][0].as_str(), Some("memories"));
        assert_eq!(parsed["context"][0]["source"].as_str(), Some("memory"));
    }

    #[test]
    fn test_gather_context_payload_forwards_identity_scope_filters() {
        let store = test_store();

        let (worker_a_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(
                &worker_a_id,
                Some("Worker A login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let (worker_b_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
            )
            .unwrap();
        store
            .session_end(
                &worker_b_id,
                Some("Worker B login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let mut args = gather_args("login");
        args.project_root = Some("/repo/demo".to_string());
        args.worktree_id = Some("wt-alpha".to_string());
        args.scope = Some("worker-a".to_string());
        args.include = vec!["sessions".to_string()];

        let payload = gather_context_envelope(&store, &args, Some("demo")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let context = parsed["context"].as_array().unwrap();

        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(context.len(), 1);
        assert!(
            context[0]["content"]
                .as_str()
                .unwrap()
                .contains("Worker A login implementation")
        );
        assert!(
            !context[0]["content"]
                .as_str()
                .unwrap()
                .contains("Worker B login implementation")
        );
    }

    #[test]
    fn test_gather_context_payload_requires_project_with_full_identity() {
        let store = test_store();

        let mut args = gather_args("login");
        args.project_root = Some("/repo/demo".to_string());
        args.worktree_id = Some("wt-alpha".to_string());
        args.include = vec!["sessions".to_string()];

        let err = gather_context_envelope(&store, &args, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("project is required when project_root and worktree_id are provided")
        );
    }

    #[test]
    fn test_effective_project_preserves_unscoped_requests() {
        let mut args = gather_args("login");
        args.all_projects = true;

        assert_eq!(effective_project(&args, Some("demo")), None);
        assert_eq!(effective_project(&args, None), None);
    }
}
