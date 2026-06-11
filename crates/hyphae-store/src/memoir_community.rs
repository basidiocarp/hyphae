use std::collections::HashMap;

use hyphae_core::{ConceptId, HyphaeError, HyphaeResult, LinkInput, MemoirId, MemoirStore};
use petgraph::prelude::*;
use petgraph::visit::Bfs;
use rusqlite::params;

use crate::SqliteStore;

/// Run connected-component clustering over a memoir's concept graph.
/// Assigns `community_N` labels to each concept in a single transaction.
/// Returns the number of distinct communities found.
pub fn cluster_memoir(store: &SqliteStore, memoir_id: &MemoirId) -> HyphaeResult<usize> {
    let concepts = store.list_concepts(memoir_id)?;
    if concepts.is_empty() {
        return Ok(0);
    }
    let links = store.list_all_links(memoir_id)?;

    // Build undirected graph: nodes are concept IDs, edges from links
    let mut graph = Graph::<&ConceptId, u32, Undirected>::new_undirected();
    let mut node_index: HashMap<&ConceptId, NodeIndex> = HashMap::new();
    for concept in &concepts {
        let idx = graph.add_node(&concept.id);
        node_index.insert(&concept.id, idx);
    }
    for link in &links {
        if let (Some(&src), Some(&tgt)) = (
            node_index.get(&link.source_id),
            node_index.get(&link.target_id),
        ) {
            graph.add_edge(src, tgt, link.link_count);
        }
    }

    // BFS over all unvisited nodes to assign component IDs
    let mut community_map: HashMap<NodeIndex, usize> = HashMap::new();
    let mut community_count = 0usize;
    for start in graph.node_indices() {
        if community_map.contains_key(&start) {
            continue;
        }
        let mut bfs = Bfs::new(&graph, start);
        while let Some(node) = bfs.next(&graph) {
            community_map.insert(node, community_count);
        }
        community_count += 1;
    }

    // Collect assignments then write in one transaction. All concepts in the
    // memoir are unconditionally overwritten, so callers do not need to
    // pre-clear existing community labels.
    let assignments: Vec<(&ConceptId, String)> = concepts
        .iter()
        .map(|c| {
            let idx = node_index[&c.id];
            let cid = community_map[&idx];
            (&c.id, format!("community_{cid}"))
        })
        .collect();

    let tx = store
        .conn
        .unchecked_transaction()
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    for (concept_id, community_id) in &assignments {
        tx.execute(
            "UPDATE concepts SET community_id = ?2 WHERE id = ?1",
            params![concept_id.as_ref(), community_id.as_str()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.commit()
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    Ok(community_count)
}

/// Infer memoir concept links from memory recall co-occurrence.
///
/// Pairs of memories that appear together in the same recall event are
/// semantically related. This function:
///
/// 1. Queries `recall_events` for all pairs of memory IDs that co-occur in
///    the same event, counting how many events each pair shares.
/// 2. Keeps only pairs whose co-occurrence count is strictly greater than
///    `min_cooccurrence`.
/// 3. Resolves each memory's topic to a concept in the given memoir by name.
///    Pairs where either topic cannot be resolved are skipped silently.
/// 4. Builds `LinkInput` records with `relation = "related_to"` and a weight
///    proportional to the co-occurrence count, then calls `upsert_links`.
///
/// Returns the number of links created or updated.
pub fn infer_cooccurrence_links(
    store: &SqliteStore,
    memoir_id: &MemoirId,
    min_cooccurrence: u32,
) -> HyphaeResult<usize> {
    // Step 1 + 2: find co-occurring memory-ID pairs above the threshold.
    // json_each expands the memory_ids JSON array; self-join on event ID with
    // an ordering guard (mi1.value < mi2.value) deduplicates (a,b)/(b,a).
    // COUNT(DISTINCT re.id) counts the number of distinct recall events the pair
    // co-occurred in — log_recall_event serializes the caller's slice verbatim,
    // so a duplicate ID within a single event's array must not inflate the count.
    let sql = "
        SELECT mi1.value             AS mem_id_a,
               mi2.value             AS mem_id_b,
               COUNT(DISTINCT re.id) AS cooccurrence_count
        FROM   recall_events re
        JOIN   json_each(re.memory_ids) AS mi1
        JOIN   json_each(re.memory_ids) AS mi2
               ON mi1.value < mi2.value
        GROUP  BY mi1.value, mi2.value
        HAVING COUNT(DISTINCT re.id) > ?1
    ";

    let mut stmt = store
        .conn
        .prepare_cached(sql)
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Collect (mem_id_a, mem_id_b, count) rows.
    let pairs: Vec<(String, String, u32)> = stmt
        .query_map(params![min_cooccurrence as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .map_err(|e| HyphaeError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if pairs.is_empty() {
        return Ok(0);
    }

    // Step 2 + 3: map each memory_id to its topic, then resolve topic → concept name.
    // We batch-fetch all required memory topics in one query.
    let all_mem_ids: Vec<String> = pairs
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Build placeholders for the IN clause.
    let placeholders = all_mem_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let topic_sql = format!("SELECT id, topic FROM memories WHERE id IN ({placeholders})");
    let mut topic_stmt = store
        .conn
        .prepare(&topic_sql)
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = all_mem_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();

    let id_to_topic: HashMap<String, String> = topic_stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| HyphaeError::Database(e.to_string()))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Step 3 cont.: resolve topic → concept name within the memoir.
    // `upsert_links` keys on concept name, so we resolve by name.
    let mut link_inputs: Vec<LinkInput> = Vec::new();

    for (mem_id_a, mem_id_b, count) in &pairs {
        let topic_a = match id_to_topic.get(mem_id_a) {
            Some(t) => t,
            None => continue,
        };
        let topic_b = match id_to_topic.get(mem_id_b) {
            Some(t) => t,
            None => continue,
        };

        // Resolve each topic to a concept by name in this memoir.
        let concept_a = store.get_concept_by_name(memoir_id, topic_a)?;
        let concept_b = store.get_concept_by_name(memoir_id, topic_b)?;

        let (ca, cb) = match (concept_a, concept_b) {
            (Some(a), Some(b)) => (a, b),
            _ => continue, // skip if either topic doesn't map to a concept
        };

        link_inputs.push(LinkInput {
            source_name: ca.name,
            target_name: cb.name,
            relation: "related_to".to_string(),
            weight: *count as f32,
        });
    }

    if link_inputs.is_empty() {
        return Ok(0);
    }

    // Step 4 + 5: upsert links (do NOT wrap in a transaction — upsert_links
    // opens its own unchecked_transaction internally; nesting would panic).
    let report = store.upsert_links(memoir_id, &link_inputs)?;
    Ok(report.created + report.updated)
}

#[cfg(test)]
mod tests {
    use hyphae_core::{Concept, ConceptLink, Memoir, MemoirStore, MemoryStore, Relation};

    use crate::SqliteStore;

    use super::*;

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_cluster_empty_memoir() {
        let store = make_store();
        let memoir = Memoir::new("test".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();
        let count = cluster_memoir(&store, &memoir.id).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cluster_disconnected_concepts_each_own_community() {
        let store = make_store();
        let memoir = Memoir::new("test".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        let c1 = Concept::new(memoir.id.clone(), "Alpha".to_string(), "def".to_string());
        let c2 = Concept::new(memoir.id.clone(), "Beta".to_string(), "def".to_string());
        store.add_concept(c1.clone()).unwrap();
        store.add_concept(c2.clone()).unwrap();

        let count = cluster_memoir(&store, &memoir.id).unwrap();
        assert_eq!(count, 2, "two isolated nodes => two communities");

        let updated1 = store.get_concept(&c1.id).unwrap().unwrap();
        let updated2 = store.get_concept(&c2.id).unwrap().unwrap();
        assert_ne!(updated1.community_id, updated2.community_id);
        assert!(updated1.community_id.is_some());
        assert!(updated2.community_id.is_some());
    }

    #[test]
    fn test_cluster_connected_concepts_share_community() {
        let store = make_store();
        let memoir = Memoir::new("test".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        let c1 = Concept::new(memoir.id.clone(), "Alpha".to_string(), "def".to_string());
        let c2 = Concept::new(memoir.id.clone(), "Beta".to_string(), "def".to_string());
        store.add_concept(c1.clone()).unwrap();
        store.add_concept(c2.clone()).unwrap();

        let link = ConceptLink::new(c1.id.clone(), c2.id.clone(), Relation::RelatedTo);
        store.add_link(link).unwrap();

        let count = cluster_memoir(&store, &memoir.id).unwrap();
        assert_eq!(count, 1, "connected nodes => one community");

        let updated1 = store.get_concept(&c1.id).unwrap().unwrap();
        let updated2 = store.get_concept(&c2.id).unwrap().unwrap();
        assert_eq!(updated1.community_id, updated2.community_id);
    }

    // -----------------------------------------------------------------------
    // infer_cooccurrence_links tests
    // -----------------------------------------------------------------------

    /// Seed two memories and a recall event that references both, then
    /// assert that a single link is created between the two matching concepts.
    #[test]
    fn test_infer_cooccurrence_creates_link_above_threshold() {
        use hyphae_core::{Importance, Memory};

        let store = make_store();
        let memoir = Memoir::new("cooccur".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        // Two concepts whose names match the memory topics we will insert.
        let c1 = Concept::new(memoir.id.clone(), "rust".to_string(), "def".to_string());
        let c2 = Concept::new(memoir.id.clone(), "memory".to_string(), "def".to_string());
        store.add_concept(c1.clone()).unwrap();
        store.add_concept(c2.clone()).unwrap();

        // Two memories with topics matching the concept names.
        let m1 = Memory::new(
            "rust".to_string(),
            "Rust memory management".to_string(),
            Importance::Medium,
        );
        let m2 = Memory::new(
            "memory".to_string(),
            "Memory safety concepts".to_string(),
            Importance::Medium,
        );
        store.store(m1.clone()).unwrap();
        store.store(m2.clone()).unwrap();

        // Insert 2 recall events that contain both memory IDs (co-occurrence count = 2).
        store
            .log_recall_event(
                None,
                "query1",
                &[m1.id.to_string(), m2.id.to_string()],
                None,
            )
            .unwrap();
        store
            .log_recall_event(
                None,
                "query2",
                &[m1.id.to_string(), m2.id.to_string()],
                None,
            )
            .unwrap();

        // min_cooccurrence = 1 means strictly > 1, i.e., >= 2. Count is 2, so should pass.
        let created = infer_cooccurrence_links(&store, &memoir.id, 1).unwrap();
        assert_eq!(created, 1, "expected exactly one link to be created");

        let links = store.list_all_links(&memoir.id).unwrap();
        assert_eq!(links.len(), 1, "memoir should have one link");
    }

    /// When the co-occurrence count does not exceed min_cooccurrence, no links
    /// are created.
    #[test]
    fn test_infer_cooccurrence_below_threshold_creates_no_link() {
        use hyphae_core::{Importance, Memory};

        let store = make_store();
        let memoir = Memoir::new("threshold".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        let c1 = Concept::new(memoir.id.clone(), "alpha".to_string(), "def".to_string());
        let c2 = Concept::new(memoir.id.clone(), "beta".to_string(), "def".to_string());
        store.add_concept(c1).unwrap();
        store.add_concept(c2).unwrap();

        let m1 = Memory::new(
            "alpha".to_string(),
            "Alpha topic".to_string(),
            Importance::Medium,
        );
        let m2 = Memory::new(
            "beta".to_string(),
            "Beta topic".to_string(),
            Importance::Medium,
        );
        store.store(m1.clone()).unwrap();
        store.store(m2.clone()).unwrap();

        // One recall event: co-occurrence count = 1.
        store
            .log_recall_event(None, "q", &[m1.id.to_string(), m2.id.to_string()], None)
            .unwrap();

        // min_cooccurrence = 1 means strictly > 1; count is 1, so no link.
        let created = infer_cooccurrence_links(&store, &memoir.id, 1).unwrap();
        assert_eq!(
            created, 0,
            "count not strictly > threshold; no link expected"
        );

        let links = store.list_all_links(&memoir.id).unwrap();
        assert!(links.is_empty());
    }

    /// A memory whose topic does not match any concept name in the memoir is
    /// silently skipped — no panic, no link created.
    #[test]
    fn test_infer_cooccurrence_unresolved_topic_is_skipped() {
        use hyphae_core::{Importance, Memory};

        let store = make_store();
        let memoir = Memoir::new("partial".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        // Only one concept in the memoir; the second topic has no match.
        let c1 = Concept::new(memoir.id.clone(), "known".to_string(), "def".to_string());
        store.add_concept(c1).unwrap();

        let m1 = Memory::new(
            "known".to_string(),
            "Known topic".to_string(),
            Importance::Medium,
        );
        let m2 = Memory::new(
            "unknown_topic".to_string(),
            "No concept for this".to_string(),
            Importance::Medium,
        );
        store.store(m1.clone()).unwrap();
        store.store(m2.clone()).unwrap();

        // Two recall events so the pair exceeds min_cooccurrence = 1.
        store
            .log_recall_event(None, "q1", &[m1.id.to_string(), m2.id.to_string()], None)
            .unwrap();
        store
            .log_recall_event(None, "q2", &[m1.id.to_string(), m2.id.to_string()], None)
            .unwrap();

        // Should not panic; the pair is skipped because "unknown_topic" has no concept.
        let created = infer_cooccurrence_links(&store, &memoir.id, 1).unwrap();
        assert_eq!(created, 0, "unresolved topic pair should be skipped");

        let links = store.list_all_links(&memoir.id).unwrap();
        assert!(links.is_empty());
    }

    /// A single recall event whose `memory_ids` array repeats a pair must count
    /// as ONE co-occurrence, not several. `log_recall_event` serializes the
    /// caller's slice verbatim, so duplicate IDs within one event are possible;
    /// `COUNT(DISTINCT re.id)` must keep the count at the number of distinct
    /// events. With one event the count is 1, which is not strictly > 1.
    #[test]
    fn test_infer_cooccurrence_duplicate_ids_in_single_event_not_inflated() {
        use hyphae_core::{Importance, Memory};

        let store = make_store();
        let memoir = Memoir::new("dupe".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        let c1 = Concept::new(memoir.id.clone(), "x".to_string(), "def".to_string());
        let c2 = Concept::new(memoir.id.clone(), "y".to_string(), "def".to_string());
        store.add_concept(c1).unwrap();
        store.add_concept(c2).unwrap();

        let m1 = Memory::new("x".to_string(), "X topic".to_string(), Importance::Medium);
        let m2 = Memory::new("y".to_string(), "Y topic".to_string(), Importance::Medium);
        store.store(m1.clone()).unwrap();
        store.store(m2.clone()).unwrap();

        // ONE event, but the pair appears multiple times within its array.
        // Under COUNT(*) this would inflate to a passing count; under
        // COUNT(DISTINCT re.id) it is 1 distinct event, so no link at min=1.
        store
            .log_recall_event(
                None,
                "q",
                &[
                    m1.id.to_string(),
                    m2.id.to_string(),
                    m1.id.to_string(),
                    m2.id.to_string(),
                ],
                None,
            )
            .unwrap();

        let created = infer_cooccurrence_links(&store, &memoir.id, 1).unwrap();
        assert_eq!(
            created, 0,
            "duplicate IDs within one event must not inflate the co-occurrence count"
        );

        let links = store.list_all_links(&memoir.id).unwrap();
        assert!(links.is_empty());
    }
}
