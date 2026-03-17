use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::commands::memoir::MemoirArgs;

#[derive(Parser)]
#[command(name = "hyphae", version)]
#[command(about = "Persistent memory system for AI agents", long_about = None)]
pub(crate) struct Cli {
    /// Path to database file
    #[arg(short, long)]
    pub(crate) db: Option<PathBuf>,

    /// Project namespace for memory isolation
    #[arg(short = 'P', long, global = true)]
    pub(crate) project: Option<String>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
        /// Input text file (reads stdin if omitted)
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

    /// Ingest a file or directory into the document store
    Ingest {
        /// Path to file or directory to ingest
        path: PathBuf,
        /// Recursively ingest subdirectories
        #[arg(short, long)]
        recursive: bool,
        /// Re-ingest even if source already exists
        #[arg(short, long)]
        force: bool,
    },

    /// Search ingested documents
    SearchDocs {
        /// Query text
        query: String,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },

    /// List all ingested document sources
    ListSources,

    /// Remove an ingested document source
    ForgetSource {
        /// Source path to remove
        path: String,
    },

    /// Search across memories and documents
    SearchAll {
        /// Query text
        query: String,
        /// Maximum total results
        #[arg(short, long, default_value = "10")]
        limit: usize,
        /// Include document chunks in results
        #[arg(long, default_value = "true")]
        include_docs: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Configure editor MCP settings
    Init {
        /// Specific editor to configure (auto-detects if omitted)
        #[arg(short, long, value_enum)]
        editor: Option<crate::init::Editor>,
    },

    /// Watch a directory and auto-ingest file changes
    Watch {
        /// Path to watch
        path: PathBuf,
        /// Watch recursively
        #[arg(short, long, default_value_t = true)]
        recursive: bool,
    },

    /// Start MCP server on stdio
    Serve {
        /// Enable compact output mode
        #[arg(long)]
        compact: bool,
    },

    /// Manage semantic knowledge graphs (memoirs)
    Memoir(MemoirArgs),

    /// Benchmark memory store write and search throughput
    Bench {
        /// Number of write and search operations to perform
        #[arg(long, default_value = "100")]
        count: usize,
    },

    /// Prune expired and low-weight memories
    Prune {
        /// Also prune memories with weight below this threshold
        #[arg(short, long)]
        threshold: Option<f32>,
        /// Show what would be pruned without deleting
        #[arg(long)]
        dry_run: bool,
    },
}
