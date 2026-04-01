use std::collections::BTreeMap;

use rusqlite::params;
use serde::Serialize;

use hyphae_core::{HyphaeError, HyphaeResult};

use super::{SqliteStore, TopicMemoryOrder};

const DEFAULT_TOPIC_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LessonCategory {
    Corrections,
    Errors,
    Tests,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LessonRecord {
    pub id: String,
    pub category: LessonCategory,
    pub description: String,
    pub frequency: usize,
    pub source_topics: Vec<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HyphaeAnalytics {
    pub importance_distribution: ImportanceDistribution,
    pub lifecycle: LifecycleAnalytics,
    pub memoir_stats: MemoirAnalytics,
    pub memory_utilization: MemoryUtilization,
    pub search_stats: Option<SearchAnalytics>,
    pub top_topics: Vec<TopTopicAnalytics>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HyphaeActivitySnapshot {
    pub activity: RecentMemoryActivity,
    pub memories: usize,
    pub memoirs: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImportanceDistribution {
    pub critical: usize,
    pub ephemeral: usize,
    pub high: usize,
    pub low: usize,
    pub medium: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LifecycleAnalytics {
    pub avg_weight: f32,
    pub created_last_7d: usize,
    pub created_last_30d: usize,
    pub decayed: usize,
    pub min_weight: f32,
    pub pruned: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MemoirAnalytics {
    pub code_memoirs: usize,
    pub total: usize,
    pub total_concepts: usize,
    pub total_links: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemoryUtilization {
    pub rate: f64,
    pub recalled: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchAnalytics {
    pub empty_results: usize,
    pub hit_rate: usize,
    pub total_searches: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopTopicAnalytics {
    pub avg_weight: f32,
    pub count: usize,
    pub latest_created_at: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecentMemoryActivity {
    pub codex_memory_count: usize,
    pub last_codex_memory_at: Option<String>,
    pub last_session_memory_at: Option<String>,
    pub last_session_topic: Option<String>,
    pub recent_session_memory_count: usize,
}

#[derive(Debug, Clone)]
struct LessonGroup {
    description: String,
    frequency: usize,
    keywords: Vec<String>,
}

impl SqliteStore {
    pub fn extract_lessons(
        &self,
        project: Option<&str>,
        per_topic_limit: usize,
    ) -> HyphaeResult<Vec<LessonRecord>> {
        let limit = per_topic_limit.max(1);
        let corrections =
            self.recent_topic_memories("corrections", project, limit.min(DEFAULT_TOPIC_LIMIT))?;
        let resolved_errors =
            self.recent_topic_memories("errors/resolved", project, limit.min(DEFAULT_TOPIC_LIMIT))?;
        let resolved_tests =
            self.recent_topic_memories("tests/resolved", project, limit.min(DEFAULT_TOPIC_LIMIT))?;

        let mut next_id = 0usize;
        let mut lessons = Vec::new();
        lessons.extend(self.build_lessons_for_category(
            &corrections,
            LessonCategory::Corrections,
            "correction",
            "corrections",
            correction_group_key,
            &mut next_id,
        ));
        lessons.extend(self.build_lessons_for_category(
            &resolved_errors,
            LessonCategory::Errors,
            "error",
            "errors/resolved",
            secondary_group_key,
            &mut next_id,
        ));
        lessons.extend(self.build_lessons_for_category(
            &resolved_tests,
            LessonCategory::Tests,
            "test",
            "tests/resolved",
            secondary_group_key,
            &mut next_id,
        ));

        lessons.sort_by(|left, right| {
            right
                .frequency
                .cmp(&left.frequency)
                .then_with(|| left.description.cmp(&right.description))
                .then_with(|| left.id.cmp(&right.id))
        });

        Ok(lessons)
    }

    pub fn analytics_snapshot(&self, project: Option<&str>) -> HyphaeResult<HyphaeAnalytics> {
        let total = self.query_count(
            "SELECT COUNT(*) FROM memories WHERE project = ?1 OR ?1 IS NULL",
            params![project],
        )?;
        let recalled = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE access_count > 0
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;
        let created_last_7d = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE julianday(created_at) > julianday('now', '-7 days')
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;
        let created_last_30d = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE julianday(created_at) > julianday('now', '-30 days')
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;
        let decayed = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE weight < 0.3
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;

        let (avg_weight, min_weight): (Option<f32>, Option<f32>) = self
            .conn
            .query_row(
                "SELECT AVG(weight), MIN(weight)
                 FROM memories
                 WHERE project = ?1 OR ?1 IS NULL",
                params![project],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| HyphaeError::Database(format!("failed to query memory weights: {e}")))?;

        let memoir_stats = MemoirAnalytics {
            code_memoirs: self
                .query_count("SELECT COUNT(*) FROM memoirs WHERE name LIKE 'code:%'", [])?,
            total: self.query_count("SELECT COUNT(*) FROM memoirs", [])?,
            total_concepts: self.query_count("SELECT COUNT(*) FROM concepts", [])?,
            total_links: self.query_count("SELECT COUNT(*) FROM concept_links", [])?,
        };

        let top_topics = self.query_top_topics(project)?;
        let importance_distribution = self.query_importance_distribution(project)?;

        Ok(HyphaeAnalytics {
            importance_distribution,
            lifecycle: LifecycleAnalytics {
                avg_weight: avg_weight.unwrap_or(0.0),
                created_last_7d,
                created_last_30d,
                decayed,
                min_weight: min_weight.unwrap_or(0.0),
                pruned: 0,
            },
            memoir_stats,
            memory_utilization: MemoryUtilization {
                rate: if total == 0 {
                    0.0
                } else {
                    recalled as f64 / total as f64
                },
                recalled,
                total,
            },
            search_stats: None,
            top_topics,
        })
    }

    pub fn activity_snapshot(&self, project: Option<&str>) -> HyphaeResult<HyphaeActivitySnapshot> {
        let memories = self.query_count(
            "SELECT COUNT(*) FROM memories WHERE project = ?1 OR ?1 IS NULL",
            params![project],
        )?;
        let memoirs = self.query_count("SELECT COUNT(*) FROM memoirs", [])?;
        let codex_memory_count = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE keywords LIKE '%host:codex%'
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;

        let (last_codex_memory_at,): (Option<String>,) = self
            .conn
            .query_row(
                "SELECT created_at FROM memories
                 WHERE keywords LIKE '%host:codex%'
                   AND (project = ?1 OR ?1 IS NULL)
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![project],
                |row| Ok((row.get(0)?,)),
            )
            .unwrap_or((None,));

        let (last_session_topic, last_session_memory_at): (Option<String>, Option<String>) = self
            .conn
            .query_row(
                "SELECT topic, created_at FROM memories
                 WHERE topic LIKE 'session/%'
                   AND (project = ?1 OR ?1 IS NULL)
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![project],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((None, None));

        let recent_session_memory_count = self.query_count(
            "SELECT COUNT(*) FROM memories
             WHERE topic LIKE 'session/%'
               AND created_at >= datetime('now', '-1 day')
               AND (project = ?1 OR ?1 IS NULL)",
            params![project],
        )?;

        Ok(HyphaeActivitySnapshot {
            activity: RecentMemoryActivity {
                codex_memory_count,
                last_codex_memory_at,
                last_session_memory_at,
                last_session_topic,
                recent_session_memory_count,
            },
            memories,
            memoirs,
        })
    }

    fn recent_topic_memories(
        &self,
        topic: &str,
        project: Option<&str>,
        limit: usize,
    ) -> HyphaeResult<Vec<hyphae_core::Memory>> {
        let mut memories =
            self.get_by_topic_with_options(topic, project, false, TopicMemoryOrder::CreatedAtDesc)?;
        if memories.len() > limit {
            memories.truncate(limit);
        }
        Ok(memories)
    }

    fn build_lessons_for_category<F>(
        &self,
        memories: &[hyphae_core::Memory],
        category: LessonCategory,
        id_prefix: &str,
        source_topic: &str,
        group_key: F,
        next_id: &mut usize,
    ) -> Vec<LessonRecord>
    where
        F: Fn(&hyphae_core::Memory) -> String,
    {
        let mut groups: BTreeMap<String, LessonGroup> = BTreeMap::new();
        for memory in memories {
            let entry = groups
                .entry(group_key(memory))
                .or_insert_with(|| LessonGroup {
                    description: memory.summary.clone(),
                    frequency: 0,
                    keywords: memory.keywords.clone(),
                });
            entry.frequency += 1;
        }

        groups
            .into_values()
            .map(|group| {
                let id = format!("{id_prefix}-{}", *next_id);
                *next_id += 1;
                LessonRecord {
                    id,
                    category,
                    description: group.description,
                    frequency: group.frequency,
                    source_topics: vec![source_topic.to_string()],
                    keywords: group.keywords,
                }
            })
            .collect()
    }

    fn query_count<P>(&self, sql: &str, params: P) -> HyphaeResult<usize>
    where
        P: rusqlite::Params,
    {
        self.conn
            .query_row(sql, params, |row| row.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(format!("failed to execute count query: {e}")))
    }

    fn query_top_topics(&self, project: Option<&str>) -> HyphaeResult<Vec<TopTopicAnalytics>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT topic, COUNT(*) AS count, COALESCE(AVG(weight), 0.0) AS avg_weight, MAX(created_at) AS latest_created_at
                 FROM memories
                 WHERE project = ?1 OR ?1 IS NULL
                 GROUP BY topic
                 ORDER BY count DESC, topic ASC
                 LIMIT 10",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare top topics query: {e}")))?;

        let rows = stmt
            .query_map(params![project], |row| {
                Ok(TopTopicAnalytics {
                    name: row.get(0)?,
                    count: row.get::<_, i64>(1).map(|n| n as usize)?,
                    avg_weight: row.get(2)?,
                    latest_created_at: row.get(3)?,
                })
            })
            .map_err(|e| HyphaeError::Database(format!("failed to query top topics: {e}")))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(format!("failed to collect top topics: {e}")))
    }

    fn query_importance_distribution(
        &self,
        project: Option<&str>,
    ) -> HyphaeResult<ImportanceDistribution> {
        let mut distribution = ImportanceDistribution {
            critical: 0,
            ephemeral: 0,
            high: 0,
            low: 0,
            medium: 0,
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT importance, COUNT(*)
                 FROM memories
                 WHERE project = ?1 OR ?1 IS NULL
                 GROUP BY importance",
            )
            .map_err(|e| {
                HyphaeError::Database(format!(
                    "failed to prepare importance distribution query: {e}"
                ))
            })?;

        let rows = stmt
            .query_map(params![project], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1).map(|n| n as usize)?,
                ))
            })
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query importance distribution: {e}"))
            })?;

        for row in rows {
            let (importance, count) = row.map_err(|e| {
                HyphaeError::Database(format!("failed to read importance distribution row: {e}"))
            })?;
            match importance.as_str() {
                "critical" => distribution.critical = count,
                "ephemeral" => distribution.ephemeral = count,
                "high" => distribution.high = count,
                "low" => distribution.low = count,
                "medium" => distribution.medium = count,
                _ => {}
            }
        }

        Ok(distribution)
    }
}

