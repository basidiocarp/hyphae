use hyphae_core::Memory;

pub(crate) fn print_memory(memory: &Memory, score: Option<f32>) {
    if let Some(s) = score {
        print!("[{s:.3}] ");
    }
    println!(
        "[{}] [{}] {}",
        memory.importance, memory.topic, memory.summary
    );
    if let Some(p) = &memory.project {
        println!("  project: {p}");
    }
    if let Some(branch) = &memory.branch {
        println!("  branch: {branch}");
    }
    if let Some(worktree) = &memory.worktree {
        println!("  worktree: {worktree}");
    }
    if let Some(invalidated_at) = memory.invalidated_at {
        println!(
            "  invalidated_at: {}",
            invalidated_at.format("%Y-%m-%d %H:%M:%S UTC")
        );
    }
    if let Some(reason) = &memory.invalidation_reason {
        println!("  invalidation_reason: {reason}");
    }
    if let Some(superseded_by) = &memory.superseded_by {
        println!("  superseded_by: {superseded_by}");
    }
}

/// Truncate `s` to at most `max` bytes, appending `…` if truncated.
/// Truncation respects UTF-8 character boundaries.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= max)
        .last()
        .unwrap_or(max);
    format!("{}…", &s[..end])
}
