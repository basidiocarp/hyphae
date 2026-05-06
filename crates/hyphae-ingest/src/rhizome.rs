//! Rhizome MCP integration for AST-level symbol boundary extraction.
//!
//! Calls the rhizome MCP `get_symbols` tool to get symbol locations, then
//! parses the JSON response into [`SymbolBoundary`] values. Falls back
//! gracefully when rhizome is unavailable.

use std::path::Path;

use spore::{McpClient, Tool, discover};

/// A symbol boundary extracted from rhizome output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolBoundary {
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
}

/// Check whether rhizome is available on this system.
#[must_use]
pub fn is_available() -> bool {
    discover(Tool::Rhizome).is_some()
}

/// Get chunk boundaries for a file via rhizome MCP `get_chunk_boundaries` tool.
///
/// This is optimized for chunking and returns boundaries based on the specified
/// strategy. Returns `Ok(vec)` on success (possibly empty), or `Err` if rhizome
/// is unavailable or the call fails.
pub fn get_chunk_boundaries_for_chunking(
    file: &Path,
    strategy: &str,
) -> Result<Vec<ChunkBoundary>, RhizomeError> {
    // Confirm rhizome is discoverable before spawning an MCP client
    if !is_available() {
        return Err(RhizomeError::NotAvailable);
    }

    let file_str = file
        .to_str()
        .ok_or_else(|| RhizomeError::CommandFailed("invalid file path encoding".into()))?;

    let mut client = McpClient::spawn(Tool::Rhizome, &[])
        .map_err(|e| RhizomeError::CommandFailed(format!("failed to start rhizome MCP: {e}")))?;

    let result = client
        .call_tool(
            "get_chunk_boundaries",
            serde_json::json!({ "file": file_str, "strategy": strategy }),
        )
        .map_err(|e| RhizomeError::CommandFailed(format!("get_chunk_boundaries failed: {e}")))?;

    parse_chunk_boundaries_response(result)
}

/// Parse rhizome MCP `get_chunk_boundaries` response into boundaries.
///
/// The response has shape: `[{"type":"text","text":"<JSON object with boundaries array>"}]`
/// where boundaries is an array of boundary objects with start_line, end_line, kind, name.
fn parse_chunk_boundaries_response(
    value: serde_json::Value,
) -> Result<Vec<ChunkBoundary>, RhizomeError> {
    let text = value
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            RhizomeError::CommandFailed("unexpected get_chunk_boundaries response shape".into())
        })?;

    let response: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        RhizomeError::CommandFailed(format!("failed to parse chunk boundaries JSON: {e}"))
    })?;

    let boundaries = response
        .get("boundaries")
        .and_then(|b| b.as_array())
        .ok_or_else(|| RhizomeError::CommandFailed("missing or invalid boundaries array".into()))?;

    let mut result = Vec::new();
    for boundary in boundaries {
        let start_line = boundary
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let end_line = boundary
            .get("end_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Skip boundaries with invalid ranges
        if start_line == 0 || end_line == 0 || start_line > end_line {
            continue;
        }

        let kind = boundary
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let name = boundary
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous")
            .to_string();

        result.push(ChunkBoundary {
            start_line,
            end_line,
            kind,
            name,
        });
    }
    Ok(result)
}

/// A chunk boundary extracted from rhizome's get_chunk_boundaries output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBoundary {
    pub start_line: u32,
    pub end_line: u32,
    pub kind: String,
    pub name: String,
}

/// Get symbol boundaries for a file via rhizome MCP `get_symbols` tool.
///
/// Returns `Ok(vec)` on success (possibly empty), or `Err` if rhizome is
/// unavailable or the call fails.
pub fn get_symbol_boundaries(file: &Path) -> Result<Vec<SymbolBoundary>, RhizomeError> {
    // Confirm rhizome is discoverable before spawning an MCP client
    if !is_available() {
        return Err(RhizomeError::NotAvailable);
    }

    let file_str = file
        .to_str()
        .ok_or_else(|| RhizomeError::CommandFailed("invalid file path encoding".into()))?;

    let mut client = McpClient::spawn(Tool::Rhizome, &[])
        .map_err(|e| RhizomeError::CommandFailed(format!("failed to start rhizome MCP: {e}")))?;

    let result = client
        .call_tool("get_symbols", serde_json::json!({ "file": file_str }))
        .map_err(|e| RhizomeError::CommandFailed(format!("get_symbols failed: {e}")))?;

    parse_mcp_symbols_response(result)
}

