use std::sync::OnceLock;

use chrono::Utc;
use hyphae_core::chunk::{Chunk, ChunkMetadata, SourceType};
use hyphae_core::ids::{ChunkId, DocumentId};
use regex::Regex;

fn heading_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^(#{1,6})\s+(.+)$").expect("heading regex is valid"))
}

fn build_error_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^(error\[|warning\[|error:)").expect("build error regex is valid")
    })
}

fn diff_header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^diff --git").expect("diff header regex is valid"))
}

fn test_boundary_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^(---- .+ ----|FAIL|test .+\.\.\.)").expect("test boundary regex is valid")
    })
}

fn log_timestamp_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}")
            .expect("log timestamp regex is valid")
    })
}

/// The type of structured command output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputType {
    TestResult,
    BuildError,
    Diff,
    Log,
    Generic,
}

/// Detect the output type from content by scanning for characteristic patterns.
pub fn detect_output_type(content: &str) -> OutputType {
    for line in content.lines().take(50) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("error[E") || trimmed.starts_with("error:") {
            return OutputType::BuildError;
        }
        if trimmed.starts_with("diff --git") {
            return OutputType::Diff;
        }
        if trimmed.starts_with("test result:")
            || trimmed.starts_with("FAIL")
            || (trimmed.starts_with("---- ") && trimmed.ends_with(" ----"))
        {
            return OutputType::TestResult;
        }
        if log_timestamp_regex().is_match(trimmed) {
            return OutputType::Log;
        }
    }
    OutputType::Generic
}

#[derive(Debug, Clone)]
pub enum ChunkStrategy {
    SlidingWindow {
        size: usize,
        overlap: usize,
    },
    ByHeading {
        max_tokens: usize,
    },
    ByFunction {
        language: String,
    },
    ByStructuredOutput {
        output_type: OutputType,
        max_tokens: usize,
    },
}

impl ChunkStrategy {
    pub fn for_source_type(source_type: &SourceType) -> Self {
        match source_type {
            SourceType::Markdown => ChunkStrategy::ByHeading { max_tokens: 500 },
            SourceType::Code => ChunkStrategy::ByFunction {
                language: "generic".into(),
            },
            _ => ChunkStrategy::SlidingWindow {
                size: 500,
                overlap: 50,
            },
        }
    }
}

/// Chunk text content into `Chunk` values using the given strategy.
///
/// The `document_id` is set to a placeholder; callers should update it after
/// creating the parent `Document`.
pub fn chunk_text(content: &str, metadata: ChunkMetadata, strategy: ChunkStrategy) -> Vec<Chunk> {
    let placeholder_doc_id = DocumentId::from("pending");

    match strategy {
        ChunkStrategy::SlidingWindow { size, overlap } => {
            chunk_sliding_window(content, &metadata, &placeholder_doc_id, size, overlap)
        }
        ChunkStrategy::ByHeading { max_tokens } => {
            chunk_by_heading(content, &metadata, &placeholder_doc_id, max_tokens)
        }
        ChunkStrategy::ByFunction { ref language } => {
            chunk_by_function(content, &metadata, &placeholder_doc_id, language)
        }
        ChunkStrategy::ByStructuredOutput {
            ref output_type,
            max_tokens,
        } => chunk_structured_output(
            content,
            &metadata,
            &placeholder_doc_id,
            output_type,
            max_tokens,
        ),
    }
}

fn chunk_sliding_window(
    content: &str,
    metadata: &ChunkMetadata,
    doc_id: &DocumentId,
    size: usize,
    overlap: usize,
) -> Vec<Chunk> {
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let mut chunk_index: u32 = 0;

    while start < words.len() {
        let end = (start + size).min(words.len());
        let chunk_content = words[start..end].join(" ");

        chunks.push(Chunk {
            id: ChunkId::new(),
            document_id: doc_id.clone(),
            chunk_index,
            content: chunk_content,
            metadata: metadata.clone(),
            embedding: None,
            created_at: Utc::now(),
        });

        chunk_index += 1;

        if end >= words.len() {
            break;
        }

        let step = if size > overlap { size - overlap } else { 1 };
        start += step;
    }

    chunks
}

