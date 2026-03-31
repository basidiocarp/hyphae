use anyhow::Result;
use clap::{Args, Subcommand};

use hyphae_store::SqliteStore;

const SESSION_STATUS_SCHEMA_VERSION: &str = "1.0";
const SESSION_LIST_SCHEMA_VERSION: &str = "1.0";
const SESSION_TIMELINE_SCHEMA_VERSION: &str = "1.0";

#[derive(Args)]
pub(crate) struct SessionArgs {
    #[command(subcommand)]
    pub(crate) command: SessionCommand,
}

#[derive(Subcommand)]
pub(crate) enum SessionCommand {
    /// Start a new coding session
    Start {
        /// Project name for the session
        #[arg(short, long)]
        project: String,
        /// Optional task description
        #[arg(short, long)]
        task: Option<String>,
        /// Optional repository root for identity v1 lookup
        #[arg(long)]
        project_root: Option<String>,
        /// Optional worktree identifier for identity v1 lookup
        #[arg(long)]
        worktree_id: Option<String>,
        /// Optional worker or runtime scope for parallel sessions
        #[arg(long)]
        scope: Option<String>,
        /// Optional external runtime session id for cross-tool correlation
        #[arg(long)]
        runtime_session_id: Option<String>,
    },

    /// End an active coding session
    End {
        /// Session ID returned by `hyphae session start`
        #[arg(short = 'i', long)]
        id: String,
        /// Optional summary to persist with the session
        #[arg(short, long)]
        summary: Option<String>,
        /// Files modified during the session
        #[arg(long = "file")]
        file: Vec<String>,
        /// Number of errors encountered during the session
        #[arg(long)]
        errors: Option<i64>,
    },

    /// Show recent sessions for a project
    Context {
        /// Project name to query
        #[arg(short, long)]
        project: String,
        /// Optional repository root for identity v1 lookup
        #[arg(long)]
        project_root: Option<String>,
        /// Optional worktree identifier for identity v1 lookup
        #[arg(long)]
        worktree_id: Option<String>,
        /// Optional worker or runtime scope filter
        #[arg(long)]
        scope: Option<String>,
        /// Maximum number of sessions to show
        #[arg(short, long, default_value = "5")]
        limit: i64,
    },

    /// Show recent sessions as JSON
    List {
        /// Project name to query
        #[arg(short, long)]
        project: Option<String>,
        /// Optional repository root for identity v1 lookup
        #[arg(long)]
        project_root: Option<String>,
        /// Optional worktree identifier for identity v1 lookup
        #[arg(long)]
        worktree_id: Option<String>,
        /// Optional worker or runtime scope filter
        #[arg(long)]
        scope: Option<String>,
        /// Maximum number of sessions to show
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },

    /// Show a project-scoped session timeline as JSON
    Timeline {
        /// Project name to query
        #[arg(short, long)]
        project: Option<String>,
        /// Optional repository root for identity v1 lookup
        #[arg(long)]
        project_root: Option<String>,
        /// Optional worktree identifier for identity v1 lookup
        #[arg(long)]
        worktree_id: Option<String>,
        /// Optional worker or runtime scope filter
        #[arg(long)]
        scope: Option<String>,
        /// Maximum number of sessions to show
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },

    /// Show structured status for one session id
    Status {
        /// Session ID returned by `hyphae session start`
        #[arg(short = 'i', long)]
        id: String,
    },
}

