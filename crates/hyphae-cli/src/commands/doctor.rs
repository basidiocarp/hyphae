//! `hyphae doctor` — diagnose common issues with the hyphae installation.

use anyhow::Result;
use spore::{Tool, discover};
use std::path::PathBuf;

use crate::config::Config;
use crate::init::{
    CLAUDE_HOOK_EVENTS, Editor, claude_hooks_dir, claude_settings_path, config_path_for,
    detect_editors, resolve_hyphae_binary,
};
use crate::paths::{default_config_path, default_db_path};

#[derive(Debug, Clone)]
struct ConfigInspection {
    path: PathBuf,
    exists: bool,
    parse_error: Option<String>,
    validation_error: Option<String>,
    configured_db_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct RegisteredServer {
    command: String,
    args: Vec<String>,
}

pub fn run(fix: bool, cli_db_path: Option<PathBuf>) -> Result<()> {
    println!();
    println!("\x1b[1mHyphae Doctor\x1b[0m");
    println!("{}", "\u{2500}".repeat(45));
    println!();

    let config = inspect_config();
    let db_path = cli_db_path
        .or_else(|| config.configured_db_path.clone())
        .unwrap_or_else(default_db_path);
    let resolved_binary = resolve_hyphae_binary().ok();

    let mut errors = 0u32;
    let mut warnings = 0u32;

    // ─────────────────────────────────────────────────────────────────────────
    // Database
    // ─────────────────────────────────────────────────────────────────────────
    println!("\x1b[1mDatabase\x1b[0m");

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
                                        Ok(_) => {
                                            errors = errors.saturating_sub(1);
                                            pass("FTS index rebuilt");
                                        }
                                        Err(e) => fail(&format!("FTS rebuild failed: {e}")),
                                    }
                                }
                                Err(e) => fail(&format!("Cannot open DB for writing: {e}")),
                            }
                            // Re-open read-only to continue checks
                            // (skip remaining DB checks after fix)
                            println!();
                            println!("\x1b[1mMCP Server\x1b[0m");
                            check_mcp(resolved_binary.as_deref(), &mut warnings, &mut errors);
                            println!();
                            println!("\x1b[1mConfiguration\x1b[0m");
                            check_config(&config, &mut warnings, &mut errors);
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
    check_mcp(resolved_binary.as_deref(), &mut warnings, &mut errors);

    // ─────────────────────────────────────────────────────────────────────────
    // Configuration
    // ─────────────────────────────────────────────────────────────────────────
    println!();
    println!("\x1b[1mConfiguration\x1b[0m");
    check_config(&config, &mut warnings, &mut errors);

    // ─────────────────────────────────────────────────────────────────────────
    // Ecosystem
    // ─────────────────────────────────────────────────────────────────────────
    println!();
    println!("\x1b[1mEcosystem\x1b[0m");
    check_ecosystem(&mut warnings);

    print_summary(errors, warnings)
}

fn inspect_config() -> ConfigInspection {
    let path = default_config_path().unwrap_or_else(|| PathBuf::from(".config/hyphae/config.toml"));
    inspect_config_at_path(path)
}

fn inspect_config_at_path(path: PathBuf) -> ConfigInspection {
    let mut inspection = ConfigInspection {
        path: path.clone(),
        exists: path.exists(),
        parse_error: None,
        validation_error: None,
        configured_db_path: None,
    };

    if !inspection.exists {
        return inspection;
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            inspection.parse_error = Some(format!("reading {}: {error}", path.display()));
            return inspection;
        }
    };

    let raw: toml::Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            inspection.parse_error = Some(format!("parsing {}: {error}", path.display()));
            return inspection;
        }
    };

    inspection.configured_db_path = raw
        .get("store")
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get("path"))
        .and_then(toml::Value::as_str)
        .map(PathBuf::from);

    match toml::from_str::<Config>(&content) {
        Ok(config) => {
            if let Err(error) = config.validate() {
                inspection.validation_error = Some(error.to_string());
            }
        }
        Err(error) => {
            inspection.parse_error = Some(format!("parsing {}: {error}", path.display()));
        }
    }

    inspection
}

