//! `hyphae doctor` — diagnose common issues with the hyphae installation.

use anyhow::Result;
use std::path::PathBuf;

use crate::init::{Editor, config_path_for, detect_editors};
use crate::paths::{default_config_path, default_db_path};

pub fn run(fix: bool) -> Result<()> {
    println!();
    println!("\x1b[1mHyphae Doctor\x1b[0m");
    println!("{}", "\u{2500}".repeat(45));
    println!();

    let mut errors = 0u32;
    let mut warnings = 0u32;

    // ─────────────────────────────────────────────────────────────────────────
    // Database
    // ─────────────────────────────────────────────────────────────────────────
    println!("\x1b[1mDatabase\x1b[0m");

    let db_path = default_db_path();
    if db_path.exists() {
        let size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        pass(&format!(
            "Database exists at {} ({:.0} KB)",
            db_path.display(),
            size as f64 / 1024.0
        ));

        // Try opening read-only
        match rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(conn) => {
                pass("Database readable");

                // Integrity check
                match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
                    Ok(ref result) if result == "ok" => pass("Integrity check passed"),
                    Ok(result) => {
                        fail(&format!("Integrity check failed: {result}"));
                        errors += 1;
                    }
                    Err(e) => {
                        fail(&format!("Integrity check error: {e}"));
                        errors += 1;
                    }
                }

                // FTS health
                let _ = conn.execute("PRAGMA trusted_schema=ON", []);
                match conn.query_row("SELECT COUNT(*) FROM memories_fts", [], |row| {
                    row.get::<_, i64>(0)
                }) {
                    Ok(_) => pass("FTS index healthy"),
                    Err(e) => {
                        fail(&format!("FTS index corrupted: {e}"));
                        errors += 1;
                        if fix {
                            print!("  Rebuilding FTS index... ");
                            // Need read-write for rebuild
                            drop(conn);
                            match rusqlite::Connection::open(&db_path) {
                                Ok(rw_conn) => {
                                    let _ = rw_conn.execute("PRAGMA trusted_schema=ON", []);
                                    match rw_conn.execute(
                                        "INSERT INTO memories_fts(memories_fts) VALUES('rebuild')",
                                        [],
                                    ) {
                                        Ok(_) => pass("FTS index rebuilt"),
                                        Err(e) => fail(&format!("FTS rebuild failed: {e}")),
                                    }
                                }
                                Err(e) => fail(&format!("Cannot open DB for writing: {e}")),
                            }
                            // Re-open read-only to continue checks
                            // (skip remaining DB checks after fix)
                            println!();
                            println!("\x1b[1mMCP Server\x1b[0m");
                            check_mcp(&mut warnings);
                            println!();
                            println!("\x1b[1mConfiguration\x1b[0m");
                            check_config(&mut warnings);
                            println!();
                            println!("\x1b[1mEcosystem\x1b[0m");
                            check_ecosystem(&mut warnings);
                            return print_summary(errors, warnings);
                        }
                    }
                }

                // Counts
                let memories: i64 = conn
                    .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                    .unwrap_or(0);
                let memoirs: i64 = conn
                    .query_row("SELECT COUNT(*) FROM memoirs", [], |r| r.get(0))
                    .unwrap_or(0);
                let expired: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                pass(&format!(
                    "{memories} memories, {memoirs} memoirs, {expired} expired"
                ));
            }
            Err(e) => {
                fail(&format!("Cannot open database: {e}"));
                errors += 1;
            }
        }
    } else {
        fail(&format!("Database not found at {}", db_path.display()));
        errors += 1;
        warn("Run: hyphae store --topic test --content \"init\" to create it");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // MCP Server
    // ─────────────────────────────────────────────────────────────────────────
    println!();
    println!("\x1b[1mMCP Server\x1b[0m");
    check_mcp(&mut warnings);

    // ─────────────────────────────────────────────────────────────────────────
    // Configuration
    // ─────────────────────────────────────────────────────────────────────────
    println!();
    println!("\x1b[1mConfiguration\x1b[0m");
    check_config(&mut warnings);

    // ─────────────────────────────────────────────────────────────────────────
    // Ecosystem
    // ─────────────────────────────────────────────────────────────────────────
    println!();
    println!("\x1b[1mEcosystem\x1b[0m");
    check_ecosystem(&mut warnings);

    print_summary(errors, warnings)
}

