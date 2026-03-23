# Hyphae Setup by Tool

Step-by-step MCP configuration for every AI coding tool Hyphae supports. In most cases `hyphae init` handles everything automatically — the sections below show what it does under the hood and how to configure manually when needed.

---

## Claude Code

**Setup:**
```bash
hyphae init  # Automatically configures ~/.claude.json
```

**Manual configuration:**
```bash
claude mcp add hyphae -- hyphae serve
```

**Config file:** `~/.claude.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Slash commands (optional):**
```bash
hyphae init --mode skill
```
Installs `/recall` and `/remember` in `~/.claude/commands/`.

**PostToolUse hook (optional):**
```bash
hyphae init --mode hook
```
Installs the recommended Claude Code lifecycle hooks:
- `PostToolUse` for periodic context extraction after tool calls
- `PreCompact` for a lightweight capture marker before compaction
- `SessionEnd` for session-end breadcrumbs

**Compact mode recommended:** Claude Code benefits from compact mode to save tokens. Enable in `~/.config/hyphae/config.toml`:
```toml
[mcp]
compact = true
```

**CLAUDE.md instructions (optional):**
```bash
hyphae init --mode cli
```
Adds Hyphae instructions to the `CLAUDE.md` of the current project.

---

## Cursor

**Setup:**
```bash
hyphae init  # Automatically configures ~/.cursor/mcp.json
```

**Config file:** `~/.cursor/mcp.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Cursor rule (optional):**
```bash
hyphae init --mode skill
```
Creates `~/.cursor/rules/hyphae.mdc` with an `alwaysApply: true` rule that reminds the agent to use Hyphae.

**After configuration:** Restart Cursor. The Hyphae tools appear in the MCP palette.

---

## VS Code / GitHub Copilot

**Setup:**
```bash
hyphae init  # Automatically configures ~/Library/.../Code/User/mcp.json
```

**Config file:**
- macOS: `~/Library/Application Support/Code/User/mcp.json`
- Linux: `~/.config/Code/User/mcp.json`

```json
{
  "servers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Note:** VS Code uses `"servers"` instead of `"mcpServers"`. `hyphae init` handles this difference automatically.

---

## Windsurf

**Setup:**
```bash
hyphae init  # Automatically configures ~/.codeium/windsurf/mcp_config.json
```

**Config file:** `~/.codeium/windsurf/mcp_config.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

---

## Zed

**Setup:**
```bash
hyphae init  # Automatically configures ~/.zed/settings.json
```

Zed uses a different format with `context_servers`:
```json
{
  "context_servers": {
    "hyphae": {
      "command": {
        "path": "/path/to/hyphae",
        "args": ["serve"]
      },
      "settings": {}
    }
  }
}
```

---

## Amp

**Setup:**
```bash
hyphae init  # Automatically configures ~/.config/amp/settings.json
```

**Slash commands (optional):**
```bash
hyphae init --mode skill
```
Installs `/hyphae-recall` and `/hyphae-remember` in `~/.config/amp/skills/`.

---

## OpenAI Codex CLI

**Setup:**
```bash
hyphae init  # Automatically configures ~/.codex/config.toml
```

**Config file (TOML):** `~/.codex/config.toml`
```toml
[mcp_servers.hyphae]
command = "/path/to/hyphae"
args = ["serve"]
```

---

## Claude Desktop

**Setup:**
```bash
hyphae init  # Automatically configures
```

**Config file:** `~/Library/Application Support/Claude/claude_desktop_config.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

---

## Other tools (Gemini, Amazon Q, Cline, Roo Code, Kilo Code, OpenCode)

All are automatically configured by `hyphae init`. The format is always the same:

```json
{
  "command": "/path/to/hyphae",
  "args": ["serve"]
}
```

The only difference is the config file and the JSON key. `hyphae init` handles all these variations.

---

## See also

- [GUIDE.md](GUIDE.md) — Core guide: memory models, MCP tools reference, init modes
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — Fix tool-detection issues, MCP connection errors, and more
