use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde_json::{Value, json};
use spore::logging::workflow_span;

use hyphae_core::{
    Concept, ConceptLink, Label, Memoir, MemoirStore, Relation,
    memoir_store::{ConceptInput, LinkInput},
};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{
    ToolTraceContext, get_bounded_i64, get_str, resolve_memoir, validate_max_length,
    validate_required_string, workflow_span_context,
};

// ─────────────────────────────────────────────────────────────────────────────
// Gap 7: Memoir staleness detection
// ─────────────────────────────────────────────────────────────────────────────

const MEMOIR_STALE_DAYS: i64 = 7;
const CODE_GRAPH_SCHEMA_VERSION: &str = "1.0";

pub(crate) fn tool_memoir_create(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let description = get_str(args, "description").unwrap_or("");
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memoir_create", &workflow_context).entered();

    let memoir = Memoir::new(name.into(), description.into());
    match store.create_memoir(memoir) {
        Ok(id) => ToolResult::text(format!("Created memoir '{name}': {id}")),
        Err(e) => ToolResult::error(format!("failed to create memoir: {e}")),
    }
}

pub(crate) fn tool_memoir_list(store: &SqliteStore, trace: &ToolTraceContext) -> ToolResult {
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memoir_list", &workflow_context).entered();
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

pub(crate) fn tool_memoir_show(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let name = match validate_required_string(args, "name") {
        Ok(n) => n,
        Err(e) => return e,
    };
    let workflow_context = workflow_span_context(trace, None, Some(name));
    let _workflow_span = workflow_span("memoir_show", &workflow_context).entered();

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

    // Gap 7: staleness warning
    let days_since_update = (Utc::now() - memoir.updated_at).num_days();
    if days_since_update >= MEMOIR_STALE_DAYS {
        output.push_str(&format!(
            "\n⚠ This memoir was last updated {days_since_update} days ago. \
             Concepts may be outdated. Consider running hyphae_import_code_graph \
             or hyphae_memoir_refine to refresh.\n"
        ));
    }

    ToolResult::text(output)
}

pub(crate) fn tool_memoir_add_concept(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
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
    let workflow_context = workflow_span_context(trace, None, Some(name));
    let _workflow_span = workflow_span("memoir_add_concept", &workflow_context).entered();

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

pub(crate) fn tool_memoir_refine(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
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
    let workflow_context = workflow_span_context(trace, None, Some(name));
    let _workflow_span = workflow_span("memoir_refine", &workflow_context).entered();

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

pub(crate) fn tool_memoir_search(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let memoir_name = match get_str(args, "memoir") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: memoir".into()),
    };
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let workflow_context = workflow_span_context(trace, None, Some(memoir_name));
    let _workflow_span = workflow_span("memoir_search", &workflow_context).entered();
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

pub(crate) fn tool_memoir_search_all(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memoir_search_all", &workflow_context).entered();
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

pub(crate) fn tool_memoir_link(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
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
    let workflow_context = workflow_span_context(trace, None, Some(memoir_name));
    let _workflow_span = workflow_span("memoir_link", &workflow_context).entered();

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

pub(crate) fn tool_memoir_inspect(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let memoir_name = match get_str(args, "memoir") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: memoir".into()),
    };
    let name = match get_str(args, "name") {
        Some(n) => n,
        None => return ToolResult::error("missing required field: name".into()),
    };
    let workflow_context = workflow_span_context(trace, None, Some(memoir_name));
    let _workflow_span = workflow_span("memoir_inspect", &workflow_context).entered();
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

pub(crate) fn tool_import_code_graph(
    store: &SqliteStore,
    args: &Value,
    compact: bool,
    _project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    match args.get("schema_version").and_then(|value| value.as_str()) {
        Some(CODE_GRAPH_SCHEMA_VERSION) => {}
        Some(version) => {
            return ToolResult::error(format!(
                "unsupported code graph schema_version: {version} (expected {CODE_GRAPH_SCHEMA_VERSION})"
            ));
        }
        None => {
            return ToolResult::error("missing required field: schema_version".to_string());
        }
    }

    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let workflow_context = workflow_span_context(trace, None, Some(project));
    let _workflow_span = workflow_span("import_code_graph", &workflow_context).entered();

    let nodes_raw = match args.get("nodes").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return ToolResult::error("missing required field: nodes".into()),
    };

    let edges_raw = match args.get("edges").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return ToolResult::error("missing required field: edges".into()),
    };

    // Default prune to true
    let prune = args.get("prune").and_then(|v| v.as_bool()).unwrap_or(true);

    // Parse nodes into ConceptInput
    let mut concept_inputs: Vec<ConceptInput> = Vec::with_capacity(nodes_raw.len());
    let mut node_names: HashSet<String> = HashSet::with_capacity(nodes_raw.len());

    for (i, node) in nodes_raw.iter().enumerate() {
        let name = match node.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.trim().is_empty() => n.to_string(),
            Some(_) => {
                return ToolResult::error(format!("nodes[{i}]: name must not be empty"));
            }
            None => {
                return ToolResult::error(format!("nodes[{i}]: missing field 'name'"));
            }
        };

        let description = node
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Parse labels array: each element is a plain string → Label { namespace: "code", value: ... }
        let labels: Vec<Label> =
            if let Some(labels_arr) = node.get("labels").and_then(|v| v.as_array()) {
                let mut parsed = Vec::with_capacity(labels_arr.len());
                for lv in labels_arr {
                    if let Some(s) = lv.as_str() {
                        if !s.is_empty() {
                            parsed.push(Label {
                                namespace: "code".to_string(),
                                value: s.to_string(),
                            });
                        }
                    }
                }
                parsed
            } else {
                Vec::new()
            };

        node_names.insert(name.clone());
        concept_inputs.push(ConceptInput {
            name,
            labels,
            description,
        });
    }

    // Parse edges into LinkInput
    let mut link_inputs: Vec<LinkInput> = Vec::with_capacity(edges_raw.len());

    for (i, edge) in edges_raw.iter().enumerate() {
        let source = match edge.get("source").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            Some(_) => {
                return ToolResult::error(format!("edges[{i}]: source must not be empty"));
            }
            None => {
                return ToolResult::error(format!("edges[{i}]: missing field 'source'"));
            }
        };
        let target = match edge.get("target").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            Some(_) => {
                return ToolResult::error(format!("edges[{i}]: target must not be empty"));
            }
            None => {
                return ToolResult::error(format!("edges[{i}]: missing field 'target'"));
            }
        };
        let relation = edge
            .get("relation")
            .and_then(|v| v.as_str())
            .unwrap_or("related_to")
            .to_string();
        let weight = edge
            .get("weight")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(1.0);

        link_inputs.push(LinkInput {
            source_name: source,
            target_name: target,
            relation,
            weight,
        });
    }

    // Validate: all edge source/target names must appear in nodes
    for (i, link) in link_inputs.iter().enumerate() {
        if !node_names.contains(&link.source_name) {
            return ToolResult::error(format!(
                "edges[{i}]: source '{}' not found in nodes",
                link.source_name
            ));
        }
        if !node_names.contains(&link.target_name) {
            return ToolResult::error(format!(
                "edges[{i}]: target '{}' not found in nodes",
                link.target_name
            ));
        }
    }

    // Find or create memoir code:{project}
    let memoir_name = format!("code:{project}");
    let memoir = match store.get_memoir_by_name(&memoir_name) {
        Ok(Some(m)) => m,
        Ok(None) => {
            let new_memoir = Memoir::new(
                memoir_name.clone(),
                format!("Code symbol graph for project '{project}'"),
            );
            match store.create_memoir(new_memoir) {
                Ok(id) => match store.get_memoir(&id) {
                    Ok(Some(m)) => m,
                    Ok(None) => {
                        return ToolResult::error(
                            "failed to retrieve memoir after creation".into(),
                        );
                    }
                    Err(e) => return ToolResult::error(format!("db error after create: {e}")),
                },
                Err(e) => return ToolResult::error(format!("failed to create memoir: {e}")),
            }
        }
        Err(e) => return ToolResult::error(format!("db error looking up memoir: {e}")),
    };

    // Upsert concepts
    let concept_report = match store.upsert_concepts(&memoir.id, &concept_inputs) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("failed to upsert concepts: {e}")),
    };

    // Upsert links
    let link_report = match store.upsert_links(&memoir.id, &link_inputs) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("failed to upsert links: {e}")),
    };

    // Optionally prune concepts not in this graph
    let pruned = if prune {
        let keep_names: Vec<String> = concept_inputs.iter().map(|c| c.name.clone()).collect();
        match store.prune_concepts(&memoir.id, &keep_names) {
            Ok(n) => n,
            Err(e) => return ToolResult::error(format!("failed to prune concepts: {e}")),
        }
    } else {
        0
    };

    tracing::info!(
        memoir = memoir_name,
        concepts_created = concept_report.created,
        concepts_updated = concept_report.updated,
        concepts_unchanged = concept_report.unchanged,
        concepts_pruned = pruned,
        links_created = link_report.created,
        links_updated = link_report.updated,
        links_unchanged = link_report.unchanged,
        "import_code_graph complete"
    );

    if compact {
        let text = format!(
            "Imported {memoir_name}: concepts +{}/{}/{} pruned={pruned} links +{}/{}/{}",
            concept_report.created,
            concept_report.updated,
            concept_report.unchanged,
            link_report.created,
            link_report.updated,
            link_report.unchanged,
        );
        return ToolResult::text(text);
    }

    let result = json!({
        "memoir": memoir_name,
        "concepts": {
            "created": concept_report.created,
            "updated": concept_report.updated,
            "unchanged": concept_report.unchanged,
            "pruned": pruned
        },
        "links": {
            "created": link_report.created,
            "updated": link_report.updated,
            "unchanged": link_report.unchanged
        }
    });

    ToolResult::text(result.to_string())
}

