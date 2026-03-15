use serde_json::Value;

use hyphae_core::{Concept, ConceptLink, Label, Memoir, MemoirStore, Relation};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{
    get_bounded_i64, get_str, resolve_memoir, validate_max_length, validate_required_string,
};

pub(crate) fn tool_memoir_create(store: &SqliteStore, args: &Value) -> ToolResult {
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let description = get_str(args, "description").unwrap_or("");

    let memoir = Memoir::new(name.into(), description.into());
    match store.create_memoir(memoir) {
        Ok(id) => ToolResult::text(format!("Created memoir '{name}': {id}")),
        Err(e) => ToolResult::error(format!("failed to create memoir: {e}")),
    }
}

pub(crate) fn tool_memoir_list(store: &SqliteStore) -> ToolResult {
    let memoirs = match store.list_memoirs() {
        Ok(m) => m,
        Err(e) => return ToolResult::error(format!("failed to list memoirs: {e}")),
    };

    if memoirs.is_empty() {
        return ToolResult::text("No memoirs yet.".into());
    }

    let mut output = String::from("Memoirs:\n");
    for m in &memoirs {
        let stats = store.memoir_stats(&m.id).ok();
        let concept_count = stats.map(|s| s.total_concepts).unwrap_or(0);
        output.push_str(&format!(
            "  {} ({} concepts) — {}\n",
            m.name, concept_count, m.description
        ));
    }
    ToolResult::text(output)
}

pub(crate) fn tool_memoir_show(store: &SqliteStore, args: &Value) -> ToolResult {
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };

    let memoir = match resolve_memoir(store, name) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let stats = match store.memoir_stats(&memoir.id) {
        Ok(s) => s,
        Err(e) => return ToolResult::error(format!("failed to get stats: {e}")),
    };
    let concepts = match store.list_concepts(&memoir.id) {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("failed to list concepts: {e}")),
    };

    let mut output = format!(
        "Memoir: {}\nDescription: {}\nConcepts: {}\nLinks: {}\nAvg confidence: {:.2}\n",
        memoir.name,
        memoir.description,
        stats.total_concepts,
        stats.total_links,
        stats.avg_confidence
    );

    if !stats.label_counts.is_empty() {
        output.push_str("Labels:\n");
        for (label, count) in &stats.label_counts {
            output.push_str(&format!("  {label} ({count})\n"));
        }
    }

    if !concepts.is_empty() {
        output.push_str("\nConcepts:\n");
        for c in &concepts {
            let labels_str = c
                .labels
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!(
                "  {} [r{} c{:.2}]{}\n    {}\n",
                c.name,
                c.revision,
                c.confidence.value(),
                if labels_str.is_empty() {
                    String::new()
                } else {
                    format!(" ({labels_str})")
                },
                c.definition
            ));
        }
    }

    ToolResult::text(output)
}

pub(crate) fn tool_memoir_add_concept(store: &SqliteStore, args: &Value) -> ToolResult {
    let memoir_name = match validate_required_string(args, "memoir") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let definition = match validate_required_string(args, "definition") {
        Ok(d) => d,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(definition, "definition", 32768) {
        return e;
    }

    let memoir = match resolve_memoir(store, memoir_name) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let mut concept = Concept::new(memoir.id, name.into(), definition.into());

    if let Some(labels_str) = get_str(args, "labels") {
        concept.labels = labels_str
            .split(',')
            .filter_map(|s| s.trim().parse::<Label>().ok())
            .collect();
    }

    match store.add_concept(concept) {
        Ok(id) => ToolResult::text(format!(
            "Added concept '{name}' to memoir '{memoir_name}': {id}"
        )),
        Err(e) => ToolResult::error(format!("failed to add concept: {e}")),
    }
}

pub(crate) fn tool_memoir_refine(store: &SqliteStore, args: &Value) -> ToolResult {
    let memoir_name = match validate_required_string(args, "memoir") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let definition = match validate_required_string(args, "definition") {
        Ok(d) => d,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(definition, "definition", 32768) {
        return e;
    }

    let memoir = match resolve_memoir(store, memoir_name) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let concept = match store.get_concept_by_name(&memoir.id, name) {
        Ok(Some(c)) => c,
        Ok(None) => return ToolResult::error(format!("concept not found: {name}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    if let Err(e) = store.refine_concept(&concept.id, definition, &[]) {
        return ToolResult::error(format!("failed to refine: {e}"));
    }

    let updated = match store.get_concept(&concept.id) {
        Ok(Some(c)) => c,
        _ => return ToolResult::text(format!("Refined concept '{name}'")),
    };

    ToolResult::text(format!(
        "Refined '{name}' (r{}, confidence={:.2})",
        updated.revision,
        updated.confidence.value()
    ))
}

pub(crate) fn tool_memoir_search(store: &SqliteStore, args: &Value) -> ToolResult {
    let memoir_name = match get_str(args, "memoir") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: memoir".into()),
    };
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 100) as usize;
    let label_str = get_str(args, "label");

    let memoir = match resolve_memoir(store, memoir_name) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let results = if let Some(lbl) = label_str {
        let parsed: Label = match lbl.parse() {
            Ok(l) => l,
            Err(e) => return ToolResult::error(format!("invalid label: {e}")),
        };
        let mut by_label = match store.search_concepts_by_label(&memoir.id, &parsed, limit) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        };
        if !query.is_empty() {
            let q = query.to_lowercase();
            by_label.retain(|c| {
                c.name.to_lowercase().contains(&q) || c.definition.to_lowercase().contains(&q)
            });
        }
        by_label
    } else {
        match store.search_concepts_fts(&memoir.id, query, limit) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
    };

    if results.is_empty() {
        return ToolResult::text("No concepts found.".into());
    }

    let mut output = String::new();
    for c in &results {
        let labels_str = c
            .labels
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!(
            "--- {} [r{} c{:.2}] ---\n  {}\n",
            c.name,
            c.revision,
            c.confidence.value(),
            c.definition
        ));
        if !labels_str.is_empty() {
            output.push_str(&format!("  labels: {labels_str}\n"));
        }
        output.push('\n');
    }

    ToolResult::text(output)
}

