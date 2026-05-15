//! Cap-compatible JSON-output methods for the socket server.
//!
//! These methods mirror the CLI `--json` output shapes and allow Cap to read
//! hyphae state directly without spawning processes or parsing CLI output.
//!
//! Each method signature is:
//! ```ignore
//! fn cap_<name>(store: &SqliteStore, params: &serde_json::Value) -> serde_json::Value
//! ```
//! Returns versioned JSON matching the CLI output shape.

use hyphae_core::{ConsolidationConfig, MemoirStore, MemoryStore, SearchOrder};
use hyphae_store::SqliteStore;
use serde_json::{Value, json};

/// Server-side FTS result cap to prevent unbounded memory allocation.
const MAX_FTS_RESULTS: usize = 500;

pub fn dispatch_cap_method(store: &SqliteStore, method: &str, params: &Value) -> Value {
    match method {
        "cap_stats" => cap_stats(store, params),
        "cap_health" => cap_health(store, params),
        "cap_topics" => cap_topics(store, params),
        "cap_search" => cap_search(store, params),
        "cap_search_all" => cap_search_all(store, params),
        "cap_session_list" => cap_session_list(store, params),
        "cap_session_timeline" => cap_session_timeline(store, params),
        "cap_memoir_list" => cap_memoir_list(store, params),
        "cap_memoir_show" => cap_memoir_show(store, params),
        "cap_memoir_search" => cap_memoir_search(store, params),
        "cap_memoir_search_all" => cap_memoir_search_all(store, params),
        "cap_memoir_inspect" => cap_memoir_inspect(store, params),
        "cap_lessons" => cap_lessons(store, params),
        _ => json!({"error": "unknown cap method", "method": method, "schema_version": "1.0"}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Memory Stats & Health
// ─────────────────────────────────────────────────────────────────────────────

fn cap_stats(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let include_invalidated = params
        .get("include_invalidated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match store.stats_with_options(project, include_invalidated) {
        Ok(stats) => json!({
            "schema_version": "1.0",
            "project": project,
            "total_memories": stats.total_memories,
            "total_topics": stats.total_topics,
            "avg_weight": stats.avg_weight,
            "oldest_memory": stats.oldest_memory,
            "newest_memory": stats.newest_memory,
        }),
        Err(e) => json!({"error": format!("stats failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_health(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let topic = params.get("topic").and_then(|v| v.as_str());
    let include_invalidated = params
        .get("include_invalidated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Get consolidation config from default (since we don't have it passed in)
    let consolidation = ConsolidationConfig::default();

    match (|| {
        let topics = if let Some(topic_name) = topic {
            let health =
                store.topic_health_with_options(topic_name, project, include_invalidated)?;
            let memories = store.get_by_topic_with_options(
                topic_name,
                project,
                include_invalidated,
                hyphae_core::TopicMemoryOrder::CreatedAtDesc,
            )?;
            vec![to_topic_health_payload(&health, &memories, &consolidation)]
        } else {
            let topic_names = store.list_topics_with_options(project, include_invalidated)?;
            topic_names
                .into_iter()
                .map(|(topic_name, _)| {
                    let health = store.topic_health_with_options(
                        &topic_name,
                        project,
                        include_invalidated,
                    )?;
                    let memories = store.get_by_topic_with_options(
                        &topic_name,
                        project,
                        include_invalidated,
                        hyphae_core::TopicMemoryOrder::CreatedAtDesc,
                    )?;
                    Ok(to_topic_health_payload(&health, &memories, &consolidation))
                })
                .collect::<Result<Vec<_>, hyphae_core::HyphaeError>>()?
        };

        let topics_needing_consolidation = topics
            .iter()
            .filter(|t: &&serde_json::Value| {
                t.get("needs_consolidation")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .count();
        let total_stale_entries: usize = topics
            .iter()
            .filter_map(|t: &Value| {
                t.get("stale_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
            })
            .sum();

        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "project": project,
            "requested_topic": topic,
            "total_topics": topics.len(),
            "topics_needing_consolidation": topics_needing_consolidation,
            "total_stale_entries": total_stale_entries,
            "topics": topics,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("health failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_topics(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let include_invalidated = params
        .get("include_invalidated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match (|| {
        let topics = store.list_topics_with_options(project, include_invalidated)?;
        let total_memories: usize = topics.iter().map(|(_, count)| *count).sum();

        let topic_payloads: Result<Vec<_>, hyphae_core::HyphaeError> = topics
            .into_iter()
            .map(|(topic_name, count)| {
                let health =
                    store.topic_health_with_options(&topic_name, project, include_invalidated)?;
                Ok(json!({
                    "topic": topic_name,
                    "count": count,
                    "avg_weight": health.avg_weight,
                    "oldest": health.oldest,
                    "newest": health.newest,
                }))
            })
            .collect();

        let topics_list = topic_payloads?;
        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "project": project,
            "total_topics": topics_list.len(),
            "total_memories": total_memories,
            "topics": topics_list,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("topics failed: {e}"), "schema_version": "1.0"}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Memory Search
// ─────────────────────────────────────────────────────────────────────────────

fn cap_search(store: &SqliteStore, params: &Value) -> Value {
    let query = match params.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return json!({"error": "missing query parameter", "schema_version": "1.0"}),
    };

    let topic = params.get("topic").and_then(|v| v.as_str());
    let project = params.get("project").and_then(|v| v.as_str());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(1000) as usize;
    let include_invalidated = params
        .get("include_invalidated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match (|| {
        let total =
            store.search_fts_count_with_options(query, topic, project, include_invalidated)?;
        let results = store.search_fts_with_options(
            query,
            topic,
            limit,
            0,
            project,
            include_invalidated,
            SearchOrder::RankAsc,
        )?;

        let memory_payloads: Vec<Value> = results.iter().map(to_memory_payload).collect();

        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "project": project,
            "query": query,
            "topic": topic,
            "limit": limit,
            "total": total,
            "results": memory_payloads,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("search failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_search_all(store: &SqliteStore, params: &Value) -> Value {
    let query = match params.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return json!({"error": "missing query parameter", "schema_version": "1.0"}),
    };

    let project = params.get("project").and_then(|v| v.as_str());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(1000) as usize;
    let include_invalidated = params
        .get("include_invalidated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match (|| {
        let total =
            store.search_fts_count_with_options(query, None, project, include_invalidated)?;
        let results = store.search_fts_with_options(
            query,
            None,
            limit,
            0,
            project,
            include_invalidated,
            SearchOrder::RankAsc,
        )?;

        let memory_payloads: Vec<Value> = results.iter().map(to_memory_payload).collect();

        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "project": project,
            "query": query,
            "limit": limit,
            "total": total,
            "results": memory_payloads,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("search_all failed: {e}"), "schema_version": "1.0"}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sessions
// ─────────────────────────────────────────────────────────────────────────────

fn cap_session_list(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(1000) as i64;

    let result = if let Some(project_name) = project {
        store.session_context(project_name, limit)
    } else {
        store.session_context_all(limit)
    };

    match result {
        Ok(sessions) => {
            let session_payloads: Vec<Value> = sessions
                .into_iter()
                .map(|session| {
                    json!({
                        "id": session.id,
                        "started_at": session.started_at,
                        "ended_at": session.ended_at,
                        "project": session.project,
                        "task": session.task,
                        "status": if session.ended_at.is_some() { "ended" } else { "active" },
                    })
                })
                .collect();
            json!({
                "schema_version": "1.0",
                "project": project,
                "limit": limit,
                "sessions": session_payloads,
            })
        }
        Err(e) => json!({"error": format!("session_list failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_session_timeline(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(1000) as i64;

    let result = if let Some(project_name) = project {
        store.session_context(project_name, limit)
    } else {
        store.session_context_all(limit)
    };

    match result {
        Ok(sessions) => {
            let events: Vec<Value> = sessions
                .iter()
                .flat_map(|session| {
                    let mut events = vec![json!({
                        "type": "session_start",
                        "session_id": session.id,
                        "timestamp": session.started_at,
                        "project": session.project,
                        "task": session.task,
                    })];
                    if let Some(ref ended_at) = session.ended_at {
                        events.push(json!({
                            "type": "session_end",
                            "session_id": session.id,
                            "timestamp": ended_at,
                            "project": session.project,
                        }));
                    }
                    events
                })
                .collect();

            json!({
                "schema_version": "1.0",
                "project": project,
                "limit": limit,
                "events": events,
            })
        }
        Err(e) => {
            json!({"error": format!("session_timeline failed: {e}"), "schema_version": "1.0"})
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Memoirs
// ─────────────────────────────────────────────────────────────────────────────

fn cap_memoir_list(store: &SqliteStore, _params: &Value) -> Value {
    match (|| {
        let memoirs = store.list_memoirs()?;
        let memoir_payloads: Result<Vec<Value>, hyphae_core::HyphaeError> = memoirs
            .into_iter()
            .map(|memoir| {
                let stats = store.memoir_stats(&memoir.id)?;
                Ok(json!({
                    "memoir": memoir,
                    "concept_count": stats.total_concepts,
                    "link_count": stats.total_links,
                }))
            })
            .collect();

        let payloads = memoir_payloads?;
        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "memoirs": payloads,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("memoir_list failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_memoir_show(store: &SqliteStore, params: &Value) -> Value {
    let memoir_name = match params.get("memoir").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return json!({"error": "missing memoir parameter", "schema_version": "1.0"}),
    };

    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(1000) as usize;
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    match (|| {
        let memoir = store.get_memoir_by_name(memoir_name)?.ok_or_else(|| {
            hyphae_core::HyphaeError::NotFound(format!("memoir not found: {memoir_name}"))
        })?;

        let all_concepts = if let Some(ref q) = query {
            store.search_concepts_fts(&memoir.id, q, MAX_FTS_RESULTS)?
        } else {
            store.list_concepts(&memoir.id)?
        };

        let total = all_concepts.len();
        let concepts: Vec<Value> = all_concepts
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "memoir_id": c.memoir_id.to_string(),
                    "name": c.name,
                    "definition": c.definition,
                    "confidence": c.confidence.value(),
                    "revision": c.revision,
                    "labels": c.labels,
                })
            })
            .collect();

        let stats = store.memoir_stats(&memoir.id)?;
        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "memoir": {
                "id": memoir.id.to_string(),
                "name": memoir.name,
                "description": memoir.description,
                "created_at": memoir.created_at,
            },
            "stats": {
                "total_concepts": stats.total_concepts,
                "total_links": stats.total_links,
                "avg_confidence": stats.avg_confidence,
                "label_counts": stats.label_counts.into_iter().map(|(label, count)| {
                    json!({ "label": label, "count": count })
                }).collect::<Vec<_>>(),
            },
            "query": query,
            "limit": limit,
            "offset": offset,
            "total": total,
            "concepts": concepts,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("memoir_show failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_memoir_search(store: &SqliteStore, params: &Value) -> Value {
    let query = match params.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return json!({"error": "missing query parameter", "schema_version": "1.0"}),
    };

    let memoir_name = params.get("memoir").and_then(|v| v.as_str());
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(1000) as usize;
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    match (|| {
        let (memoir_opt, total, results) = if let Some(memoir_name_str) = memoir_name {
            let memoir = store.get_memoir_by_name(memoir_name_str)?.ok_or_else(|| {
                hyphae_core::HyphaeError::NotFound(format!("memoir not found: {memoir_name_str}"))
            })?;
            let all_results = store.search_concepts_fts(&memoir.id, query, MAX_FTS_RESULTS)?;
            let total = all_results.len();
            let limited: Vec<_> = all_results.into_iter().skip(offset).take(limit).collect();
            (Some(memoir), total, limited)
        } else {
            return Err(hyphae_core::HyphaeError::NotFound(
                "memoir parameter required for cap_memoir_search".to_string(),
            ));
        };

        let concept_payloads: Vec<Value> = results
            .into_iter()
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "memoir_id": c.memoir_id.to_string(),
                    "name": c.name,
                    "definition": c.definition,
                    "confidence": c.confidence.value(),
                    "revision": c.revision,
                })
            })
            .collect();

        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "memoir": memoir_opt.map(|m| {
                json!({
                    "id": m.id.to_string(),
                    "name": m.name,
                    "description": m.description,
                })
            }),
            "query": query,
            "limit": limit,
            "offset": offset,
            "total": total,
            "results": concept_payloads,
        }))
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("memoir_search failed: {e}"), "schema_version": "1.0"}),
    }
}

fn cap_memoir_search_all(store: &SqliteStore, params: &Value) -> Value {
    let query = match params.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return json!({"error": "missing query parameter", "schema_version": "1.0"}),
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(1000) as usize;
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    match (|| {
        let memoirs = store.list_memoirs()?;
        let mut all_hits = Vec::new();

        for memoir in &memoirs {
            let concepts = store.search_concepts_fts(&memoir.id, query, MAX_FTS_RESULTS)?;
            for concept in concepts {
                all_hits.push((memoir.clone(), concept));
            }
        }

        let total = all_hits.len();
        let hits: Vec<Value> = all_hits
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(memoir, concept)| {
                json!({
                    "memoir": {
                        "id": memoir.id.to_string(),
                        "name": memoir.name,
                        "description": memoir.description,
                    },
                    "concept": {
                        "id": concept.id.to_string(),
                        "name": concept.name,
                        "definition": concept.definition,
                        "confidence": concept.confidence.value(),
                    },
                })
            })
            .collect();

        Ok::<_, hyphae_core::HyphaeError>(json!({
            "schema_version": "1.0",
            "query": query,
            "limit": limit,
            "offset": offset,
            "total": total,
            "results": hits,
        }))
    })() {
        Ok(result) => result,
        Err(e) => {
            json!({"error": format!("memoir_search_all failed: {e}"), "schema_version": "1.0"})
        }
    }
}

fn cap_memoir_inspect(store: &SqliteStore, params: &Value) -> Value {
    let memoir_name = match params.get("memoir").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return json!({"error": "missing memoir parameter", "schema_version": "1.0"}),
    };

    let concept_name = params.get("concept").and_then(|v| v.as_str());
    let depth = params
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .min(5) as usize;

    match (|| {
        let memoir = store.get_memoir_by_name(memoir_name)?.ok_or_else(|| {
            hyphae_core::HyphaeError::NotFound(format!("memoir not found: {memoir_name}"))
        })?;

        if let Some(cname) = concept_name {
            let concept = store
                .get_concept_by_name(&memoir.id, cname)?
                .ok_or_else(|| {
                    hyphae_core::HyphaeError::NotFound(format!("concept not found: {cname}"))
                })?;

            // Get neighborhood links
            let mut neighborhood_concepts = vec![concept.clone()];
            let mut neighborhood_links = Vec::new();

            // Get links from and to this concept
            let links_from = store.get_links_from(&concept.id)?;
            let links_to = store.get_links_to(&concept.id)?;

            for link in &links_from {
                neighborhood_links.push(link.clone());
                if let Ok(Some(target)) = store.get_concept(&link.target_id) {
                    neighborhood_concepts.push(target);
                }
            }

            for link in &links_to {
                neighborhood_links.push(link.clone());
                if let Ok(Some(source)) = store.get_concept(&link.source_id) {
                    neighborhood_concepts.push(source);
                }
            }

            Ok::<_, hyphae_core::HyphaeError>(json!({
                "schema_version": "1.0",
                "memoir": {
                    "id": memoir.id.to_string(),
                    "name": memoir.name,
                },
                "concept": {
                    "id": concept.id.to_string(),
                    "name": concept.name,
                    "definition": concept.definition,
                    "confidence": concept.confidence.value(),
                    "revision": concept.revision,
                    "labels": concept.labels,
                },
                "depth": depth,
                "neighborhood": {
                    "concepts": neighborhood_concepts.into_iter().map(|c| {
                        json!({
                            "id": c.id.to_string(),
                            "name": c.name,
                            "definition": c.definition,
                            "confidence": c.confidence.value(),
                        })
                    }).collect::<Vec<_>>(),
                    "links": neighborhood_links.into_iter().map(|l| {
                        json!({
                            "source_id": l.source_id.to_string(),
                            "target_id": l.target_id.to_string(),
                            "relation": l.relation.to_string(),
                            "weight": l.weight.value(),
                        })
                    }).collect::<Vec<_>>(),
                },
            }))
        } else {
            // Return memoir overview (same as show)
            let stats = store.memoir_stats(&memoir.id)?;
            let all_concepts = store.list_concepts(&memoir.id)?;

            Ok::<_, hyphae_core::HyphaeError>(json!({
                "schema_version": "1.0",
                "memoir": {
                    "id": memoir.id.to_string(),
                    "name": memoir.name,
                    "description": memoir.description,
                    "created_at": memoir.created_at,
                },
                "stats": {
                    "total_concepts": stats.total_concepts,
                    "total_links": stats.total_links,
                    "avg_confidence": stats.avg_confidence,
                },
                "concept_count": all_concepts.len(),
            }))
        }
    })() {
        Ok(result) => result,
        Err(e) => json!({"error": format!("memoir_inspect failed: {e}"), "schema_version": "1.0"}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lessons & Context
// ─────────────────────────────────────────────────────────────────────────────

fn cap_lessons(store: &SqliteStore, params: &Value) -> Value {
    let project = params.get("project").and_then(|v| v.as_str());
    let per_topic_limit = params
        .get("per_topic_limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(200) as usize;

    match store.extract_lessons(project, per_topic_limit) {
        Ok(lessons) => json!({
            "schema_version": "1.0",
            "lessons": lessons,
        }),
        Err(e) => json!({"error": format!("lessons failed: {e}"), "schema_version": "1.0"}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

fn to_memory_payload(memory: &hyphae_core::Memory) -> Value {
    json!({
        "id": memory.id.to_string(),
        "created_at": memory.created_at,
        "updated_at": memory.updated_at,
        "last_accessed": memory.last_accessed,
        "access_count": memory.access_count,
        "weight": memory.weight.value(),
        "topic": memory.topic,
        "summary": memory.summary,
        "raw_excerpt": memory.raw_excerpt,
        "keywords": memory.keywords,
        "importance": memory.importance.to_string(),
        "source": match &memory.source {
            hyphae_core::MemorySource::AgentSession {
                host,
                session_id,
                file_path,
            } => {
                json!({
                    "type": "agent_session",
                    "host": match host {
                        hyphae_core::SessionHost::ClaudeCode => "claude-code",
                        hyphae_core::SessionHost::Codex => "codex",
                    },
                    "session_id": session_id,
                    "file_path": file_path,
                })
            }
            hyphae_core::MemorySource::Manual => json!({"type": "manual"}),
        },
        "related_ids": memory.related_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "project": memory.project,
        "branch": memory.branch,
        "worktree": memory.worktree,
        "agent_id": memory.agent_id,
        "expires_at": memory.expires_at,
        "invalidated_at": memory.invalidated_at,
        "invalidation_reason": memory.invalidation_reason,
        "superseded_by": memory.superseded_by.as_ref().map(|id| id.to_string()),
        "has_embedding": memory.embedding.is_some(),
    })
}

fn to_topic_health_payload(
    health: &hyphae_core::TopicHealth,
    memories: &[hyphae_core::Memory],
    consolidation: &ConsolidationConfig,
) -> Value {
    let low_weight_count = memories.iter().filter(|m| m.weight.value() < 0.3).count();
    let critical_count = memories
        .iter()
        .filter(|m| matches!(m.importance, hyphae_core::Importance::Critical))
        .count();
    let high_count = memories
        .iter()
        .filter(|m| matches!(m.importance, hyphae_core::Importance::High))
        .count();
    let medium_count = memories
        .iter()
        .filter(|m| matches!(m.importance, hyphae_core::Importance::Medium))
        .count();
    let low_count = memories
        .iter()
        .filter(|m| matches!(m.importance, hyphae_core::Importance::Low))
        .count();

    let needs_consolidation = match consolidation.threshold_for_topic(&health.topic) {
        Some(threshold) => memories.len() >= threshold,
        None => false,
    };

    json!({
        "topic": health.topic,
        "entry_count": health.entry_count,
        "avg_weight": health.avg_weight,
        "avg_access_count": health.avg_access_count,
        "oldest": health.oldest,
        "newest": health.newest,
        "last_accessed": health.last_accessed,
        "needs_consolidation": needs_consolidation,
        "stale_count": health.stale_count,
        "low_weight_count": low_weight_count,
        "critical_count": critical_count,
        "high_count": high_count,
        "medium_count": medium_count,
        "low_count": low_count,
    })
}
