use anyhow::Result;
use clap::{Args, Subcommand};

use hyphae_store::SqliteStore;

#[derive(Args)]
pub(crate) struct FeedbackArgs {
    #[command(subcommand)]
    pub(crate) command: FeedbackCommand,
}

#[derive(Subcommand)]
pub(crate) enum FeedbackCommand {
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

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
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
}