/// Parse rhizome MCP `get_symbols` response into boundaries.
///
/// The response has shape: `[{"type":"text","text":"<JSON array>"}]`
/// where the JSON array contains symbol objects with fields like name, kind,
/// and location with line_start and line_end.
fn parse_mcp_symbols_response(
    value: serde_json::Value,
) -> Result<Vec<SymbolBoundary>, RhizomeError> {
    let text = value
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            RhizomeError::CommandFailed("unexpected get_symbols response shape".into())
        })?;

    let symbols: Vec<serde_json::Value> = serde_json::from_str(text)
        .map_err(|e| RhizomeError::CommandFailed(format!("failed to parse symbols JSON: {e}")))?;

    let mut result = Vec::new();
    for sym in &symbols {
        let name = match sym.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.is_empty() => n,
            _ => continue, // skip symbols without a usable name
        };
        let kind = match sym.get("kind").and_then(|v| v.as_str()) {
            Some(k) if !k.is_empty() => k,
            _ => continue, // skip symbols without a kind (consistent with CLI parser which skips malformed lines)
        };
        let loc = sym.get("location");
        let line_start = loc
            .and_then(|l| l.get("line_start"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let line_end = loc
            .and_then(|l| l.get("line_end"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        result.push(SymbolBoundary {
            name: name.to_string(),
            kind: kind.to_string(),
            line_start,
            line_end,
        });
    }
    Ok(result)
}

/// Parse rhizome's flat symbol output into boundaries.
///
/// Each line has the format: `kind name [line_start:col_start-line_end:col_end]`
/// Optionally followed by an indented signature line.
pub fn parse_symbols_output(output: &str) -> Vec<SymbolBoundary> {
    let mut symbols = Vec::new();

    for line in output.lines() {
        // Skip indented signature lines
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(boundary) = parse_symbol_line(trimmed) {
            symbols.push(boundary);
        }
    }

    symbols
}

/// Parse a single symbol line like `fn main [1:0-10:1]`.
fn parse_symbol_line(line: &str) -> Option<SymbolBoundary> {
    // Format: kind name [line_start:col_start-line_end:col_end]
    let bracket_start = line.find('[')?;
    let bracket_end = line.find(']')?;
    if bracket_start >= bracket_end {
        return None;
    }

    let prefix = line[..bracket_start].trim();
    let location = &line[bracket_start + 1..bracket_end];

    // Split prefix into kind and name
    let mut parts = prefix.splitn(2, ' ');
    let kind = parts.next()?.trim();
    let name = parts.next()?.trim();

    if kind.is_empty() || name.is_empty() {
        return None;
    }

    // Parse location: line_start:col_start-line_end:col_end
    let mut halves = location.split('-');
    let start_part = halves.next()?;
    let end_part = halves.next()?;

    let line_start: u32 = start_part.split(':').next()?.parse().ok()?;
    let line_end: u32 = end_part.split(':').next()?.parse().ok()?;

    Some(SymbolBoundary {
        name: name.to_string(),
        kind: kind.to_string(),
        line_start,
        line_end,
    })
}

/// Errors from rhizome integration.
#[derive(Debug, thiserror::Error)]
pub enum RhizomeError {
    #[error("rhizome is not available")]
    NotAvailable,
    #[error("rhizome command failed: {0}")]
    CommandFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mcp_symbols_response_basic() {
        let sym_json = serde_json::json!([{
            "name": "main",
            "qualified_name": "main",
            "stable_id": "src/main.rs::main@1:0",
            "kind": "Function",
            "location": {
                "file": "src/main.rs",
                "line_start": 1,
                "line_end": 10,
                "column_start": 0,
                "column_end": 1,
            },
            "signature": null,
        }]);
        let response = serde_json::json!([{
            "type": "text",
            "text": sym_json.to_string(),
        }]);
        let symbols = parse_mcp_symbols_response(response).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, "Function");
        assert_eq!(symbols[0].line_start, 1);
        assert_eq!(symbols[0].line_end, 10);
    }

    #[test]
    fn parse_mcp_symbols_response_empty_array() {
        // Empty symbol array returns empty vec, not an error
        let response = serde_json::json!([{
            "type": "text",
            "text": "[]",
        }]);
        let symbols = parse_mcp_symbols_response(response).unwrap();
        assert!(symbols.is_empty());
    }

    #[test]
    fn parse_mcp_symbols_response_skips_missing_name() {
        // Symbols without a name are skipped
        let sym_json = serde_json::json!([
            { "kind": "Function", "location": { "line_start": 1, "line_end": 5 } },
            { "name": "", "kind": "Function", "location": { "line_start": 7, "line_end": 10 } },
            { "name": "real_fn", "kind": "Function", "location": { "line_start": 12, "line_end": 15 } },
        ]);
        let response = serde_json::json!([{ "type": "text", "text": sym_json.to_string() }]);
        let symbols = parse_mcp_symbols_response(response).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "real_fn");
    }

    #[test]
    fn parse_mcp_symbols_response_skips_missing_kind() {
        // Symbols without a kind are skipped (consistent with CLI parser skipping malformed lines)
        let sym_json = serde_json::json!([
            { "name": "no_kind", "location": { "line_start": 1, "line_end": 5 } },
            { "name": "has_kind", "kind": "Struct", "location": { "line_start": 7, "line_end": 20 } },
        ]);
        let response = serde_json::json!([{ "type": "text", "text": sym_json.to_string() }]);
        let symbols = parse_mcp_symbols_response(response).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "has_kind");
        assert_eq!(symbols[0].kind, "Struct");
    }

    #[test]
    fn parse_mcp_symbols_response_missing_location_uses_zero() {
        // Missing location fields default to 0
        let sym_json = serde_json::json!([{
            "name": "no_loc",
            "kind": "Function",
        }]);
        let response = serde_json::json!([{ "type": "text", "text": sym_json.to_string() }]);
        let symbols = parse_mcp_symbols_response(response).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].line_start, 0);
        assert_eq!(symbols[0].line_end, 0);
    }

    #[test]
    fn parse_mcp_symbols_response_wrong_shape_returns_err() {
        // Outer value is not an array — should return Err
        let bad = serde_json::json!({"type": "text", "text": "[]"});
        assert!(parse_mcp_symbols_response(bad).is_err());

        // Outer array is empty — .first() returns None — should return Err
        let empty_arr = serde_json::json!([]);
        assert!(parse_mcp_symbols_response(empty_arr).is_err());
    }

    #[test]
    fn parse_mcp_symbols_response_invalid_inner_json_returns_err() {
        // text field contains non-JSON — should return Err
        let response = serde_json::json!([{ "type": "text", "text": "not valid json" }]);
        assert!(parse_mcp_symbols_response(response).is_err());
    }

    #[test]
    fn parse_single_function() {
        let output = "fn main [1:0-10:1]\n";
        let symbols = parse_symbols_output(output);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, "fn");
        assert_eq!(symbols[0].line_start, 1);
        assert_eq!(symbols[0].line_end, 10);
    }

    #[test]
    fn parse_multiple_symbols() {
        let output = "\
fn hello [1:0-3:1]
  pub fn hello()
fn world [5:0-7:1]
  pub fn world()
struct Config [9:0-15:1]
";
        let symbols = parse_symbols_output(output);
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, "fn");
        assert_eq!(symbols[0].line_start, 1);
        assert_eq!(symbols[0].line_end, 3);
        assert_eq!(symbols[1].name, "world");
        assert_eq!(symbols[1].line_start, 5);
        assert_eq!(symbols[1].line_end, 7);
        assert_eq!(symbols[2].name, "Config");
        assert_eq!(symbols[2].kind, "struct");
        assert_eq!(symbols[2].line_start, 9);
        assert_eq!(symbols[2].line_end, 15);
    }

    #[test]
    fn parse_empty_output() {
        assert!(parse_symbols_output("").is_empty());
        assert!(parse_symbols_output("\n\n").is_empty());
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let output = "not a valid line\nfn good [1:0-5:1]\nbad format\n";
        let symbols = parse_symbols_output(output);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "good");
    }

    #[test]
    fn parse_location_with_extra_dashes() {
        // A malformed location with extra dashes should be skipped gracefully.
        // The parser splits on `-` and takes only the first two halves, so
        // the extra `-extra` segment is ignored by `splitn(2, '-')` — but
        // we actually use `split('-')` which yields three segments. The second
        // call to `halves.next()?` gets the middle part and parses it
        // successfully, while the third segment is just ignored.
        let output = "fn foo [1:0-5:1-extra]\n";
        let symbols = parse_symbols_output(output);
        // The parser uses split('-') which yields ["1:0", "5:1", "extra"].
        // halves.next() gets "1:0" (start), halves.next() gets "5:1" (end).
        // The extra segment is never consumed, so parsing succeeds.
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "foo");
        assert_eq!(symbols[0].line_start, 1);
        assert_eq!(symbols[0].line_end, 5);
    }

    #[test]
    fn parse_various_kinds() {
        let output = "\
fn dispatch [1:0-20:1]
method handle [22:4-30:1]
class MyClass [32:0-50:1]
struct Point [52:0-55:1]
enum Color [57:0-62:1]
trait Drawable [64:0-70:1]
const MAX [72:0-72:30]
mod utils [74:0-100:1]
";
        let symbols = parse_symbols_output(output);
        assert_eq!(symbols.len(), 8);
        let kinds: Vec<&str> = symbols.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "fn", "method", "class", "struct", "enum", "trait", "const", "mod"
            ]
        );
    }

    #[test]
    fn parse_chunk_boundaries_response_basic() {
        let boundaries_json = serde_json::json!({
            "boundaries": [
                {
                    "start_line": 1,
                    "end_line": 10,
                    "kind": "Function",
                    "name": "main",
                },
                {
                    "start_line": 12,
                    "end_line": 25,
                    "kind": "Struct",
                    "name": "Config",
                },
            ]
        });
        let response = serde_json::json!([{
            "type": "text",
            "text": boundaries_json.to_string(),
        }]);
        let boundaries = parse_chunk_boundaries_response(response).unwrap();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0].start_line, 1);
        assert_eq!(boundaries[0].end_line, 10);
        assert_eq!(boundaries[0].name, "main");
        assert_eq!(boundaries[0].kind, "Function");
        assert_eq!(boundaries[1].start_line, 12);
        assert_eq!(boundaries[1].end_line, 25);
        assert_eq!(boundaries[1].name, "Config");
    }

    #[test]
    fn parse_chunk_boundaries_response_empty_array() {
        let boundaries_json = serde_json::json!({ "boundaries": [] });
        let response = serde_json::json!([{
            "type": "text",
            "text": boundaries_json.to_string(),
        }]);
        let boundaries = parse_chunk_boundaries_response(response).unwrap();
        assert!(boundaries.is_empty());
    }

    #[test]
    fn parse_chunk_boundaries_response_skips_invalid_ranges() {
        let boundaries_json = serde_json::json!({
            "boundaries": [
                { "start_line": 0, "end_line": 10, "kind": "Function", "name": "bad1" },
                { "start_line": 5, "end_line": 0, "kind": "Function", "name": "bad2" },
                { "start_line": 10, "end_line": 5, "kind": "Function", "name": "bad3" },
                { "start_line": 15, "end_line": 20, "kind": "Function", "name": "good" },
            ]
        });
        let response = serde_json::json!([{
            "type": "text",
            "text": boundaries_json.to_string(),
        }]);
        let boundaries = parse_chunk_boundaries_response(response).unwrap();
        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0].name, "good");
    }

    #[test]
    fn parse_chunk_boundaries_response_wrong_shape_returns_err() {
        let bad = serde_json::json!({"type": "text", "text": "{}"});
        assert!(parse_chunk_boundaries_response(bad).is_err());

        let empty_arr = serde_json::json!([]);
        assert!(parse_chunk_boundaries_response(empty_arr).is_err());
    }
}