pub(crate) fn dispatch(store: &SqliteStore, args: SessionArgs) -> Result<()> {
    match args.command {
        SessionCommand::Start {
            project,
            task,
            project_root,
            worktree_id,
            scope,
            runtime_session_id,
        } => cmd_start(
            store,
            &project,
            task.as_deref(),
            project_root.as_deref(),
            worktree_id.as_deref(),
            scope.as_deref(),
            runtime_session_id.as_deref(),
        ),
        SessionCommand::End {
            id,
            summary,
            file,
            errors,
        } => cmd_end(store, &id, summary.as_deref(), &file, errors),
        SessionCommand::Context {
            project,
            project_root,
            worktree_id,
            scope,
            limit,
        } => cmd_context(
            store,
            &project,
            project_root.as_deref(),
            worktree_id.as_deref(),
            scope.as_deref(),
            limit,
        ),
        SessionCommand::List {
            project,
            project_root,
            worktree_id,
            scope,
            limit,
        } => cmd_list(
            store,
            project.as_deref(),
            project_root.as_deref(),
            worktree_id.as_deref(),
            scope.as_deref(),
            limit,
        ),
        SessionCommand::Timeline {
            project,
            project_root,
            worktree_id,
            scope,
            limit,
        } => cmd_timeline(
            store,
            project.as_deref(),
            project_root.as_deref(),
            worktree_id.as_deref(),
            scope.as_deref(),
            limit,
        ),
        SessionCommand::Status { id } => cmd_status(store, &id),
    }
}

fn cmd_start(
    store: &SqliteStore,
    project: &str,
    task: Option<&str>,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    runtime_session_id: Option<&str>,
) -> Result<()> {
    let (project_root, worktree_id) = normalize_identity(project_root, worktree_id);
    let (session_id, _started_at) = store.session_start_identity_with_runtime(
        project,
        task,
        project_root,
        worktree_id,
        scope,
        runtime_session_id,
    )?;
    println!("{session_id}");
    Ok(())
}

fn cmd_end(
    store: &SqliteStore,
    session_id: &str,
    summary: Option<&str>,
    files: &[String],
    errors: Option<i64>,
) -> Result<()> {
    let files_modified = (!files.is_empty())
        .then(|| serde_json::to_string(files))
        .transpose()?;
    let errors_string = errors.map(|count| count.to_string());

    let (project, _started_at, _task, _ended_at, duration_minutes) = store.session_end(
        session_id,
        summary,
        files_modified.as_deref(),
        errors_string.as_deref(),
    )?;

    println!("Ended session {session_id} for {project} ({duration_minutes} min)");
    Ok(())
}

fn cmd_context(
    store: &SqliteStore,
    project: &str,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    limit: i64,
) -> Result<()> {
    let (project_root, worktree_id) = normalize_identity(project_root, worktree_id);
    let sessions =
        store.session_context_identity(project, project_root, worktree_id, scope, limit)?;

    if sessions.is_empty() {
        if let (Some(project_root), Some(worktree_id)) = (project_root, worktree_id) {
            println!(
                "No sessions found for project {project} with project_root {project_root} and worktree_id {worktree_id}."
            );
        } else if let Some(scope) = scope {
            println!("No sessions found for project {project} with scope {scope}.");
        } else {
            println!("No sessions found for project {project}.");
        }
        return Ok(());
    }

    for session in &sessions {
        println!("{}", format_session_context_entry(session));
    }

    Ok(())
}

fn cmd_list(
    store: &SqliteStore,
    project: Option<&str>,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    limit: i64,
) -> Result<()> {
    let sessions = recent_sessions(store, project, project_root, worktree_id, scope, limit)?;
    println!("{}", session_list_payload(&sessions));
    Ok(())
}

fn cmd_status(store: &SqliteStore, session_id: &str) -> Result<()> {
    let Some(session) = store.session_status(session_id)? else {
        anyhow::bail!("no session with id '{session_id}'");
    };

    println!("{}", session_status_payload(&session));
    Ok(())
}

fn recent_sessions(
    store: &SqliteStore,
    project: Option<&str>,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    limit: i64,
) -> Result<Vec<hyphae_store::Session>> {
    let (project_root, worktree_id) = normalize_identity(project_root, worktree_id);
    if project.is_none() && project_root.is_some() && worktree_id.is_some() {
        anyhow::bail!("project is required when project_root and worktree_id are provided");
    }

    let mut sessions = if let Some(project) = project {
        store.session_context_identity(project, project_root, worktree_id, scope, limit)?
    } else {
        store.session_context_all(i64::MAX)?
    };

    if project.is_none() {
        if let Some(scope) = scope {
            sessions.retain(|session| session.scope.as_deref() == Some(scope));
        }
        if let Ok(limit) = usize::try_from(limit)
            && sessions.len() > limit
        {
            sessions.truncate(limit);
        }
    }

    Ok(sessions)
}