fn chunk_by_heading(
    content: &str,
    metadata: &ChunkMetadata,
    doc_id: &DocumentId,
    max_tokens: usize,
) -> Vec<Chunk> {
    let heading_re = heading_regex();

    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    let mut last_end = 0;

    for cap in heading_re.find_iter(content) {
        // Capture text before this heading as part of the previous section
        if cap.start() > last_end {
            let before = content[last_end..cap.start()].trim();
            if !before.is_empty() {
                if let Some(last) = sections.last_mut() {
                    last.1.push('\n');
                    last.1.push_str(before);
                } else {
                    sections.push((None, before.to_string()));
                }
            }
        }

        let full_match = cap.as_str();
        let heading_text = heading_re
            .captures(full_match)
            .and_then(|c| c.get(2))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();

        sections.push((Some(heading_text), String::new()));
        last_end = cap.end();
    }

    // Remaining content after the last heading
    if last_end < content.len() {
        let remaining = content[last_end..].trim();
        if !remaining.is_empty() {
            if let Some(last) = sections.last_mut() {
                if last.1.is_empty() {
                    last.1 = remaining.to_string();
                } else {
                    last.1.push('\n');
                    last.1.push_str(remaining);
                }
            } else {
                sections.push((None, remaining.to_string()));
            }
        }
    }

    let mut chunks = Vec::new();
    let mut chunk_index: u32 = 0;

    for (heading, section_content) in sections {
        if section_content.is_empty() {
            continue;
        }

        let token_count = section_content.split_whitespace().count();

        if token_count > max_tokens {
            // Recursively split oversized sections with SlidingWindow
            let mut section_meta = metadata.clone();
            section_meta.heading = heading;
            let sub_chunks = chunk_sliding_window(
                &section_content,
                &section_meta,
                doc_id,
                max_tokens,
                max_tokens / 10,
            );
            for mut sub in sub_chunks {
                sub.chunk_index = chunk_index;
                chunk_index += 1;
                chunks.push(sub);
            }
        } else {
            let mut chunk_meta = metadata.clone();
            chunk_meta.heading = heading;

            chunks.push(Chunk {
                id: ChunkId::new(),
                document_id: doc_id.clone(),
                chunk_index,
                content: section_content,
                metadata: chunk_meta,
                embedding: None,
                created_at: Utc::now(),
            });
            chunk_index += 1;
        }
    }

    chunks
}

fn chunk_by_function(
    content: &str,
    metadata: &ChunkMetadata,
    doc_id: &DocumentId,
    language: &str,
) -> Vec<Chunk> {
    let blocks: Vec<&str> = content.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut chunk_index: u32 = 0;

    for block in blocks {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut chunk_meta = metadata.clone();
        chunk_meta.language = Some(language.to_string());

        chunks.push(Chunk {
            id: ChunkId::new(),
            document_id: doc_id.clone(),
            chunk_index,
            content: trimmed.to_string(),
            metadata: chunk_meta,
            embedding: None,
            created_at: Utc::now(),
        });
        chunk_index += 1;
    }

    chunks
}

/// Chunk structured command output, splitting on output-type-specific boundaries.
fn chunk_structured_output(
    content: &str,
    metadata: &ChunkMetadata,
    doc_id: &DocumentId,
    output_type: &OutputType,
    max_tokens: usize,
) -> Vec<Chunk> {
    let sections = match output_type {
        OutputType::TestResult => split_test_results(content),
        OutputType::BuildError => split_build_errors(content),
        OutputType::Diff => split_diffs(content),
        OutputType::Log => split_log_entries(content),
        OutputType::Generic => split_generic(content, max_tokens),
    };

    sections_to_chunks(sections, metadata, doc_id, max_tokens)
}

