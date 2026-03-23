use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::{Path, PathBuf};

const HYPHAE_BIN_PLACEHOLDER: &str = "__HYPHAE_BIN__";
const HOOK_POST_TOOL_TEMPLATE: &str = include_str!("../../../scripts/hooks/hyphae-post-tool.sh");
const HOOK_PRE_COMPACT_TEMPLATE: &str = include_str!("../../../scripts/hooks/hyphae-precompact.sh");
const HOOK_SESSION_END_TEMPLATE: &str =
    include_str!("../../../scripts/hooks/hyphae-session-end.sh");

#[derive(Debug, Clone, ValueEnum)]
pub enum Editor {
    ClaudeCode,
    Cursor,
    VsCode,
    Zed,
    Windsurf,
    Amp,
    ClaudeDesktop,
    CodexCli,
}

impl std::fmt::Display for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Editor::ClaudeCode => write!(f, "Claude Code"),
            Editor::Cursor => write!(f, "Cursor"),
            Editor::VsCode => write!(f, "VS Code"),
            Editor::Zed => write!(f, "Zed"),
            Editor::Windsurf => write!(f, "Windsurf"),
            Editor::Amp => write!(f, "Amp"),
            Editor::ClaudeDesktop => write!(f, "Claude Desktop"),
            Editor::CodexCli => write!(f, "Codex CLI"),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum InitMode {
    Mcp,
    Hook,
    All,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

pub fn detect_editors() -> Vec<Editor> {
    let mut editors = Vec::new();
    let Some(home) = home_dir() else {
        return editors;
    };

    if home.join(".claude.json").exists() {
        editors.push(Editor::ClaudeCode);
    }

    if home.join(".cursor").is_dir() {
        editors.push(Editor::Cursor);
    }

    #[cfg(target_os = "macos")]
    let vscode_path = home.join("Library/Application Support/Code");
    #[cfg(not(target_os = "macos"))]
    let vscode_path = home.join(".config/Code");
    if vscode_path.exists() {
        editors.push(Editor::VsCode);
    }

    if home.join(".zed").is_dir() {
        editors.push(Editor::Zed);
    }

    if home.join(".codeium/windsurf").is_dir() {
        editors.push(Editor::Windsurf);
    }

    if home.join(".config/amp").is_dir() {
        editors.push(Editor::Amp);
    }

    #[cfg(target_os = "macos")]
    let claude_desktop_path = home.join("Library/Application Support/Claude");
    #[cfg(not(target_os = "macos"))]
    let claude_desktop_path = home.join(".config/Claude");
    if claude_desktop_path.exists() {
        editors.push(Editor::ClaudeDesktop);
    }

    if home.join(".codex").is_dir() {
        editors.push(Editor::CodexCli);
    }

    editors
}

pub fn resolve_hyphae_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            return Ok(canonical);
        }
    }
    which_hyphae().context("could not locate hyphae binary in PATH")
}

fn which_hyphae() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hyphae");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn config_path_for(editor: &Editor) -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
    match editor {
        Editor::ClaudeCode => home.join(".claude.json"),
        Editor::Cursor => home.join(".cursor/mcp.json"),
        Editor::VsCode => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support/Code/User/settings.json")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".config/Code/User/settings.json")
            }
        }
        Editor::Zed => home.join(".zed/settings.json"),
        Editor::Windsurf => home.join(".codeium/windsurf/mcp_config.json"),
        Editor::Amp => home.join(".config/amp/settings.json"),
        Editor::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support/Claude/claude_desktop_config.json")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".config/Claude/claude_desktop_config.json")
            }
        }
        Editor::CodexCli => home.join(".codex/config.toml"),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.display()))
}

fn claude_dir() -> Result<PathBuf> {
    home_dir()
        .map(|home| home.join(".claude"))
        .context("could not determine home directory for Claude Code settings")
}

fn claude_hooks_dir() -> Result<PathBuf> {
    Ok(claude_dir()?.join("hooks"))
}

fn claude_settings_path() -> Result<PathBuf> {
    Ok(claude_dir()?.join("settings.json"))
}

pub fn write_config(editor: &Editor, binary_path: &Path) -> Result<()> {
    let config_path = config_path_for(editor);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    match editor {
        Editor::CodexCli => write_toml_config(&config_path, binary_path),
        _ => write_json_config(editor, &config_path, binary_path),
    }
}

#[derive(Clone, Copy)]
struct HookSpec {
    event: &'static str,
    matcher: Option<&'static str>,
    file_name: &'static str,
    template: &'static str,
    status_message: &'static str,
    timeout_secs: u64,
}