fn recent_timeline(
    store: &SqliteStore,
    project: Option<&str>,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    limit: i64,
) -> Result<Vec<hyphae_store::SessionTimelineRecord>> {
    let (project_root, worktree_id) = normalize_identity(project_root, worktree_id);
    if project.is_none() && project_root.is_some() && worktree_id.is_some() {
        anyhow::bail!("project is required when project_root and worktree_id are provided");
    }

    let mut timeline = if let Some(project) = project {
        store.session_timeline_identity(project, project_root, worktree_id, scope, limit)?
    } else {
        store.session_timeline_all(i64::MAX)?
    };

    if project.is_none() {
        if let Some(scope) = scope {
            timeline.retain(|session| session.scope.as_deref() == Some(scope));
        }
        if let Ok(limit) = usize::try_from(limit)
            && timeline.len() > limit
        {
            timeline.truncate(limit);
        }
    }

    Ok(timeline)
}

fn cmd_timeline(
    store: &SqliteStore,
    project: Option<&str>,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
    scope: Option<&str>,
    limit: i64,
) -> Result<()> {
    let timeline = recent_timeline(store, project, project_root, worktree_id, scope, limit)?;
    println!("{}", session_timeline_payload(&timeline));
    Ok(())
}

fn session_status_payload(session: &hyphae_store::Session) -> serde_json::Value {
    serde_json::json!({
        "schema_version": SESSION_STATUS_SCHEMA_VERSION,
        "session_id": session.id,
        "project": session.project,
        "project_root": session.project_root,
        "worktree_id": session.worktree_id,
        "scope": session.scope,
        "runtime_session_id": session.runtime_session_id,
        "task": session.task,
        "started_at": session.started_at,
        "ended_at": session.ended_at,
        "summary": session.summary,
        "files_modified": session.files_modified,
        "errors": session.errors,
        "status": session.status,
        "active": session.status == "active",
    })
}

fn session_record_payload(session: &hyphae_store::Session) -> serde_json::Value {
    serde_json::json!({
        "id": session.id,
        "project": session.project,
        "project_root": session.project_root,
        "worktree_id": session.worktree_id,
        "scope": session.scope,
        "runtime_session_id": session.runtime_session_id,
        "task": session.task,
        "started_at": session.started_at,
        "ended_at": session.ended_at,
        "summary": session.summary,
        "files_modified": session.files_modified,
        "errors": session.errors,
        "status": session.status,
    })
}

fn session_list_payload(sessions: &[hyphae_store::Session]) -> serde_json::Value {
    serde_json::json!({
        "schema_version": SESSION_LIST_SCHEMA_VERSION,
        "sessions": sessions
            .iter()
            .map(session_record_payload)
            .collect::<Vec<_>>(),
    })
}

fn session_timeline_payload(timeline: &[hyphae_store::SessionTimelineRecord]) -> serde_json::Value {
    serde_json::json!({
        "schema_version": SESSION_TIMELINE_SCHEMA_VERSION,
        "timeline": timeline,
    })
}

fn format_session_context_entry(session: &hyphae_store::Session) -> String {
    let task = session.task.as_deref().unwrap_or("(no task)");
    let summary = session.summary.as_deref().unwrap_or("(no summary)");
    let project_root = session
        .project_root
        .as_deref()
        .map(|value| format!(" project_root={value}"))
        .unwrap_or_default();
    let worktree_id = session
        .worktree_id
        .as_deref()
        .map(|value| format!(" worktree_id={value}"))
        .unwrap_or_default();
    let scope = session
        .scope
        .as_deref()
        .map(|value| format!(" scope={value}"))
        .unwrap_or_default();
    let runtime_session_id = session
        .runtime_session_id
        .as_deref()
        .map(|value| format!(" runtime_session_id={value}"))
        .unwrap_or_default();
    format!(
        "{} [{}]{}{}{}{} {} -> {}",
        session.id,
        session.status,
        project_root,
        worktree_id,
        scope,
        runtime_session_id,
        task,
        crate::display::truncate(summary, 100)
    )
}

