use anyhow::Result;
use clap::{Args, Subcommand};

use hyphae_core::{Importance, Memory, MemoryStore};
use hyphae_store::SqliteStore;

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
        /// Optional worker or runtime scope for parallel sessions
        #[arg(long)]
        scope: Option<String>,
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
        /// Optional worker or runtime scope filter
        #[arg(long)]
        scope: Option<String>,
        /// Maximum number of sessions to show
        #[arg(short, long, default_value = "5")]
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
            scope,
        } => cmd_start(store, &project, task.as_deref(), scope.as_deref()),
        SessionCommand::End {
            id,
            summary,
            file,
            errors,
        } => cmd_end(store, &id, summary.as_deref(), &file, errors),
        SessionCommand::Context {
            project,
            scope,
            limit,
        } => cmd_context(store, &project, scope.as_deref(), limit),
        SessionCommand::Status { id } => cmd_status(store, &id),
    }
}

fn cmd_start(
    store: &SqliteStore,
    project: &str,
    task: Option<&str>,
    scope: Option<&str>,
) -> Result<()> {
    let (session_id, _started_at) = store.session_start_scoped(project, task, scope)?;
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

    let (project, _started_at, task, _ended_at, duration_minutes) = store.session_end(
        session_id,
        summary,
        files_modified.as_deref(),
        errors_string.as_deref(),
    )?;

    if let Some(summary_text) = summary {
        let topic = format!("session/{project}");
        let content = if let Some(task_desc) = &task {
            format!("Session completed: {task_desc}. {summary_text}")
        } else {
            format!("Session completed. {summary_text}")
        };

        let memory = Memory::builder(topic, content, Importance::Medium)
            .keywords(vec!["session".to_string(), project.clone()])
            .project(project.clone())
            .build();
        if let Err(e) = store.store(memory) {
            tracing::warn!(
                "session {session_id} ended but failed to store compatibility session memory: {e}"
            );
        }
    }

    println!("Ended session {session_id} for {project} ({duration_minutes} min)");
    Ok(())
}

fn cmd_context(store: &SqliteStore, project: &str, scope: Option<&str>, limit: i64) -> Result<()> {
    let sessions = store.session_context_scoped(project, scope, limit)?;

    if sessions.is_empty() {
        if let Some(scope) = scope {
            println!("No sessions found for project {project} with scope {scope}.");
        } else {
            println!("No sessions found for project {project}.");
        }
        return Ok(());
    }

    for session in &sessions {
        let task = session.task.as_deref().unwrap_or("(no task)");
        let status = &session.status;
        let summary = session.summary.as_deref().unwrap_or("(no summary)");
        let scope = session
            .scope
            .as_deref()
            .map(|value| format!(" scope={value}"))
            .unwrap_or_default();
        println!(
            "{} [{}]{} {} -> {}",
            session.id,
            status,
            scope,
            task,
            crate::display::truncate(summary, 100)
        );
    }

    Ok(())
}

fn cmd_status(store: &SqliteStore, session_id: &str) -> Result<()> {
    let Some(session) = store.session_status(session_id)? else {
        anyhow::bail!("no session with id '{session_id}'");
    };

    println!(
        "{}",
        serde_json::json!({
            "session_id": session.id,
            "project": session.project,
            "scope": session.scope,
            "task": session.task,
            "started_at": session.started_at,
            "ended_at": session.ended_at,
            "summary": session.summary,
            "files_modified": session.files_modified,
            "errors": session.errors,
            "status": session.status,
            "active": session.status == "active",
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        )
        .unwrap();
        let sessions = store.session_context("demo-project", 5).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[0].task.as_deref(), Some("implement feedback loop"));
    }

    #[test]
    fn test_session_start_with_scope_keeps_parallel_sessions_distinct() {
        let store = test_store();

        cmd_start(&store, "demo-project", Some("worker one"), Some("worker-a")).unwrap();
        cmd_start(&store, "demo-project", Some("worker two"), Some("worker-b")).unwrap();
        let sessions = store.session_context("demo-project", 5).unwrap();

        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_session_end_stores_summary_memory() {
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
            .search_by_keywords(&["session", "demo-project"], 10, 0, Some("demo-project"))
            .unwrap();
        assert!(!memories.is_empty());
        assert!(memories[0].summary.contains("Integrated Cortina"));

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

        cmd_start(&store, "demo-project", Some("worker one"), Some("worker-a")).unwrap();
        cmd_start(&store, "demo-project", Some("worker two"), Some("worker-b")).unwrap();

        let sessions = store
            .session_context_scoped("demo-project", Some("worker-a"), 5)
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].scope.as_deref(), Some("worker-a"));
    }

    #[test]
    fn test_dispatch_context_with_scope_succeeds() {
        let store = test_store();

        cmd_start(&store, "demo-project", Some("worker one"), Some("worker-a")).unwrap();

        let args = SessionArgs {
            command: SessionCommand::Context {
                project: "demo-project".to_string(),
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
}