fn correction_group_key(memory: &hyphae_core::Memory) -> String {
    if memory.keywords.is_empty() {
        truncate_chars(&memory.summary, 20)
    } else {
        memory
            .keywords
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join("|")
    }
}

fn secondary_group_key(memory: &hyphae_core::Memory) -> String {
    memory
        .keywords
        .first()
        .cloned()
        .unwrap_or_else(|| truncate_chars(&memory.summary, 30))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{
        Concept, ConceptLink, Importance, Memoir, MemoirStore, Memory, MemoryStore, Relation,
    };

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn store_memory(
        store: &SqliteStore,
        topic: &str,
        summary: &str,
        importance: Importance,
        keywords: &[&str],
        project: Option<&str>,
    ) {
        let mut builder = Memory::builder(topic.to_string(), summary.to_string(), importance)
            .keywords(keywords.iter().map(|value| (*value).to_string()).collect());
        if let Some(project) = project {
            builder = builder.project(project.to_string());
        }
        store.store(builder.build()).unwrap();
    }

    #[test]
    fn test_extract_lessons_matches_cap_shape() {
        let store = test_store();
        store_memory(
            &store,
            "corrections",
            "Fix duplicate lesson extraction",
            Importance::Medium,
            &["lesson", "extraction"],
            Some("cap"),
        );
        store_memory(
            &store,
            "corrections",
            "Fix duplicate lesson extraction again",
            Importance::Medium,
            &["lesson", "extraction"],
            Some("cap"),
        );
        store_memory(
            &store,
            "errors/resolved",
            "Resolved session parser panic",
            Importance::High,
            &["parser"],
            Some("cap"),
        );
        store_memory(
            &store,
            "tests/resolved",
            "Fixed flaky analytics contract test",
            Importance::High,
            &["analytics"],
            Some("cap"),
        );

        let lessons = store.extract_lessons(Some("cap"), 50).unwrap();

        assert_eq!(lessons.len(), 3);
        assert_eq!(lessons[0].category, LessonCategory::Corrections);
        assert_eq!(lessons[0].frequency, 2);
        assert_eq!(lessons[0].source_topics, vec!["corrections".to_string()]);
        assert_eq!(
            lessons[0].keywords,
            vec!["lesson".to_string(), "extraction".to_string()]
        );
        assert!(lessons[0].id.starts_with("correction-"));

        let error_lesson = lessons
            .iter()
            .find(|lesson| lesson.category == LessonCategory::Errors)
            .unwrap();
        assert_eq!(
            error_lesson.source_topics,
            vec!["errors/resolved".to_string()]
        );

        let test_lesson = lessons
            .iter()
            .find(|lesson| lesson.category == LessonCategory::Tests)
            .unwrap();
        assert_eq!(
            test_lesson.source_topics,
            vec!["tests/resolved".to_string()]
        );
    }

    #[test]
    fn test_extract_lessons_respects_project_scope() {
        let store = test_store();
        store_memory(
            &store,
            "corrections",
            "Cap-only lesson",
            Importance::Medium,
            &["cap"],
            Some("cap"),
        );
        store_memory(
            &store,
            "corrections",
            "Cap-only lesson repeated",
            Importance::Medium,
            &["cap"],
            Some("cap"),
        );
        store_memory(
            &store,
            "corrections",
            "Hyphae-only lesson",
            Importance::Medium,
            &["hyphae"],
            Some("hyphae"),
        );
        store_memory(
            &store,
            "corrections",
            "Hyphae-only lesson repeated",
            Importance::Medium,
            &["hyphae"],
            Some("hyphae"),
        );

        let cap_lessons = store.extract_lessons(Some("cap"), 50).unwrap();
        let global_lessons = store.extract_lessons(None, 50).unwrap();

        assert_eq!(cap_lessons.len(), 1);
        assert_eq!(cap_lessons[0].frequency, 2);
        assert_eq!(global_lessons.len(), 2);
    }

    #[test]
    fn test_analytics_snapshot_matches_current_cap_contract() {
        let store = test_store();
        store_memory(
            &store,
            "lessons",
            "High-priority lesson",
            Importance::High,
            &["lesson"],
            Some("cap"),
        );
        store_memory(
            &store,
            "errors/resolved",
            "Critical fix",
            Importance::Critical,
            &["fix"],
            Some("cap"),
        );
        store_memory(
            &store,
            "docs",
            "Ephemeral note",
            Importance::Ephemeral,
            &["note"],
            Some("cap"),
        );

        store
            .conn
            .execute(
                "UPDATE memories SET access_count = 1, weight = 0.2 WHERE topic = 'lessons'",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET access_count = 3, weight = 0.9 WHERE topic = 'errors/resolved'",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET access_count = 0, weight = 0.4 WHERE topic = 'docs'",
                [],
            )
            .unwrap();

        let memoir = Memoir::new("code:cap".to_string(), "Cap knowledge".to_string());
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let concept_a = Concept::new(
            memoir_id.clone(),
            "analytics".to_string(),
            "Analytics page".to_string(),
        );
        let concept_a_id = concept_a.id.clone();
        store.add_concept(concept_a).unwrap();

        let concept_b = Concept::new(memoir_id, "lessons".to_string(), "Lessons page".to_string());
        let concept_b_id = concept_b.id.clone();
        store.add_concept(concept_b).unwrap();

        store
            .add_link(ConceptLink::new(
                concept_a_id,
                concept_b_id,
                Relation::DependsOn,
            ))
            .unwrap();

        let analytics = store.analytics_snapshot(Some("cap")).unwrap();

        assert_eq!(analytics.importance_distribution.critical, 1);
        assert_eq!(analytics.importance_distribution.high, 1);
        assert_eq!(analytics.importance_distribution.ephemeral, 1);
        assert_eq!(analytics.memory_utilization.total, 3);
        assert_eq!(analytics.memory_utilization.recalled, 2);
        assert!(analytics.memory_utilization.rate > 0.6);
        assert_eq!(analytics.lifecycle.decayed, 1);
        assert_eq!(analytics.lifecycle.pruned, 0);
        assert_eq!(analytics.memoir_stats.code_memoirs, 1);
        assert_eq!(analytics.memoir_stats.total, 1);
        assert_eq!(analytics.memoir_stats.total_concepts, 2);
        assert_eq!(analytics.memoir_stats.total_links, 1);
        assert_eq!(analytics.search_stats, None);
        assert_eq!(analytics.top_topics.len(), 3);
    }

    #[test]
    fn test_activity_snapshot_matches_status_shape() {
        let store = test_store();
        store_memory(
            &store,
            "session/start",
            "Started Codex worker session",
            Importance::Medium,
            &["host:codex"],
            Some("cap"),
        );
        store_memory(
            &store,
            "session/end",
            "Ended Codex worker session",
            Importance::Medium,
            &["host:codex"],
            Some("cap"),
        );
        store_memory(
            &store,
            "notes",
            "General project note",
            Importance::Low,
            &["host:claude"],
            Some("cap"),
        );

        let memoir = Memoir::new("code:cap".to_string(), "Cap knowledge".to_string());
        store.create_memoir(memoir).unwrap();

        let snapshot = store.activity_snapshot(Some("cap")).unwrap();

        assert_eq!(snapshot.memories, 3);
        assert_eq!(snapshot.memoirs, 1);
        assert_eq!(snapshot.activity.codex_memory_count, 2);
        assert_eq!(snapshot.activity.recent_session_memory_count, 2);
        assert_eq!(
            snapshot.activity.last_session_topic.as_deref(),
            Some("session/end")
        );
        assert!(snapshot.activity.last_codex_memory_at.is_some());
        assert!(snapshot.activity.last_session_memory_at.is_some());
    }
}
