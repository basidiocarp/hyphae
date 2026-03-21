use chrono::{DateTime, Duration, NaiveDate, Utc};
use std::collections::BTreeMap;

use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

fn parse_date_argument(since_str: &str, _default_days: i64) -> anyhow::Result<DateTime<Utc>> {
    // ─────────────────────────────────────────────────────────────────────────────
    // Parse various date formats
    // ─────────────────────────────────────────────────────────────────────────────

    match since_str {
        "yesterday" => Ok(Utc::now() - Duration::days(1)),
        "today" => {
            let today = Utc::now().date_naive();
            Ok(DateTime::<Utc>::from_naive_utc_and_offset(
                today.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            ))
        }
        "last-week" => Ok(Utc::now() - Duration::days(7)),
        "last-month" => Ok(Utc::now() - Duration::days(30)),
        _ => {
            // Try ISO 8601
            DateTime::parse_from_rfc3339(since_str)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    // Try YYYY-MM-DD HH:MM:SS
                    chrono::NaiveDateTime::parse_from_str(since_str, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
                })
                .or_else(|_| {
                    // Try YYYY-MM-DD
                    NaiveDate::parse_from_str(since_str, "%Y-%m-%d")
                        .map(|nd| {
                            DateTime::<Utc>::from_naive_utc_and_offset(
                                nd.and_hms_opt(0, 0, 0).unwrap(),
                                Utc,
                            )
                        })
                })
                .map_err(|_| {
                    anyhow::anyhow!(
                        "invalid date format: {}\nSupported: 'yesterday', 'today', 'last-week', 'last-month', or ISO 8601",
                        since_str
                    )
                })
        }
    }
}

pub fn cmd_changelog(
    store: &SqliteStore,
    days: i64,
    since: Option<String>,
    project: Option<String>,
) -> anyhow::Result<()> {
    let cutoff = if let Some(since_str) = since {
        parse_date_argument(&since_str, days)?
    } else {
        Utc::now() - Duration::days(days)
    };

    let proj = project.as_deref().unwrap_or("default");

    println!("═══════════════════════════════════════════════════════════════════════════════");
    println!("Changelog since {}", cutoff.format("%Y-%m-%d"));
    println!("═══════════════════════════════════════════════════════════════════════════════\n");

    // ─────────────────────────────────────────────────────────────────────────────
    // Sessions
    // ─────────────────────────────────────────────────────────────────────────────

    let session_topic = format!("session/{proj}");
    let session_memories = store
        .get_by_topic(&session_topic, project.as_deref())
        .unwrap_or_default();

    let recent_sessions: Vec<_> = session_memories
        .iter()
        .filter(|m| m.created_at > cutoff)
        .collect();

    if !recent_sessions.is_empty() {
        println!("Sessions: {}", recent_sessions.len());
        for session in &recent_sessions {
            let days_ago = (Utc::now() - session.created_at).num_days();
            let days_text = if days_ago == 0 {
                "today".to_string()
            } else if days_ago == 1 {
                "1 day ago".to_string()
            } else {
                format!("{} days ago", days_ago)
            };
            println!("  - {} ({})", session.summary, days_text);
        }
        println!();
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // All memories grouped by topic
    // ─────────────────────────────────────────────────────────────────────────────

    let all_topics = store
        .list_topics(project.as_deref())
        .unwrap_or_default();

    let mut topic_counts: BTreeMap<String, usize> = BTreeMap::new();

    for (topic, _) in all_topics {
        let memories = store
            .get_by_topic(&topic, project.as_deref())
            .unwrap_or_default();

        let recent_count = memories
            .iter()
            .filter(|m| m.created_at > cutoff)
            .count();

        if recent_count > 0 {
            topic_counts.insert(topic, recent_count);
        }
    }

    if !topic_counts.is_empty() {
        let total: usize = topic_counts.values().sum();
        println!("Memories: {} stored", total);
        for (topic, count) in topic_counts.iter() {
            println!("  {}: {}", topic, count);
        }
        println!();
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Errors resolved
    // ─────────────────────────────────────────────────────────────────────────────

    let resolved_errors = store
        .get_by_topic("errors/resolved", project.as_deref())
        .unwrap_or_default();

    let recent_resolved: Vec<_> = resolved_errors
        .iter()
        .filter(|m| m.created_at > cutoff)
        .collect();

    if !recent_resolved.is_empty() {
        println!("Errors resolved: {}", recent_resolved.len());
        for error in recent_resolved.iter().take(5) {
            println!("  - {}", error.summary);
        }
        if recent_resolved.len() > 5 {
            println!("  ... and {} more", recent_resolved.len() - 5);
        }
        println!();
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Lessons learned
    // ─────────────────────────────────────────────────────────────────────────────

    let corrections = store
        .get_by_topic("corrections", project.as_deref())
        .unwrap_or_default();

    let lessons = store
        .get_by_topic("lessons", project.as_deref())
        .unwrap_or_default();

    let all_lessons: Vec<_> = corrections
        .iter()
        .chain(lessons.iter())
        .filter(|m| m.created_at > cutoff)
        .collect();

    if !all_lessons.is_empty() {
        println!("Lessons learned: {}", all_lessons.len());
        for lesson in all_lessons.iter().take(5) {
            println!("  - {}", lesson.summary);
        }
        if all_lessons.len() > 5 {
            println!("  ... and {} more", all_lessons.len() - 5);
        }
    }

    Ok(())
}