impl HookSpec {
    fn hook_entry(&self, command: &str) -> serde_json::Value {
        let mut entry = serde_json::Map::new();
        if let Some(matcher) = self.matcher {
            entry.insert(
                "matcher".to_string(),
                serde_json::Value::String(matcher.to_string()),
            );
        }
        entry.insert(
            "hooks".to_string(),
            serde_json::json!([{
                "type": "command",
                "command": command,
                "timeout": self.timeout_secs,
                "statusMessage": self.status_message,
            }]),
        );
        serde_json::Value::Object(entry)
    }
}

const CLAUDE_HOOK_SPECS: [HookSpec; 3] = [
    HookSpec {
        event: "PostToolUse",
        matcher: None,
        file_name: "hyphae-post-tool.sh",
        template: HOOK_POST_TOOL_TEMPLATE,
        status_message: "Hyphae extracting tool context",
        timeout_secs: 2,
    },
    HookSpec {
        event: "PreCompact",
        matcher: None,
        file_name: "hyphae-precompact.sh",
        template: HOOK_PRE_COMPACT_TEMPLATE,
        status_message: "Hyphae capturing compaction context",
        timeout_secs: 2,
    },
    HookSpec {
        event: "SessionEnd",
        matcher: None,
        file_name: "hyphae-session-end.sh",
        template: HOOK_SESSION_END_TEMPLATE,
        status_message: "Hyphae capturing session end",
        timeout_secs: 1,
    },
];

fn shell_single_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn render_hook_script(template: &str, binary_path: &Path) -> String {
    template.replace(
        HYPHAE_BIN_PLACEHOLDER,
        &shell_single_quote(&binary_path.to_string_lossy()),
    )
}

fn hook_command_for(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn command_matches(existing: &str, expected: &str, hook_name: &str) -> bool {
    existing == expected || (existing.contains(hook_name) && expected.contains(hook_name))
}

fn hook_entry_present(root: &serde_json::Value, spec: HookSpec, hook_command: &str) -> bool {
    let Some(entries) = root
        .get("hooks")
        .and_then(|hooks| hooks.get(spec.event))
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };

    entries
        .iter()
        .filter_map(|entry| entry.get("hooks")?.as_array())
        .flatten()
        .filter_map(|hook| hook.get("command")?.as_str())
        .any(|existing| command_matches(existing, hook_command, spec.file_name))
}

fn insert_hook_entry(root: &mut serde_json::Value, spec: HookSpec, hook_command: &str) {
    let root_obj = match root.as_object_mut() {
        Some(obj) => obj,
        None => {
            *root = serde_json::json!({});
            root.as_object_mut()
                .expect("fresh object must be present after initialization")
        }
    };

    let hooks = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("hooks must be an object");

    let event_hooks = hooks
        .entry(spec.event)
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .expect("event hook list must be an array");

    event_hooks.push(spec.hook_entry(hook_command));
}

fn write_claude_hook_settings(settings_path: &Path, hook_dir: &Path) -> Result<Vec<&'static str>> {
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    let existing: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(settings_path)
            .with_context(|| format!("reading {}", settings_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parsing JSON from {}", settings_path.display()))?
    } else {
        serde_json::json!({})
    };

    let mut root = existing;
    let mut installed = Vec::new();

    for spec in CLAUDE_HOOK_SPECS {
        let command = hook_command_for(&hook_dir.join(spec.file_name));
        if !hook_entry_present(&root, spec, &command) {
            insert_hook_entry(&mut root, spec, &command);
            installed.push(spec.event);
        }
    }

    if installed.is_empty() {
        return Ok(installed);
    }

    if settings_path.exists() {
        let bak = backup_path(settings_path);
        std::fs::copy(settings_path, &bak)
            .with_context(|| format!("creating backup {}", bak.display()))?;
    }

    let json_str = serde_json::to_string_pretty(&root).context("serializing hook settings")?;
    std::fs::write(settings_path, json_str)
        .with_context(|| format!("writing {}", settings_path.display()))?;

    Ok(installed)
}

fn install_hook_scripts(hook_dir: &Path, binary_path: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(hook_dir)
        .with_context(|| format!("creating directory {}", hook_dir.display()))?;

    let mut installed = Vec::new();
    for spec in CLAUDE_HOOK_SPECS {
        let path = hook_dir.join(spec.file_name);
        let content = render_hook_script(spec.template, binary_path);
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)
                .with_context(|| format!("reading metadata for {}", path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)
                .with_context(|| format!("marking {} executable", path.display()))?;
        }
        installed.push(path);
    }
    Ok(installed)
}

