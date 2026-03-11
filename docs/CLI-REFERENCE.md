# Hyphae -- CLI Reference

This file documents all 29 CLI commands exposed by the `hyphae` binary. Each entry covers syntax, option tables, and concrete examples. Commands are grouped into four categories: episodic memory, memoir knowledge graphs, administration/maintenance, and configuration/setup — plus the benchmark suite.

All commands accept the global `--db <path>` flag to override the default database location.

## Table of Contents

- [Global option](#global-option)
- [Memories (episodic)](#memories-episodic)
  - [`hyphae store`](#hyphae-store----store-a-memory)
  - [`hyphae recall`](#hyphae-recall----search-memories)
  - [`hyphae list`](#hyphae-list----list-memories)
  - [`hyphae forget`](#hyphae-forget----delete-a-memory)
  - [`hyphae extract`](#hyphae-extract----fact-extraction-zero-llm-cost)
  - [`hyphae recall-context`](#hyphae-recall-context----context-injection)
- [Memoir (knowledge graphs)](#memoir-knowledge-graphs)
  - [`hyphae memoir create`](#hyphae-memoir-create----create-a-memoir)
  - [`hyphae memoir list`](#hyphae-memoir-list----list-memoirs)
  - [`hyphae memoir show`](#hyphae-memoir-show----show-a-memoir)
  - [`hyphae memoir delete`](#hyphae-memoir-delete----delete-a-memoir)
  - [`hyphae memoir add-concept`](#hyphae-memoir-add-concept----add-a-concept)
  - [`hyphae memoir refine`](#hyphae-memoir-refine----refine-a-concept)
  - [`hyphae memoir search`](#hyphae-memoir-search----search-within-a-memoir)
  - [`hyphae memoir search-all`](#hyphae-memoir-search-all----search-across-all-memoirs)
  - [`hyphae memoir link`](#hyphae-memoir-link----link-two-concepts)
  - [`hyphae memoir inspect`](#hyphae-memoir-inspect----inspect-a-concept-and-its-neighborhood)
  - [`hyphae memoir distill`](#hyphae-memoir-distill----distill-memories-into-concepts)
- [Administration and maintenance](#administration-and-maintenance)
  - [`hyphae topics`](#hyphae-topics----list-topics)
  - [`hyphae stats`](#hyphae-stats----global-statistics)
  - [`hyphae decay`](#hyphae-decay----apply-decay-manually)
  - [`hyphae prune`](#hyphae-prune----delete-low-weight-memories)
  - [`hyphae consolidate`](#hyphae-consolidate----consolidate-a-topic)
  - [`hyphae embed`](#hyphae-embed----generate-embeddings)
- [Configuration and setup](#configuration-and-setup)
  - [`hyphae init`](#hyphae-init----automatic-configuration)
  - [`hyphae config`](#hyphae-config----show-configuration)
  - [`hyphae serve`](#hyphae-serve----start-the-mcp-server)
- [Benchmarks](#benchmarks)
  - [`hyphae bench`](#hyphae-bench----storage-performance-benchmark)
  - [`hyphae bench-recall`](#hyphae-bench-recall----knowledge-retention-benchmark)
  - [`hyphae bench-agent`](#hyphae-bench-agent----agent-efficiency-benchmark)
- [See also](#see-also)

---

## Global option

Available on all commands:

```
--db <path>    Path to the SQLite database (default: platform path)
```

---

## Memories (episodic)

### `hyphae store` -- Store a memory

```
hyphae store -t <topic> -c <content> [-i <importance>] [-k <keywords>] [-r <raw-excerpt>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--topic` | `-t` | yes | -- | Category/namespace of the memory |
| `--content` | `-c` | yes | -- | Content to memorize |
| `--importance` | `-i` | no | `medium` | `critical`, `high`, `medium`, `low` |
| `--keywords` | `-k` | no | -- | Comma-separated keywords |
| `--raw` | `-r` | no | -- | Verbatim excerpt (code, error message) |

**Examples:**

```bash
# Architecture decision
hyphae store -t "decisions-api" -c "Chose REST over GraphQL for v1 simplicity" -i high

# Resolved error with keywords
hyphae store -t "errors" -c "CORS fixed by adding Origin header in nginx" -i medium -k "cors,nginx,fix"

# Critical fact (never forgotten)
hyphae store -t "infra" -c "Prod DB is on port 5433, not 5432" -i critical

# With raw excerpt
hyphae store -t "errors" -c "Compilation error fixed" -r "error[E0382]: borrow of moved value"
```

If embeddings are enabled, the memory is automatically vectorized on storage.

---

### `hyphae recall` -- Search memories

```
hyphae recall <query> [-t <topic>] [-l <limit>] [-k <keyword>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `query` | -- | yes (positional) | -- | Natural language query |
| `--topic` | `-t` | no | -- | Filter by topic |
| `--limit` | `-l` | no | `5` | Max number of results |
| `--keyword` | `-k` | no | -- | Filter by exact keyword |

**Examples:**

```bash
# Broad search
hyphae recall "database choice"

# Filtered by topic
hyphae recall "authentication" --topic "decisions-api" --limit 10

# Filtered by keyword
hyphae recall "nginx error" --keyword "cors"
```

**Automatic behavior:**
- Applies decay if >24h since last run
- Updates the access counter for each result
- Search pipeline: hybrid (if embeddings) -> FTS5 -> keyword LIKE

---

### `hyphae list` -- List memories

```
hyphae list [-t <topic>] [-a] [-s <sort>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--topic` | `-t` | no | -- | Filter by topic |
| `--all` | `-a` | no | false | List all memories |
| `--sort` | `-s` | no | `weight` | Sort by: `weight`, `created`, `accessed` |

**Examples:**

```bash
# List a topic
hyphae list -t "decisions-api"

# All memories sorted by creation date
hyphae list --all --sort created

# Sorted by last access
hyphae list -t "errors" --sort accessed
```

---

### `hyphae forget` -- Delete a memory

```
hyphae forget <id>
```

| Argument | Required | Description |
|----------|----------|-------------|
| `id` | yes (positional) | ULID ID of the memory to delete |

**Example:**

```bash
hyphae forget 01HWXYZ123456789ABCDEF
```

---

### `hyphae extract` -- Fact extraction (zero LLM cost)

```
hyphae extract [-p <project>] [-t <text>] [--dry-run]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--project` | `-p` | no | `project` | Project name for topic namespace |
| `--text` | `-t` | no | stdin | Source text (reads stdin if omitted) |
| `--dry-run` | -- | no | false | Display without storing |

**Examples:**

```bash
# From stdin
echo "The parser uses the Pratt algorithm" | hyphae extract -p my-project

# From a file
cat session-log.txt | hyphae extract -p backend

# Preview without storing
echo "Migrated from MySQL to PostgreSQL for JSONB support" | hyphae extract -p api --dry-run
```

**Detected signals:**

| Signal | Keywords | Score |
|--------|----------|-------|
| Architecture | `uses`, `architecture`, `pattern`, `algorithm` | +3 |
| Error/Fix | `error`, `fixed`, `bug`, `workaround` | +3 |
| Decision | `decided`, `chose`, `prefer`, `switched to` | +4 |
| Config | `configured`, `setup`, `installed`, `enabled` | +2 |
| Dev | `commit`, `deploy`, `migrate`, `refactor` | +2 |

---

### `hyphae recall-context` -- Context injection

```
hyphae recall-context <query> [-l <limit>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `query` | -- | yes (positional) | -- | Search query |
| `--limit` | `-l` | no | `10` | Max number of memories |

Returns a formatted block ready for prompt injection. Used by the SessionStart hook for automatic context loading.

```bash
hyphae recall-context "my-project backend API"
hyphae recall-context "authentication" --limit 20
```

---

## Memoir (knowledge graphs)

### `hyphae memoir create` -- Create a memoir

```
hyphae memoir create -n <name> [-d <description>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--name` | `-n` | yes | -- | Unique memoir name |
| `--description` | `-d` | no | `""` | Memoir description |

```bash
hyphae memoir create -n "backend-arch" -d "Backend architecture decisions"
```

---

### `hyphae memoir list` -- List memoirs

```
hyphae memoir list
```

No arguments. Displays all memoirs with their concept counts.

---

### `hyphae memoir show` -- Show a memoir

```
hyphae memoir show <name>
```

| Argument | Required | Description |
|----------|----------|-------------|
| `name` | yes (positional) | Memoir name |

```bash
hyphae memoir show backend-arch
```

Displays stats, labels used, and all concepts in the memoir.

---

### `hyphae memoir delete` -- Delete a memoir

```
hyphae memoir delete <name>
```

| Argument | Required | Description |
|----------|----------|-------------|
| `name` | yes (positional) | Memoir name |

**Warning:** Cascades deletion of all concepts and links in the memoir.

```bash
hyphae memoir delete old-project
```

---

### `hyphae memoir add-concept` -- Add a concept

```
hyphae memoir add-concept -m <memoir> -n <name> -d <definition> [-l <labels>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--memoir` | `-m` | yes | -- | Memoir name |
| `--name` | `-n` | yes | -- | Concept name (unique within the memoir) |
| `--definition` | `-d` | yes | -- | Dense definition of the concept |
| `--labels` | `-l` | no | -- | Comma-separated labels (`namespace:value` or simple tag) |

```bash
hyphae memoir add-concept -m "backend-arch" -n "user-service" \
  -d "Handles registration, authentication (JWT + OAuth2) and profiles" \
  -l "domain:auth,type:microservice"

hyphae memoir add-concept -m "backend-arch" -n "postgres" \
  -d "Primary database for users and transactions" \
  -l "type:database"
```

---

### `hyphae memoir refine` -- Refine a concept

```
hyphae memoir refine -m <memoir> -n <name> -d <new-definition>
```

| Option | Short | Required | Description |
|--------|-------|----------|-------------|
| `--memoir` | `-m` | yes | Memoir name |
| `--name` | `-n` | yes | Existing concept name |
| `--definition` | `-d` | yes | New definition (replaces the old one) |

Increments the revision and increases concept confidence.

```bash
hyphae memoir refine -m "backend-arch" -n "user-service" \
  -d "Handles registration, auth (JWT + OAuth2), profiles and 2FA. Rate limiting via Redis."
```

---

### `hyphae memoir search` -- Search within a memoir

```
hyphae memoir search -m <memoir> <query> [-L <label>] [-l <limit>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--memoir` | `-m` | yes | -- | Memoir name |
| `query` | -- | yes (positional) | -- | Search query |
| `--label` | `-L` | no | -- | Filter by label (e.g., `domain:auth`) |
| `--limit` | `-l` | no | `10` | Max number of results |

```bash
hyphae memoir search -m "backend-arch" "authentication"
hyphae memoir search -m "backend-arch" "service" --label "domain:auth"
```

---

### `hyphae memoir search-all` -- Search across all memoirs

```
hyphae memoir search-all <query> [-l <limit>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `query` | -- | yes (positional) | -- | Search query |
| `--limit` | `-l` | no | `10` | Max number of results |

```bash
hyphae memoir search-all "database"
```

---

### `hyphae memoir link` -- Link two concepts

```
hyphae memoir link -m <memoir> --from <source> --to <target> -r <relation>
```

| Option | Short | Required | Description |
|--------|-------|----------|-------------|
| `--memoir` | `-m` | yes | Memoir name |
| `--from` | -- | yes | Source concept name |
| `--to` | -- | yes | Target concept name |
| `--relation` | `-r` | yes | Relation type (see below) |

**9 relation types:**

| Relation | Meaning | Example |
|----------|---------|---------|
| `part-of` | A is part of B | `cache-layer` part-of `api-gateway` |
| `depends-on` | A requires B | `user-service` depends-on `postgres` |
| `related-to` | A is associated with B | `auth` related-to `session-mgmt` |
| `contradicts` | A contradicts B | `rest-api` contradicts `graphql-api` |
| `refines` | A refines B | `jwt-auth-v2` refines `jwt-auth` |
| `alternative-to` | A can replace B | `redis` alternative-to `memcached` |
| `caused-by` | A is caused by B | `perf-issue` caused-by `n-plus-one` |
| `instance-of` | A is an instance of B | `user-db` instance-of `postgres` |
| `superseded-by` | A is replaced by B | `mysql-setup` superseded-by `postgres-setup` |

```bash
hyphae memoir link -m "backend-arch" --from "user-service" --to "postgres" -r depends-on
hyphae memoir link -m "backend-arch" --from "user-service" --to "redis" -r depends-on
```

**Use `superseded-by`** to mark obsolete facts instead of deleting them -- the history is valuable.

---

### `hyphae memoir inspect` -- Inspect a concept and its neighborhood

```
hyphae memoir inspect -m <memoir> <name> [-D <depth>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--memoir` | `-m` | yes | -- | Memoir name |
| `name` | -- | yes (positional) | -- | Concept name |
| `--depth` | `-D` | no | `1` | BFS depth for graph exploration |

```bash
# Direct neighbors
hyphae memoir inspect -m "backend-arch" "user-service"

# 2-hop neighborhood
hyphae memoir inspect -m "backend-arch" "user-service" -D 2
```

---

### `hyphae memoir distill` -- Distill memories into concepts

```
hyphae memoir distill --from-topic <topic> --into <memoir>
```

| Option | Required | Description |
|--------|----------|-------------|
| `--from-topic` | yes | Source topic (memories) |
| `--into` | yes | Target memoir (must already exist) |

Transforms memories from a topic into concepts in a memoir. The first keyword becomes the concept name. If a concept with the same name already exists, the definition is merged (refined).

```bash
# Create the memoir first
hyphae memoir create -n "arch-v2" -d "Architecture v2"

# Distill decisions into the memoir
hyphae memoir distill --from-topic "decisions-api" --into "arch-v2"
```

---

## Administration and maintenance

### `hyphae topics` -- List topics

```
hyphae topics
```

No arguments. Displays all topics with entry counts.

```
Topic                          Count
----------------------------------------
decisions-api                  12
errors-resolved                8
preferences                    3
```

---

### `hyphae stats` -- Global statistics

```
hyphae stats
```

```
Memories:  23
Topics:    3
Avg weight: 0.847
Oldest:    2024-01-15 09:30
Newest:    2024-03-05 14:22
```

---

### `hyphae decay` -- Apply decay manually

```
hyphae decay [-f <factor>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--factor` | `-f` | no | `0.95` | Decay factor (0.0 to 1.0) |

```bash
# Standard decay
hyphae decay

# Aggressive decay
hyphae decay --factor 0.8
```

Normally, decay runs automatically during a `recall` if >24h since last run.

---

### `hyphae prune` -- Delete low-weight memories

```
hyphae prune [-t <threshold>] [--dry-run]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--threshold` | `-t` | no | `0.1` | Weight threshold (below = deleted) |
| `--dry-run` | -- | no | false | Preview without deleting |

**Important:** `critical` and `high` memories are never pruned, regardless of their weight.

```bash
# Preview
hyphae prune --threshold 0.2 --dry-run

# Execute
hyphae prune --threshold 0.1
```

---

### `hyphae consolidate` -- Consolidate a topic

```
hyphae consolidate -t <topic> [--keep-originals]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--topic` | `-t` | yes | -- | Topic to consolidate |
| `--keep-originals` | -- | no | false | Keep originals after consolidation |

Consolidation merges all memories in a topic into a single summary. The consolidated summary's importance is the highest of the originals. Keywords are merged.

```bash
# Replace all memories with a summary
hyphae consolidate --topic "errors-resolved"

# Keep originals
hyphae consolidate --topic "errors-resolved" --keep-originals
```

---

### `hyphae embed` -- Generate embeddings

```
hyphae embed [-t <topic>] [--force] [-b <batch-size>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--topic` | `-t` | no | -- | Limit to a topic |
| `--force` | -- | no | false | Re-embed even those that already have an embedding |
| `--batch-size` | `-b` | no | `32` | Embedding batch size |

Requires the `embeddings` feature. If compiled without it, the command fails with an explicit message.

```bash
# All memories without an embedding
hyphae embed

# Re-embed everything (after model change)
hyphae embed --force

# A single topic
hyphae embed --topic "decisions-api"
```

---

## Configuration and setup

### `hyphae init` -- Automatic configuration

```
hyphae init [-m <mode>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--mode` | `-m` | no | `mcp` | Mode: `mcp`, `cli`, `skill`, `hook`, `all` |

**Modes:**

| Mode | Action | Description                                                       |
|------|--------|-------------------------------------------------------------------|
| `mcp` | Configure the MCP server | Auto-detects and configures 14 AI tools                           |
| `cli` | Inject into CLAUDE.md | Adds `hyphae store`/`hyphae recall` instructions                  |
| `skill` | Install slash commands | `/recall`, `/remember` for Claude Code, `.mdc` for Cursor, etc.   |
| `hook` | Install PostToolUse hook | Automatic extraction after each tool call                         |
| `all` | All of the above | Configure MCP + CLI + Skills + Hook                               |

**14 supported tools (MCP mode):**

| Tool | Config file |
|------|-------------|
| Claude Code | `~/.claude.json` |
| Claude Desktop | `~/Library/.../claude_desktop_config.json` |
| Cursor | `~/.cursor/mcp.json` |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` |
| VS Code / Copilot | `~/Library/.../Code/User/mcp.json` |
| Gemini Code Assist | `~/.gemini/settings.json` |
| Zed | `~/.zed/settings.json` |
| Amp | `~/.config/amp/settings.json` |
| Amazon Q | `~/.aws/amazonq/mcp.json` |
| Cline | VS Code globalStorage |
| Roo Code | VS Code globalStorage |
| Kilo Code | VS Code globalStorage |
| OpenAI Codex CLI | `~/.codex/config.toml` |
| OpenCode | `~/.config/opencode/opencode.json` |

```bash
# Standard setup
hyphae init

# Install everything
hyphae init --mode all

# Just slash commands
hyphae init --mode skill
```

---

### `hyphae config` -- Show configuration

```
hyphae config
```

No arguments. Displays the active configuration with all sections.

```
Config: ~/.config/hyphae/config.toml (loaded)

[store]
  path = (default platform path)

[memory]
  default_importance = medium
  decay_rate = 0.95
  prune_threshold = 0.1

[embeddings]
  model = BAAI/bge-small-en-v1.5

[extraction]
  enabled = true
  min_score = 3.0
  max_facts = 10

[recall]
  enabled = true
  limit = 15

[mcp]
  transport = stdio
  compact = true
```

---

### `hyphae serve` -- Start the MCP server

```
hyphae serve [--compact]
```

| Option | Required | Default | Description |
|--------|----------|---------|-------------|
| `--compact` | no | false | Short responses (~40% fewer tokens) |

The `--compact` flag takes precedence. Otherwise, the value from `config.toml` (`[mcp] compact = true`) is used.

```bash
# Standard
hyphae serve

# Compact mode (saves ~40% tokens)
hyphae serve --compact

# Quick test
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | hyphae serve
```

---

## Benchmarks

### `hyphae bench` -- Storage performance benchmark

```
hyphae bench [-c <count>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--count` | `-c` | no | `1000` | Number of memories to generate |

```bash
hyphae bench --count 1000
```

Typical result:
```
Store (no embeddings)      1000 ops      34.2 ms      34.2 us/op
Store (with embeddings)    1000 ops      51.6 ms      51.6 us/op
FTS5 search                 100 ops       4.7 ms      46.6 us/op
Vector search (KNN)         100 ops      59.0 ms     590.0 us/op
Hybrid search               100 ops      95.1 ms     951.1 us/op
Decay (batch)                 1 ops       5.8 ms       5.8 ms/op
```

---

### `hyphae bench-recall` -- Knowledge retention benchmark

```
hyphae bench-recall [-m <model>] [-r <runs>] [-v]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--model` | `-m` | no | `sonnet` | Model to use |
| `--runs` | `-r` | no | `1` | Number of runs to average |
| `--verbose` | `-v` | no | false | Show injected context |

Measures the agent's ability to recall facts from a technical document across sessions. Uses real API calls.

```bash
hyphae bench-recall --model haiku --runs 5
```

---

### `hyphae bench-agent` -- Agent efficiency benchmark

```
hyphae bench-agent [-s <sessions>] [-m <model>] [-r <runs>] [-v]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--sessions` | `-s` | no | `10` | Number of sessions per mode |
| `--model` | `-m` | no | `sonnet` | Model to use |
| `--runs` | `-r` | no | `1` | Number of runs to average |
| `--verbose` | `-v` | no | false | Show extracted facts and context |

Compares turns, tokens and costs with and without HYPHAE on a real Rust project.

```bash
hyphae bench-agent --sessions 10 --model haiku --runs 3
```

---

## See also

- **[MCP-TOOLS.md](MCP-TOOLS.md)** — All 18 MCP tool definitions (parameters, request/response examples) for AI agent integration
- **[FEATURES.md](FEATURES.md)** — Conceptual guides: Memory vs Memoir, multi-session workflows, topic organization, consolidation, importance levels, decay model, and complete configuration reference
