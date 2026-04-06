use serde_json::Value;

use hyphae_store::{SqliteStore, collect_evaluation_window};

use crate::protocol::ToolResult;

use super::super::get_bounded_i64;

pub(crate) fn tool_evaluate(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let days = get_bounded_i64(args, "days", 14, 2, 365);
    let midpoint = days / 2;
    let previous_window_days = days - midpoint;
    let proj_name = project.unwrap_or("all projects");
    let recent_window = match collect_evaluation_window(store, 0, midpoint, project) {
        Ok(window) => window,
        Err(e) => {
            return ToolResult::error(format!("failed to collect recent evaluation window: {e}"));
        }
    };
    let previous_window = match collect_evaluation_window(store, midpoint, days, project) {
        Ok(window) => window,
        Err(e) => {
            return ToolResult::error(format!("failed to collect previous evaluation window: {e}"));
        }
    };

    if recent_window.session_count == 0 && previous_window.session_count == 0 {
        return ToolResult::text(
            "Insufficient data: no sessions found in the evaluation window. \
            Metrics require at least 1 session per window. Try extending --days or checking that structured sessions are being recorded."
                .into(),
        );
    }
    let recent_error_rate = recent_window.error_rate();
    let previous_error_rate = previous_window.error_rate();
    let recent_correction_rate = recent_window.correction_rate();
    let previous_correction_rate = previous_window.correction_rate();
    let recent_resolution_rate = recent_window.resolution_rate();
    let previous_resolution_rate = previous_window.resolution_rate();
    let recent_test_rate = recent_window.test_fix_rate();
    let previous_test_rate = previous_window.test_fix_rate();

    let trend_improving = |prev: f64, recent: f64, lower_is_better: bool| -> (bool, f64) {
        if prev == 0.0 {
            return (false, 0.0);
        }
        let delta = ((recent - prev) / prev).abs();
        let improving = if lower_is_better {
            recent < prev
        } else {
            recent > prev
        };
        (improving, delta * 100.0)
    };

    let (error_improving, error_pct) =
        trend_improving(previous_error_rate, recent_error_rate, true);
    let (correction_improving, correction_pct) =
        trend_improving(previous_correction_rate, recent_correction_rate, true);
    let (resolution_improving, resolution_pct) =
        trend_improving(previous_resolution_rate, recent_resolution_rate, false);
    let (test_improving, test_pct) = trend_improving(previous_test_rate, recent_test_rate, false);

    let mut output = String::new();
    output.push_str(&format!("\nAgent Evaluation Report (last {days} days)\n"));
    output.push_str(&format!("Project: {}\n\n", proj_name));
    output.push_str(&format!(
        "{:<25} {:>14} {:>14} {}\n",
        "Metric",
        format!("Previous {}d", previous_window_days),
        format!("Recent {}d", midpoint),
        "Trend"
    ));
    output.push_str(&format!(
        "{:<25} {:>14} {:>14} {}\n",
        "-".repeat(25),
        "-".repeat(14),
        "-".repeat(14),
        "-".repeat(30)
    ));

    output.push_str(&format!(
        "{:<25} {:>14.2} {:>14.2} {}\n",
        "Errors per session",
        previous_error_rate,
        recent_error_rate,
        if error_improving {
            format!("↓ {:.0}% better", error_pct)
        } else {
            format!("↑ {:.0}% worse", error_pct)
        }
    ));

    output.push_str(&format!(
        "{:<25} {:>14.2} {:>14.2} {}\n",
        "Self-corrections/session",
        previous_correction_rate,
        recent_correction_rate,
        if correction_improving {
            format!("↓ {:.0}% better", correction_pct)
        } else {
            format!("↑ {:.0}% worse", correction_pct)
        }
    ));

    output.push_str(&format!(
        "{:<25} {:>13.0}% {:>13.0}% {}\n",
        "Error resolution rate",
        previous_resolution_rate * 100.0,
        recent_resolution_rate * 100.0,
        if resolution_improving {
            format!("↑ {:.0}% better", resolution_pct)
        } else {
            format!("↓ {:.0}% worse", resolution_pct)
        }
    ));

    output.push_str(&format!(
        "{:<25} {:>13.0}% {:>13.0}% {}\n",
        "Test fix rate",
        previous_test_rate * 100.0,
        recent_test_rate * 100.0,
        if test_improving {
            format!("↑ {:.0}% better", test_pct)
        } else {
            format!("↓ {:.0}% worse", test_pct)
        }
    ));

    output.push_str(&format!(
        "{:<25} {:>14} {:>14}\n",
        "Sessions", previous_window.session_count, recent_window.session_count
    ));

    output.push('\n');

    let improving_count = [
        error_improving,
        correction_improving,
        resolution_improving,
        test_improving,
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

    output.push_str(&format!("Overall: {}\n", assessment));

    ToolResult::text(output)
}