fn check_mcp(warnings: &mut u32) {
    match which::which("hyphae") {
        Ok(path) => pass(&format!("hyphae binary at {}", path.display())),
        Err(_) => {
            warn("hyphae binary not in PATH");
            *warnings += 1;
        }
    }

    let version = env!("CARGO_PKG_VERSION");
    pass(&format!("Version: {version}"));

    let detected = detect_editors();
    if detected.is_empty() {
        warn("No supported host adapters detected for MCP registration checks");
        *warnings += 1;
        return;
    }

    for editor in detected {
        check_editor_registration(editor, warnings);
    }
}

fn check_editor_registration(editor: Editor, warnings: &mut u32) {
    let Ok(config_path) = config_path_for(editor) else {
        warn(&format!("Could not resolve {} config path", editor));
        *warnings += 1;
        return;
    };

    if !config_path.exists() {
        warn(&format!(
            "{} config not found at {}",
            editor,
            config_path.display()
        ));
        *warnings += 1;
        return;
    }

    if editor.uses_toml() {
        match toml_config_has_hyphae(&config_path) {
            Ok(true) => pass(&format!("Registered in {} config", editor)),
            Ok(false) => {
                warn(&format!("Not registered in {} config", editor));
                *warnings += 1;
            }
            Err(error) => {
                warn(&format!("Could not read {} config: {error}", editor));
                *warnings += 1;
            }
        }

        if matches!(editor, Editor::CodexCli) {
            match codex_notify_configured(&config_path) {
                Ok(true) => pass("Codex CLI notify hooks configured"),
                Ok(false) => {
                    warn("Codex CLI notify hooks missing `hyphae` / `codex-notify`");
                    *warnings += 1;
                }
                Err(error) => {
                    warn(&format!("Could not read Codex CLI notify config: {error}"));
                    *warnings += 1;
                }
            }
        }
    } else {
        match json_config_has_hyphae(&config_path, editor.mcp_key()) {
            Ok(true) => pass(&format!("Registered in {} config", editor)),
            Ok(false) => {
                warn(&format!("Not registered in {} config", editor));
                *warnings += 1;
            }
            Err(error) => {
                warn(&format!("Could not read {} config: {error}", editor));
                *warnings += 1;
            }
        }
    }

    if matches!(editor, Editor::ClaudeCode) && which::which("claude").is_ok() {
        match std::process::Command::new("claude")
            .args(["mcp", "list"])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("hyphae") {
                    pass("Registered in Claude Code CLI runtime");
                } else {
                    warn("Not registered in Claude Code CLI runtime");
                    *warnings += 1;
                }
            }
            Err(_) => {
                warn("Could not check Claude Code CLI runtime registration");
                *warnings += 1;
            }
        }
    }
}

fn json_config_has_hyphae(config_path: &std::path::Path, mcp_key: &str) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(false);
    }
    let value: serde_json::Value = serde_json::from_str(&content)?;
    Ok(value
        .get(mcp_key)
        .and_then(serde_json::Value::as_object)
        .is_some_and(|servers| servers.contains_key("hyphae")))
}

fn toml_config_has_hyphae(config_path: &std::path::Path) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(false);
    }
    let value: toml::Value = toml::from_str(&content)?;
    Ok(value
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .is_some_and(|servers| servers.contains_key("hyphae")))
}

fn codex_notify_configured(config_path: &std::path::Path) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(false);
    }
    let value: toml::Value = toml::from_str(&content)?;
    let Some(notify) = value.get("notify").and_then(toml::Value::as_array) else {
        return Ok(false);
    };
    Ok(notify.iter().any(|entry| entry.as_str() == Some("hyphae"))
        && notify
            .iter()
            .any(|entry| entry.as_str() == Some("codex-notify")))
}

fn check_config(warnings: &mut u32) {
    let config_path =
        default_config_path().unwrap_or_else(|| PathBuf::from(".config/hyphae/config.toml"));

    if config_path.exists() {
        pass(&format!("Config file: {}", config_path.display()));
    } else {
        warn(&format!(
            "No config file at {} (using defaults)",
            config_path.display()
        ));
        *warnings += 1;
    }

    // Check embeddings
    check_embeddings(warnings);
}

