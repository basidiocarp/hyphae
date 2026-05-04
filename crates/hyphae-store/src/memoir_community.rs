use std::collections::HashMap;

use hyphae_core::{ConceptId, HyphaeError, HyphaeResult, MemoirId, MemoirStore};
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

#[cfg(test)]
mod tests {
    use hyphae_core::{Concept, ConceptLink, Memoir, MemoirStore, Relation};

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
}