fn install_claude_hooks(binary_path: &Path) -> Result<()> {
    let hook_dir = claude_hooks_dir()?;
    let settings_path = claude_settings_path()?;
    let installed_files = install_hook_scripts(&hook_dir, binary_path)?;
    let installed_events = write_claude_hook_settings(&settings_path, &hook_dir)?;

    for path in installed_files {
        println!("Installed Claude Code hook script {}", path.display());
    }

    if installed_events.is_empty() {
        println!(
            "Claude Code lifecycle hooks already configured in {}",
            settings_path.display()
        );
    } else {
        println!(
            "Configured Claude Code lifecycle hooks ({}) in {}",
            installed_events.join(", "),
            settings_path.display()
        );
        println!("Restart Claude Code to pick up the new hooks.");
    }

    Ok(())
}

fn write_json_config(editor: &Editor, config_path: &Path, binary_path: &Path) -> Result<()> {
    let existing: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parsing JSON from {}", config_path.display()))?
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    if config_path.exists() {
        let bak = backup_path(config_path);
        std::fs::copy(config_path, &bak)
            .with_context(|| format!("creating backup {}", bak.display()))?;
    }

    let mut root = match existing {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };

    let binary_str = binary_path.to_string_lossy().to_string();
    let hyphae_entry = match editor {
        Editor::VsCode => serde_json::json!({
            "command": binary_str,
            "args": ["serve"],
            "type": "stdio"
        }),
        Editor::Zed => serde_json::json!({
            "command": {
                "path": binary_str,
                "args": ["serve"]
            }
        }),
        _ => serde_json::json!({
            "command": binary_str,
            "args": ["serve"]
        }),
    };

    let top_key = match editor {
        Editor::VsCode => "servers",
        Editor::Zed => "context_servers",
        _ => "mcpServers",
    };

    let servers = root
        .entry(top_key)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(map) = servers {
        map.insert("hyphae".to_string(), hyphae_entry);
    }

    let json_str = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serializing JSON config")?;
    std::fs::write(config_path, json_str)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(())
}

