use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::commands::memoir::MemoirArgs;
use crate::commands::project::ProjectArgs;

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

    /// Test embedding functionality (HTTP or fastembed)
    TestEmbed {
        /// Text to embed
        #[arg(short, long)]
        text: String,
    },

    /// Generate embeddings for all memories that don't have one yet
    EmbedAll {
        /// Only embed memories in this topic
        #[arg(short, long)]
        topic: Option<String>,
        /// Batch size for embedding requests
        #[arg(short, long, default_value = "32")]
        batch: usize,
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

    /// Cross-project knowledge management
    Project(ProjectArgs),

    /// Benchmark memory store write and search throughput
    Bench {
        /// Number of write and search operations to perform
        #[arg(long, default_value = "100")]
        count: usize,
    },

    /// Check for and install updates
    SelfUpdate {
        /// Only check for updates, don't download
        #[arg(long)]
        check: bool,
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

    /// Import Claude Code auto-memories from ~/.claude/projects/*/memory/
    ImportClaudeMemory {
        /// Path to a specific Claude project memory directory
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Show what would be imported without storing
        #[arg(long)]
        dry_run: bool,
        /// Re-import even if already imported (skip deduplication)
        #[arg(long)]
        force: bool,
        /// Watch for new/changed memory files and import continuously
        #[arg(long)]
        watch: bool,
    },

    /// Diagnose common issues with the hyphae installation
    Doctor {
        /// Attempt to fix detected issues (e.g. rebuild FTS index)
        #[arg(long)]
        fix: bool,
    },

    /// Ingest Claude Code session transcripts into hyphae memory
    IngestSessions {
        /// Path to a specific session directory
        #[arg(short, long)]
        path: Option<PathBuf>,
        /// Only ingest sessions newer than this date (YYYY-MM-DD)
        #[arg(short, long)]
        since: Option<String>,
        /// Show what would be ingested without storing
        #[arg(long)]
        dry_run: bool,
    },

    /// Export memories as training data
    ExportTrainingData {
        /// Output format: sft, dpo, or alpaca
        #[arg(short, long)]
        format: String,
        /// Only export specific topic
        #[arg(short, long)]
        topic: Option<String>,
        /// Only export memories with weight above this threshold
        #[arg(long)]
        min_weight: Option<f32>,
    },

    /// Evaluate agent improvement over time
    Evaluate {
        /// Total evaluation window in days (compares two equal halves)
        #[arg(long, default_value = "14")]
        days: i64,
    },

    /// Backup the database
    Backup {
        /// Output path for backup file (defaults to hyphae-backup-{timestamp}.db)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Restore database from a backup
    Restore {
        /// Path to backup file to restore
        path: PathBuf,
    },

    /// Scan memories for common secret patterns
    AuditSecrets {
        /// Only check memories in this topic
        #[arg(short, long)]
        topic: Option<String>,
        /// Show details for each finding
        #[arg(long)]
        detailed: bool,
    },

    /// Purge memories and related data (GDPR/retention compliance)
    Purge {
        /// Delete all memories for a specific project
        #[arg(long)]
        project: Option<String>,
        /// Delete all memories created before this date (YYYY-MM-DD or ISO 8601)
        #[arg(long)]
        before: Option<String>,
        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// View recent activity and lessons learned
    Changelog {
        /// Include activity from the last N days
        #[arg(long, default_value = "7")]
        days: i64,
        /// Only show activity since this date (YYYY-MM-DD HH:MM:SS)
        #[arg(long)]
        since: Option<String>,
    },
}
