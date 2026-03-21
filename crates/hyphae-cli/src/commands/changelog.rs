use chrono::{DateTime, Duration, Utc};

use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

pub fn cmd_changelog(
    store: &SqliteStore,
    days: i64,
    since: Option<String>,
    project: Option<String>,
) -> anyhow::Result<()> {
    let cutoff = if let Some(since_str) = since {
        DateTime::parse_from_rfc3339(&since_str)
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|_| {
                // Try parsing as YYYY-MM-DD HH:MM:SS
                chrono::NaiveDateTime::parse_from_str(&since_str, "%Y-%m-%d %H:%M:%S")
                    .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
            })
            .map_err(|_| anyhow::anyhow!("invalid date format: {since_str}"))?
    } else {
        Utc::now() - Duration::days(days)
    };

    let proj = project.as_deref().unwrap_or("default");

    println!("═══════════════════════════════════════════════════════════════════════════════");
    println!(
        "What happened ({} days from {})",
        days,
        cutoff.format("%Y-%m-%d")
    );
    println!("═══════════════════════════════════════════════════════════════════════════════\n");

    // Session memories (session/{project} topic)
    let session_topic = format!("session/{proj}");
    let session_memories = store
        .get_by_topic(&session_topic, project.as_deref())
        .unwrap_or_default();

    let recent_sessions: Vec<_> = session_memories
        .iter()
        .filter(|m| m.created_at > cutoff)
        .collect();

    if !recent_sessions.is_empty() {
        println!("Sessions:");
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

    // Resolved errors
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

    // Lessons learned (infer from corrected errors or explicit lessons topic)
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
        println!("Lessons learned:");
        for lesson in all_lessons.iter().take(5) {
            println!("  - {}", lesson.summary);
        }
        if all_lessons.len() > 5 {
            println!("  ... and {} more", all_lessons.len() - 5);
        }
        println!();
    }

    // Summary stats
    println!("───────────────────────────────────────────────────────────────────────────────");
    println!(
        "Total: {} sessions, {} errors resolved, {} lessons learned",
        recent_sessions.len(),
        recent_resolved.len(),
        all_lessons.len()
    );

    Ok(())
}
