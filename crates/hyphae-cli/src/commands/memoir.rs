use anyhow::Result;
use clap::{Args, Subcommand};
use hyphae_core::{Concept, ConceptLink, Label, Memoir, MemoirStore, Relation};
use hyphae_store::SqliteStore;
use serde::Serialize;

const SQLITE_LIMIT_MAX: usize = i64::MAX as usize;
const MEMOIR_LIST_SCHEMA_VERSION: &str = "1.0";
const MEMOIR_SHOW_SCHEMA_VERSION: &str = "1.0";
const MEMOIR_SEARCH_SCHEMA_VERSION: &str = "1.0";
const MEMOIR_SEARCH_ALL_SCHEMA_VERSION: &str = "1.0";
const MEMOIR_INSPECT_SCHEMA_VERSION: &str = "1.0";

#[derive(Args)]
pub(crate) struct MemoirArgs {
    #[command(subcommand)]
    pub(crate) cmd: MemoirCommand,
}

#[derive(Subcommand)]
pub(crate) enum MemoirCommand {
    /// Create a new memoir (knowledge graph)
    Create {
        /// Memoir name
        #[arg(short, long)]
        name: String,
        /// Memoir description
        #[arg(short, long, default_value = "")]
        description: String,
    },
    /// List all memoirs
    List {
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show a memoir's concepts and stats
    Show {
        /// Memoir name
        memoir: String,
        /// Optional query used to filter concepts within the memoir
        #[arg(short, long)]
        query: Option<String>,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
        /// Pagination offset
        #[arg(long, default_value = "0")]
        offset: usize,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Delete a memoir by name
    Delete {
        /// Memoir name
        name: String,
    },
    /// Add a concept to a memoir
    AddConcept {
        /// Memoir name
        #[arg(short, long)]
        memoir: String,
        /// Concept name
        #[arg(short, long)]
        name: String,
        /// Concept definition
        #[arg(short, long)]
        definition: String,
        /// Labels in namespace:value format (repeatable)
        #[arg(short, long)]
        label: Vec<String>,
    },
    /// Search concepts across memoirs (or within a specific memoir)
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
        /// Pagination offset
        #[arg(long, default_value = "0")]
        offset: usize,
        /// Restrict to this memoir
        #[arg(short, long)]
        memoir: Option<String>,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Search concepts across all memoirs
    SearchAll {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
        /// Pagination offset
        #[arg(long, default_value = "0")]
        offset: usize,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Inspect a memoir or one of its concepts
    Inspect {
        /// Memoir name
        memoir: String,
        /// Concept name (shows memoir overview if omitted)
        #[arg(short, long)]
        concept: Option<String>,
        /// BFS depth for graph exploration when a concept is selected
        #[arg(short = 'D', long, default_value = "1")]
        depth: usize,
        /// Emit structured JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Link two concepts within a memoir
    Link {
        /// Memoir name
        #[arg(short, long)]
        memoir: String,
        /// Source concept name
        #[arg(long)]
        from: String,
        /// Target concept name
        #[arg(long)]
        to: String,
        /// Relation type: part_of, depends_on, related_to, contradicts, refines,
        /// alternative_to, caused_by, instance_of, superseded_by
        #[arg(short, long, default_value = "related_to")]
        relation: String,
    },
}

pub(crate) fn dispatch(store: &SqliteStore, args: MemoirArgs) -> Result<()> {
    match args.cmd {
        MemoirCommand::Create { name, description } => cmd_memoir_create(store, name, description),
        MemoirCommand::List { json } => cmd_memoir_list(store, json),
        MemoirCommand::Show {
            memoir,
            query,
            limit,
            offset,
            json,
        } => cmd_memoir_show(store, memoir, query, limit, offset, json),
        MemoirCommand::Delete { name } => cmd_memoir_delete(store, name),
        MemoirCommand::AddConcept {
            memoir,
            name,
            definition,
            label,
        } => cmd_memoir_add_concept(store, memoir, name, definition, label),
        MemoirCommand::Search {
            query,
            limit,
            offset,
            memoir,
            json,
        } => cmd_memoir_search(store, query, limit, offset, memoir, json),
        MemoirCommand::SearchAll {
            query,
            limit,
            offset,
            json,
        } => cmd_memoir_search_all(store, query, limit, offset, json),
        MemoirCommand::Inspect {
            memoir,
            concept,
            depth,
            json,
        } => cmd_memoir_inspect(store, memoir, concept, depth, json),
        MemoirCommand::Link {
            memoir,
            from,
            to,
            relation,
        } => cmd_memoir_link(store, memoir, from, to, relation),
    }
}

pub(crate) fn cmd_memoir_create(
    store: &SqliteStore,
    name: String,
    description: String,
) -> Result<()> {
    let memoir = Memoir::new(name.clone(), description);
    store.create_memoir(memoir)?;
    println!("✓ Created memoir: {name}");
    Ok(())
}

pub(crate) fn cmd_memoir_list(store: &SqliteStore, json: bool) -> Result<()> {
    let payload = memoir_list_payload(store)?;
    if json {
        print_json_versioned(MEMOIR_LIST_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.memoirs.is_empty() {
        println!("No memoirs found");
    } else {
        for entry in &payload.memoirs {
            println!("{}: {}", entry.memoir.name, entry.memoir.description);
        }
        println!("\n{} memoir(s)", payload.memoirs.len());
    }
    Ok(())
}

pub(crate) fn cmd_memoir_show(
    store: &SqliteStore,
    memoir_name: String,
    query: Option<String>,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<()> {
    let payload = memoir_show_payload(store, &memoir_name, query, limit, offset)?;
    if json {
        print_json_versioned(MEMOIR_SHOW_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    let query_note = payload
        .query
        .as_deref()
        .map(|q| format!(" (query: {q})"))
        .unwrap_or_default();
    println!("Memoir:      {}", payload.memoir.name);
    println!("Description: {}", payload.memoir.description);
    println!(
        "Created:     {}",
        payload.memoir.created_at.format("%Y-%m-%d %H:%M")
    );
    println!("\nStats:");
    println!("  Concepts:       {}", payload.stats.total_concepts);
    println!("  Links:          {}", payload.stats.total_links);
    println!("  Avg confidence: {:.2}", payload.stats.avg_confidence);
    if !payload.stats.label_counts.is_empty() {
        println!("\nTop labels:");
        for label in payload.stats.label_counts.iter().take(5) {
            println!("  {}: {}", label.label, label.count);
        }
    }
    println!(
        "\nConcepts{} (showing {} of {}):",
        query_note,
        payload.concepts.len(),
        payload.total
    );
    for concept in &payload.concepts {
        println!(
            "{}: {}",
            concept.name,
            crate::display::truncate(&concept.definition, 80)
        );
    }
    Ok(())
}

pub(crate) fn cmd_memoir_delete(store: &SqliteStore, name: String) -> Result<()> {
    match store.get_memoir_by_name(&name)? {
        None => eprintln!("Memoir not found: {name}"),
        Some(m) => {
            store.delete_memoir(&m.id)?;
            println!("✓ Deleted memoir: {name}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_memoir_add_concept(
    store: &SqliteStore,
    memoir_name: String,
    name: String,
    definition: String,
    label_strs: Vec<String>,
) -> Result<()> {
    let memoir = store
        .get_memoir_by_name(&memoir_name)?
        .ok_or_else(|| anyhow::anyhow!("memoir not found: {memoir_name}"))?;

    let mut concept = Concept::new(memoir.id, name.clone(), definition);
    for ls in &label_strs {
        let label: Label = ls
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid label '{ls}': {e}"))?;
        concept.labels.push(label);
    }
    store.add_concept(concept)?;
    println!("✓ Added concept '{name}' to memoir '{memoir_name}'");
    Ok(())
}

pub(crate) fn cmd_memoir_search(
    store: &SqliteStore,
    query: String,
    limit: usize,
    offset: usize,
    memoir_name: Option<String>,
    json: bool,
) -> Result<()> {
    let payload = memoir_search_payload(store, query, limit, offset, memoir_name)?;
    if json {
        print_json_versioned(MEMOIR_SEARCH_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.results.is_empty() {
        println!("No concepts found");
    } else {
        for c in &payload.results {
            println!(
                "{}: {}",
                c.name,
                crate::display::truncate(&c.definition, 80)
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_memoir_search_all(
    store: &SqliteStore,
    query: String,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<()> {
    let payload = memoir_search_all_payload(store, query, limit, offset)?;
    if json {
        print_json_versioned(MEMOIR_SEARCH_ALL_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.results.is_empty() {
        println!("No concepts found");
    } else {
        for hit in &payload.results {
            println!(
                "{} / {}: {}",
                hit.memoir.name,
                hit.concept.name,
                crate::display::truncate(&hit.concept.definition, 80)
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_memoir_inspect(
    store: &SqliteStore,
    memoir_name: String,
    concept_name: Option<String>,
    depth: usize,
    json: bool,
) -> Result<()> {
    let memoir = store
        .get_memoir_by_name(&memoir_name)?
        .ok_or_else(|| anyhow::anyhow!("memoir not found: {memoir_name}"))?;

    if let Some(cname) = concept_name {
        let payload = memoir_inspect_payload(store, &memoir, &cname, depth)?;
        if json {
            print_json_versioned(MEMOIR_INSPECT_SCHEMA_VERSION, &payload)?;
            return Ok(());
        }

        println!("Concept:    {}", payload.concept.name);
        println!("Definition: {}", payload.concept.definition);
        println!("Confidence: {:.2}", payload.concept.confidence.value());
        println!("Revision:   {}", payload.concept.revision);
        if !payload.concept.labels.is_empty() {
            let label_str = payload
                .concept
                .labels
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("Labels:     {label_str}");
        }
        if !payload.neighborhood.links.is_empty() {
            println!("\nRelationships:");
            for link in &payload.neighborhood.links {
                if link.source_id == payload.concept.id {
                    if let Some(n) = payload
                        .neighborhood
                        .concepts
                        .iter()
                        .find(|concept| concept.id == link.target_id)
                    {
                        println!("  --[{}]--> {}", link.relation, n.name);
                    }
                } else if let Some(n) = payload
                    .neighborhood
                    .concepts
                    .iter()
                    .find(|concept| concept.id == link.source_id)
                {
                    println!("  <--[{}]-- {}", link.relation, n.name);
                }
            }
        }
    } else {
        let payload = memoir_show_payload(store, &memoir_name, None, usize::MAX, 0)?;
        if json {
            print_json_versioned(MEMOIR_SHOW_SCHEMA_VERSION, &payload)?;
            return Ok(());
        }

        println!("Memoir:      {}", payload.memoir.name);
        println!("Description: {}", payload.memoir.description);
        println!(
            "Created:     {}",
            payload.memoir.created_at.format("%Y-%m-%d %H:%M")
        );
        println!("\nStats:");
        println!("  Concepts:       {}", payload.stats.total_concepts);
        println!("  Links:          {}", payload.stats.total_links);
        println!("  Avg confidence: {:.2}", payload.stats.avg_confidence);
        if !payload.stats.label_counts.is_empty() {
            println!("\nTop labels:");
            for label in payload.stats.label_counts.iter().take(5) {
                println!("  {}: {}", label.label, label.count);
            }
        }
    }
    Ok(())
}

pub(crate) fn cmd_memoir_link(
    store: &SqliteStore,
    memoir_name: String,
    from_name: String,
    to_name: String,
    relation_str: String,
) -> Result<()> {
    let memoir = store
        .get_memoir_by_name(&memoir_name)?
        .ok_or_else(|| anyhow::anyhow!("memoir not found: {memoir_name}"))?;

    let from = store
        .get_concept_by_name(&memoir.id, &from_name)?
        .ok_or_else(|| anyhow::anyhow!("concept not found: {from_name}"))?;
    let to = store
        .get_concept_by_name(&memoir.id, &to_name)?
        .ok_or_else(|| anyhow::anyhow!("concept not found: {to_name}"))?;

    if from.id == to.id {
        anyhow::bail!("cannot link a concept to itself");
    }

    let relation: Relation = relation_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid relation: {e}"))?;
    let link = ConceptLink::new(from.id, to.id, relation);
    store.add_link(link)?;
    println!("✓ Linked '{from_name}' --[{relation}]--> '{to_name}'");
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct MemoirLabelCountPayload {
    label: String,
    count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirStatsPayload {
    total_concepts: usize,
    total_links: usize,
    avg_confidence: f32,
    label_counts: Vec<MemoirLabelCountPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirListEntryPayload {
    memoir: Memoir,
    concept_count: usize,
    link_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirListPayload {
    memoirs: Vec<MemoirListEntryPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirShowPayload {
    memoir: Memoir,
    stats: MemoirStatsPayload,
    query: Option<String>,
    limit: usize,
    offset: usize,
    total: usize,
    concepts: Vec<Concept>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirNeighborhoodPayload {
    concepts: Vec<Concept>,
    links: Vec<ConceptLink>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirInspectPayload {
    memoir: Memoir,
    concept: Concept,
    depth: usize,
    neighborhood: MemoirNeighborhoodPayload,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirSearchPayload {
    memoir: Option<Memoir>,
    query: String,
    limit: usize,
    offset: usize,
    total: usize,
    results: Vec<Concept>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirSearchAllHitPayload {
    memoir: Memoir,
    concept: Concept,
}

#[derive(Debug, Clone, Serialize)]
struct MemoirSearchAllPayload {
    query: String,
    limit: usize,
    offset: usize,
    total: usize,
    results: Vec<MemoirSearchAllHitPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct VersionedPayload<'a, T: Serialize> {
    schema_version: &'a str,
    #[serde(flatten)]
    payload: &'a T,
}

fn memoir_list_payload(store: &SqliteStore) -> Result<MemoirListPayload> {
    let memoirs = store
        .list_memoirs()?
        .into_iter()
        .map(|memoir| {
            let stats = store.memoir_stats(&memoir.id)?;
            Ok(MemoirListEntryPayload {
                memoir,
                concept_count: stats.total_concepts,
                link_count: stats.total_links,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(MemoirListPayload { memoirs })
}

fn memoir_stats_payload(
    store: &SqliteStore,
    memoir_id: &hyphae_core::MemoirId,
) -> Result<MemoirStatsPayload> {
    let stats = store.memoir_stats(memoir_id)?;
    Ok(MemoirStatsPayload {
        total_concepts: stats.total_concepts,
        total_links: stats.total_links,
        avg_confidence: stats.avg_confidence,
        label_counts: stats
            .label_counts
            .into_iter()
            .map(|(label, count)| MemoirLabelCountPayload { label, count })
            .collect(),
    })
}

fn memoir_show_payload(
    store: &SqliteStore,
    memoir_name: &str,
    query: Option<String>,
    limit: usize,
    offset: usize,
) -> Result<MemoirShowPayload> {
    let memoir = store
        .get_memoir_by_name(memoir_name)?
        .ok_or_else(|| anyhow::anyhow!("memoir not found: {memoir_name}"))?;

    let query = query.and_then(|query| {
        let trimmed = query.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });

    let all_concepts = if let Some(ref query) = query {
        store.search_concepts_fts(&memoir.id, query, SQLITE_LIMIT_MAX)?
    } else {
        store.list_concepts(&memoir.id)?
    };
    let total = all_concepts.len();
    let concepts = all_concepts.into_iter().skip(offset).take(limit).collect();

    Ok(MemoirShowPayload {
        stats: memoir_stats_payload(store, &memoir.id)?,
        memoir,
        query,
        limit,
        offset,
        total,
        concepts,
    })
}

fn memoir_inspect_payload(
    store: &SqliteStore,
    memoir: &Memoir,
    concept_name: &str,
    depth: usize,
) -> Result<MemoirInspectPayload> {
    let concept = store
        .get_concept_by_name(&memoir.id, concept_name)?
        .ok_or_else(|| anyhow::anyhow!("concept not found: {concept_name}"))?;
    let (concepts, links) = store.get_neighborhood(&concept.id, depth)?;

    Ok(MemoirInspectPayload {
        memoir: memoir.clone(),
        concept,
        depth,
        neighborhood: MemoirNeighborhoodPayload { concepts, links },
    })
}

fn memoir_search_payload(
    store: &SqliteStore,
    query: String,
    limit: usize,
    offset: usize,
    memoir_name: Option<String>,
) -> Result<MemoirSearchPayload> {
    let query = query.trim().to_string();

    let (memoir, all_results) = if let Some(name) = memoir_name {
        let memoir = store
            .get_memoir_by_name(&name)?
            .ok_or_else(|| anyhow::anyhow!("memoir not found: {name}"))?;
        let results = store.search_concepts_fts(&memoir.id, &query, SQLITE_LIMIT_MAX)?;
        (Some(memoir), results)
    } else {
        (
            None,
            store.search_all_concepts_fts(&query, SQLITE_LIMIT_MAX)?,
        )
    };

    let total = all_results.len();
    let results = all_results.into_iter().skip(offset).take(limit).collect();

    Ok(MemoirSearchPayload {
        memoir,
        query,
        limit,
        offset,
        total,
        results,
    })
}

fn memoir_search_all_payload(
    store: &SqliteStore,
    query: String,
    limit: usize,
    offset: usize,
) -> Result<MemoirSearchAllPayload> {
    let query = query.trim().to_string();
    let all_results = store.search_all_concepts_fts(&query, SQLITE_LIMIT_MAX)?;
    let total = all_results.len();
    let results = all_results
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|concept| {
            let memoir = store
                .get_memoir(&concept.memoir_id)?
                .ok_or_else(|| anyhow::anyhow!("memoir not found: {}", concept.memoir_id))?;
            Ok(MemoirSearchAllHitPayload { memoir, concept })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(MemoirSearchAllPayload {
        query,
        limit,
        offset,
        total,
        results,
    })
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
    use hyphae_core::{Concept, ConceptLink, Confidence, Memoir, Relation};
    use serde_json::Value;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn make_memoir(store: &SqliteStore, name: &str) -> Memoir {
        let memoir = Memoir::new(name.to_string(), format!("{name} description"));
        store.create_memoir(memoir.clone()).unwrap();
        memoir
    }

    fn make_concept(
        memoir_id: &hyphae_core::MemoirId,
        name: &str,
        definition: &str,
        confidence: f32,
    ) -> Concept {
        let mut concept = Concept::new(memoir_id.clone(), name.to_string(), definition.to_string());
        concept.confidence = Confidence::new_clamped(confidence);
        concept
    }

    fn concept_name(payload: &Value, index: usize) -> &str {
        payload["memoirs"][index]["memoir"]["name"]
            .as_str()
            .expect("memoir name")
    }

    #[test]
    fn test_memoir_list_payload_includes_counts() {
        let store = test_store();
        let alpha = make_memoir(&store, "Alpha");
        let beta = make_memoir(&store, "Beta");

        store
            .add_concept(make_concept(&alpha.id, "A1", "alpha concept", 0.9))
            .unwrap();
        store
            .add_concept(make_concept(&alpha.id, "A2", "alpha concept", 0.8))
            .unwrap();
        store
            .add_concept(make_concept(&beta.id, "B1", "beta concept", 0.7))
            .unwrap();

        let payload = memoir_list_payload(&store).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["memoirs"].as_array().unwrap().len(), 2);
        assert_eq!(concept_name(&value, 0), "Alpha");
        assert_eq!(value["memoirs"][0]["concept_count"].as_u64(), Some(2));
        assert_eq!(value["memoirs"][1]["concept_count"].as_u64(), Some(1));
    }

    #[test]
    fn test_memoir_show_payload_applies_query_and_offset() {
        let store = test_store();
        let memoir = make_memoir(&store, "Backend");

        store
            .add_concept(make_concept(&memoir.id, "Alpha", "shared token first", 0.9))
            .unwrap();
        store
            .add_concept(make_concept(&memoir.id, "Beta", "shared token second", 0.8))
            .unwrap();
        store
            .add_concept(make_concept(&memoir.id, "Gamma", "unrelated text", 0.7))
            .unwrap();

        let payload =
            memoir_show_payload(&store, "Backend", Some("shared".to_string()), 1, 1).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["memoir"]["name"].as_str(), Some("Backend"));
        assert_eq!(value["query"].as_str(), Some("shared"));
        assert_eq!(value["limit"].as_u64(), Some(1));
        assert_eq!(value["offset"].as_u64(), Some(1));
        assert_eq!(value["total"].as_u64(), Some(2));
        assert_eq!(value["concepts"].as_array().unwrap().len(), 1);
        assert_eq!(value["concepts"][0]["name"].as_str(), Some("Beta"));
    }

    #[test]
    fn test_memoir_search_payload_supports_offset() {
        let store = test_store();
        let memoir = make_memoir(&store, "Backend");

        store
            .add_concept(make_concept(&memoir.id, "Alpha", "shared token first", 0.9))
            .unwrap();
        store
            .add_concept(make_concept(&memoir.id, "Beta", "shared token second", 0.8))
            .unwrap();

        let payload = memoir_search_payload(
            &store,
            "shared".to_string(),
            1,
            1,
            Some("Backend".to_string()),
        )
        .unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["memoir"]["name"].as_str(), Some("Backend"));
        assert_eq!(value["results"].as_array().unwrap().len(), 1);
        assert_eq!(value["results"][0]["name"].as_str(), Some("Beta"));
    }

    #[test]
    fn test_memoir_search_all_payload_includes_memoir_names() {
        let store = test_store();
        let alpha = make_memoir(&store, "Alpha");
        let beta = make_memoir(&store, "Beta");

        store
            .add_concept(make_concept(
                &alpha.id,
                "AlphaConcept",
                "shared token first",
                0.9,
            ))
            .unwrap();
        store
            .add_concept(make_concept(
                &beta.id,
                "BetaConcept",
                "shared token second",
                0.8,
            ))
            .unwrap();

        let payload = memoir_search_all_payload(&store, "shared".to_string(), 1, 1).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["query"].as_str(), Some("shared"));
        assert_eq!(value["limit"].as_u64(), Some(1));
        assert_eq!(value["offset"].as_u64(), Some(1));
        assert_eq!(value["total"].as_u64(), Some(2));
        assert_eq!(value["results"].as_array().unwrap().len(), 1);
        assert_eq!(value["results"][0]["memoir"]["name"].as_str(), Some("Beta"));
        assert_eq!(
            value["results"][0]["concept"]["name"].as_str(),
            Some("BetaConcept")
        );
    }

    #[test]
    fn test_memoir_inspect_payload_honors_depth() {
        let store = test_store();
        let memoir = make_memoir(&store, "Backend");

        let root = make_concept(&memoir.id, "Root", "root concept", 0.9);
        let root_id = root.id.clone();
        store.add_concept(root).unwrap();

        let child = make_concept(&memoir.id, "Child", "child concept", 0.8);
        let child_id = child.id.clone();
        store.add_concept(child).unwrap();

        let grandchild = make_concept(&memoir.id, "Grandchild", "grandchild concept", 0.7);
        let grandchild_id = grandchild.id.clone();
        store.add_concept(grandchild).unwrap();

        store
            .add_link(ConceptLink::new(
                root_id.clone(),
                child_id.clone(),
                Relation::RelatedTo,
            ))
            .unwrap();
        store
            .add_link(ConceptLink::new(
                child_id,
                grandchild_id,
                Relation::RelatedTo,
            ))
            .unwrap();

        let payload = memoir_inspect_payload(&store, &memoir, "Root", 1).unwrap();
        let value = serde_json::to_value(&payload).unwrap();
        let concept_names = value["neighborhood"]["concepts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|concept| concept["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(value["memoir"]["name"].as_str(), Some("Backend"));
        assert_eq!(value["depth"].as_u64(), Some(1));
        assert_eq!(
            value["neighborhood"]["concepts"].as_array().unwrap().len(),
            2
        );
        assert!(concept_names.contains(&"Root"));
        assert!(concept_names.contains(&"Child"));
        assert!(!concept_names.contains(&"Grandchild"));
    }
}