pub(crate) fn tool_memoir_search_all(store: &SqliteStore, args: &Value) -> ToolResult {
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 100) as usize;

    let results = match store.search_all_concepts_fts(query, limit) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("search error: {e}")),
    };

    if results.is_empty() {
        return ToolResult::text("No concepts found.".into());
    }

    // Group by memoir for readable output
    let memoirs: std::collections::HashMap<String, String> = match store.list_memoirs() {
        Ok(list) => list
            .into_iter()
            .map(|m| (m.id.to_string(), m.name))
            .collect(),
        Err(e) => {
            tracing::warn!("list_memoirs failed: {e}");
            std::collections::HashMap::new()
        }
    };

    let mut output = String::new();
    for c in &results {
        let memoir_name = memoirs
            .get(c.memoir_id.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("?");
        let labels_str = c
            .labels
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!(
            "--- {} ({}) [r{} c{:.2}] ---\n  {}\n",
            c.name,
            memoir_name,
            c.revision,
            c.confidence.value(),
            c.definition
        ));
        if !labels_str.is_empty() {
            output.push_str(&format!("  labels: {labels_str}\n"));
        }
        output.push('\n');
    }

    ToolResult::text(output)
}

pub(crate) fn tool_memoir_link(store: &SqliteStore, args: &Value) -> ToolResult {
    let memoir_name = match get_str(args, "memoir") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: memoir".into()),
    };
    let from_name = match get_str(args, "from") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: from".into()),
    };
    let to_name = match get_str(args, "to") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: to".into()),
    };
    let relation_str = match get_str(args, "relation") {
        Some(r) => r,
        None => return ToolResult::error("missing required field: relation".into()),
    };

    let relation: Relation = match relation_str.parse() {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("invalid relation: {e}")),
    };

    if from_name == to_name {
        return ToolResult::error("cannot link a concept to itself".to_string());
    }

    let memoir = match resolve_memoir(store, memoir_name) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let from = match store.get_concept_by_name(&memoir.id, from_name) {
        Ok(Some(c)) => c,
        Ok(None) => return ToolResult::error(format!("concept not found: {from_name}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };
    let to = match store.get_concept_by_name(&memoir.id, to_name) {
        Ok(Some(c)) => c,
        Ok(None) => return ToolResult::error(format!("concept not found: {to_name}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    let link = ConceptLink::new(from.id, to.id, relation);
    match store.add_link(link) {
        Ok(id) => ToolResult::text(format!(
            "Linked: {from_name} --{relation}--> {to_name} ({id})"
        )),
        Err(e) => ToolResult::error(format!("failed to link: {e}")),
    }
}

pub(crate) fn tool_memoir_inspect(store: &SqliteStore, args: &Value) -> ToolResult {
    let memoir_name = match get_str(args, "memoir") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: memoir".into()),
    };
    let name = match get_str(args, "name") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: name".into()),
    };
    let depth = get_bounded_i64(args, "depth", 2, 1, 10) as usize;

    let memoir = match resolve_memoir(store, memoir_name) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let concept = match store.get_concept_by_name(&memoir.id, name) {
        Ok(Some(c)) => c,
        Ok(None) => return ToolResult::error(format!("concept not found: {name}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    let labels_str = concept
        .labels
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let mut output = format!(
        "Concept: {}\n  id: {}\n  definition: {}\n  confidence: {:.2}\n  revision: {}\n",
        concept.name,
        concept.id,
        concept.definition,
        concept.confidence.value(),
        concept.revision
    );
    if !labels_str.is_empty() {
        output.push_str(&format!("  labels: {labels_str}\n"));
    }

    let (neighbors, links) = match store.get_neighborhood(&concept.id, depth) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("graph error: {e}")),
    };

    if links.is_empty() {
        output.push_str("\n(no links)\n");
    } else {
        output.push_str(&format!("\nGraph (depth={depth}):\n"));
        for link in &links {
            let src = neighbors
                .iter()
                .find(|c| c.id == link.source_id)
                .map(|c| c.name.as_str())
                .unwrap_or("?");
            let tgt = neighbors
                .iter()
                .find(|c| c.id == link.target_id)
                .map(|c| c.name.as_str())
                .unwrap_or("?");
            output.push_str(&format!("  {src} --{}--> {tgt}\n", link.relation));
        }
    }

    ToolResult::text(output)
}
