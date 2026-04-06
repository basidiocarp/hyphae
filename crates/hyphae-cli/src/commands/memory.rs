use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use hyphae_core::{
    ConsolidationConfig, Embedder, GitContext, Memory, MemoryId, MemorySource, MemoryStore,
    SessionHost,
};
use hyphae_store::{SearchOrder as StoreSearchOrder, SqliteStore, TopicMemoryOrder};
use regex::Regex;
use serde::Serialize;

use crate::project;

const STATS_SCHEMA_VERSION: &str = "1.0";
const TOPICS_SCHEMA_VERSION: &str = "1.0";
const SEARCH_SCHEMA_VERSION: &str = "1.0";
const MEMORY_LOOKUP_SCHEMA_VERSION: &str = "1.0";
const TOPIC_MEMORIES_SCHEMA_VERSION: &str = "1.0";
const HEALTH_SCHEMA_VERSION: &str = "1.0";

#[derive(Args)]
pub(crate) struct MemoryArgs {
    #[command(subcommand)]
    pub(crate) cmd: MemoryCommand,
}

#[derive(Subcommand)]
pub(crate) enum MemoryCommand {
    /// Show a memory by id
    Get {
        /// Memory id
        id: String,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// List memories in a topic
    Topic {
        /// Topic name
        topic: String,
        /// Maximum memories to emit
        #[arg(short, long, default_value = "50")]
        limit: usize,
        /// Include invalidated memories in read-only output
        #[arg(long)]
        include_invalidated: bool,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SearchOrder {
    Rank,
    Weight,
}

#[derive(Serialize)]
struct StoreStatsPayload {
    project: Option<String>,
    total_memories: usize,
    total_topics: usize,
    avg_weight: f32,
    oldest_memory: Option<chrono::DateTime<chrono::Utc>>,
    newest_memory: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
struct TopicCountPayload {
    topic: String,
    count: usize,
    avg_weight: f32,
    oldest: Option<chrono::DateTime<chrono::Utc>>,
    newest: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
struct TopicsPayload {
    project: Option<String>,
    total_topics: usize,
    total_memories: usize,
    topics: Vec<TopicCountPayload>,
}

#[derive(Serialize)]
struct MemoryPayload {
    id: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_accessed: chrono::DateTime<chrono::Utc>,
    access_count: u32,
    weight: f32,
    topic: String,
    summary: String,
    raw_excerpt: Option<String>,
    keywords: Vec<String>,
    importance: String,
    source: MemorySourcePayload,
    related_ids: Vec<String>,
    project: Option<String>,
    branch: Option<String>,
    worktree: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    invalidated_at: Option<chrono::DateTime<chrono::Utc>>,
    invalidation_reason: Option<String>,
    superseded_by: Option<String>,
    has_embedding: bool,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MemorySourcePayload {
    AgentSession {
        host: String,
        session_id: String,
        file_path: Option<String>,
    },
    Manual,
}

#[derive(Serialize)]
struct MemoryLookupPayload {
    project: Option<String>,
    memory: MemoryPayload,
}

#[derive(Serialize)]
struct TopicMemoriesPayload {
    project: Option<String>,
    topic: String,
    total: usize,
    memories: Vec<MemoryPayload>,
}

#[derive(Serialize)]
struct SearchPayload {
    project: Option<String>,
    query: String,
    topic: Option<String>,
    limit: usize,
    total: usize,
    results: Vec<MemoryPayload>,
}

#[derive(Serialize)]
struct TopicHealthPayload {
    topic: String,
    entry_count: usize,
    avg_weight: f32,
    avg_access_count: f32,
    oldest: Option<chrono::DateTime<chrono::Utc>>,
    newest: Option<chrono::DateTime<chrono::Utc>>,
    last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    needs_consolidation: bool,
    stale_count: usize,
    low_weight_count: usize,
    critical_count: usize,
    high_count: usize,
    medium_count: usize,
    low_count: usize,
}

#[derive(Serialize)]
struct HealthPayload {
    project: Option<String>,
    requested_topic: Option<String>,
    total_topics: usize,
    topics_needing_consolidation: usize,
    total_stale_entries: usize,
    topics: Vec<TopicHealthPayload>,
}

#[derive(Serialize)]
struct VersionedPayload<'a, T: Serialize> {
    schema_version: &'a str,
    #[serde(flatten)]
    payload: &'a T,
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

// ─────────────────────────────────────────────────────────────────────────────
// Secrets Detection
// ─────────────────────────────────────────────────────────────────────────────

const SECRET_PATTERNS: &[(&str, &str)] = &[
    (r"(?i)(api[_-]?key|apikey)\s*[:=]\s*\S{10,}", "API key"),
    (
        r"(?i)(secret|password|passwd|pwd)\s*[:=]\s*\S{8,}",
        "password/secret",
    ),
    (r"sk-[a-zA-Z0-9]{20,}", "OpenAI API key"),
    (r"ghp_[a-zA-Z0-9]{36}", "GitHub personal access token"),
    (r"(?i)bearer\s+[a-zA-Z0-9._-]{20,}", "Bearer token"),
    (r"AKIA[0-9A-Z]{16}", "AWS access key"),
    (
        r"(?i)(token|auth)\s*[:=]\s*[a-zA-Z0-9._-]{20,}",
        "auth token",
    ),
    (r"-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----", "private key"),
];

/// Detect common secret patterns in content.
fn detect_secrets(content: &str) -> Vec<String> {
    let mut detected = Vec::new();

    for (pattern, secret_type) in SECRET_PATTERNS {
        if let Ok(regex) = Regex::new(pattern) {
            if regex.is_match(content) {
                detected.push(secret_type.to_string());
            }
        }
    }

    detected
}

pub(crate) fn dispatch(
    store: &SqliteStore,
    args: MemoryArgs,
    project: Option<String>,
) -> Result<()> {
    match args.cmd {
        MemoryCommand::Get { id, json } => cmd_get(store, id, json, project),
        MemoryCommand::Topic {
            topic,
            limit,
            include_invalidated,
            json,
        } => cmd_topic_memories(store, topic, limit, include_invalidated, json, project),
    }
}

pub(crate) fn cmd_store(
    store: &SqliteStore,
    topic: String,
    content: String,
    importance: &str,
    project: Option<String>,
) -> Result<()> {
    // Check for secrets before storing
    let warnings = detect_secrets(&content);
    if !warnings.is_empty() {
        eprintln!(
            "Warning: possible secrets detected: {}",
            warnings.join(", ")
        );
    }

    let mut mem = hyphae_core::Memory::new(topic, content, parse_importance(importance));
    mem.project = project;
    let GitContext { branch, worktree } = project::detect_git_context();
    mem.branch = branch;
    mem.worktree = worktree;
    store.store(mem)?;
    println!("Memory stored");
    Ok(())
}

pub(crate) fn cmd_search(
    store: &SqliteStore,
    query: String,
    topic: Option<String>,
    limit: usize,
    include_invalidated: bool,
    order: SearchOrder,
    json: bool,
    project: Option<String>,
) -> Result<()> {
    let payload = search_payload(
        store,
        &query,
        topic.as_deref(),
        limit,
        project.as_deref(),
        include_invalidated,
        order,
    )?;
    if json {
        print_json_versioned(SEARCH_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    for mem in &payload.results {
        crate::display::print_memory(&memory_from_payload(mem), None);
    }
    Ok(())
}

pub(crate) fn cmd_invalidate(
    store: &SqliteStore,
    id: String,
    reason: Option<String>,
    superseded_by: Option<String>,
) -> Result<()> {
    let memory_id = MemoryId::from(id.clone());
    let superseded_by_id = superseded_by.as_deref().map(MemoryId::from);

    store.invalidate(&memory_id, reason.as_deref(), superseded_by_id.as_ref())?;
    println!("Memory invalidated: {id}");
    Ok(())
}

pub(crate) fn cmd_list_invalidated(
    store: &SqliteStore,
    limit: usize,
    project: Option<String>,
) -> Result<()> {
    let results = store.list_invalidated(limit, 0, project.as_deref())?;
    if results.is_empty() {
        println!("No invalidated memories.");
        return Ok(());
    }

    for mem in &results {
        crate::display::print_memory(mem, None);
    }
    Ok(())
}

pub(crate) fn cmd_stats(
    store: &SqliteStore,
    json: bool,
    project: Option<String>,
    include_invalidated: bool,
) -> Result<()> {
    let payload = stats_payload(store, project.as_deref(), include_invalidated)?;
    if json {
        print_json_versioned(STATS_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    println!("Database Statistics:");
    println!("  Total memories: {}", payload.total_memories);
    println!("  Total topics: {}", payload.total_topics);
    println!("  Average weight: {:.3}", payload.avg_weight);
    if let Some(oldest) = payload.oldest_memory {
        println!("  Oldest memory: {}", oldest.format("%Y-%m-%d %H:%M"));
    }
    if let Some(newest) = payload.newest_memory {
        println!("  Newest memory: {}", newest.format("%Y-%m-%d %H:%M"));
    }
    Ok(())
}

pub(crate) fn cmd_topics(
    store: &SqliteStore,
    json: bool,
    project: Option<String>,
    include_invalidated: bool,
) -> Result<()> {
    let payload = topics_payload(store, project.as_deref(), include_invalidated)?;
    if json {
        print_json_versioned(TOPICS_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.topics.is_empty() {
        println!("No topics yet.");
        return Ok(());
    }

    println!("Topics:");
    for topic in &payload.topics {
        println!("  {}: {} memories", topic.topic, topic.count);
    }
    Ok(())
}

pub(crate) fn cmd_health(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    topic: Option<String>,
    include_invalidated: bool,
    json: bool,
    project: Option<String>,
) -> Result<()> {
    let payload = health_payload(
        store,
        consolidation,
        topic.as_deref(),
        project.as_deref(),
        include_invalidated,
    )?;
    if json {
        print_json_versioned(HEALTH_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.topics.is_empty() {
        println!("No topics yet.");
        return Ok(());
    }

    println!("Memory Health Report:");
    println!();
    for topic in &payload.topics {
        let status = if topic.needs_consolidation && topic.stale_count > 0 {
            "needs attention"
        } else if topic.needs_consolidation {
            "consolidate"
        } else if topic.stale_count > 0 {
            "has stale entries"
        } else {
            "healthy"
        };
        println!(
            "  {}: {}\n    entries: {}  avg_weight: {:.2}  stale: {}  avg_access: {:.1}",
            topic.topic,
            status,
            topic.entry_count,
            topic.avg_weight,
            topic.stale_count,
            topic.avg_access_count,
        );
    }

    println!();
    println!(
        "Summary: {} topics, {} need consolidation, {} stale entries total",
        payload.total_topics, payload.topics_needing_consolidation, payload.total_stale_entries
    );
    Ok(())
}

pub(crate) fn cmd_get(
    store: &SqliteStore,
    id: String,
    json: bool,
    project: Option<String>,
) -> Result<()> {
    let payload = memory_lookup_payload(store, &id, project.as_deref())?;
    if json {
        print_json_versioned(MEMORY_LOOKUP_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    crate::display::print_memory(&memory_from_payload(&payload.memory), None);
    Ok(())
}

pub(crate) fn cmd_topic_memories(
    store: &SqliteStore,
    topic: String,
    limit: usize,
    include_invalidated: bool,
    json: bool,
    project: Option<String>,
) -> Result<()> {
    let payload = topic_memories_payload(
        store,
        &topic,
        project.as_deref(),
        include_invalidated,
        limit,
    )?;
    if json {
        print_json_versioned(TOPIC_MEMORIES_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.memories.is_empty() {
        println!("No memories found for topic: {}", payload.topic);
        return Ok(());
    }

    for memory in &payload.memories {
        crate::display::print_memory(&memory_from_payload(memory), None);
    }
    Ok(())
}

pub(crate) fn cmd_embed_all(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    topic_filter: Option<String>,
    batch_size: usize,
    project: Option<String>,
) -> Result<()> {
    let embedder = embedder.ok_or_else(|| {
        anyhow::anyhow!(
            "no embedder available\n\
             Set HYPHAE_EMBEDDING_URL and HYPHAE_EMBEDDING_MODEL for HTTP embeddings,\n\
             or build with embeddings feature: cargo install hyphae"
        )
    })?;

    let project_ref = project.as_deref();

    // Collect all memories, optionally filtered by topic
    let memories = if let Some(ref t) = topic_filter {
        store.get_by_topic(t, project_ref)?
    } else {
        let topics = store.list_topics(project_ref)?;
        let mut all = Vec::new();
        for (t, _) in &topics {
            let mems = store.get_by_topic(t, project_ref)?;
            all.extend(mems);
        }
        all
    };

    // Filter to those without embeddings
    let to_embed: Vec<_> = memories.iter().filter(|m| m.embedding.is_none()).collect();

    if to_embed.is_empty() {
        println!("All memories already have embeddings.");
        return Ok(());
    }

    let total = to_embed.len();
    println!("Embedding {total} memories (batch size: {batch_size})...");

    let mut embedded = 0usize;
    let mut errors = 0usize;

    for chunk in to_embed.chunks(batch_size) {
        let texts: Vec<String> = chunk
            .iter()
            .map(|m| format!("{} {}", m.topic, m.summary))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        match embedder.embed_batch(&text_refs) {
            Ok(vecs) => {
                for (mem, vec) in chunk.iter().zip(vecs) {
                    let mut updated = (*mem).clone();
                    updated.embedding = Some(vec);
                    if store.update(&updated).is_ok() {
                        embedded += 1;
                    } else {
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("Batch embedding failed: {e}");
                errors += chunk.len();
            }
        }

        // Progress
        let done = embedded + errors;
        let pct = done * 100 / total;
        let bar_width = 30;
        let filled = bar_width * done / total;
        let bar: String = "=".repeat(filled) + &" ".repeat(bar_width - filled);
        eprint!("\rEmbedding: {done}/{total} [{bar}] {pct}%");
    }

    eprintln!(); // newline after progress bar
    println!("Done: {embedded} embedded, {errors} errors (of {total} total)");

    Ok(())
}

fn stats_payload(
    store: &SqliteStore,
    project: Option<&str>,
    include_invalidated: bool,
) -> Result<StoreStatsPayload> {
    let stats = store.stats_with_options(project, include_invalidated)?;
    Ok(StoreStatsPayload {
        project: project.map(str::to_string),
        total_memories: stats.total_memories,
        total_topics: stats.total_topics,
        avg_weight: stats.avg_weight,
        oldest_memory: stats.oldest_memory,
        newest_memory: stats.newest_memory,
    })
}

fn topics_payload(
    store: &SqliteStore,
    project: Option<&str>,
    include_invalidated: bool,
) -> Result<TopicsPayload> {
    let topics = store.list_topics_with_options(project, include_invalidated)?;
    let total_memories = topics.iter().map(|(_, count)| *count).sum();
    Ok(TopicsPayload {
        project: project.map(str::to_string),
        total_topics: topics.len(),
        total_memories,
        topics: topics
            .into_iter()
            .map(|(topic, count)| {
                let health =
                    store.topic_health_with_options(&topic, project, include_invalidated)?;
                Ok(TopicCountPayload {
                    topic,
                    count,
                    avg_weight: health.avg_weight,
                    oldest: health.oldest,
                    newest: health.newest,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

fn memory_lookup_payload(
    store: &SqliteStore,
    id: &str,
    project: Option<&str>,
) -> Result<MemoryLookupPayload> {
    let memory = store
        .get(&MemoryId::from(id))
        .map_err(|e| anyhow::anyhow!("failed to read memory: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("memory not found: {id}"))?;

    if let Some(project) = project {
        if memory.project.as_deref() != Some(project) {
            anyhow::bail!("memory not found: {id}");
        }
    }

    Ok(MemoryLookupPayload {
        project: project.map(str::to_string),
        memory: to_memory_payload(&memory),
    })
}

fn topic_memories_payload(
    store: &SqliteStore,
    topic: &str,
    project: Option<&str>,
    include_invalidated: bool,
    limit: usize,
) -> Result<TopicMemoriesPayload> {
    let memories = store.get_by_topic_with_options(
        topic,
        project,
        include_invalidated,
        TopicMemoryOrder::CreatedAtDesc,
    )?;
    let total = memories.len();
    let limited_memories = memories.into_iter().take(limit).collect::<Vec<_>>();
    Ok(TopicMemoriesPayload {
        project: project.map(str::to_string),
        topic: topic.to_string(),
        total,
        memories: limited_memories.iter().map(to_memory_payload).collect(),
    })
}

fn search_payload(
    store: &SqliteStore,
    query: &str,
    topic: Option<&str>,
    limit: usize,
    project: Option<&str>,
    include_invalidated: bool,
    order: SearchOrder,
) -> Result<SearchPayload> {
    let total = store.search_fts_count_with_options(query, topic, project, include_invalidated)?;
    let results = store.search_fts_with_options(
        query,
        topic,
        limit,
        0,
        project,
        include_invalidated,
        match order {
            SearchOrder::Rank => StoreSearchOrder::RankAsc,
            SearchOrder::Weight => StoreSearchOrder::WeightDesc,
        },
    )?;
    Ok(SearchPayload {
        project: project.map(str::to_string),
        query: query.to_string(),
        topic: topic.map(str::to_string),
        limit,
        total,
        results: results.iter().map(to_memory_payload).collect(),
    })
}

fn health_payload(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    requested_topic: Option<&str>,
    project: Option<&str>,
    include_invalidated: bool,
) -> Result<HealthPayload> {
    let topics = if let Some(topic) = requested_topic {
        vec![to_topic_health_payload(
            &store.topic_health_with_options(topic, project, include_invalidated)?,
            &store.get_by_topic_with_options(
                topic,
                project,
                include_invalidated,
                TopicMemoryOrder::CreatedAtDesc,
            )?,
            consolidation,
        )]
    } else {
        let topic_names = store.list_topics_with_options(project, include_invalidated)?;
        topic_names
            .into_iter()
            .map(|(topic, _)| {
                let memories = store.get_by_topic_with_options(
                    &topic,
                    project,
                    include_invalidated,
                    TopicMemoryOrder::CreatedAtDesc,
                )?;
                store
                    .topic_health_with_options(&topic, project, include_invalidated)
                    .map(|health| to_topic_health_payload(&health, &memories, consolidation))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let topics_needing_consolidation = topics
        .iter()
        .filter(|topic| topic.needs_consolidation)
        .count();
    let total_stale_entries = topics.iter().map(|topic| topic.stale_count).sum();

    Ok(HealthPayload {
        project: project.map(str::to_string),
        requested_topic: requested_topic.map(str::to_string),
        total_topics: topics.len(),
        topics_needing_consolidation,
        total_stale_entries,
        topics,
    })
}

fn to_topic_health_payload(
    health: &hyphae_core::TopicHealth,
    memories: &[Memory],
    consolidation: &ConsolidationConfig,
) -> TopicHealthPayload {
    let low_weight_count = memories
        .iter()
        .filter(|memory| memory.weight.value() < 0.3)
        .count();
    let critical_count = memories
        .iter()
        .filter(|memory| matches!(memory.importance, hyphae_core::Importance::Critical))
        .count();
    let high_count = memories
        .iter()
        .filter(|memory| matches!(memory.importance, hyphae_core::Importance::High))
        .count();
    let medium_count = memories
        .iter()
        .filter(|memory| matches!(memory.importance, hyphae_core::Importance::Medium))
        .count();
    let low_count = memories
        .iter()
        .filter(|memory| matches!(memory.importance, hyphae_core::Importance::Low))
        .count();

    let needs_consolidation = match consolidation.threshold_for_topic(&health.topic) {
        Some(threshold) => memories.len() >= threshold,
        None => false,
    };

    TopicHealthPayload {
        topic: health.topic.clone(),
        entry_count: health.entry_count,
        avg_weight: health.avg_weight,
        avg_access_count: health.avg_access_count,
        oldest: health.oldest,
        newest: health.newest,
        last_accessed: health.last_accessed,
        needs_consolidation,
        stale_count: health.stale_count,
        low_weight_count,
        critical_count,
        high_count,
        medium_count,
        low_count,
    }
}

fn to_memory_payload(memory: &Memory) -> MemoryPayload {
    MemoryPayload {
        id: memory.id.to_string(),
        created_at: memory.created_at,
        updated_at: memory.updated_at,
        last_accessed: memory.last_accessed,
        access_count: memory.access_count,
        weight: memory.weight.value(),
        topic: memory.topic.clone(),
        summary: memory.summary.clone(),
        raw_excerpt: memory.raw_excerpt.clone(),
        keywords: memory.keywords.clone(),
        importance: memory.importance.to_string(),
        source: to_memory_source_payload(&memory.source),
        related_ids: memory.related_ids.iter().map(ToString::to_string).collect(),
        project: memory.project.clone(),
        branch: memory.branch.clone(),
        worktree: memory.worktree.clone(),
        expires_at: memory.expires_at,
        invalidated_at: memory.invalidated_at,
        invalidation_reason: memory.invalidation_reason.clone(),
        superseded_by: memory.superseded_by.as_ref().map(ToString::to_string),
        has_embedding: memory.embedding.is_some(),
    }
}

fn to_memory_source_payload(source: &MemorySource) -> MemorySourcePayload {
    match source {
        MemorySource::AgentSession {
            host,
            session_id,
            file_path,
        } => MemorySourcePayload::AgentSession {
            host: match host {
                SessionHost::ClaudeCode => "claude-code".to_string(),
                SessionHost::Codex => "codex".to_string(),
            },
            session_id: session_id.clone(),
            file_path: file_path.clone(),
        },
        MemorySource::Manual => MemorySourcePayload::Manual,
    }
}

fn memory_from_payload(payload: &MemoryPayload) -> Memory {
    Memory {
        id: MemoryId::from(payload.id.clone()),
        created_at: payload.created_at,
        updated_at: payload.updated_at,
        last_accessed: payload.last_accessed,
        access_count: payload.access_count,
        weight: hyphae_core::Weight::new_clamped(payload.weight),
        topic: payload.topic.clone(),
        summary: payload.summary.clone(),
        raw_excerpt: payload.raw_excerpt.clone(),
        keywords: payload.keywords.clone(),
        importance: parse_importance(&payload.importance),
        source: match &payload.source {
            MemorySourcePayload::AgentSession {
                host,
                session_id,
                file_path,
            } => MemorySource::agent_session(
                match host.as_str() {
                    "codex" => SessionHost::Codex,
                    _ => SessionHost::ClaudeCode,
                },
                session_id.clone(),
                file_path.clone(),
            ),
            MemorySourcePayload::Manual => MemorySource::Manual,
        },
        related_ids: payload
            .related_ids
            .iter()
            .cloned()
            .map(MemoryId::from)
            .collect(),
        project: payload.project.clone(),
        branch: payload.branch.clone(),
        worktree: payload.worktree.clone(),
        expires_at: payload.expires_at,
        invalidated_at: payload.invalidated_at,
        invalidation_reason: payload.invalidation_reason.clone(),
        superseded_by: payload.superseded_by.as_ref().cloned().map(MemoryId::from),
        embedding: None,
    }
}

fn print_json<T: Serialize>(payload: &T) -> Result<()> {
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}

fn print_json_versioned<T: Serialize>(schema_version: &'static str, payload: &T) -> Result<()> {
    let versioned = VersionedPayload {
        schema_version,
        payload,
    };
    print_json(&versioned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, MemorySource, MemoryStore};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn store_memory(
        store: &SqliteStore,
        topic: &str,
        summary: &str,
        weight: f32,
        source: MemorySource,
        project: Option<&str>,
    ) -> Memory {
        let mut builder = Memory::builder(topic.to_string(), summary.to_string(), Importance::High)
            .keywords(vec!["rust".to_string(), "sqlite".to_string()])
            .source(source)
            .weight(weight);
        if let Some(project) = project {
            builder = builder.project(project.to_string());
        }
        let memory = builder.build();
        store.store(memory.clone()).unwrap();
        memory
    }

    #[test]
    fn test_stats_payload_reports_counts() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Use SQLite",
            0.8,
            MemorySource::Manual,
            Some("cap"),
        );
        store_memory(
            &store,
            "errors/build",
            "Fix borrow checker issue",
            0.6,
            MemorySource::Manual,
            Some("cap"),
        );

        let payload = stats_payload(&store, Some("cap"), false).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["project"].as_str(), Some("cap"));
        assert_eq!(value["total_memories"].as_u64(), Some(2));
        assert_eq!(value["total_topics"].as_u64(), Some(2));
        assert!(value["avg_weight"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_topics_payload_lists_topic_counts() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Use SQLite",
            0.8,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "decisions/api",
            "Prefer WAL mode",
            0.7,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "errors/build",
            "Fix borrow checker issue",
            0.6,
            MemorySource::Manual,
            None,
        );

        let payload = topics_payload(&store, None, false).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["total_topics"].as_u64(), Some(2));
        assert_eq!(value["total_memories"].as_u64(), Some(3));
        assert_eq!(value["topics"][0]["topic"].as_str(), Some("decisions/api"));
        assert_eq!(value["topics"][0]["count"].as_u64(), Some(2));
        assert!(value["topics"][0]["avg_weight"].as_f64().unwrap() > 0.0);
        assert_eq!(value["topics"][1]["topic"].as_str(), Some("errors/build"));
    }

    #[test]
    fn test_memory_lookup_payload_uses_stable_fields() {
        let store = test_store();
        let memory = store_memory(
            &store,
            "sessions/demo",
            "Remembered session outcome",
            0.9,
            MemorySource::agent_session(
                SessionHost::Codex,
                "sess-123",
                Some("logs/session.jsonl".to_string()),
            ),
            Some("demo-project"),
        );

        let payload =
            memory_lookup_payload(&store, memory.id.as_ref(), Some("demo-project")).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["project"].as_str(), Some("demo-project"));
        assert_eq!(value["memory"]["id"].as_str(), Some(memory.id.as_ref()));
        assert_eq!(value["memory"]["importance"].as_str(), Some("high"));
        assert_eq!(
            value["memory"]["source"]["type"].as_str(),
            Some("agent_session")
        );
        assert_eq!(value["memory"]["source"]["host"].as_str(), Some("codex"));
        assert_eq!(value["memory"]["has_embedding"].as_bool(), Some(false));
    }

    #[test]
    fn test_topic_memories_payload_lists_memories() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Use SQLite",
            0.8,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "decisions/api",
            "Prefer WAL mode",
            0.7,
            MemorySource::Manual,
            None,
        );

        let payload = topic_memories_payload(&store, "decisions/api", None, false, 50).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["topic"].as_str(), Some("decisions/api"));
        assert_eq!(value["total"].as_u64(), Some(2));
        assert_eq!(value["memories"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_search_payload_returns_structured_results() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Use SQLite for local storage",
            0.8,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "errors/build",
            "Rust borrow checker fix",
            0.7,
            MemorySource::Manual,
            None,
        );

        let payload = search_payload(
            &store,
            "local storage",
            None,
            5,
            None,
            false,
            SearchOrder::Weight,
        )
        .unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["query"].as_str(), Some("local storage"));
        assert!(value["topic"].is_null());
        assert_eq!(value["limit"].as_u64(), Some(5));
        assert_eq!(value["total"].as_u64(), Some(1));
        assert_eq!(value["results"][0]["topic"].as_str(), Some("decisions/api"));
    }

    #[test]
    fn test_search_payload_filters_to_topic() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Use SQLite for local storage",
            0.8,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "errors/build",
            "Local storage error to fix",
            0.7,
            MemorySource::Manual,
            None,
        );

        let payload = search_payload(
            &store,
            "local storage",
            Some("decisions/api"),
            5,
            None,
            false,
            SearchOrder::Rank,
        )
        .unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["topic"].as_str(), Some("decisions/api"));
        assert_eq!(value["total"].as_u64(), Some(1));
        assert_eq!(value["results"][0]["topic"].as_str(), Some("decisions/api"));
    }

    #[test]
    fn test_search_payload_reports_total_before_limit() {
        let store = test_store();
        store_memory(
            &store,
            "decisions/api",
            "Local storage keeps the cache near the repo root",
            0.9,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "decisions/api",
            "Local storage helps preserve offline context",
            0.8,
            MemorySource::Manual,
            None,
        );
        store_memory(
            &store,
            "errors/build",
            "Local storage regression surfaced during tests",
            0.7,
            MemorySource::Manual,
            None,
        );

        let payload = search_payload(
            &store,
            "local storage",
            None,
            2,
            None,
            false,
            SearchOrder::Weight,
        )
        .unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["total"].as_u64(), Some(3));
        assert_eq!(value["results"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_health_payload_includes_summary_and_filter() {
        let store = test_store();
        for idx in 0..16 {
            store_memory(
                &store,
                "decisions/api",
                &format!("decision {idx}"),
                0.4,
                MemorySource::Manual,
                Some("demo-project"),
            );
        }
        store_memory(
            &store,
            "errors/build",
            "Fix borrow checker issue",
            0.9,
            MemorySource::Manual,
            Some("demo-project"),
        );

        let all_payload = health_payload(
            &store,
            &ConsolidationConfig::default(),
            None,
            Some("demo-project"),
            false,
        )
        .unwrap();
        let all_value = serde_json::to_value(&all_payload).unwrap();
        assert_eq!(all_value["project"].as_str(), Some("demo-project"));
        assert_eq!(all_value["total_topics"].as_u64(), Some(2));
        assert_eq!(all_value["topics_needing_consolidation"].as_u64(), Some(1));

        let filtered_payload = health_payload(
            &store,
            &ConsolidationConfig::default(),
            Some("decisions/api"),
            None,
            false,
        )
        .unwrap();
        let filtered_value = serde_json::to_value(&filtered_payload).unwrap();
        assert_eq!(
            filtered_value["requested_topic"].as_str(),
            Some("decisions/api")
        );
        assert_eq!(filtered_value["topics"].as_array().unwrap().len(), 1);
        assert_eq!(
            filtered_value["topics"][0]["needs_consolidation"].as_bool(),
            Some(true)
        );
        assert_eq!(
            filtered_value["topics"][0]["medium_count"].as_u64(),
            Some(0)
        );
        assert_eq!(filtered_value["topics"][0]["high_count"].as_u64(), Some(16));
        assert_eq!(
            filtered_value["topics"][0]["low_weight_count"].as_u64(),
            Some(0)
        );
    }

    #[test]
    fn test_memory_lookup_payload_rejects_cross_project_memory() {
        let store = test_store();
        let memory = store_memory(
            &store,
            "sessions/demo",
            "Remembered session outcome",
            0.9,
            MemorySource::Manual,
            Some("project-a"),
        );

        let err = memory_lookup_payload(&store, memory.id.as_ref(), Some("project-b"))
            .err()
            .expect("cross-project lookup should fail");
        assert!(err.to_string().contains("memory not found"));
    }

    #[test]
    fn test_stats_payload_can_include_invalidated_memories() {
        let store = test_store();
        let memory = store_memory(
            &store,
            "decisions/api",
            "Use SQLite",
            0.8,
            MemorySource::Manual,
            Some("cap"),
        );
        store
            .invalidate(&memory.id, Some("obsolete"), None)
            .unwrap();

        let active_only = stats_payload(&store, Some("cap"), false).unwrap();
        let include_invalidated = stats_payload(&store, Some("cap"), true).unwrap();

        assert_eq!(active_only.total_memories, 0);
        assert_eq!(include_invalidated.total_memories, 1);
    }

    #[test]
    fn test_topic_memories_payload_respects_limit() {
        let store = test_store();
        for idx in 0..3 {
            store_memory(
                &store,
                "decisions/api",
                &format!("decision {idx}"),
                0.5 + idx as f32,
                MemorySource::Manual,
                None,
            );
        }

        let payload = topic_memories_payload(&store, "decisions/api", None, false, 2).unwrap();

        assert_eq!(payload.total, 3);
        assert_eq!(payload.memories.len(), 2);
    }
}
