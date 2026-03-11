use hyphae_core::chunk::SourceType;
use hyphae_core::error::{HyphaeError, HyphaeResult};
use std::fs;
use std::path::Path;

/// Read a file and detect its source type from extension.
///
/// Returns an error for binary files (detected via null bytes in first 8KB).
pub fn read_file(path: &Path) -> HyphaeResult<(String, SourceType)> {
    let raw = fs::read(path)?;

    // Binary detection: check first 8KB for null bytes
    let check_len = raw.len().min(8192);
    if raw[..check_len].contains(&0) {
        return Err(HyphaeError::Ingest(format!(
            "binary file: {}",
            path.display()
        )));
    }

    let content =
        String::from_utf8(raw).map_err(|e| HyphaeError::Ingest(format!("invalid UTF-8: {e}")))?;

    let source_type = detect_source_type(path);
    Ok((content, source_type))
}

fn detect_source_type(path: &Path) -> SourceType {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "rs" | "py" | "js" | "ts" | "tsx" | "go" | "java" | "c" | "cpp" | "h" | "cs" | "rb"
        | "swift" | "kt" => SourceType::Code,
        "md" | "mdx" => SourceType::Markdown,
        "txt" | "log" | "csv" | "json" | "toml" | "yaml" | "yml" => SourceType::Text,
        _ => SourceType::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_file_type_detection() {
        let cases = vec![
            ("test.rs", SourceType::Code),
            ("test.py", SourceType::Code),
            ("test.js", SourceType::Code),
            ("test.ts", SourceType::Code),
            ("test.tsx", SourceType::Code),
            ("test.go", SourceType::Code),
            ("test.java", SourceType::Code),
            ("test.c", SourceType::Code),
            ("test.cpp", SourceType::Code),
            ("test.h", SourceType::Code),
            ("test.cs", SourceType::Code),
            ("test.rb", SourceType::Code),
            ("test.swift", SourceType::Code),
            ("test.kt", SourceType::Code),
            ("test.md", SourceType::Markdown),
            ("test.mdx", SourceType::Markdown),
            ("test.txt", SourceType::Text),
            ("test.log", SourceType::Text),
            ("test.json", SourceType::Text),
            ("test.toml", SourceType::Text),
            ("test.yaml", SourceType::Text),
            ("test.yml", SourceType::Text),
            ("test.csv", SourceType::Text),
            ("test.unknown", SourceType::Text),
        ];

        for (filename, expected) in cases {
            let path = Path::new(filename);
            let result = detect_source_type(path);
            assert_eq!(result, expected, "failed for {filename}");
        }
    }

    #[test]
    fn test_read_file_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.rs");
        fs::write(&path, "fn main() {}").unwrap();

        let (content, source_type) = read_file(&path).unwrap();
        assert_eq!(content, "fn main() {}");
        assert_eq!(source_type, SourceType::Code);
    }

    #[test]
    fn test_read_file_binary_rejected() {
        let mut tmp = NamedTempFile::with_suffix(".bin").unwrap();
        tmp.write_all(&[0x00, 0x01, 0x02, 0x00]).unwrap();

        let result = read_file(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HyphaeError::Ingest(_)));
        assert!(err.to_string().contains("binary file"));
    }

    #[test]
    fn test_read_file_not_found() {
        let result = read_file(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HyphaeError::Io(_)));
    }
}
