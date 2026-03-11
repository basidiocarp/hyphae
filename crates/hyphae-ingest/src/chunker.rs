use std::sync::OnceLock;

use chrono::Utc;
use hyphae_core::chunk::{Chunk, ChunkMetadata, SourceType};
use hyphae_core::ids::{ChunkId, DocumentId};
use regex::Regex;

fn heading_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^(#{1,6})\s+(.+)$").expect("heading regex is valid"))
}

#[derive(Debug, Clone)]
pub enum ChunkStrategy {
    SlidingWindow { size: usize, overlap: usize },
    ByHeading { max_tokens: usize },
    ByFunction { language: String },
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
}