/// Convert (content, heading) pairs into Chunk values, subdividing oversized sections.
fn sections_to_chunks(
    sections: Vec<(String, Option<String>)>,
    metadata: &ChunkMetadata,
    doc_id: &DocumentId,
    max_tokens: usize,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut chunk_index: u32 = 0;

    for (section_content, heading) in sections {
        if section_content.trim().is_empty() {
            continue;
        }

        let token_count = section_content.split_whitespace().count();

        if token_count > max_tokens {
            let mut section_meta = metadata.clone();
            section_meta.heading = heading;
            let sub_chunks = chunk_sliding_window(
                &section_content,
                &section_meta,
                doc_id,
                max_tokens,
                max_tokens / 10,
            );
            for mut sub in sub_chunks {
                sub.chunk_index = chunk_index;
                chunk_index += 1;
                chunks.push(sub);
            }
        } else {
            let mut chunk_meta = metadata.clone();
            chunk_meta.heading = heading;
            chunks.push(Chunk {
                id: ChunkId::new(),
                document_id: doc_id.clone(),
                chunk_index,
                content: section_content,
                metadata: chunk_meta,
                embedding: None,
                created_at: Utc::now(),
            });
            chunk_index += 1;
        }
    }

    chunks
}

/// Split test output on test boundaries, returning (content, test_name) pairs.
fn split_test_results(content: &str) -> Vec<(String, Option<String>)> {
    let re = test_boundary_regex();
    split_on_boundary_regex(content, re, |line| {
        let trimmed = line.trim();
        if trimmed.starts_with("---- ") && trimmed.ends_with(" ----") {
            Some(trimmed[5..trimmed.len() - 5].to_string())
        } else if trimmed.starts_with("FAIL") {
            Some(trimmed.to_string())
        } else if trimmed.starts_with("test ") {
            let name = trimmed
                .trim_start_matches("test ")
                .split("...")
                .next()
                .unwrap_or(trimmed);
            Some(name.trim().to_string())
        } else {
            None
        }
    })
}

/// Split build output on error/warning boundaries.
fn split_build_errors(content: &str) -> Vec<(String, Option<String>)> {
    let re = build_error_regex();
    split_on_boundary_regex(content, re, |line| {
        let trimmed = line.trim();
        // Extract error code like "error[E0308]" or first meaningful part of "error: ..."
        if let Some(bracket_end) = trimmed.find(']') {
            Some(trimmed[..=bracket_end].to_string())
        } else {
            let heading = trimmed.lines().next().unwrap_or(trimmed);
            Some(heading.chars().take(80).collect())
        }
    })
}

/// Split diff output on `diff --git` boundaries, heading = file path.
fn split_diffs(content: &str) -> Vec<(String, Option<String>)> {
    let re = diff_header_regex();
    split_on_boundary_regex(content, re, |line| {
        // Extract file path from "diff --git a/path b/path"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            Some(parts[3].trim_start_matches("b/").to_string())
        } else {
            Some(line.trim().to_string())
        }
    })
}

/// Split log output on blank lines, heading = first timestamp in group.
fn split_log_entries(content: &str) -> Vec<(String, Option<String>)> {
    let ts_re = log_timestamp_regex();
    let mut sections: Vec<(String, Option<String>)> = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut current_heading: Option<String> = None;

    for line in content.lines() {
        if line.trim().is_empty() {
            if !current_lines.is_empty() {
                sections.push((current_lines.join("\n"), current_heading.take()));
                current_lines = Vec::new();
            }
            continue;
        }

        if current_heading.is_none() {
            if let Some(m) = ts_re.find(line) {
                current_heading = Some(m.as_str().to_string());
            }
        }

        current_lines.push(line);
    }

    if !current_lines.is_empty() {
        sections.push((current_lines.join("\n"), current_heading.take()));
    }

    sections
}

/// Generic fallback: split into sliding window chunks, no headings.
fn split_generic(content: &str, max_tokens: usize) -> Vec<(String, Option<String>)> {
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }

    let size = max_tokens.min(500);
    let overlap = size / 10;
    let step = if size > overlap { size - overlap } else { 1 };
    let mut sections = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let end = (start + size).min(words.len());
        sections.push((words[start..end].join(" "), None));
        if end >= words.len() {
            break;
        }
        start += step;
    }

    sections
}

