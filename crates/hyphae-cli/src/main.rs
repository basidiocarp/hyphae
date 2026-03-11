use anyhow::Result;
use clap::{Parser, Subcommand};
#[cfg(feature = "embeddings")]
use hyphae_core::Embedder;
use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;
use std::path::PathBuf;

#[cfg(test)]
#[allow(unused)]
mod bench_data;
#[cfg(test)]
#[allow(unused)]
mod bench_knowledge;
mod config;
mod extract;

#[derive(Parser)]
#[command(name = "hyphae")]
#[command(about = "Persistent memory system for AI agents", long_about = None)]
struct Cli {
    /// Path to database file
    #[arg(short, long)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a memory in the system
    Store {
        /// Topic/category for the memory
        #[arg(short, long)]
        topic: String,

        /// Memory content text
        #[arg(short, long)]
        content: String,

        /// Importance level: critical, high, medium, low
        #[arg(short, long, default_value = "medium")]
        importance: String,
    },

    /// Search memories
    Search {
        /// Query text
        #[arg(short, long)]
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Extract facts from input text
    Extract {
        /// Input text file
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Project name for extraction context
        #[arg(short, long)]
        project: String,
    },

    /// Get system statistics
    Stats,

    /// Show config
    Config,

    /// Test embedding functionality
    #[cfg(feature = "embeddings")]
    TestEmbed {
        /// Text to embed
        #[arg(short, long)]
        text: String,
    },
}

fn parse_importance(s: &str) -> hyphae_core::Importance {
    match s.parse() {
        Ok(importance) => importance,
        Err(_) => {
            tracing::warn!("unrecognized importance level: {s}, defaulting to medium");
            hyphae_core::Importance::Medium
        }
    }
}

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

#[cfg(feature = "embeddings")]
fn init_embedder(model: &str) -> Option<hyphae_core::FastEmbedder> {
    hyphae_core::FastEmbedder::with_model(model)
        .map_err(|e| {
            tracing::warn!("embedder init failed: {e}");
        })
        .ok()
}

#[cfg(not(feature = "embeddings"))]
#[allow(dead_code)]
fn init_embedder(_model: &str) -> Option<()> {
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

    #[cfg(feature = "embeddings")]
    let embedder = init_embedder(&cfg.embeddings.model);

    #[cfg(feature = "embeddings")]
    let embedding_dims = embedder.as_ref().map(|e| e.dimensions()).unwrap_or(384);

    #[cfg(not(feature = "embeddings"))]
    let embedding_dims = 384;

    let store = open_store(cli.db, embedding_dims)?;

    match cli.command {
        Commands::Store {
            topic,
            content,
            importance,
        } => {
            let mem = hyphae_core::Memory::new(topic, content, parse_importance(&importance));
            store.store(mem)?;
            println!("Memory stored");
        }

        Commands::Search { query, limit } => {
            let results = store.search_fts(&query, limit)?;
            for mem in results {
                println!("{}: {}", mem.topic, mem.summary);
            }
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
            let stats = store.stats()?;
            println!("Database Statistics:");
            println!("  Total memories: {}", stats.total_memories);
        }

        Commands::Config => {
            println!("Configuration:");
            println!("  Config Path: {}", config::show_config_path());
            println!();
            println!("  Memory Settings:");
            println!("    Default Importance: {}", cfg.memory.default_importance);
            println!("    Decay Rate: {}", cfg.memory.decay_rate);
            println!("    Prune Threshold: {}", cfg.memory.prune_threshold);
            println!();
            println!("  Extraction Settings:");
            println!("    Enabled: {}", cfg.extraction.enabled);
            println!("    Min Score: {}", cfg.extraction.min_score);
            println!("    Max Facts: {}", cfg.extraction.max_facts);
            println!();
            println!("  Recall Settings:");
            println!("    Enabled: {}", cfg.recall.enabled);
            println!("    Limit: {}", cfg.recall.limit);
            println!();
            println!("  Embeddings Settings:");
            println!("    Model: {}", cfg.embeddings.model);
            println!();
            println!("  MCP Settings:");
            println!("    Transport: {}", cfg.mcp.transport);
            println!("    Compact Mode: {}", cfg.mcp.compact);
            if let Some(instructions) = &cfg.mcp.instructions {
                println!("    Instructions: {}", instructions);
            }
        }

        #[cfg(feature = "embeddings")]
        Commands::TestEmbed { text } => {
            if let Some(e) = embedder {
                let embedding = e.embed(&text)?;
                println!("Embedding dimensions: {}", embedding.len());
                println!("First 5 values: {:?}", &embedding[..5.min(embedding.len())]);
            } else {
                println!("Embeddings not enabled");
            }
        }
    }

    Ok(())
}
