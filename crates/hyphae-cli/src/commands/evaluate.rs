use anyhow::Result;
use chrono::Utc;
use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation Metrics Structure
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct EvaluationWindow {
    error_count: usize,
    correction_count: usize,
    resolved_count: usize,
    failed_test_count: usize,
    resolved_test_count: usize,
    #[allow(dead_code)]
    total_session_length: usize,
    session_count: usize,
    recalled_memory_count: usize,
}

impl EvaluationWindow {
    fn error_rate(&self) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        self.error_count as f64 / self.session_count as f64
    }

    fn correction_rate(&self) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        self.correction_count as f64 / self.session_count as f64
    }

    fn resolution_rate(&self) -> f64 {
        let total = self.error_count + self.resolved_count;
        if total == 0 {
            return 0.0;
        }
        self.resolved_count as f64 / total as f64
    }

    fn test_fix_rate(&self) -> f64 {
        let total = self.failed_test_count + self.resolved_test_count;
        if total == 0 {
            return 0.0;
        }
        self.resolved_test_count as f64 / total as f64
    }

    fn memory_utilization(&self) -> f64 {
        // For evaluation purposes, we approximate memory utilization
        // as the ratio of memories that were accessed (inferred from count)
        if self.session_count == 0 {
            return 0.0;
        }
        (self.recalled_memory_count as f64 / (self.recalled_memory_count + 5) as f64) * 100.0
    }
}

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

// ─────────────────────────────────────────────────────────────────────────────
// Data Collection
// ─────────────────────────────────────────────────────────────────────────────

fn get_memories_in_window(
    store: &SqliteStore,
    topic_pattern: &str,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> Result<Vec<hyphae_core::Memory>> {
    // Get all memories in the topic and filter by created_at
    let all_memories = store.get_by_topic(topic_pattern, project)?;

    let cutoff_start = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_start))
        .unwrap_or(Utc::now());
    let cutoff_end = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_end))
        .unwrap_or(Utc::now());

    let filtered = all_memories
        .into_iter()
        .filter(|m| m.created_at >= cutoff_end && m.created_at <= cutoff_start)
        .collect();

    Ok(filtered)
}

fn collect_window_data(
    store: &SqliteStore,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> Result<EvaluationWindow> {
    // Count errors in the window
    let errors = get_memories_in_window(
        store,
        "errors/active",
        days_ago_start,
        days_ago_end,
        project,
    )?;
    let error_count = errors.len();

    // Count corrections
    let corrections =
        get_memories_in_window(store, "corrections", days_ago_start, days_ago_end, project)?;
    let correction_count = corrections.len();

    // Count resolved errors
    let resolved = get_memories_in_window(
        store,
        "errors/resolved",
        days_ago_start,
        days_ago_end,
        project,
    )?;
    let resolved_count = resolved.len();

    // Count failed tests
    let failed_tests =
        get_memories_in_window(store, "tests/failed", days_ago_start, days_ago_end, project)?;
    let failed_test_count = failed_tests.len();

    // Count resolved tests
    let resolved_tests = get_memories_in_window(
        store,
        "tests/resolved",
        days_ago_start,
        days_ago_end,
        project,
    )?;
    let resolved_test_count = resolved_tests.len();

    // Count sessions in the window
    let proj_name = project.unwrap_or("default");
    let session_topic = format!("session/{}", proj_name);
    let sessions =
        get_memories_in_window(store, &session_topic, days_ago_start, days_ago_end, project)?;
    let session_count = sessions.len();

    // Calculate total session length (proxy for complexity)
    let total_session_length: usize = sessions.iter().map(|s| s.summary.len()).sum();

    // Count memories with access_count > 0 (recalled)
    let all_memories: Vec<hyphae_core::Memory> = {
        let topics = store.list_topics(project)?;
        let mut all = Vec::new();
        for (t, _) in &topics {
            let mems = store.get_by_topic(t, project)?;
            all.extend(mems);
        }
        all
    };

    let recalled_memory_count = all_memories.iter().filter(|m| m.access_count > 0).count();

    Ok(EvaluationWindow {
        error_count,
        correction_count,
        resolved_count,
        failed_test_count,
        resolved_test_count,
        total_session_length,
        session_count,
        recalled_memory_count,
    })
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

    // Collect data for both windows
    let recent_window = collect_window_data(store, 0, midpoint, project_ref)?;
    let previous_window = collect_window_data(store, midpoint, days, project_ref)?;

    // Check if we have enough data
    if recent_window.session_count < 1 && previous_window.session_count < 1 {
        println!("Insufficient data: no sessions found in the evaluation window");
        println!(
            "Metrics require at least 1 session per window. Try extending --days or checking that session memories exist."
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
    let proj_name = project_ref.unwrap_or("default");
    println!("\nAgent Evaluation Report (last {days} days)");
    println!("Project: {}", proj_name);
    println!();

    // Print metrics table
    println!(
        "{:<25} {:>14} {:>14} {}",
        "Metric", "Previous {}", "Recent {}", "Trend"
    );
    println!(
        "{:<25} {:>14} {:>14} {}",
        "-".repeat(25),
        format!("{}-d", midpoint),
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
