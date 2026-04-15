//! Rhizome CLI integration for AST-level symbol boundary extraction.
//!
//! Calls `rhizome symbols <file>` to get symbol locations, then parses the
//! flat text output into [`SymbolBoundary`] values. Falls back gracefully when
//! rhizome is unavailable.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use spore::{Tool, discover};

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

/// Get symbol boundaries for a file by calling `rhizome symbols <file>`.
///
/// Returns `Ok(vec)` on success (possibly empty), or `Err` if rhizome is
/// unavailable or the command fails.
pub fn get_symbol_boundaries(file: &Path) -> Result<Vec<SymbolBoundary>, RhizomeError> {
    let info = discover(Tool::Rhizome).ok_or(RhizomeError::NotAvailable)?;

    let mut child = Command::new(&info.binary_path)
        .arg("symbols")
        .arg(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| RhizomeError::CommandFailed(format!("failed to spawn rhizome: {e}")))?;

    let timeout = Duration::from_secs(10);
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait(); // reap the zombie
                    return Err(RhizomeError::CommandFailed(
                        "rhizome symbols timed out after 10s".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(RhizomeError::CommandFailed(format!("wait error: {e}")));
            }
        }
    };

    if !status.success() {
        return Err(RhizomeError::CommandFailed(format!(
            "rhizome symbols exited with {}",
            status,
        )));
    }

    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout)
            .map_err(|e| RhizomeError::CommandFailed(format!("failed to read stdout: {e}")))?;
    }

    Ok(parse_symbols_output(&stdout))
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
}