pub(crate) fn tool_code_query(
    store: &SqliteStore,
    args: &Value,
    compact: bool,
    _project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };

    let query_type = match validate_required_string(args, "query_type") {
        Ok(qt) => qt,
        Err(e) => return e,
    };
    let workflow_context = workflow_span_context(trace, None, Some(&project));
    let _workflow_span = workflow_span("code_query", &workflow_context).entered();

    // Look up memoir by name
    let memoir_name = format!("code:{project}");
    let memoir = match store.get_memoir_by_name(&memoir_name) {
        Ok(Some(m)) => m,
        Ok(None) => return ToolResult::error(format!("memoir not found: {memoir_name}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    // For symbol-based queries, resolve the concept
    let concept_opt = if matches!(
        query_type,
        "callers" | "callees" | "implementors" | "structure"
    ) {
        let symbol = match validate_required_string(args, "symbol") {
            Ok(s) => s,
            Err(e) => return e,
        };
        match store.get_concept_by_name(&memoir.id, symbol) {
            Ok(Some(c)) => Some(c),
            Ok(None) => return ToolResult::error(format!("concept not found: {symbol}")),
            Err(e) => return ToolResult::error(format!("db error: {e}")),
        }
    } else {
        None
    };

    let result = match query_type {
        "symbols" => query_symbols(store, &memoir, args),
        "callers" => match concept_opt {
            Some(c) => query_callers(store, &c),
            None => {
                return ToolResult::error(
                    "missing required symbol for query_type 'callers'".into(),
                );
            }
        },
        "callees" => match concept_opt {
            Some(c) => query_callees(store, &c),
            None => {
                return ToolResult::error(
                    "missing required symbol for query_type 'callees'".into(),
                );
            }
        },
        "implementors" => match concept_opt {
            Some(c) => query_implementors(store, &c),
            None => {
                return ToolResult::error(
                    "missing required symbol for query_type 'implementors'".into(),
                );
            }
        },
        "structure" => match concept_opt {
            Some(c) => query_structure(store, &c),
            None => {
                return ToolResult::error(
                    "missing required symbol for query_type 'structure'".into(),
                );
            }
        },
        _ => {
            return ToolResult::error(format!(
                "invalid query_type: {query_type}. Must be one of: symbols, callers, callees, implementors, structure"
            ));
        }
    };

    match result {
        Ok(data) => {
            if compact {
                let text = match query_type {
                    "symbols" => {
                        if let Value::Array(concepts) = &data {
                            let names: Vec<&str> = concepts
                                .iter()
                                .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                                .collect();
                            format!("{} symbols: {}", names.len(), names.join(", "))
                        } else {
                            "0 symbols".to_string()
                        }
                    }
                    _ => {
                        // For graph queries, show count and key relationships
                        if let Value::Object(obj) = &data {
                            let concepts = obj
                                .get("concepts")
                                .and_then(|c| c.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            let links = obj
                                .get("links")
                                .and_then(|l| l.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            format!("Graph: {concepts} concepts, {links} links")
                        } else {
                            "Graph query".to_string()
                        }
                    }
                };
                ToolResult::text(text)
            } else {
                ToolResult::text(data.to_string())
            }
        }
        Err(e) => ToolResult::error(e),
    }
}

fn query_symbols(store: &SqliteStore, memoir: &Memoir, args: &Value) -> Result<Value, String> {
    let concepts = if let Some(labels_arr) = args.get("labels").and_then(|v| v.as_array()) {
        if labels_arr.is_empty() {
            store
                .list_concepts(&memoir.id)
                .map_err(|e| format!("db error: {e}"))
        } else {
            // Filter by each label, return intersection
            let mut results: Option<Vec<Concept>> = None;

            for label_val in labels_arr {
                if let Some(label_str) = label_val.as_str() {
                    let label = Label {
                        namespace: "code".to_string(),
                        value: label_str.to_string(),
                    };
                    let labeled = store
                        .search_concepts_by_label(&memoir.id, &label, 1000)
                        .map_err(|e| format!("db error: {e}"))?;

                    if let Some(ref mut current) = results {
                        let labeled_ids: HashSet<_> = labeled.iter().map(|c| &c.id).collect();
                        current.retain(|c| labeled_ids.contains(&c.id));
                    } else {
                        results = Some(labeled);
                    }
                }
            }

            Ok(results.unwrap_or_default())
        }
    } else {
        store
            .list_concepts(&memoir.id)
            .map_err(|e| format!("db error: {e}"))
    }?;

    let json_concepts: Vec<Value> = concepts
        .iter()
        .map(|c| {
            let labels_strs: Vec<String> = c.labels.iter().map(|l| l.to_string()).collect();
            json!({
                "name": c.name,
                "definition": c.definition,
                "labels": labels_strs,
                "confidence": c.confidence.value(),
            })
        })
        .collect();

    Ok(Value::Array(json_concepts))
}

fn matches_call_relation(relation: &Relation) -> bool {
    // Code graphs use "calls", "call", etc., which parse as RelatedTo when unknown.
    // Also match against depends_on which is a known variant.
    matches!(relation, Relation::RelatedTo | Relation::DependsOn)
}

fn matches_implements_relation(relation: &Relation) -> bool {
    // Code graphs use "implements", which parses as RelatedTo when unknown.
    matches!(relation, Relation::RelatedTo | Relation::InstanceOf)
}

fn query_callers(store: &SqliteStore, concept: &Concept) -> Result<Value, String> {
    let links = store
        .get_links_to(&concept.id)
        .map_err(|e| format!("db error: {e}"))?;

    // For callers: filter for call-like relations
    let calls_links: Vec<&ConceptLink> = links
        .iter()
        .filter(|link| matches_call_relation(&link.relation))
        .collect();

    let mut callers = Vec::new();
    for link in calls_links {
        if let Ok(Some(caller)) = store.get_concept(&link.source_id) {
            callers.push(json!({
                "name": caller.name,
                "definition": caller.definition,
                "weight": link.weight.value(),
            }));
        }
    }

    Ok(json!({
        "symbol": concept.name,
        "callers": callers,
    }))
}

fn query_callees(store: &SqliteStore, concept: &Concept) -> Result<Value, String> {
    let links = store
        .get_links_from(&concept.id)
        .map_err(|e| format!("db error: {e}"))?;

    // For callees: filter for call-like relations
    let calls_links: Vec<&ConceptLink> = links
        .iter()
        .filter(|link| matches_call_relation(&link.relation))
        .collect();

    let mut callees = Vec::new();
    for link in calls_links {
        if let Ok(Some(callee)) = store.get_concept(&link.target_id) {
            callees.push(json!({
                "name": callee.name,
                "definition": callee.definition,
                "weight": link.weight.value(),
            }));
        }
    }

    Ok(json!({
        "symbol": concept.name,
        "callees": callees,
    }))
}

fn query_implementors(store: &SqliteStore, concept: &Concept) -> Result<Value, String> {
    let links = store
        .get_links_to(&concept.id)
        .map_err(|e| format!("db error: {e}"))?;

    // For implementors: filter for implementation-like relations
    let implements_links: Vec<&ConceptLink> = links
        .iter()
        .filter(|link| matches_implements_relation(&link.relation))
        .collect();

    let mut implementors = Vec::new();
    for link in implements_links {
        if let Ok(Some(impl_concept)) = store.get_concept(&link.source_id) {
            implementors.push(json!({
                "name": impl_concept.name,
                "definition": impl_concept.definition,
                "weight": link.weight.value(),
            }));
        }
    }

    Ok(json!({
        "symbol": concept.name,
        "implementors": implementors,
    }))
}

fn query_structure(store: &SqliteStore, concept: &Concept) -> Result<Value, String> {
    let (neighbors, links) = store
        .get_neighborhood(&concept.id, 2)
        .map_err(|e| format!("db error: {e}"))?;

    let concepts_json: Vec<Value> = neighbors
        .iter()
        .map(|c| {
            let labels_strs: Vec<String> = c.labels.iter().map(|l| l.to_string()).collect();
            json!({
                "id": c.id.to_string(),
                "name": c.name,
                "definition": c.definition,
                "labels": labels_strs,
                "confidence": c.confidence.value(),
            })
        })
        .collect();

    // Build a map for name lookup
    let id_to_name: HashMap<String, String> = neighbors
        .iter()
        .map(|c| (c.id.to_string(), c.name.clone()))
        .collect();

    let links_json: Vec<Value> = links
        .iter()
        .map(|link| {
            let source_name = id_to_name
                .get(link.source_id.as_ref())
                .map(|s| s.as_str())
                .unwrap_or("?");
            let target_name = id_to_name
                .get(link.target_id.as_ref())
                .map(|s| s.as_str())
                .unwrap_or("?");
            json!({
                "source": source_name,
                "target": target_name,
                "relation": link.relation.to_string(),
                "weight": link.weight.value(),
            })
        })
        .collect();

    Ok(json!({
        "symbol": concept.name,
        "concepts": concepts_json,
        "links": links_json,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_import_code_graph_creates_memoir_and_concepts() {
        let store = test_store();
        let args = json!({
            "schema_version": "1.0",
            "project": "mycelium",
            "nodes": [
                { "name": "dispatch", "labels": ["function", "public"], "description": "pub fn dispatch(...)" },
                { "name": "run_fallback", "labels": ["function"], "description": "fn run_fallback(...)" },
                { "name": "filter_output", "labels": ["function"], "description": "fn filter_output(...)" }
            ],
            "edges": [
                { "source": "dispatch", "target": "run_fallback", "relation": "depends_on", "weight": 0.8 },
                { "source": "dispatch", "target": "filter_output", "relation": "depends_on", "weight": 0.5 }
            ],
            "prune": false
        });

        let result =
            tool_import_code_graph(&store, &args, false, None, &ToolTraceContext::default());
        assert!(
            !result.is_error,
            "Expected success, got: {:?}",
            result.content
        );

        // Verify memoir was created
        let memoir = store
            .get_memoir_by_name("code:mycelium")
            .unwrap()
            .expect("memoir should exist");

        // Verify concepts were created
        let concepts = store.list_concepts(&memoir.id).unwrap();
        assert_eq!(concepts.len(), 3);

        let dispatch = store
            .get_concept_by_name(&memoir.id, "dispatch")
            .unwrap()
            .expect("dispatch concept should exist");
        assert_eq!(dispatch.definition, "pub fn dispatch(...)");
        assert_eq!(dispatch.labels.len(), 2);
        assert!(
            dispatch
                .labels
                .iter()
                .any(|l| l.namespace == "code" && l.value == "function")
        );
        assert!(
            dispatch
                .labels
                .iter()
                .any(|l| l.namespace == "code" && l.value == "public")
        );

        // Verify output is valid JSON with expected structure
        let output_text = &result.content[0].text;
        let output: serde_json::Value = serde_json::from_str(output_text).unwrap();
        assert_eq!(output["memoir"], "code:mycelium");
        assert_eq!(output["concepts"]["created"], 3);
        assert_eq!(output["concepts"]["updated"], 0);
        assert_eq!(output["links"]["created"], 2);
    }

    #[test]
    fn test_import_code_graph_edge_validation_fails_for_unknown_source() {
        let store = test_store();
        let args = json!({
            "schema_version": "1.0",
            "project": "test",
            "nodes": [
                { "name": "a", "labels": [], "description": "node a" }
            ],
            "edges": [
                { "source": "nonexistent", "target": "a", "relation": "depends_on", "weight": 1.0 }
            ]
        });

        let result =
            tool_import_code_graph(&store, &args, false, None, &ToolTraceContext::default());
        assert!(result.is_error);
        assert!(result.content[0].text.contains("nonexistent"));
    }

    #[test]
    fn test_import_code_graph_compact_mode() {
        let store = test_store();
        let args = json!({
            "schema_version": "1.0",
            "project": "compact_test",
            "nodes": [
                { "name": "foo", "labels": ["function"], "description": "fn foo()" }
            ],
            "edges": [],
            "prune": false
        });

        let result =
            tool_import_code_graph(&store, &args, true, None, &ToolTraceContext::default());
        assert!(!result.is_error);
        let text = &result.content[0].text;
        assert!(text.contains("code:compact_test"));
        // Compact mode is plain text, not JSON
        assert!(serde_json::from_str::<serde_json::Value>(text).is_err());
    }

    #[test]
    fn test_code_query_symbols() {
        let store = test_store();
        let import_args = json!({
            "schema_version": "1.0",
            "project": "test_project",
            "nodes": [
                { "name": "foo", "labels": ["function", "public"], "description": "fn foo()" },
                { "name": "bar", "labels": ["function"], "description": "fn bar()" },
                { "name": "baz", "labels": ["struct"], "description": "struct baz" }
            ],
            "edges": [],
            "prune": false
        });

        let _import_result = tool_import_code_graph(
            &store,
            &import_args,
            false,
            None,
            &ToolTraceContext::default(),
        );

        // Query all symbols
        let query_args = json!({
            "project": "test_project",
            "query_type": "symbols"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        let symbols: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(symbols.len(), 3);
        assert!(symbols.iter().any(|s| s["name"] == "foo"));

        // Query with label filter
        let query_args_filtered = json!({
            "project": "test_project",
            "query_type": "symbols",
            "labels": ["function"]
        });
        let result = tool_code_query(
            &store,
            &query_args_filtered,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        let symbols: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn test_code_query_callers() {
        let store = test_store();
        let import_args = json!({
            "schema_version": "1.0",
            "project": "caller_test",
            "nodes": [
                { "name": "main", "labels": ["function"], "description": "fn main()" },
                { "name": "helper", "labels": ["function"], "description": "fn helper()" },
                { "name": "util", "labels": ["function"], "description": "fn util()" }
            ],
            "edges": [
                { "source": "main", "target": "helper", "relation": "calls", "weight": 0.8 },
                { "source": "main", "target": "util", "relation": "calls", "weight": 0.5 }
            ],
            "prune": false
        });

        let _import_result = tool_import_code_graph(
            &store,
            &import_args,
            false,
            None,
            &ToolTraceContext::default(),
        );

        // Query callers of helper
        let query_args = json!({
            "project": "caller_test",
            "query_type": "callers",
            "symbol": "helper"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["symbol"], "helper");
        let callers = data["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["name"], "main");
    }

    #[test]
    fn test_code_query_callees() {
        let store = test_store();
        let import_args = json!({
            "schema_version": "1.0",
            "project": "callee_test",
            "nodes": [
                { "name": "main", "labels": ["function"], "description": "fn main()" },
                { "name": "helper", "labels": ["function"], "description": "fn helper()" },
                { "name": "util", "labels": ["function"], "description": "fn util()" }
            ],
            "edges": [
                { "source": "main", "target": "helper", "relation": "calls", "weight": 0.8 },
                { "source": "main", "target": "util", "relation": "calls", "weight": 0.5 }
            ],
            "prune": false
        });

        let _import_result = tool_import_code_graph(
            &store,
            &import_args,
            false,
            None,
            &ToolTraceContext::default(),
        );

        // Query callees of main
        let query_args = json!({
            "project": "callee_test",
            "query_type": "callees",
            "symbol": "main"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["symbol"], "main");
        let callees = data["callees"].as_array().unwrap();
        assert_eq!(callees.len(), 2);
        let names: Vec<&str> = callees.iter().filter_map(|c| c["name"].as_str()).collect();
        assert!(names.contains(&"helper"));
        assert!(names.contains(&"util"));
    }

    #[test]
    fn test_code_query_structure() {
        let store = test_store();
        let import_args = json!({
            "schema_version": "1.0",
            "project": "structure_test",
            "nodes": [
                { "name": "a", "labels": ["function"], "description": "fn a()" },
                { "name": "b", "labels": ["function"], "description": "fn b()" },
                { "name": "c", "labels": ["function"], "description": "fn c()" }
            ],
            "edges": [
                { "source": "a", "target": "b", "relation": "calls", "weight": 1.0 },
                { "source": "b", "target": "c", "relation": "calls", "weight": 1.0 }
            ],
            "prune": false
        });

        let _import_result = tool_import_code_graph(
            &store,
            &import_args,
            false,
            None,
            &ToolTraceContext::default(),
        );

        // Query structure around b
        let query_args = json!({
            "project": "structure_test",
            "query_type": "structure",
            "symbol": "b"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["symbol"], "b");
        let concepts = data["concepts"].as_array().unwrap();
        assert!(!concepts.is_empty());
        let concept_names: Vec<&str> = concepts.iter().filter_map(|c| c["name"].as_str()).collect();
        assert!(concept_names.contains(&"b"));
    }

    #[test]
    fn test_code_query_nonexistent_memoir() {
        let store = test_store();
        let query_args = json!({
            "project": "nonexistent",
            "query_type": "symbols"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn test_code_query_nonexistent_symbol() {
        let store = test_store();
        let import_args = json!({
            "schema_version": "1.0",
            "project": "sym_test",
            "nodes": [
                { "name": "exists", "labels": ["function"], "description": "fn exists()" }
            ],
            "edges": [],
            "prune": false
        });

        let _import_result = tool_import_code_graph(
            &store,
            &import_args,
            false,
            None,
            &ToolTraceContext::default(),
        );

        let query_args = json!({
            "project": "sym_test",
            "query_type": "callers",
            "symbol": "does_not_exist"
        });
        let result = tool_code_query(
            &store,
            &query_args,
            false,
            None,
            &ToolTraceContext::default(),
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn test_import_code_graph_rejects_missing_schema_version() {
        let store = test_store();
        let args = json!({
            "project": "missing_version",
            "nodes": [
                { "name": "foo", "labels": ["function"], "description": "fn foo()" }
            ],
            "edges": []
        });

        let result =
            tool_import_code_graph(&store, &args, false, None, &ToolTraceContext::default());
        assert!(result.is_error);
        assert_eq!(
            result.content[0].text,
            "missing required field: schema_version"
        );
    }

    #[test]
    fn test_import_code_graph_rejects_unknown_schema_version() {
        let store = test_store();
        let args = json!({
            "schema_version": "2.0",
            "project": "wrong_version",
            "nodes": [
                { "name": "foo", "labels": ["function"], "description": "fn foo()" }
            ],
            "edges": []
        });

        let result =
            tool_import_code_graph(&store, &args, false, None, &ToolTraceContext::default());
        assert!(result.is_error);
        assert_eq!(
            result.content[0].text,
            "unsupported code graph schema_version: 2.0 (expected 1.0)"
        );
    }
}
