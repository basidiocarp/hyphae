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
