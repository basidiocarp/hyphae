use anyhow::Result;
use clap::Parser;
use hyphae_core::Embedder;
use hyphae_store::SqliteStore;
use spore::logging::{SpanContext, root_span, workflow_span};
use std::path::PathBuf;

#[cfg(test)]
#[allow(unused)]
mod bench_data;
#[cfg(test)]
#[allow(unused)]
mod bench_knowledge;
mod cli;
mod commands;
mod config;
mod display;
mod extract;
mod init;
mod paths;
mod project;
mod watch;

use cli::{Cli, Commands};

fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Store { .. } => "store",
        Commands::Search { .. } => "search",
        Commands::ListInvalidated { .. } => "list_invalidated",
        Commands::Stats { .. } => "stats",
        Commands::Topics { .. } => "topics",
        Commands::Health { .. } => "health",
        Commands::Memory(_) => "memory",
        Commands::Invalidate { .. } => "invalidate",
        Commands::Extract { .. } => "extract",
        Commands::EmbedAll { .. } => "embed_all",
        Commands::Ingest { .. } => "ingest",
        Commands::ForgetSource { .. } => "forget_source",
        Commands::Watch { .. } => "watch",
        Commands::Serve { .. } => "serve",
        Commands::GatherContext(_) => "gather_context",
        Commands::ListSources { .. } => "list_sources",
        Commands::SearchDocs { .. } => "search_docs",
        Commands::SearchAll { .. } => "search_all",
        Commands::Config => "config",
        Commands::Memoir(_) => "memoir",
        Commands::Feedback(_) => "feedback",
        Commands::Session(_) => "session",
        Commands::Prune { .. } => "prune",
        Commands::Consolidate { .. } => "consolidate",
        Commands::Lessons { .. } => "lessons",
        Commands::Activity => "activity",
        Commands::Analytics => "analytics",
        Commands::TestEmbed { .. } => "test_embed",
        Commands::ImportClaudeMemory { .. } => "import_claude_memory",
        Commands::CodexNotify { .. } => "codex_notify",
        Commands::Completions { .. } => "completions",
        Commands::Project(_) => "project",
        Commands::Protocol => "protocol",
        Commands::Init { .. } => "init",
        Commands::Bench { .. } => "bench",
        Commands::SelfUpdate { .. } => "self_update",
        Commands::Doctor { .. } => "doctor",
        Commands::ExportTraining { .. } => "export_training",
        Commands::Evaluate { .. } => "evaluate",
        Commands::Backup { .. } => "backup",
        Commands::Restore { .. } => "restore",
        Commands::IngestSessions { .. } => "ingest_sessions",
        Commands::Purge { .. } => "purge",
        Commands::AuditSecrets { .. } => "audit_secrets",
        Commands::Audit { .. } => "audit",
        Commands::Changelog { .. } => "changelog",
    }
}

fn all_projects_allowed(command: &Commands) -> bool {
    match command {
        Commands::Search { .. }
        | Commands::ListInvalidated { .. }
        | Commands::GatherContext(_)
        | Commands::Stats { .. }
        | Commands::Topics { .. }
        | Commands::Health { .. }
        | Commands::Memory(_)
        | Commands::Lessons { .. }
        | Commands::Activity
        | Commands::Analytics
        | Commands::Config
        | Commands::Protocol
        | Commands::TestEmbed { .. }
        | Commands::SearchDocs { .. }
        | Commands::ListSources { .. }
        | Commands::SearchAll { .. }
        | Commands::Completions { .. }
        | Commands::Init { .. }
        | Commands::Bench { .. }
        | Commands::SelfUpdate { .. }
        | Commands::Doctor { .. }
        | Commands::ExportTraining { .. }
        | Commands::Evaluate { .. }
        | Commands::Backup { .. }
        | Commands::Restore { .. }
        | Commands::Audit { .. }
        | Commands::AuditSecrets { .. }
        | Commands::Changelog { .. } => true,
        Commands::Project(args) => matches!(
            args.command,
            crate::commands::project::ProjectCommand::List
                | crate::commands::project::ProjectCommand::Search { .. }
        ),
        Commands::Session(args) => matches!(
            args.command,
            crate::commands::session::SessionCommand::Context { .. }
                | crate::commands::session::SessionCommand::List { .. }
                | crate::commands::session::SessionCommand::Timeline { .. }
                | crate::commands::session::SessionCommand::Status { .. }
        ),
        Commands::Store { .. }
        | Commands::Invalidate { .. }
        | Commands::Extract { .. }
        | Commands::EmbedAll { .. }
        | Commands::Ingest { .. }
        | Commands::ForgetSource { .. }
        | Commands::Watch { .. }
        | Commands::Serve { .. }
        | Commands::Memoir(_)
        | Commands::Feedback(_)
        | Commands::Prune { .. }
        | Commands::Consolidate { .. }
        | Commands::ImportClaudeMemory { .. }
        | Commands::CodexNotify { .. }
        | Commands::IngestSessions { .. }
        | Commands::Purge { .. } => false,
    }
}

