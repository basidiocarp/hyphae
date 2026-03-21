use anyhow::Result;
use clap::Parser;
use hyphae_core::Embedder;
use hyphae_store::SqliteStore;
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
mod project;
mod watch;

use cli::{Cli, Commands};

fn open_store(db: Option<PathBuf>, embedding_dims: usize) -> Result<SqliteStore> {
    let path = db.unwrap_or_else(|| {
        directories::ProjectDirs::from("", "", "hyphae")
            .map(|d| d.data_dir().join("hyphae.db"))
            .unwrap_or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/share/hyphae/hyphae.db"))
                    .unwrap_or_else(|| PathBuf::from(".local/share/hyphae/hyphae.db"))
            })
    });

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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();
    let cfg = config::load_config()?;

    let resolved_project: Option<String> = cli
        .project
        .clone()
        .or_else(|| cfg.store.default_project.clone())
        .or_else(project::detect_project);

    // Early-return commands (no store/embedder needed)
    match &cli.command {
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
        Commands::Config => {
            commands::cmd_config(&cfg);
            return Ok(());
        }
        Commands::Init { editor } => {
            init::run_init(editor.clone())?;
            return Ok(());
        }
        Commands::SelfUpdate { check } => {
            commands::self_update::run(*check)?;
            return Ok(());
        }
        Commands::Doctor { fix } => {
            commands::doctor::run(*fix)?;
            return Ok(());
        }
        Commands::Backup { output } => {
            commands::backup::cmd_backup(output.clone())?;
            return Ok(());
        }
        Commands::Restore { path } => {
            commands::backup::cmd_restore(path.clone())?;
            return Ok(());
        }
        _ => {}
    }

    let embedder = init_embedder(&cfg.embeddings.model);
    let embedding_dims = embedder.as_ref().map(|e| e.dimensions()).unwrap_or(384);

    let store = open_store(cli.db, embedding_dims)?;

    let embedder_ref: Option<&dyn Embedder> = embedder.as_ref().map(|e| e.as_ref());

    match cli.command {
        Commands::Store {
            topic,
            content,
            importance,
        } => {
            commands::memory::cmd_store(&store, topic, content, &importance, resolved_project)?;
        }

        Commands::Search { query, limit } => {
            commands::memory::cmd_search(&store, query, limit, resolved_project)?;
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

        Commands::Stats => {
            commands::memory::cmd_stats(&store, resolved_project)?;
        }

        Commands::Config => unreachable!("handled in early-return block"),
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
                compact || cfg.mcp.compact,
                resolved_project,
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

        Commands::ListSources => {
            commands::docs::cmd_list_sources(&store, resolved_project)?;
        }

        Commands::ForgetSource { path } => {
            commands::docs::cmd_forget_source(&store, path, resolved_project)?;
        }

        Commands::SearchAll {
            query,
            limit,
            include_docs,
        } => {
            commands::docs::cmd_search_all(
                &store,
                query,
                limit,
                include_docs,
                resolved_project,
                embedder_ref,
            )?;
        }

        Commands::Memoir(args) => {
            commands::memoir::dispatch(&store, args)?;
        }

        Commands::Project(args) => {
            commands::project::dispatch(&store, args)?;
        }

        Commands::Bench { count } => {
            commands::bench::cmd_bench(count)?;
        }

        Commands::Prune { threshold, dry_run } => {
            commands::prune::cmd_prune(&store, threshold, dry_run)?;
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

        Commands::ExportTrainingData {
            format,
            topic,
            min_weight,
        } => {
            let fmt = format
                .parse::<commands::export_training::TrainingFormat>()
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            commands::export_training::cmd_export_training(
                &store,
                fmt,
                topic.clone(),
                min_weight.clone(),
                resolved_project,
            )?;
        }

        Commands::Evaluate { days } => {
            commands::evaluate::cmd_evaluate(&store, days, resolved_project)?;
        }

        Commands::Backup { .. } | Commands::Restore { .. } => {
            unreachable!("handled in early-return block")
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
        } => {
            commands::purge::cmd_purge(
                &store,
                project.clone(),
                before.clone(),
                dry_run,
                resolved_project,
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
    use crate::cli::Cli;
    use clap::CommandFactory;
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
}
