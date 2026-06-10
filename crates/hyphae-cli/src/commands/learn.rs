use anyhow::Result;
use hyphae_store::{LessonRecord, SqliteStore};
use std::path::PathBuf;

pub(crate) fn cmd_learn(
    store: &SqliteStore,
    project: Option<String>,
    limit: usize,
    target: Option<PathBuf>,
    apply: bool,
) -> Result<()> {
    let lessons = store.extract_lessons(project.as_deref(), limit)?;

    if lessons.is_empty() {
        println!("No lessons to add.");
        return Ok(());
    }

    let formatted = format_lessons(&lessons);

    if apply {
        // target is guaranteed Some when apply is true (clap requires = "target"),
        // but handle None defensively.
        let target_path =
            target.ok_or_else(|| anyhow::anyhow!("--target is required when --apply is used"))?;

        // Read existing content, if file exists.
        let existing = std::fs::read_to_string(&target_path).unwrap_or_default();

        // Build full content: existing + separator + formatted block.
        let full_content = if existing.is_empty() {
            formatted
        } else {
            format!("{existing}\n\n{formatted}")
        };

        // Atomic write: temp file + rename.
        let parent = target_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let file_name = target_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("--target must name a file, not a directory or .."))?
            .to_string_lossy();
        let temp_name = format!(".{}.{}.tmp", file_name, std::process::id());
        let temp_path = parent.join(temp_name);

        std::fs::write(&temp_path, &full_content)?;
        std::fs::rename(&temp_path, &target_path)?;

        let lesson_count = lessons.len();
        println!(
            "Added {lesson_count} lesson(s) to {}",
            target_path.display()
        );
    } else {
        // Preview to stdout.
        let target_label = target.as_ref().map_or_else(
            || "instruction file".to_string(),
            |p| p.display().to_string(),
        );
        println!("# Proposed additions to {target_label}");
        println!();
        println!("{formatted}");
    }

    Ok(())
}

/// Format lessons as markdown blocks grouped by topic.
///
/// Each lesson becomes a block:
/// ```
/// ## Lessons: <topic>
///
/// - <lesson content>
/// ```
fn format_lessons(lessons: &[LessonRecord]) -> String {
    use std::collections::BTreeMap;

    // Group lessons by (first source_topic if available, or "general").
    let mut groups: BTreeMap<String, Vec<&LessonRecord>> = BTreeMap::new();

    for lesson in lessons {
        let topic = lesson
            .source_topics
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        groups.entry(topic).or_default().push(lesson);
    }

    // Build formatted blocks.
    let blocks: Vec<String> = groups
        .into_iter()
        .map(|(topic, lessons_in_topic)| {
            let items = lessons_in_topic
                .iter()
                .map(|l| format!("- {}", l.description))
                .collect::<Vec<_>>()
                .join("\n");

            format!("## Lessons: {topic}\n\n{items}")
        })
        .collect();

    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::LessonCategory;

    #[test]
    fn format_lessons_single_topic() {
        let lessons = vec![LessonRecord {
            id: "1".to_string(),
            category: LessonCategory::Corrections,
            description: "Always check for nil pointers before dereferencing".to_string(),
            frequency: 5,
            source_topics: vec!["memory-safety".to_string()],
            keywords: vec!["nil".to_string(), "dereference".to_string()],
        }];

        let result = format_lessons(&lessons);
        assert!(result.contains("## Lessons: memory-safety"));
        assert!(result.contains("- Always check for nil pointers before dereferencing"));
    }

    #[test]
    fn format_lessons_multiple_topics() {
        let lessons = vec![
            LessonRecord {
                id: "1".to_string(),
                category: LessonCategory::Errors,
                description: "Handle all error cases explicitly".to_string(),
                frequency: 3,
                source_topics: vec!["error-handling".to_string()],
                keywords: vec!["error".to_string()],
            },
            LessonRecord {
                id: "2".to_string(),
                category: LessonCategory::Tests,
                description: "Write tests for edge cases".to_string(),
                frequency: 2,
                source_topics: vec!["testing".to_string()],
                keywords: vec!["test".to_string()],
            },
        ];

        let result = format_lessons(&lessons);
        assert!(result.contains("## Lessons: error-handling"));
        assert!(result.contains("## Lessons: testing"));
        assert!(result.contains("- Handle all error cases explicitly"));
        assert!(result.contains("- Write tests for edge cases"));
    }

    #[test]
    fn format_lessons_no_source_topics() {
        let lessons = vec![LessonRecord {
            id: "1".to_string(),
            category: LessonCategory::Corrections,
            description: "General best practice".to_string(),
            frequency: 1,
            source_topics: vec![],
            keywords: vec![],
        }];

        let result = format_lessons(&lessons);
        assert!(result.contains("## Lessons: general"));
        assert!(result.contains("- General best practice"));
    }
}