fn open_store(db: Option<PathBuf>, embedding_dims: usize) -> Result<SqliteStore> {
    let path = db.unwrap_or_else(paths::default_db_path);

    std::fs::create_dir_all(path.parent().unwrap_or(&PathBuf::from(".")))?;
    SqliteStore::with_dims(&path, embedding_dims)
        .map_err(|e| anyhow::anyhow!("failed to open database: {e}"))
}

/// Initialize the best available embedder.
///
/// Priority:
/// 1. HTTP embedder (if `HYPHAE_EMBEDDING_URL` is set) — always available
/// 2. FastEmbedder (if `embeddings` feature is compiled in)
/// 3. None — FTS-only search
fn init_embedder(model: &str) -> Option<Box<dyn Embedder>> {
    // Try HTTP embedder first (always compiled)
    match hyphae_core::HttpEmbedder::from_env() {
        Ok(Some(http)) => return Some(Box::new(http)),
        Ok(None) => {} // URL not set, try next
        Err(e) => {
            tracing::warn!("HTTP embedder config error: {e}");
        }
    }

    // Try fastembed (feature-gated)
    #[cfg(feature = "embeddings")]
    {
        match hyphae_core::FastEmbedder::with_model(model) {
            Ok(fe) => return Some(Box::new(fe)),
            Err(e) => {
                tracing::warn!("fastembed init failed: {e}");
            }
        }
    }

    let _ = model; // suppress unused warning in no-default-features build
    None
}

