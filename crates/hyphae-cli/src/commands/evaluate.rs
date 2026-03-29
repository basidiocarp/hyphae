use anyhow::Result;
use hyphae_store::{EvaluationWindow, SqliteStore, collect_evaluation_window};

// ─────────────────────────────────────────────────────────────────────────────
// Trend Detection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trend {
    Better,
    Worse,
    Stable,
}

impl std::fmt::Display for Trend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trend::Better => write!(f, "↓ {:.0}% better", 1.0), // placeholder
            Trend::Worse => write!(f, "↑ {:.0}% worse", 1.0),   // placeholder
            Trend::Stable => write!(f, "→ stable"),
        }
    }
}

fn calculate_trend(previous: f64, recent: f64, lower_is_better: bool) -> (Trend, f64) {
    if previous == 0.0 {
        return (Trend::Stable, 0.0);
    }

    let delta = ((recent - previous) / previous) * 100.0;
    let trend = if delta.abs() < 2.0 {
        Trend::Stable
    } else if lower_is_better {
        if delta < 0.0 {
            Trend::Better
        } else {
            Trend::Worse
        }
    } else if delta > 0.0 {
        Trend::Better
    } else {
        Trend::Worse
    };

    (trend, delta.abs())
}

fn collect_window_data(
    store: &SqliteStore,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> Result<EvaluationWindow> {
    Ok(collect_evaluation_window(
        store,
        days_ago_start,
        days_ago_end,
        project,
    )?)
}

// ─────────────────────────────────────────────────────────────────────────────
// Report Generation
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn cmd_evaluate(store: &SqliteStore, days: i64, project: Option<String>) -> Result<()> {
    let project_ref = project.as_deref();

    if days < 2 {
        anyhow::bail!("--days must be at least 2 (to compare two windows)");
    }

    let midpoint = days / 2;
    let previous_window_days = days - midpoint;

    // Collect data for both windows
    let recent_window = collect_window_data(store, 0, midpoint, project_ref)?;
    let previous_window = collect_window_data(store, midpoint, days, project_ref)?;

    // Check if we have enough data
    if recent_window.session_count < 1 && previous_window.session_count < 1 {
        println!("Insufficient data: no sessions found in the evaluation window");
        println!(
            "Metrics require at least 1 session per window. Try extending --days or checking that structured sessions are being recorded."
        );
        return Ok(());
    }

    // Calculate trends
    let (error_trend, error_pct) = calculate_trend(
        previous_window.error_rate(),
        recent_window.error_rate(),
        true,
    );
    let (correction_trend, correction_pct) = calculate_trend(
        previous_window.correction_rate(),
        recent_window.correction_rate(),
        true,
    );
    let (resolution_trend, resolution_pct) = calculate_trend(
        previous_window.resolution_rate(),
        recent_window.resolution_rate(),
        false,
    );
    let (test_trend, test_pct) = calculate_trend(
        previous_window.test_fix_rate(),
        recent_window.test_fix_rate(),
        false,
    );
    let (utilization_trend, utilization_pct) = calculate_trend(
        previous_window.memory_utilization(),
        recent_window.memory_utilization(),
        false,
    );

    // Print report header
    let proj_name = project_ref.unwrap_or("all projects");
    println!("\nAgent Evaluation Report (last {days} days)");
    println!("Project: {}", proj_name);
    println!();

    // Print metrics table
    println!(
        "{:<25} {:>14} {:>14} Trend",
        "Metric",
        format!("Previous {}d", previous_window_days),
        format!("Recent {}d", midpoint)
    );
    println!(
        "{:<25} {:>14} {:>14} {}",
        "-".repeat(25),
        format!("{}-d", previous_window_days),
        format!("{}-d", midpoint),
        "-".repeat(30)
    );

    // Error rate
    println!(
        "{:<25} {:>14.2} {:>14.2} {}",
        "Errors per session",
        previous_window.error_rate(),
        recent_window.error_rate(),
        format_trend_with_pct(error_trend, error_pct, true)
    );

    // Correction rate
    println!(
        "{:<25} {:>14.2} {:>14.2} {}",
        "Self-corrections/session",
        previous_window.correction_rate(),
        recent_window.correction_rate(),
        format_trend_with_pct(correction_trend, correction_pct, true)
    );

    // Resolution rate
    println!(
        "{:<25} {:>13.0}% {:>13.0}% {}",
        "Error resolution rate",
        previous_window.resolution_rate() * 100.0,
        recent_window.resolution_rate() * 100.0,
        format_trend_with_pct(resolution_trend, resolution_pct, false)
    );

    // Test fix rate
    println!(
        "{:<25} {:>13.0}% {:>13.0}% {}",
        "Test fix rate",
        previous_window.test_fix_rate() * 100.0,
        recent_window.test_fix_rate() * 100.0,
        format_trend_with_pct(test_trend, test_pct, false)
    );

    // Memory utilization
    println!(
        "{:<25} {:>13.0}% {:>13.0}% {}",
        "Memory utilization",
        previous_window.memory_utilization(),
        recent_window.memory_utilization(),
        format_trend_with_pct(utilization_trend, utilization_pct, false)
    );

    // Session count
    println!(
        "{:<25} {:>14} {:>14}",
        "Sessions", previous_window.session_count, recent_window.session_count
    );

    println!();

    // Overall assessment
    let improving_count = [
        error_trend == Trend::Better,
        correction_trend == Trend::Better,
        resolution_trend == Trend::Better,
        test_trend == Trend::Better,
    ]
    .iter()
    .filter(|&&x| x)
    .count();

    let assessment = match improving_count {
        4 => "Excellent: All metrics improving",
        3 => "Good: Most metrics improving",
        2 => "Fair: Some improvement",
        1 => "Mixed: Limited improvement",
        _ => "Needs attention: Most metrics declining or stable",
    };

    println!("Overall: {}", assessment);

    Ok(())
}

fn format_trend_with_pct(trend: Trend, pct: f64, lower_is_better: bool) -> String {
    match trend {
        Trend::Better => {
            if lower_is_better {
                format!("↓ {:.0}% better", pct)
            } else {
                format!("↑ {:.0}% better", pct)
            }
        }
        Trend::Worse => {
            if lower_is_better {
                format!("↑ {:.0}% worse", pct)
            } else {
                format!("↓ {:.0}% worse", pct)
            }
        }
        Trend::Stable => "→ stable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::MemoryStore;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_collect_window_data_prefers_structured_sessions_and_signals() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("evaluate structured path"))
            .unwrap();

        let memory = hyphae_core::Memory::builder(
            "context/demo-project".to_string(),
            "recalled memory".to_string(),
            hyphae_core::Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let memory_id = store.store(memory).unwrap();

        store
            .log_recall_event(
                Some(&session_id),
                "structured recall",
                &[memory_id.to_string()],
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let window = collect_window_data(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.session_count, 1);
        assert_eq!(window.correction_count, 1);
        assert_eq!(window.resolved_test_count, 1);
        assert_eq!(window.recalled_memory_count, 1);
    }

    #[test]
    fn test_collect_window_data_falls_back_to_legacy_session_memories() {
        let store = test_store();
        let session_memory = hyphae_core::Memory::builder(
            "session/demo-project".to_string(),
            "Session completed. legacy summary".to_string(),
            hyphae_core::Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        store.store(session_memory).unwrap();

        let window = collect_window_data(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.session_count, 1);
        assert!(window.total_session_length > 0);
    }

    #[test]
    fn test_collect_window_data_without_project_uses_all_structured_sessions() {
        let store = test_store();

        let (first_session, _) = store.session_start("project-a", Some("session a")).unwrap();
        store
            .session_end(&first_session, Some("done a"), None, Some("0"))
            .unwrap();

        let (second_session, _) = store.session_start("project-b", Some("session b")).unwrap();
        store
            .log_outcome_signal(
                Some(&second_session),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("project-b"),
            )
            .unwrap();
        store
            .session_end(&second_session, Some("done b"), None, Some("0"))
            .unwrap();

        let window = collect_window_data(&store, 0, 1, None).unwrap();

        assert_eq!(window.session_count, 2);
        assert_eq!(window.correction_count, 1);
    }

    #[test]
    fn test_collect_window_data_preserves_legacy_counts_in_mixed_mode() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("demo-project", Some("structured session"))
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let legacy_session = hyphae_core::Memory::builder(
            "session/demo-project".to_string(),
            "Session completed. legacy summary".to_string(),
            hyphae_core::Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        store.store(legacy_session).unwrap();

        let window = collect_window_data(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.session_count, 2);
        assert!(window.total_session_length > 0);
    }
}