fn check_mcp(resolved_binary: Option<&std::path::Path>, warnings: &mut u32, errors: &mut u32) {
    match discover(Tool::Hyphae) {
        Some(info) => pass(&format!("hyphae binary at {}", info.binary_path.display())),
        None => {
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
        check_editor_registration(editor, resolved_binary, warnings, errors);
    }
}

fn check_editor_registration(
    editor: Editor,
    resolved_binary: Option<&std::path::Path>,
    warnings: &mut u32,
    errors: &mut u32,
) {
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
        match toml_config_hyphae_server(&config_path) {
            Ok(Some(server)) => {
                pass(&format!("Registered in {} config", editor));
                validate_registered_server(
                    &editor.to_string(),
                    &server,
                    resolved_binary,
                    warnings,
                    errors,
                );
            }
            Ok(None) => {
                warn(&format!("Not registered in {} config", editor));
                *warnings += 1;
            }
            Err(error) => {
                fail(&format!("Invalid {} config: {error}", editor));
                *errors += 1;
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
        match json_config_hyphae_server(&config_path, editor.mcp_key()) {
            Ok(Some(server)) => {
                pass(&format!("Registered in {} config", editor));
                validate_registered_server(
                    &editor.to_string(),
                    &server,
                    resolved_binary,
                    warnings,
                    errors,
                );
            }
            Ok(None) => {
                warn(&format!("Not registered in {} config", editor));
                *warnings += 1;
            }
            Err(error) => {
                fail(&format!("Invalid {} config: {error}", editor));
                *errors += 1;
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

        check_claude_hook_health(resolved_binary, warnings, errors);
    }
}

fn json_config_hyphae_server(
    config_path: &std::path::Path,
    mcp_key: &str,
) -> Result<Option<RegisteredServer>> {
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(&content)?;
    let Some(server) = value
        .get(mcp_key)
        .and_then(serde_json::Value::as_object)
        .and_then(|servers| servers.get("hyphae"))
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };

    Ok(Some(RegisteredServer {
        command: server
            .get("command")
            .and_then(|command| {
                command.as_str().map(ToOwned::to_owned).or_else(|| {
                    command
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
            })
            .unwrap_or_default(),
        args: server
            .get("args")
            .or_else(|| {
                server
                    .get("command")
                    .and_then(|command| command.get("args"))
            })
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }))
}

fn toml_config_hyphae_server(config_path: &std::path::Path) -> Result<Option<RegisteredServer>> {
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let value: toml::Value = toml::from_str(&content)?;
    let Some(server) = value
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .and_then(|servers| servers.get("hyphae"))
        .and_then(toml::Value::as_table)
    else {
        return Ok(None);
    };

    Ok(Some(RegisteredServer {
        command: server
            .get("command")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        args: server
            .get("args")
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }))
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

fn validate_registered_server(
    label: &str,
    server: &RegisteredServer,
    resolved_binary: Option<&std::path::Path>,
    _warnings: &mut u32,
    errors: &mut u32,
) {
    if server.command.trim().is_empty() {
        fail(&format!("{label} config has an empty hyphae command"));
        *errors += 1;
        return;
    }

    if server.args.first().map(String::as_str) != Some("serve") {
        fail(&format!(
            "{label} config must launch hyphae with `serve` as the first MCP argument"
        ));
        *errors += 1;
    }

    let command_path = std::path::Path::new(&server.command);
    if command_path.is_absolute() {
        if !command_path.exists() {
            fail(&format!(
                "{label} config points to a missing hyphae binary: {}",
                command_path.display()
            ));
            *errors += 1;
            return;
        }

        if let Some(current_binary) = resolved_binary
            && let (Ok(configured), Ok(current)) =
                (command_path.canonicalize(), current_binary.canonicalize())
        {
            if configured != current {
                fail(&format!(
                    "{label} config points at {} instead of the current hyphae binary {}",
                    configured.display(),
                    current.display()
                ));
                *errors += 1;
            }
        } else if command_path.file_name().and_then(|name| name.to_str()) != Some("hyphae") {
            fail(&format!(
                "{label} config points to {} but it does not look like a hyphae binary",
                command_path.display()
            ));
            *errors += 1;
        }
    } else if server.command != "hyphae" {
        fail(&format!(
            "{label} config refers to `{}` instead of the supported `hyphae` command",
            server.command
        ));
        *errors += 1;
    } else if which::which(&server.command).is_err() {
        fail(&format!(
            "{label} config refers to `{}` but it is not resolvable in PATH",
            server.command
        ));
        *errors += 1;
    }
}

fn check_claude_hook_health(
    resolved_binary: Option<&std::path::Path>,
    warnings: &mut u32,
    errors: &mut u32,
) {
    let Ok(settings_path) = claude_settings_path() else {
        warn("Could not resolve Claude Code settings path for hook checks");
        *warnings += 1;
        return;
    };
    let Ok(hook_dir) = claude_hooks_dir() else {
        warn("Could not resolve Claude Code hook directory for hook checks");
        *warnings += 1;
        return;
    };

    let settings_exists = settings_path.exists();
    let hook_dir_exists = hook_dir.exists();
    if !settings_exists && !hook_dir_exists {
        pass("Claude Code lifecycle hooks not installed (optional)");
        return;
    }

    let mut configured_events = 0usize;
    if settings_exists {
        match std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        {
            Some(root) => {
                for (event, file_name) in CLAUDE_HOOK_EVENTS {
                    let expected_command = hook_dir.join(file_name).to_string_lossy().to_string();
                    let present = root
                        .get("hooks")
                        .and_then(|hooks| hooks.get(event))
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|entries| {
                            entries
                                .iter()
                                .filter_map(|entry| entry.get("hooks")?.as_array())
                                .flatten()
                                .filter_map(|hook| hook.get("command")?.as_str())
                                .any(|command| command == expected_command)
                        });
                    if present {
                        configured_events += 1;
                    }
                }
            }
            None => {
                fail(&format!(
                    "Claude Code settings at {} could not be parsed for hook validation",
                    settings_path.display()
                ));
                *errors += 1;
                return;
            }
        }
    }

    let mut installed_scripts = 0usize;
    for (_, file_name) in CLAUDE_HOOK_EVENTS {
        let script_path = hook_dir.join(file_name);
        if script_path.exists() {
            installed_scripts += 1;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = std::fs::metadata(&script_path)
                    && metadata.permissions().mode() & 0o111 == 0
                {
                    fail(&format!(
                        "Claude hook script {} is not executable",
                        script_path.display()
                    ));
                    *errors += 1;
                }
            }
            if let Some(current_binary) = resolved_binary
                && let Ok(content) = std::fs::read_to_string(&script_path)
                && !content.contains(current_binary.to_string_lossy().as_ref())
            {
                warn(&format!(
                    "Claude hook script {} does not reference the current hyphae binary",
                    script_path.display()
                ));
                *warnings += 1;
            }
        }
    }

    if configured_events == 0 && installed_scripts == 0 {
        pass("Claude Code lifecycle hooks not installed (optional)");
        return;
    }

    if configured_events == CLAUDE_HOOK_EVENTS.len()
        && installed_scripts == CLAUDE_HOOK_EVENTS.len()
    {
        pass("Claude Code lifecycle hooks installed");
        return;
    }

    fail(&format!(
        "Claude Code lifecycle hooks are partially installed ({} settings entries, {} scripts)",
        configured_events, installed_scripts
    ));
    *errors += 1;
}

fn check_config(config: &ConfigInspection, warnings: &mut u32, errors: &mut u32) {
    if config.exists {
        pass(&format!("Config file: {}", config.path.display()));
    } else {
        warn(&format!(
            "No config file at {} (using defaults)",
            config.path.display()
        ));
        *warnings += 1;
    }

    if let Some(error) = &config.parse_error {
        fail(&format!("Config parse error: {error}"));
        *errors += 1;
    } else if let Some(error) = &config.validation_error {
        fail(&format!("Config validation error: {error}"));
        *errors += 1;
    }

    if let Some(db_path) = &config.configured_db_path {
        pass(&format!("Configured database path: {}", db_path.display()));
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
    match discover(Tool::Mycelium) {
        Some(_) => pass("Mycelium available (chunked output integration)"),
        None => {
            warn("Mycelium not installed (optional: token-optimized CLI)");
            *warnings += 1;
        }
    }

    // Check rhizome
    match discover(Tool::Rhizome) {
        Some(_) => pass("Rhizome available (code-aware recall)"),
        None => {
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
        anyhow::bail!(
            "{errors} error(s) detected — `hyphae doctor --fix` currently only repairs FTS index corruption; other issues require manual changes"
        );
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
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_json_config_has_hyphae_detects_server() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"mcpServers":{"hyphae":{"command":"hyphae","args":["serve"]}}}"#,
        )
        .unwrap();

        let server = json_config_hyphae_server(&path, "mcpServers")
            .unwrap()
            .unwrap();
        assert_eq!(server.command, "hyphae");
        assert_eq!(server.args, vec!["serve"]);
    }

    #[test]
    fn test_toml_config_has_hyphae_detects_server() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[mcp_servers.hyphae]
command = "hyphae"
args = ["serve"]
"#,
        )
        .unwrap();

        let server = toml_config_hyphae_server(&path).unwrap().unwrap();
        assert_eq!(server.command, "hyphae");
        assert_eq!(server.args, vec!["serve"]);
    }

    #[test]
    fn test_json_config_has_hyphae_detects_nested_zed_server() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"context_servers":{"hyphae":{"command":{"path":"/usr/local/bin/hyphae","args":["serve"]},"settings":{}}}}"#,
        )
        .unwrap();

        let server = json_config_hyphae_server(&path, "context_servers")
            .unwrap()
            .unwrap();
        assert_eq!(server.command, "/usr/local/bin/hyphae");
        assert_eq!(server.args, vec!["serve"]);
    }

    #[test]
    fn test_codex_notify_configured_requires_both_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"notify = ["hyphae", "codex-notify"]"#).unwrap();

        assert!(codex_notify_configured(&path).unwrap());
    }

    #[test]
    fn test_inspect_config_reports_validation_error_and_db_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[store]
path = "/tmp/hyphae.db"

[recall]
limit = 0
"#,
        )
        .unwrap();

        let inspection = inspect_config_at_path(path);

        assert!(inspection.exists);
        assert!(inspection.validation_error.is_some());
        assert_eq!(
            inspection.configured_db_path,
            Some(PathBuf::from("/tmp/hyphae.db"))
        );
    }
}