fn check_embeddings(warnings: &mut u32) {
    // HTTP embedder check
    let http_url = std::env::var("HYPHAE_EMBEDDING_URL").unwrap_or_default();
    let http_model = std::env::var("HYPHAE_EMBEDDING_MODEL").unwrap_or_default();

    if !http_url.is_empty() && !http_model.is_empty() {
        pass(&format!(
            "HTTP embedder configured: {http_url} ({http_model})"
        ));
    } else if !http_url.is_empty() {
        warn("HYPHAE_EMBEDDING_URL set but HYPHAE_EMBEDDING_MODEL is missing");
        *warnings += 1;
    }

    // FastEmbed check
    if cfg!(feature = "embeddings") {
        pass("FastEmbed support compiled in");

        // Check model cache
        if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
            let cache_dir = home.join(".cache/hyphae/models");
            if cache_dir.exists() {
                let model_count = std::fs::read_dir(&cache_dir)
                    .map(|entries| entries.count())
                    .unwrap_or(0);
                if model_count > 0 {
                    pass(&format!(
                        "Model cache: {} ({model_count} item(s))",
                        cache_dir.display()
                    ));
                } else {
                    pass(&format!(
                        "Model cache: {} (empty, will download on first use)",
                        cache_dir.display()
                    ));
                }
            } else {
                pass("Model cache: not yet created (will download on first use)");
            }
        }
    } else if http_url.is_empty() {
        warn("No embeddings available: fastembed not compiled, HTTP embedder not configured");
        warn("  For HTTP: set HYPHAE_EMBEDDING_URL and HYPHAE_EMBEDDING_MODEL");
        warn("  For local: cargo install hyphae (includes fastembed)");
        *warnings += 1;
    } else {
        pass("FastEmbed not compiled (using HTTP embedder)");
    }

    // Report active backend
    let backend = if !http_url.is_empty() && !http_model.is_empty() {
        "http"
    } else if cfg!(feature = "embeddings") {
        "fastembed"
    } else {
        "none (FTS-only search)"
    };
    pass(&format!("Active embedding backend: {backend}"));
}

fn check_ecosystem(warnings: &mut u32) {
    // Check mycelium
    match which::which("mycelium") {
        Ok(_) => pass("Mycelium available (chunked output integration)"),
        Err(_) => {
            warn("Mycelium not installed (optional: token-optimized CLI)");
            *warnings += 1;
        }
    }

    // Check rhizome
    match which::which("rhizome") {
        Ok(_) => pass("Rhizome available (code-aware recall)"),
        Err(_) => {
            warn("Rhizome not installed (optional: code intelligence)");
            *warnings += 1;
        }
    }
}

fn print_summary(errors: u32, warnings: u32) -> Result<()> {
    println!();
    if errors == 0 && warnings == 0 {
        println!("\x1b[32m0 errors, 0 warnings\x1b[0m");
    } else if errors == 0 {
        println!("\x1b[32m0 errors\x1b[0m, \x1b[33m{warnings} warning(s)\x1b[0m");
    } else {
        println!("\x1b[31m{errors} error(s)\x1b[0m, \x1b[33m{warnings} warning(s)\x1b[0m");
    }
    println!();

    if errors > 0 {
        anyhow::bail!("{errors} error(s) detected — run `hyphae doctor --fix` to attempt repair");
    }
    Ok(())
}

fn pass(msg: &str) {
    println!("  \x1b[32m\u{2713}\x1b[0m {msg}");
}

fn warn(msg: &str) {
    println!("  \x1b[33m\u{26a0}\x1b[0m {msg}");
}

fn fail(msg: &str) {
    println!("  \x1b[31m\u{2717}\x1b[0m {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_json_config_has_hyphae_detects_server() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"hyphae":{"command":"hyphae","args":["serve"]}}}"#,
        )
        .unwrap();

        assert!(json_config_has_hyphae(&path, "mcpServers").unwrap());
    }

    #[test]
    fn test_toml_config_has_hyphae_detects_server() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[mcp_servers.hyphae]
command = "hyphae"
args = ["serve"]
"#,
        )
        .unwrap();

        assert!(toml_config_has_hyphae(&path).unwrap());
    }

    #[test]
    fn test_codex_notify_configured_requires_both_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"notify = ["hyphae", "codex-notify"]"#).unwrap();

        assert!(codex_notify_configured(&path).unwrap());
    }
}