/// Generic helper: split content into sections at lines matching a boundary regex.
fn split_on_boundary_regex(
    content: &str,
    re: &Regex,
    extract_heading: impl Fn(&str) -> Option<String>,
) -> Vec<(String, Option<String>)> {
    let mut sections: Vec<(String, Option<String>)> = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut current_heading: Option<String> = None;

    for line in content.lines() {
        if re.is_match(line) {
            // Flush the previous section
            if !current_lines.is_empty() {
                sections.push((current_lines.join("\n"), current_heading.take()));
                current_lines = Vec::new();
            }
            current_heading = extract_heading(line);
        }
        current_lines.push(line);
    }

    if !current_lines.is_empty() {
        sections.push((current_lines.join("\n"), current_heading.take()));
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metadata() -> ChunkMetadata {
        ChunkMetadata {
            source_path: "test.txt".to_string(),
            source_type: SourceType::Text,
            language: None,
            heading: None,
            line_start: None,
            line_end: None,
        }
    }

    #[test]
    fn test_chunk_sliding_window() {
        // 10 words → size=4, overlap=2 → step=2
        let text = "one two three four five six seven eight nine ten";
        let meta = make_metadata();
        let chunks = chunk_text(
            text,
            meta,
            ChunkStrategy::SlidingWindow {
                size: 4,
                overlap: 2,
            },
        );

        assert!(chunks.len() >= 3, "expected at least 3 chunks");
        assert_eq!(chunks[0].content, "one two three four");
        // Second chunk overlaps by 2 words
        assert_eq!(chunks[1].content, "three four five six");
        // Verify sequential indices
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
        }
    }

    #[test]
    fn test_chunk_sliding_window_small_input() {
        let text = "hello world";
        let meta = make_metadata();
        let chunks = chunk_text(
            text,
            meta,
            ChunkStrategy::SlidingWindow {
                size: 500,
                overlap: 50,
            },
        );

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "hello world");
    }

    #[test]
    fn test_chunk_by_heading() {
        let markdown = "# Introduction\n\nThis is the intro.\n\n## Details\n\nSome details here.\n\n## Conclusion\n\nWrap up.";
        let meta = ChunkMetadata {
            source_path: "doc.md".to_string(),
            source_type: SourceType::Markdown,
            language: None,
            heading: None,
            line_start: None,
            line_end: None,
        };

        let chunks = chunk_text(markdown, meta, ChunkStrategy::ByHeading { max_tokens: 500 });

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].metadata.heading.as_deref(), Some("Introduction"));
        assert!(chunks[0].content.contains("This is the intro."));
        assert_eq!(chunks[1].metadata.heading.as_deref(), Some("Details"));
        assert!(chunks[1].content.contains("Some details here."));
        assert_eq!(chunks[2].metadata.heading.as_deref(), Some("Conclusion"));
        assert!(chunks[2].content.contains("Wrap up."));
    }

    #[test]
    fn test_chunk_by_heading_oversized_section() {
        // Create a section that exceeds max_tokens
        let big_section = (0..100).map(|i| format!("word{i}")).collect::<Vec<_>>();
        let markdown = format!("# Big Section\n\n{}", big_section.join(" "));
        let meta = ChunkMetadata {
            source_path: "big.md".to_string(),
            source_type: SourceType::Markdown,
            language: None,
            heading: None,
            line_start: None,
            line_end: None,
        };

        let chunks = chunk_text(&markdown, meta, ChunkStrategy::ByHeading { max_tokens: 20 });

        // Should be split into multiple sub-chunks
        assert!(
            chunks.len() > 1,
            "oversized section should produce multiple chunks"
        );
        assert_eq!(chunks[0].metadata.heading.as_deref(), Some("Big Section"));
    }

    #[test]
    fn test_chunk_by_function() {
        let code = "fn hello() {\n    println!(\"hello\");\n}\n\nfn world() {\n    println!(\"world\");\n}\n\nfn main() {\n    hello();\n    world();\n}";
        let meta = ChunkMetadata {
            source_path: "main.rs".to_string(),
            source_type: SourceType::Code,
            language: None,
            heading: None,
            line_start: None,
            line_end: None,
        };

        let chunks = chunk_text(
            code,
            meta,
            ChunkStrategy::ByFunction {
                language: "rust".into(),
            },
        );

        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].content.contains("fn hello()"));
        assert!(chunks[1].content.contains("fn world()"));
        assert!(chunks[2].content.contains("fn main()"));
        // All chunks should have language set
        for chunk in &chunks {
            assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        }
    }

    #[test]
    fn test_chunk_empty_content() {
        let meta = make_metadata();
        let chunks = chunk_text(
            "",
            meta,
            ChunkStrategy::SlidingWindow {
                size: 100,
                overlap: 10,
            },
        );
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_strategy_for_source_type() {
        assert!(matches!(
            ChunkStrategy::for_source_type(&SourceType::Markdown),
            ChunkStrategy::ByHeading { max_tokens: 500 }
        ));
        assert!(matches!(
            ChunkStrategy::for_source_type(&SourceType::Code),
            ChunkStrategy::ByFunction { .. }
        ));
        assert!(matches!(
            ChunkStrategy::for_source_type(&SourceType::Text),
            ChunkStrategy::SlidingWindow {
                size: 500,
                overlap: 50
            }
        ));
        assert!(matches!(
            ChunkStrategy::for_source_type(&SourceType::Pdf),
            ChunkStrategy::SlidingWindow {
                size: 500,
                overlap: 50
            }
        ));
    }

    // --- OutputType detection tests ---

    #[test]
    fn test_detect_build_error_with_error_code() {
        let content =
            "Compiling myapp v0.1.0\nerror[E0308]: mismatched types\n  --> src/main.rs:5:14";
        assert_eq!(detect_output_type(content), OutputType::BuildError);
    }

    #[test]
    fn test_detect_build_error_with_error_colon() {
        let content = "error: could not compile `myapp`\nSome other info";
        assert_eq!(detect_output_type(content), OutputType::BuildError);
    }

    #[test]
    fn test_detect_diff() {
        let content =
            "diff --git a/src/main.rs b/src/main.rs\nindex abc..def 100644\n--- a/src/main.rs";
        assert_eq!(detect_output_type(content), OutputType::Diff);
    }

    #[test]
    fn test_detect_test_result_fail() {
        let content = "running 5 tests\nFAIL test_something\ntest result: 1 passed; 1 failed";
        assert_eq!(detect_output_type(content), OutputType::TestResult);
    }

    #[test]
    fn test_detect_test_result_boundary() {
        let content =
            "running 2 tests\n---- test_alpha ----\nthread panicked at 'assertion failed'";
        assert_eq!(detect_output_type(content), OutputType::TestResult);
    }

    #[test]
    fn test_detect_log_iso_timestamp() {
        let content =
            "2024-01-15T10:30:00Z INFO starting up\n2024-01-15T10:30:01Z DEBUG loaded config";
        assert_eq!(detect_output_type(content), OutputType::Log);
    }

    #[test]
    fn test_detect_log_space_timestamp() {
        let content = "2024-01-15 10:30:00 INFO starting up\nsome other line";
        assert_eq!(detect_output_type(content), OutputType::Log);
    }

    #[test]
    fn test_detect_generic() {
        let content = "Hello world\nJust some plain text\nNothing special";
        assert_eq!(detect_output_type(content), OutputType::Generic);
    }

    // --- Structured output chunking tests ---

    #[test]
    fn test_chunk_test_results() {
        let content = "\
running 3 tests
---- test_alpha ----
thread 'test_alpha' panicked at 'assertion failed'
note: run with RUST_BACKTRACE=1
---- test_beta ----
thread 'test_beta' panicked at 'expected 42, got 0'
note: run with RUST_BACKTRACE=1
test result: 0 passed; 2 failed";

        let meta = make_metadata();
        let chunks = chunk_text(
            content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::TestResult,
                max_tokens: 500,
            },
        );

        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks for 2 test failures, got {}",
            chunks.len()
        );

        // First chunk should be preamble or first test
        let has_alpha = chunks.iter().any(|c| c.content.contains("test_alpha"));
        let has_beta = chunks.iter().any(|c| c.content.contains("test_beta"));
        assert!(has_alpha, "should contain test_alpha");
        assert!(has_beta, "should contain test_beta");

        // At least one chunk should have a heading with a test name
        let has_heading = chunks.iter().any(|c| c.metadata.heading.is_some());
        assert!(has_heading, "at least one chunk should have a heading");
    }

    #[test]
    fn test_chunk_build_errors() {
        let content = "\
error[E0308]: mismatched types
  --> src/main.rs:5:14
   |
5  |     let x: i32 = \"hello\";
   |                  ^^^^^^^ expected `i32`, found `&str`

error[E0425]: cannot find value `y` in this scope
  --> src/main.rs:10:5
   |
10 |     y + 1
   |     ^ not found in this scope";

        let meta = make_metadata();
        let chunks = chunk_text(
            content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::BuildError,
                max_tokens: 500,
            },
        );

        assert_eq!(
            chunks.len(),
            2,
            "expected 2 chunks for 2 errors, got {}",
            chunks.len()
        );
        assert!(chunks[0].content.contains("E0308"));
        assert!(chunks[1].content.contains("E0425"));

        // Headings should contain error codes
        assert!(
            chunks[0]
                .metadata
                .heading
                .as_deref()
                .unwrap_or("")
                .contains("error[E0308]"),
            "heading should contain error code, got: {:?}",
            chunks[0].metadata.heading
        );
    }

    #[test]
    fn test_chunk_diffs() {
        let content = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!(\"hello\");
 }
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
 pub fn add() {}
+pub fn sub() {}";

        let meta = make_metadata();
        let chunks = chunk_text(
            content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::Diff,
                max_tokens: 500,
            },
        );

        assert_eq!(
            chunks.len(),
            2,
            "expected 2 chunks for 2 diffs, got {}",
            chunks.len()
        );
        assert!(chunks[0].content.contains("src/main.rs"));
        assert!(chunks[1].content.contains("src/lib.rs"));

        // Heading should be the file path
        assert_eq!(chunks[0].metadata.heading.as_deref(), Some("src/main.rs"),);
        assert_eq!(chunks[1].metadata.heading.as_deref(), Some("src/lib.rs"),);
    }

    #[test]
    fn test_chunk_log_entries() {
        let content = "\
2024-01-15T10:30:00Z INFO starting up
2024-01-15T10:30:00Z DEBUG loading config

2024-01-15T10:30:05Z WARN disk space low
2024-01-15T10:30:05Z ERROR failed to write";

        let meta = make_metadata();
        let chunks = chunk_text(
            content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::Log,
                max_tokens: 500,
            },
        );

        assert_eq!(
            chunks.len(),
            2,
            "expected 2 log groups separated by blank line, got {}",
            chunks.len()
        );
        assert!(chunks[0].content.contains("starting up"));
        assert!(chunks[1].content.contains("disk space low"));

        // Heading should be timestamp
        assert!(
            chunks[0]
                .metadata
                .heading
                .as_deref()
                .unwrap_or("")
                .starts_with("2024-01-15"),
            "heading should start with timestamp"
        );
    }

    #[test]
    fn test_chunk_generic_fallback() {
        let words: Vec<String> = (0..100).map(|i| format!("word{i}")).collect();
        let content = words.join(" ");
        let meta = make_metadata();
        let chunks = chunk_text(
            &content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::Generic,
                max_tokens: 30,
            },
        );

        assert!(
            chunks.len() > 1,
            "100 words with max_tokens=30 should produce multiple chunks"
        );
        // Generic chunks should have no heading
        for chunk in &chunks {
            assert!(chunk.metadata.heading.is_none());
        }
    }

    #[test]
    fn test_chunk_structured_output_oversized_section() {
        // A single build error with lots of content that exceeds max_tokens
        let big_note: String = (0..200)
            .map(|i| format!("note{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let content = format!("error[E0308]: mismatched types\n  --> src/main.rs:5:14\n{big_note}");

        let meta = make_metadata();
        let chunks = chunk_text(
            &content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::BuildError,
                max_tokens: 50,
            },
        );

        assert!(
            chunks.len() > 1,
            "oversized section should be subdivided, got {} chunk(s)",
            chunks.len()
        );
    }

    #[test]
    fn test_chunk_structured_output_empty() {
        let meta = make_metadata();
        let chunks = chunk_text(
            "",
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::Generic,
                max_tokens: 500,
            },
        );
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_structured_sequential_indices() {
        let content = "\
diff --git a/a.rs b/a.rs
+line1
diff --git a/b.rs b/b.rs
+line2
diff --git a/c.rs b/c.rs
+line3";

        let meta = make_metadata();
        let chunks = chunk_text(
            content,
            meta,
            ChunkStrategy::ByStructuredOutput {
                output_type: OutputType::Diff,
                max_tokens: 500,
            },
        );

        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.chunk_index, i as u32,
                "chunk indices should be sequential"
            );
        }
    }
}
