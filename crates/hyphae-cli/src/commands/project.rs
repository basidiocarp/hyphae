use anyhow::Result;
use clap::{Args, Subcommand};

use hyphae_core::{MemoryId, MemoryStore};
use hyphae_store::SqliteStore;

#[derive(Args)]
pub(crate) struct ProjectArgs {
    #[command(subcommand)]
    pub(crate) command: ProjectCommand,
}

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// List all projects with memory counts
    List,

    /// Link two projects for cross-project search
    Link {
        /// Source project name
        source: String,
        /// Target project name
        target: String,
    },

    /// Search memories across all projects
    Search {
        /// Query text
        query: String,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Promote a memory to the _shared knowledge pool
    Share {
        /// Memory ID to share globally
        id: String,
    },
}

pub(crate) fn dispatch(store: &SqliteStore, args: ProjectArgs) -> Result<()> {
    match args.command {
        ProjectCommand::List => cmd_list(store),
        ProjectCommand::Link { source, target } => cmd_link(store, &source, &target),
        ProjectCommand::Search { query, limit } => cmd_search(store, &query, limit),
        ProjectCommand::Share { id } => cmd_share(store, &id),
    }
}

fn cmd_list(store: &SqliteStore) -> Result<()> {
    let projects = store.list_projects()?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!("Projects:");
    for (name, count) in &projects {
        let linked = store.get_linked_projects(name).unwrap_or_default();
        if linked.is_empty() {
            println!("  {name}: {count} memories");
        } else {
            println!("  {name}: {count} memories (linked: {})", linked.join(", "));
        }
    }

    Ok(())
}

fn cmd_link(store: &SqliteStore, source: &str, target: &str) -> Result<()> {
    store.link_projects(source, target)?;
    println!("Linked projects: {source} <-> {target}");
    Ok(())
}

fn cmd_search(store: &SqliteStore, query: &str, limit: usize) -> Result<()> {
    let results = store.search_all_projects(query, limit)?;

    if results.is_empty() {
        println!("No memories found across any project.");
        return Ok(());
    }

    for mem in &results {
        let project_name = mem.project.as_deref().unwrap_or("(none)");
        println!(
            "[{}] [{}] [{}] {}",
            project_name, mem.importance, mem.topic, mem.summary
        );
    }

    Ok(())
}

fn cmd_share(store: &SqliteStore, id: &str) -> Result<()> {
    let memory_id = MemoryId::from(id);

    // Verify the memory exists
    let original = store
        .get(&memory_id)?
        .ok_or_else(|| anyhow::anyhow!("memory not found: {id}"))?;

    let new_id = store.promote_to_shared(&memory_id)?;
    println!(
        "Shared memory to _shared pool: {} -> {} (topic: {})",
        id, new_id, original.topic
    );

    Ok(())
}