fn write_toml_config(config_path: &Path, binary_path: &Path) -> Result<()> {
    let existing: toml::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("parsing TOML from {}", config_path.display()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    if config_path.exists() {
        let bak = backup_path(config_path);
        std::fs::copy(config_path, &bak)
            .with_context(|| format!("creating backup {}", bak.display()))?;
    }

    let mut root = match existing {
        toml::Value::Table(map) => map,
        _ => toml::map::Map::new(),
    };

    let binary_str = binary_path.to_string_lossy().to_string();

    let mut hyphae_table = toml::map::Map::new();
    hyphae_table.insert("command".to_string(), toml::Value::String(binary_str));
    hyphae_table.insert(
        "args".to_string(),
        toml::Value::Array(vec![toml::Value::String("serve".to_string())]),
    );

    let mcp_servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if let toml::Value::Table(map) = mcp_servers {
        map.insert("hyphae".to_string(), toml::Value::Table(hyphae_table));
    }

    let toml_str =
        toml::to_string_pretty(&toml::Value::Table(root)).context("serializing TOML config")?;
    std::fs::write(config_path, toml_str)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(())
}

pub fn run_init(editor: Option<Editor>, mode: InitMode) -> Result<()> {
    let binary_path = resolve_hyphae_binary()?;

    if matches!(mode, InitMode::Mcp | InitMode::All) {
        if let Some(ed) = editor {
            write_config(&ed, &binary_path)?;
            println!("Configured hyphae for {ed}");
        } else {
            let detected = detect_editors();
            if detected.is_empty() {
                println!("No supported editors detected. Supported editors:");
                println!("  --editor claude-code");
                println!("  --editor cursor");
                println!("  --editor vs-code");
                println!("  --editor zed");
                println!("  --editor windsurf");
                println!("  --editor amp");
                println!("  --editor claude-desktop");
                println!("  --editor codex-cli");
                return Err(anyhow::anyhow!("no editors detected"));
            }
            for ed in detected {
                write_config(&ed, &binary_path)?;
                println!("Configured hyphae for {ed}");
            }
        }
    }

    if matches!(mode, InitMode::Hook | InitMode::All) {
        install_claude_hooks(&binary_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fake_binary() -> PathBuf {
        PathBuf::from("/usr/local/bin/hyphae")
    }

    #[test]
    fn test_merge_preserves_existing_mcp_servers() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("claude.json");

        let existing = serde_json::json!({
            "mcpServers": {
                "other-tool": {
                    "command": "/usr/bin/other",
                    "args": ["run"]
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        // Temporarily override config_path_for by writing directly
        write_json_config(&Editor::ClaudeCode, &config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Both entries must be present
        assert!(value["mcpServers"]["other-tool"].is_object());
        assert!(value["mcpServers"]["hyphae"].is_object());
        assert_eq!(
            value["mcpServers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
    }

    #[test]
    fn test_creates_config_from_scratch() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("mcp.json");

        assert!(!config_path.exists());
        write_json_config(&Editor::Cursor, &config_path, &fake_binary()).unwrap();

        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            value["mcpServers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
        assert_eq!(
            value["mcpServers"]["hyphae"]["args"][0].as_str().unwrap(),
            "serve"
        );
    }

    #[test]
    fn test_backup_created_before_modification() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("settings.json");

        let original = serde_json::json!({ "existing": true });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&original).unwrap(),
        )
        .unwrap();

        write_json_config(&Editor::Zed, &config_path, &fake_binary()).unwrap();

        let bak_path = backup_path(&config_path);
        assert!(
            bak_path.exists(),
            "backup file should exist at {}",
            bak_path.display()
        );

        let bak_content = fs::read_to_string(&bak_path).unwrap();
        let bak_value: serde_json::Value = serde_json::from_str(&bak_content).unwrap();
        assert!(bak_value["existing"].as_bool().unwrap());
    }

    #[test]
    fn test_vscode_uses_servers_key() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("settings.json");

        write_json_config(&Editor::VsCode, &config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(value["servers"]["hyphae"].is_object());
        assert_eq!(
            value["servers"]["hyphae"]["type"].as_str().unwrap(),
            "stdio"
        );
    }

    #[test]
    fn test_codex_cli_writes_toml() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        write_toml_config(&config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            value["mcp_servers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
    }

    #[test]
    fn test_write_claude_hook_settings_installs_all_lifecycle_hooks() {
        let dir = TempDir::new().unwrap();
        let settings_path = dir.path().join("settings.json");
        let hook_dir = dir.path().join("hooks");

        let installed = write_claude_hook_settings(&settings_path, &hook_dir).unwrap();
        assert_eq!(installed, vec!["PostToolUse", "PreCompact", "SessionEnd"]);

        let content = fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            value["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
            hook_dir.join("hyphae-post-tool.sh").to_string_lossy()
        );
        assert_eq!(
            value["hooks"]["PreCompact"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
            hook_dir.join("hyphae-precompact.sh").to_string_lossy()
        );
        assert_eq!(
            value["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
            hook_dir.join("hyphae-session-end.sh").to_string_lossy()
        );
    }

    #[test]
    fn test_write_claude_hook_settings_preserves_existing_entries() {
        let dir = TempDir::new().unwrap();
        let settings_path = dir.path().join("settings.json");
        let hook_dir = dir.path().join("hooks");

        let existing = serde_json::json!({
            "hooks": {
                "PostToolUse": [{
                    "matcher": "Write",
                    "hooks": [{
                        "type": "command",
                        "command": "/tmp/existing.sh"
                    }]
                }]
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        write_claude_hook_settings(&settings_path, &hook_dir).unwrap();

        let content = fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let post_tool_use = value["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post_tool_use.len(), 2);
        assert_eq!(
            post_tool_use[0]["hooks"][0]["command"].as_str().unwrap(),
            "/tmp/existing.sh"
        );
    }

    #[test]
    fn test_write_claude_hook_settings_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let settings_path = dir.path().join("settings.json");
        let hook_dir = dir.path().join("hooks");

        write_claude_hook_settings(&settings_path, &hook_dir).unwrap();
        let installed = write_claude_hook_settings(&settings_path, &hook_dir).unwrap();
        assert!(installed.is_empty());

        let content = fs::read_to_string(&settings_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(value["hooks"]["PreCompact"].as_array().unwrap().len(), 1);
        assert_eq!(value["hooks"]["SessionEnd"].as_array().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_install_hook_scripts_embeds_binary_path_and_sets_executable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let hook_dir = dir.path().join("hooks");
        let binary = PathBuf::from("/opt/hyphae/bin/hyphae");

        let installed = install_hook_scripts(&hook_dir, &binary).unwrap();
        assert_eq!(installed.len(), 3);

        let script = fs::read_to_string(hook_dir.join("hyphae-session-end.sh")).unwrap();
        assert!(script.contains("/opt/hyphae/bin/hyphae"));
        assert!(!script.contains(HYPHAE_BIN_PLACEHOLDER));

        let mode = fs::metadata(hook_dir.join("hyphae-session-end.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0);
    }
}
