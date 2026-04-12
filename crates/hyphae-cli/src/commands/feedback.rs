use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use hyphae_store::SqliteStore;

#[derive(Args)]
pub(crate) struct FeedbackArgs {
    #[command(subcommand)]
    pub(crate) command: FeedbackCommand,
}

#[derive(Subcommand)]
pub(crate) enum FeedbackCommand {
    /// Recompute recall-effectiveness scores from existing session data
    Compute {
        /// Recompute a single completed session
        #[arg(short = 'i', long, conflicts_with = "all")]
        session_id: Option<String>,
        /// Recompute all completed sessions
        #[arg(long, default_value_t = false)]
        all: bool,
    },
    /// Record a structured outcome signal for the feedback loop
    Signal {
        /// Session ID to associate with the signal
        #[arg(short = 'i', long)]
        session_id: String,
        /// Signal type, for example correction or session_success
        #[arg(short = 't', long)]
        signal_type: String,
        /// Signal value. Positive values help, negative values penalize.
        #[arg(short, long)]
        value: i64,
        /// Signal source, for example cortina.post_tool_use
        #[arg(short, long)]
        source: Option<String>,
        /// Optional project override
        #[arg(short, long)]
        project: Option<String>,
    },
}

pub(crate) fn dispatch(store: &SqliteStore, args: FeedbackArgs) -> Result<()> {
    match args.command {
        FeedbackCommand::Compute { session_id, all } => {
            cmd_compute(store, session_id.as_deref(), all)
        }
        FeedbackCommand::Signal {
            session_id,
            signal_type,
            value,
            source,
            project,
        } => cmd_signal(
            store,
            &session_id,
            &signal_type,
            value,
            source.as_deref(),
            project.as_deref(),
        ),
    }
}

fn cmd_compute(store: &SqliteStore, session_id: Option<&str>, all: bool) -> Result<()> {
    let written = compute_effectiveness(store, session_id, all)?;

    if all {
        println!("recomputed {written} recall-effectiveness rows across completed sessions");
    } else {
        let session_id = session_id.expect("validated session_id");
        println!("recomputed {written} recall-effectiveness rows for session {session_id}");
    }

    Ok(())
}

fn compute_effectiveness(
    store: &SqliteStore,
    session_id: Option<&str>,
    all: bool,
) -> Result<usize> {
    match (session_id, all) {
        (Some(session_id), false) => Ok(store.recompute_recall_effectiveness(session_id)?),
        (None, true) => Ok(store.recompute_recall_effectiveness_all()?),
        (Some(_), true) => bail!("pass either --session-id or --all, not both"),
        (None, false) => bail!("pass --session-id <id> or --all"),
    }
}

fn cmd_signal(
    store: &SqliteStore,
    session_id: &str,
    signal_type: &str,
    value: i64,
    source: Option<&str>,
    project: Option<&str>,
) -> Result<()> {
    let signal_id =
        store.log_outcome_signal(Some(session_id), signal_type, value, source, project)?;
    println!("{signal_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    fn make_memory(topic: &str, summary: &str) -> Memory {
        Memory::new(topic.into(), summary.into(), Importance::Medium)
    }

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn scored_session(store: &SqliteStore) -> (String, String) {
        let memory = make_memory("demo-project", "remember this");
        let memory_id = memory.id.to_string();
        store.store(memory).unwrap();

        let (session_id, _) = store
            .session_start("demo-project", Some("feedback"))
            .unwrap();
        store
            .log_recall_event(
                Some(&session_id),
                "remember this",
                std::slice::from_ref(&memory_id),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();

        (session_id, memory_id)
    }

    #[test]
    fn test_feedback_signal_persists_signal() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("feedback"))
            .unwrap();

        cmd_signal(
            &store,
            &session_id,
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            Some("demo-project"),
        )
        .unwrap();

        let count = store
            .count_outcome_signals(Some(&session_id), Some("correction"), Some(-1))
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_feedback_signal_rejects_unknown_session() {
        let store = test_store();
        let result = cmd_signal(
            &store,
            "ses_missing",
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            Some("demo-project"),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_feedback_compute_recomputes_for_target_session() {
        let store = test_store();
        let (session_id, memory_id) = scored_session(&store);

        let written = compute_effectiveness(&store, Some(&session_id), false).unwrap();
        let scores = store
            .recall_effectiveness_for_memory_ids(std::slice::from_ref(&memory_id))
            .unwrap();

        assert!(written > 0);
        assert!(scores.contains_key(&memory_id));
    }

    #[test]
    fn test_feedback_compute_recomputes_all_completed_sessions() {
        let store = test_store();
        let (_session_id, memory_id) = scored_session(&store);

        let written = compute_effectiveness(&store, None, true).unwrap();
        let scores = store
            .recall_effectiveness_for_memory_ids(std::slice::from_ref(&memory_id))
            .unwrap();

        assert!(written > 0);
        assert!(scores.contains_key(&memory_id));
    }

    #[test]
    fn test_feedback_compute_requires_scope() {
        let store = test_store();
        let result = compute_effectiveness(&store, None, false);

        assert!(result.is_err());
    }

    #[test]
    fn test_feedback_compute_rejects_active_session() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("feedback"))
            .unwrap();

        let result = compute_effectiveness(&store, Some(&session_id), false);

        assert!(result.is_err());
    }
}