fn normalize_identity<'a>(
    project_root: Option<&'a str>,
    worktree_id: Option<&'a str>,
) -> (Option<&'a str>, Option<&'a str>) {
    match (project_root, worktree_id) {
        (Some(project_root), Some(worktree_id)) => (Some(project_root), Some(worktree_id)),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::MemoryStore;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_session_start_and_context() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("implement feedback loop"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let sessions = store.session_context("demo-project", 5).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[0].task.as_deref(), Some("implement feedback loop"));
    }

    #[test]
    fn test_session_start_with_identity_v1_and_scope_keeps_parallel_sessions_distinct() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("worker one"),
            Some("/repo/demo"),
            Some("wt-alpha"),
            Some("worker-a"),
            None,
        )
        .unwrap();
        cmd_start(
            &store,
            "demo-project",
            Some("worker two"),
            Some("/repo/demo"),
            Some("wt-alpha"),
            Some("worker-b"),
            None,
        )
        .unwrap();
        let worker_a_sessions = store
            .session_context_identity(
                "demo-project",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
                5,
            )
            .unwrap();
        let worker_b_sessions = store
            .session_context_identity(
                "demo-project",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
                5,
            )
            .unwrap();

        assert_eq!(worker_a_sessions.len(), 1);
        assert_eq!(worker_b_sessions.len(), 1);
        assert_ne!(worker_a_sessions[0].id, worker_b_sessions[0].id);
    }

    #[test]
    fn test_session_end_does_not_store_compatibility_session_memory() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("add session bridge"))
            .unwrap();

        cmd_end(
            &store,
            &session_id,
            Some("Integrated Cortina with session lifecycle"),
            &["src/main.rs".to_string()],
            Some(0),
        )
        .unwrap();

        let sessions = store.session_context("demo-project", 5).unwrap();
        assert_eq!(sessions[0].status, "completed");

        let memories = store
            .get_by_topic("session/demo-project", Some("demo-project"))
            .unwrap();
        assert!(memories.is_empty());

        let signal_count = store
            .count_outcome_signals(Some(&session_id), Some("session_success"), Some(2))
            .unwrap();
        assert_eq!(signal_count, 1);
    }

    #[test]
    fn test_session_end_with_errors_stores_failure_signal() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("recover build"))
            .unwrap();

        cmd_end(
            &store,
            &session_id,
            Some("left known failures"),
            &[],
            Some(3),
        )
        .unwrap();

        let signal_count = store
            .count_outcome_signals(Some(&session_id), Some("session_failure"), Some(-2))
            .unwrap();
        assert_eq!(signal_count, 1);
    }

    #[test]
    fn test_session_context_scope_filters_parallel_sessions() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("worker one"),
            None,
            None,
            Some("worker-a"),
            None,
        )
        .unwrap();
        cmd_start(
            &store,
            "demo-project",
            Some("worker two"),
            None,
            None,
            Some("worker-b"),
            None,
        )
        .unwrap();

        let sessions = store
            .session_context_scoped("demo-project", Some("worker-a"), 5)
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].scope.as_deref(), Some("worker-a"));
    }

    #[test]
    fn test_dispatch_context_with_scope_succeeds() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("worker one"),
            None,
            None,
            Some("worker-a"),
            None,
        )
        .unwrap();

        let args = SessionArgs {
            command: SessionCommand::Context {
                project: "demo-project".to_string(),
                project_root: None,
                worktree_id: None,
                scope: Some("worker-a".to_string()),
                limit: 5,
            },
        };

        let result = dispatch(&store, args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_status_succeeds_for_known_session() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_scoped("demo-project", Some("worker one"), Some("worker-a"))
            .unwrap();

        let result = cmd_status(&store, &session_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_status_fails_for_unknown_session() {
        let store = test_store();
        let result = cmd_status(&store, "ses_missing");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_start_accepts_identity_v1_fields() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("worker one"),
            Some("/repo/demo"),
            Some("wt-alpha"),
            Some("worker-a"),
            None,
        )
        .unwrap();

        let sessions = store
            .session_context_identity(
                "demo-project",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
                5,
            )
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].project_root.as_deref(), Some("/repo/demo"));
        assert_eq!(sessions[0].worktree_id.as_deref(), Some("wt-alpha"));
    }

    #[test]
    fn test_session_start_partial_identity_normalizes_to_legacy_behavior() {
        let store = test_store();

        cmd_start(
            &store,
            "demo-project",
            Some("worker one"),
            Some("/repo/demo"),
            None,
            Some("worker-a"),
            None,
        )
        .unwrap();

        let sessions = store
            .session_context_scoped("demo-project", Some("worker-a"), 5)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].project_root.is_none());
        assert!(sessions[0].worktree_id.is_none());
    }

    #[test]
    fn test_session_status_payload_includes_identity_v1_fields() {
        let session = hyphae_store::Session {
            id: "ses_test".to_string(),
            project: "demo-project".to_string(),
            project_root: Some("/repo/demo".to_string()),
            worktree_id: Some("wt-alpha".to_string()),
            scope: Some("worker-a".to_string()),
            runtime_session_id: Some("claude-session-1".to_string()),
            task: Some("task".to_string()),
            started_at: "2026-03-29T00:00:00Z".to_string(),
            ended_at: None,
            summary: None,
            files_modified: None,
            errors: None,
            status: "active".to_string(),
        };

        let payload = session_status_payload(&session);
        assert_eq!(payload["schema_version"].as_str(), Some("1.0"));
        assert_eq!(payload["project_root"].as_str(), Some("/repo/demo"));
        assert_eq!(payload["worktree_id"].as_str(), Some("wt-alpha"));
        assert_eq!(
            payload["runtime_session_id"].as_str(),
            Some("claude-session-1")
        );
    }

    #[test]
    fn test_session_list_payload_includes_identity_v1_fields() {
        let session = hyphae_store::Session {
            id: "ses_test".to_string(),
            project: "demo-project".to_string(),
            project_root: Some("/repo/demo".to_string()),
            worktree_id: Some("wt-alpha".to_string()),
            scope: Some("worker-a".to_string()),
            runtime_session_id: Some("claude-session-1".to_string()),
            task: Some("task".to_string()),
            started_at: "2026-03-29T00:00:00Z".to_string(),
            ended_at: None,
            summary: None,
            files_modified: None,
            errors: None,
            status: "active".to_string(),
        };

        let payload = session_list_payload(&[session]);
        assert_eq!(payload["schema_version"].as_str(), Some("1.0"));
        let record = payload["sessions"].as_array().unwrap().first().unwrap();
        assert_eq!(record["id"].as_str(), Some("ses_test"));
        assert_eq!(record["project_root"].as_str(), Some("/repo/demo"));
        assert_eq!(record["worktree_id"].as_str(), Some("wt-alpha"));
        assert_eq!(
            record["runtime_session_id"].as_str(),
            Some("claude-session-1")
        );
        assert!(record.get("session_id").is_none());
        assert!(record.get("active").is_none());
    }

    #[test]
    fn test_format_session_context_entry_includes_identity_v1_fields() {
        let session = hyphae_store::Session {
            id: "ses_test".to_string(),
            project: "demo-project".to_string(),
            project_root: Some("/repo/demo".to_string()),
            worktree_id: Some("wt-alpha".to_string()),
            scope: Some("worker-a".to_string()),
            runtime_session_id: Some("claude-session-1".to_string()),
            task: Some("task".to_string()),
            started_at: "2026-03-29T00:00:00Z".to_string(),
            ended_at: None,
            summary: Some("summary".to_string()),
            files_modified: None,
            errors: None,
            status: "active".to_string(),
        };

        let line = format_session_context_entry(&session);
        assert!(line.contains("project_root=/repo/demo"));
        assert!(line.contains("worktree_id=wt-alpha"));
        assert!(line.contains("scope=worker-a"));
        assert!(line.contains("runtime_session_id=claude-session-1"));
    }

    #[test]
    fn test_recent_sessions_applies_identity_filters() {
        let store = test_store();

        let (worker_a_id, _) = store
            .session_start_identity(
                "demo-project",
                Some("worker a"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&worker_a_id, Some("worker a done"), None, Some("0"))
            .unwrap();

        let (worker_b_id, _) = store
            .session_start_identity(
                "demo-project",
                Some("worker b"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
            )
            .unwrap();
        store
            .session_end(&worker_b_id, Some("worker b done"), None, Some("0"))
            .unwrap();

        let sessions = recent_sessions(
            &store,
            Some("demo-project"),
            Some("/repo/demo"),
            Some("wt-alpha"),
            Some("worker-a"),
            10,
        )
        .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, worker_a_id);
        assert_eq!(sessions[0].project_root.as_deref(), Some("/repo/demo"));
        assert_eq!(sessions[0].worktree_id.as_deref(), Some("wt-alpha"));
    }

    #[test]
    fn test_recent_sessions_requires_project_with_full_identity() {
        let store = test_store();

        let err = recent_sessions(
            &store,
            None,
            Some("/repo/demo"),
            Some("wt-alpha"),
            Some("worker-a"),
            10,
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("project is required when project_root and worktree_id are provided")
        );
    }

    #[test]
    fn test_recent_sessions_filters_scope_without_project() {
        let store = test_store();

        let (worker_a_id, _) = store
            .session_start_identity(
                "demo-project",
                Some("worker a"),
                None,
                None,
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&worker_a_id, Some("worker a done"), None, Some("0"))
            .unwrap();

        let (worker_b_id, _) = store
            .session_start_identity(
                "other-project",
                Some("worker b"),
                None,
                None,
                Some("worker-b"),
            )
            .unwrap();
        store
            .session_end(&worker_b_id, Some("worker b done"), None, Some("0"))
            .unwrap();

        let sessions = recent_sessions(&store, None, None, None, Some("worker-a"), 10).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, worker_a_id);
        assert_eq!(sessions[0].scope.as_deref(), Some("worker-a"));
    }

    #[test]
    fn test_session_timeline_payload_preserves_cap_contract() {
        let payload = session_timeline_payload(&[hyphae_store::SessionTimelineRecord {
            id: "ses_test".to_string(),
            project: "cap".to_string(),
            project_root: Some("/repo/cap".to_string()),
            worktree_id: Some("wt-alpha".to_string()),
            scope: Some("worker-a".to_string()),
            runtime_session_id: Some("claude-session-1".to_string()),
            task: Some("build session timeline".to_string()),
            started_at: "2026-03-27T12:00:00Z".to_string(),
            ended_at: Some("2026-03-27T12:10:00Z".to_string()),
            summary: Some("Connected session recall and outcome signals.".to_string()),
            files_modified: Some("[\"src/pages/Sessions.tsx\"]".to_string()),
            errors: Some("2".to_string()),
            status: "completed".to_string(),
            events: vec![hyphae_store::SessionTimelineEvent {
                id: "rec_1".to_string(),
                kind: "recall".to_string(),
                title: "Recalled 3 memories".to_string(),
                detail: Some("session attribution bridge".to_string()),
                occurred_at: "2026-03-27T12:02:00Z".to_string(),
                recall_event_id: Some("rec_1".to_string()),
                memory_count: Some(3),
                signal_type: None,
                signal_value: None,
                source: None,
            }],
            last_activity_at: "2026-03-27T12:10:00Z".to_string(),
            recall_count: 1,
            outcome_count: 0,
        }]);

        assert_eq!(payload["schema_version"].as_str(), Some("1.0"));
        let record = payload["timeline"].as_array().unwrap().first().unwrap();
        assert_eq!(record["id"].as_str(), Some("ses_test"));
        assert_eq!(record["project"].as_str(), Some("cap"));
        assert_eq!(record["scope"].as_str(), Some("worker-a"));
        assert_eq!(record["task"].as_str(), Some("build session timeline"));
        assert_eq!(record["started_at"].as_str(), Some("2026-03-27T12:00:00Z"));
        assert_eq!(record["ended_at"].as_str(), Some("2026-03-27T12:10:00Z"));
        assert_eq!(
            record["summary"].as_str(),
            Some("Connected session recall and outcome signals.")
        );
        assert_eq!(
            record["files_modified"].as_str(),
            Some("[\"src/pages/Sessions.tsx\"]")
        );
        assert_eq!(record["errors"].as_str(), Some("2"));
        assert_eq!(record["status"].as_str(), Some("completed"));
        assert_eq!(
            record["last_activity_at"].as_str(),
            Some("2026-03-27T12:10:00Z")
        );
        assert_eq!(record["recall_count"].as_u64(), Some(1));
        assert_eq!(record["outcome_count"].as_u64(), Some(0));
        assert_eq!(record["project_root"].as_str(), Some("/repo/cap"));
        assert_eq!(record["worktree_id"].as_str(), Some("wt-alpha"));
        assert_eq!(
            record["runtime_session_id"].as_str(),
            Some("claude-session-1")
        );

        let event = record["events"].as_array().unwrap().first().unwrap();
        assert_eq!(event["id"].as_str(), Some("rec_1"));
        assert_eq!(event["kind"].as_str(), Some("recall"));
        assert_eq!(event["title"].as_str(), Some("Recalled 3 memories"));
        assert_eq!(event["detail"].as_str(), Some("session attribution bridge"));
        assert_eq!(event["occurred_at"].as_str(), Some("2026-03-27T12:02:00Z"));
        assert_eq!(event["recall_event_id"].as_str(), Some("rec_1"));
        assert_eq!(event["memory_count"].as_i64(), Some(3));
        assert!(event["signal_type"].is_null());
        assert!(event["signal_value"].is_null());
        assert!(event["source"].is_null());
    }

    #[test]
    fn test_cmd_timeline_succeeds_for_known_project() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_identity(
                "cap",
                Some("build session timeline"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .log_recall_event(
                Some(&session_id),
                "session attribution bridge",
                &["mem_1".to_string()],
                Some("cap"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let result = cmd_timeline(
            &store,
            Some("cap"),
            Some("/repo/cap"),
            Some("wt-alpha"),
            Some("worker-a"),
            10,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_timeline_succeeds_without_project() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_identity(
                "cap",
                Some("build session timeline"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let result = cmd_timeline(&store, None, None, None, None, 10);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_timeline_requires_project_with_full_identity() {
        let store = test_store();
        let result = cmd_timeline(&store, None, Some("/repo/cap"), Some("wt-alpha"), None, 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_cmd_timeline_filters_scope_without_project() {
        let store = test_store();
        let (worker_a_id, _) = store
            .session_start_identity(
                "cap",
                Some("build session timeline"),
                None,
                None,
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&worker_a_id, Some("done"), None, Some("0"))
            .unwrap();

        let (worker_b_id, _) = store
            .session_start_identity(
                "other",
                Some("build other timeline"),
                None,
                None,
                Some("worker-b"),
            )
            .unwrap();
        store
            .session_end(&worker_b_id, Some("done"), None, Some("0"))
            .unwrap();

        let timeline = recent_timeline(&store, None, None, None, Some("worker-a"), 10).unwrap();

        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].id, worker_a_id);
        assert_eq!(timeline[0].scope.as_deref(), Some("worker-a"));
    }
}