fn main() -> Result<()> {
    spore::logging::init_app("hyphae", tracing::Level::WARN);

    let cli = Cli::parse();
    let mut span_context = SpanContext::for_app("hyphae");
    if let Ok(cwd) = std::env::current_dir() {
        span_context = span_context.with_workspace_root(cwd.display().to_string());
    }
    let _runtime_span = root_span(&span_context).entered();
    let _command_span = workflow_span(command_name(&cli.command), &span_context).entered();

    // Early-return commands that must remain available even if config parsing fails.
    match &cli.command {
        Commands::Doctor { fix } => {
            commands::doctor::run(*fix, cli.db.clone())?;
            return Ok(());
        }
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            generate(
                *shell,
                &mut Cli::command(),
                "hyphae",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
        Commands::Init { editor, mode } => {
            init::run_init(*editor, *mode)?;
            return Ok(());
        }
        Commands::SelfUpdate { check } => {
            commands::self_update::run(*check)?;
            return Ok(());
        }
        Commands::Backup { output, list } => {
            if *list {
                commands::backup::cmd_backup_list()?;
                return Ok(());
            }
            let cfg = config::load_config()?;
            let resolved_db_path =
                paths::resolve_db_path(cli.db.clone(), cfg.store.path.as_deref());
            commands::backup::cmd_backup(output.clone(), resolved_db_path.clone())?;
            return Ok(());
        }
        Commands::Restore { path } => {
            let cfg = config::load_config()?;
            let resolved_db_path =
                paths::resolve_db_path(cli.db.clone(), cfg.store.path.as_deref());
            commands::backup::cmd_restore(path.clone(), resolved_db_path.clone())?;
            return Ok(());
        }
        _ => {}
    }

    let cfg = config::load_config()?;
    let resolved_db_path = paths::resolve_db_path(cli.db.clone(), cfg.store.path.as_deref());
    if cli.all_projects && !all_projects_allowed(&cli.command) {
        anyhow::bail!("--all-projects is only supported for read-only query commands");
    }
    let resolved_project: Option<String> = if cli.all_projects {
        None
    } else {
        cli.project
            .clone()
            .or_else(|| cfg.store.default_project.clone())
            .or_else(project::detect_project)
    };

    if matches!(cli.command, Commands::Config) {
        commands::cmd_config(&cfg);
        return Ok(());
    }

    let embedder = init_embedder(&cfg.embeddings.model);
    let embedding_dims = embedder.as_ref().map(|e| e.dimensions()).unwrap_or(384);

    let store = open_store(Some(resolved_db_path.clone()), embedding_dims)?;

    let embedder_ref: Option<&dyn Embedder> = embedder.as_ref().map(|e| e.as_ref());

    match cli.command {
        Commands::Store {
            topic,
            content,
            importance,
        } => {
            commands::memory::cmd_store(&store, topic, content, &importance, resolved_project)?;
        }

        Commands::Search {
            query,
            topic,
            limit,
            include_invalidated,
            order,
            json,
            raw,
        } => {
            let effective_query = if raw {
                query
            } else {
                let sanitized = hyphae_core::sanitize_query(&query);
                if sanitized.was_sanitized {
                    tracing::debug!(
                        original = %query,
                        sanitized = %sanitized.text,
                        removed = ?sanitized.removed,
                        "query sanitized before search"
                    );
                }
                if sanitized.text.is_empty() { query } else { sanitized.text }
            };
            commands::memory::cmd_search(
                &store,
                effective_query,
                topic,
                limit,
                include_invalidated,
                order,
                json,
                resolved_project,
            )?;
        }

        Commands::Invalidate {
            id,
            reason,
            superseded_by,
        } => {
            commands::memory::cmd_invalidate(&store, id, reason, superseded_by)?;
        }

        Commands::ListInvalidated { limit } => {
            commands::memory::cmd_list_invalidated(&store, limit, resolved_project)?;
        }

        Commands::Extract { file, project } => {
            let text = if let Some(path) = file {
                std::fs::read_to_string(&path)?
            } else {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            };
            let stored = extract::extract_and_store(&store, &text, &project)?;
            println!("Extracted and stored {} facts", stored);
        }

        Commands::GatherContext(args) => {
            commands::context::dispatch(&store, args, resolved_project.as_deref())?;
        }

        Commands::Stats {
            include_invalidated,
            json,
        } => {
            commands::memory::cmd_stats(&store, json, resolved_project, include_invalidated)?;
        }

        Commands::Topics {
            include_invalidated,
            json,
        } => {
            commands::memory::cmd_topics(&store, json, resolved_project, include_invalidated)?;
        }

        Commands::Health {
            topic,
            include_invalidated,
            json,
        } => {
            commands::memory::cmd_health(
                &store,
                &cfg.consolidation,
                topic,
                include_invalidated,
                json,
                resolved_project,
            )?;
        }

        Commands::Memory(args) => {
            commands::memory::dispatch(&store, args, resolved_project)?;
        }

        Commands::Config => unreachable!("handled in early-return block"),
        Commands::Protocol => {
            commands::protocol::cmd_protocol(resolved_project.as_deref())?;
        }
        Commands::Completions { .. } => unreachable!("handled in early-return block"),
        Commands::Init { .. } => unreachable!("handled in early-return block"),
        Commands::SelfUpdate { .. } => unreachable!("handled in early-return block"),
        Commands::Doctor { .. } => unreachable!("handled in early-return block"),

        Commands::Watch { path, recursive } => {
            watch::run_watch(
                watch::WatchOptions {
                    path,
                    recursive,
                    debounce_ms: cfg.watch.debounce_ms,
                    project: resolved_project,
                },
                &store,
            )?;
        }

        Commands::Serve { compact } => {
            hyphae_mcp::run_server(
                &store,
                embedder_ref,
                &cfg.consolidation,
                compact || cfg.mcp.compact,
                resolved_project,
                cfg.memory.reject_secrets,
            )?;
        }

        Commands::TestEmbed { text } => {
            if let Some(e) = &embedder {
                let embedding = e.embed(&text)?;
                println!("Embedding dimensions: {}", embedding.len());
                println!("First 5 values: {:?}", &embedding[..5.min(embedding.len())]);
            } else {
                println!("Embeddings not available");
                println!("Set HYPHAE_EMBEDDING_URL and HYPHAE_EMBEDDING_MODEL for HTTP embeddings");
                if !cfg!(feature = "embeddings") {
                    println!("Or build with: cargo install hyphae (includes fastembed)");
                }
            }
        }

        Commands::EmbedAll { topic, batch } => {
            commands::memory::cmd_embed_all(&store, embedder_ref, topic, batch, resolved_project)?;
        }

        Commands::Ingest {
            path,
            recursive,
            force,
        } => {
            commands::docs::cmd_ingest(
                &store,
                path,
                recursive,
                force,
                resolved_project,
                embedder_ref,
            )?;
        }

        Commands::SearchDocs { query, limit } => {
            commands::docs::cmd_search_docs(&store, query, limit, resolved_project, embedder_ref)?;
        }

        Commands::ListSources { json } => {
            commands::docs::cmd_list_sources(&store, json, resolved_project)?;
        }

        Commands::ForgetSource { path } => {
            commands::docs::cmd_forget_source(&store, path, resolved_project)?;
        }

        Commands::SearchAll {
            query,
            limit,
            include_docs,
            project_root,
            worktree_id,
        } => {
            commands::docs::cmd_search_all(
                &store,
                query,
                limit,
                include_docs,
                resolved_project,
                project_root.as_deref(),
                worktree_id.as_deref(),
                embedder_ref,
            )?;
        }

        Commands::Memoir(args) => {
            commands::memoir::dispatch(&store, args)?;
        }

        Commands::Project(args) => {
            commands::project::dispatch(&store, args)?;
        }

        Commands::Session(args) => {
            commands::session::dispatch(&store, embedder_ref, args)?;
        }

        Commands::Feedback(args) => {
            commands::feedback::dispatch(&store, args)?;
        }

        Commands::Bench { count } => {
            commands::bench::cmd_bench(count)?;
        }

        Commands::Lessons { limit } => {
            commands::lessons::cmd_lessons(&store, resolved_project, limit)?;
        }

        Commands::Activity => {
            commands::activity::cmd_activity(&store, resolved_project)?;
        }

        Commands::Analytics => {
            commands::analytics::cmd_analytics(&store, resolved_project)?;
        }

        Commands::Prune { threshold, dry_run } => {
            commands::prune::cmd_prune(&store, threshold, dry_run)?;
        }

        Commands::Consolidate {
            topic,
            all,
            above_threshold,
            dry_run,
            yes,
            no_backup,
        } => {
            commands::consolidate::cmd_consolidate(
                &store,
                &cfg.consolidation,
                topic,
                all,
                above_threshold,
                dry_run,
                yes,
                no_backup,
                resolved_project,
                resolved_db_path.as_path(),
            )?;
        }

        Commands::ImportClaudeMemory {
            path,
            dry_run,
            force,
            watch,
        } => {
            if watch {
                commands::import_claude_memory::watch(&store, path, force)?;
            } else {
                commands::import_claude_memory::run(&store, path, dry_run, force)?;
            }
        }

        Commands::IngestSessions {
            path,
            since,
            dry_run,
        } => {
            commands::transcript::run(&store, path, since, dry_run, resolved_project.as_deref())?;
        }

        Commands::CodexNotify { notification } => {
            commands::codex_notify::run(&store, notification.clone(), resolved_project.as_deref())?;
        }

        Commands::ExportTraining {
            format,
            topic,
            min_weight,
            min_recalls,
            min_effectiveness,
            output,
        } => {
            let fmt = format
                .parse::<commands::export_training::TrainingFormat>()
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            commands::export_training::cmd_export_training(
                &store,
                fmt,
                topic.clone(),
                min_weight,
                min_recalls,
                min_effectiveness,
                output.clone(),
                resolved_project,
            )?;
        }

        Commands::Evaluate { days } => {
            commands::evaluate::cmd_evaluate(&store, days, resolved_project)?;
        }

        Commands::Backup { .. } | Commands::Restore { .. } => {
            unreachable!("handled in early-return block")
        }

        Commands::Audit { command } => {
            commands::audit::dispatch(&store, command)?;
        }

        Commands::AuditSecrets { topic, detailed } => {
            commands::audit_secrets::cmd_audit_secrets(
                &store,
                topic.clone(),
                detailed,
                resolved_project,
            )?;
        }

        Commands::Purge {
            project,
            before,
            dry_run,
            force,
            no_backup,
        } => {
            commands::purge::cmd_purge(
                &store,
                project.clone(),
                before.clone(),
                dry_run,
                force,
                no_backup,
                resolved_db_path.as_path(),
            )?;
        }

        Commands::Changelog { days, since } => {
            commands::changelog::cmd_changelog(&store, days, since.clone(), resolved_project)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::all_projects_allowed;
    use crate::cli::Cli;
    use crate::cli::Commands;
    use crate::commands::memory::MemoryArgs;
    use crate::commands::project::{ProjectArgs, ProjectCommand};
    use crate::commands::session::{SessionArgs, SessionCommand};
    use clap::CommandFactory;
    use clap::Parser;
    use clap_complete::{Shell, generate};

    #[test]
    fn test_completions_nonempty() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
            let mut buf = Vec::new();
            generate(shell, &mut Cli::command(), "hyphae", &mut buf);
            assert!(
                !buf.is_empty(),
                "completions should not be empty for {shell:?}"
            );
        }
    }

    #[test]
    fn test_all_projects_allowed_for_read_only_commands() {
        assert!(all_projects_allowed(&Commands::Search {
            query: "query".to_string(),
            topic: None,
            limit: 10,
            include_invalidated: false,
            order: crate::commands::memory::SearchOrder::Weight,
            json: true,
            raw: false,
        }));
        assert!(all_projects_allowed(&Commands::Session(SessionArgs {
            command: SessionCommand::List {
                project: None,
                project_root: None,
                worktree_id: None,
                scope: None,
                limit: 20,
            },
        })));
        assert!(all_projects_allowed(&Commands::Project(ProjectArgs {
            command: ProjectCommand::Search {
                query: "query".to_string(),
                limit: 10,
            },
        })));
        assert!(all_projects_allowed(&Commands::Memory(MemoryArgs {
            cmd: crate::commands::memory::MemoryCommand::Get {
                id: "mem_1".to_string(),
                json: true,
            },
        })));
        assert!(all_projects_allowed(&Commands::Lessons { limit: 50 }));
        assert!(all_projects_allowed(&Commands::Analytics));
    }

    #[test]
    fn test_all_projects_rejected_for_mutating_commands() {
        assert!(!all_projects_allowed(&Commands::Store {
            topic: "topic".to_string(),
            content: "content".to_string(),
            importance: "medium".to_string(),
        }));
        assert!(!all_projects_allowed(&Commands::Session(SessionArgs {
            command: SessionCommand::Start {
                project: "cap".to_string(),
                task: None,
                project_root: None,
                worktree_id: None,
                scope: None,
                runtime_session_id: None,
                recent_files: Vec::new(),
                active_errors: Vec::new(),
                git_branch: None,
            },
        })));
        assert!(!all_projects_allowed(&Commands::Project(ProjectArgs {
            command: ProjectCommand::Share {
                id: "mem_1".to_string(),
            },
        })));
        assert!(!all_projects_allowed(&Commands::Serve { compact: false }));
    }

    #[test]
    fn test_search_all_accepts_identity_pair() {
        let cli = Cli::try_parse_from([
            "hyphae",
            "search-all",
            "auth",
            "--project-root",
            "/repo/demo",
            "--worktree-id",
            "wt-alpha",
        ])
        .expect("search-all should accept a full identity pair");

        match cli.command {
            Commands::SearchAll {
                project_root,
                worktree_id,
                ..
            } => {
                assert_eq!(project_root.as_deref(), Some("/repo/demo"));
                assert_eq!(worktree_id.as_deref(), Some("wt-alpha"));
            }
            _ => panic!("expected SearchAll command"),
        }
    }

    #[test]
    fn test_search_all_rejects_partial_identity_pair() {
        let err = match Cli::try_parse_from([
            "hyphae",
            "search-all",
            "auth",
            "--project-root",
            "/repo/demo",
        ]) {
            Ok(_) => panic!("search-all should reject a partial identity pair"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("--worktree-id"));
    }
}
