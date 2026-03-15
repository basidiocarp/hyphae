use anyhow::Result;
use clap::{Args, Subcommand};
use hyphae_core::{Concept, ConceptLink, Label, Memoir, MemoirStore, Relation};
use hyphae_store::SqliteStore;

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
    List,
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
        /// Restrict to this memoir
        #[arg(short, long)]
        memoir: Option<String>,
    },
    /// Inspect a memoir or one of its concepts
    Inspect {
        /// Memoir name
        memoir: String,
        /// Concept name (shows memoir overview if omitted)
        #[arg(short, long)]
        concept: Option<String>,
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
        MemoirCommand::List => cmd_memoir_list(store),
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
            memoir,
        } => cmd_memoir_search(store, query, limit, memoir),
        MemoirCommand::Inspect { memoir, concept } => cmd_memoir_inspect(store, memoir, concept),
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

pub(crate) fn cmd_memoir_list(store: &SqliteStore) -> Result<()> {
    let memoirs = store.list_memoirs()?;
    if memoirs.is_empty() {
        println!("No memoirs found");
    } else {
        for m in &memoirs {
            println!("{}: {}", m.name, m.description);
        }
        println!("\n{} memoir(s)", memoirs.len());
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
    memoir_name: Option<String>,
) -> Result<()> {
    let concepts = if let Some(name) = memoir_name {
        let memoir = store
            .get_memoir_by_name(&name)?
            .ok_or_else(|| anyhow::anyhow!("memoir not found: {name}"))?;
        store.search_concepts_fts(&memoir.id, &query, limit)?
    } else {
        store.search_all_concepts_fts(&query, limit)?
    };

    if concepts.is_empty() {
        println!("No concepts found");
    } else {
        for c in &concepts {
            println!(
                "{}: {}",
                c.name,
                crate::display::truncate(&c.definition, 80)
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_memoir_inspect(
    store: &SqliteStore,
    memoir_name: String,
    concept_name: Option<String>,
) -> Result<()> {
    let memoir = store
        .get_memoir_by_name(&memoir_name)?
        .ok_or_else(|| anyhow::anyhow!("memoir not found: {memoir_name}"))?;

    if let Some(cname) = concept_name {
        let concept = store
            .get_concept_by_name(&memoir.id, &cname)?
            .ok_or_else(|| anyhow::anyhow!("concept not found: {cname}"))?;
        println!("Concept:    {}", concept.name);
        println!("Definition: {}", concept.definition);
        println!("Confidence: {:.2}", concept.confidence.value());
        println!("Revision:   {}", concept.revision);
        if !concept.labels.is_empty() {
            let label_str = concept
                .labels
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("Labels:     {label_str}");
        }
        let links_from = store.get_links_from(&concept.id)?;
        let links_to = store.get_links_to(&concept.id)?;
        if !links_from.is_empty() || !links_to.is_empty() {
            println!("\nRelationships:");
            for link in &links_from {
                if let Some(n) = store.get_concept(&link.target_id)? {
                    println!("  --[{}]--> {}", link.relation, n.name);
                }
            }
            for link in &links_to {
                if let Some(n) = store.get_concept(&link.source_id)? {
                    println!("  <--[{}]-- {}", link.relation, n.name);
                }
            }
        }
    } else {
        let stats = store.memoir_stats(&memoir.id)?;
        println!("Memoir:      {}", memoir.name);
        println!("Description: {}", memoir.description);
        println!(
            "Created:     {}",
            memoir.created_at.format("%Y-%m-%d %H:%M")
        );
        println!("\nStats:");
        println!("  Concepts:       {}", stats.total_concepts);
        println!("  Links:          {}", stats.total_links);
        println!("  Avg confidence: {:.2}", stats.avg_confidence);
        if !stats.label_counts.is_empty() {
            println!("\nTop labels:");
            for (label, count) in stats.label_counts.iter().take(5) {
                println!("  {label}: {count}");
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
