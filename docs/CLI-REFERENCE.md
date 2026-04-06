# Hyphae -- CLI Reference

This file documents all CLI commands exposed by the `hyphae` binary. Each entry covers syntax, option tables, and concrete examples. Commands are grouped into five categories: episodic memory, memoir knowledge graphs, documents and RAG, administration/maintenance, and configuration/setup—plus the benchmark suite.

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
- [Documents and RAG](#documents-and-rag)
  - [`hyphae ingest`](#hyphae-ingest----ingest-files-for-rag-search)
  - [`hyphae search-docs`](#hyphae-search-docs----search-ingested-documents)
  - [`hyphae list-sources`](#hyphae-list-sources----list-ingested-sources)
  - [`hyphae forget-source`](#hyphae-forget-source----remove-an-ingested-source)
  - [`hyphae search-all`](#hyphae-search-all----unified-cross-store-search)
- [Administration and maintenance](#administration-and-maintenance)
  - [`hyphae session`](#hyphae-session----session-lifecycle-tracking)
  - [`hyphae feedback`](#hyphae-feedback----structured-feedback-signals)
  - [`hyphae topics`](#hyphae-topics----list-topics)
  - [`hyphae stats`](#hyphae-stats----global-statistics)
  - [`hyphae decay`](#hyphae-decay----apply-decay-manually)
  - [`hyphae prune`](#hyphae-prune----delete-low-weight-memories)
  - [`hyphae consolidate`](#hyphae-consolidate----consolidate-a-topic)
  - [`hyphae embed`](#hyphae-embed----generate-embeddings)
  - [`hyphae export-training`](#hyphae-export-training----export-memories-as-training-jsonl)
  - [`hyphae backup`](#hyphae-backup----backup-the-database)
  - [`hyphae restore`](#hyphae-restore----restore-from-backup)
  - [`hyphae evaluate`](#hyphae-evaluate----measure-agent-improvement)
- [Configuration and setup](#configuration-and-setup)
  - [`hyphae init`](#hyphae-init----automatic-configuration)
  - [`hyphae codex-notify`](#hyphae-codex-notify----handle-codex-notify-events)
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
hyphae memoir list [--json]
```

No arguments. Displays all memoirs with their concept counts.
Use `--json` for a structured payload shaped for programmatic consumers.

---

### `hyphae memoir show` -- Show a memoir

```
hyphae memoir show <name> [--query <query>] [--limit <n>] [--offset <n>] [--json]
```

| Argument | Required | Description |
|----------|----------|-------------|
| `name` | yes (positional) | Memoir name |

```bash
hyphae memoir show backend-arch
hyphae memoir show backend-arch --query "auth" --limit 5 --offset 10
```

Displays stats, labels used, and concepts in the memoir. Use `--json` for a structured payload.

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
hyphae memoir search -m <memoir> <query> [-L <label>] [-l <limit>] [--offset <n>] [--json]
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

Use `--json` to emit a structured search payload.

---

### `hyphae memoir search-all` -- Search across all memoirs

```
hyphae memoir search-all <query> [-l <limit>] [--offset <n>] [--json]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `query` | -- | yes (positional) | -- | Search query |
| `--limit` | `-l` | no | `10` | Max number of results |

```bash
hyphae memoir search-all "database"
```

The JSON payload includes the memoir for each hit so callers do not need to join tables themselves.

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
hyphae memoir inspect -m <memoir> <name> [-D <depth>] [--json]
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

Use `--json` to emit the concept and neighborhood graph as structured data.

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

## Documents and RAG

### `hyphae ingest` -- Ingest files for RAG search

```
hyphae ingest <path> [--recursive] [--force]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `<path>` | -- | yes | -- | File or directory to ingest |
| `--recursive` | `-r` | no | false | Recurse into subdirectories |
| `--force` | `-f` | no | false | Re-ingest even if source already exists |

Ingest a file or directory into the document store. Files are automatically chunked based on type:
- **Markdown** (`.md`, `.mdx`): split by heading (max 500 tokens per chunk)
- **Code** (`.rs`, `.py`, `.js`, `.ts`, `.go`, etc.): split by function
- **Text** (`.txt`, `.log`, `.json`, `.toml`, etc.): sliding window (500 words, 50 overlap)

Binary files are detected and skipped. Hidden files and build directories (`target/`, `node_modules/`, `.git/`) are excluded.

```bash
# Ingest a single file
hyphae ingest README.md

# Ingest a directory
hyphae ingest src/

# Ingest recursively
hyphae ingest . --recursive

# Force re-ingest
hyphae ingest src/main.rs --force
```

---

### `hyphae search-docs` -- Search ingested documents

```
hyphae search-docs <query> [--limit N]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `<query>` | -- | yes | -- | Search query |
| `--limit` | `-l` | no | `10` | Maximum results |

Searches ingested document chunks using hybrid search (vector + FTS) when embeddings are available, or FTS-only otherwise.

```bash
# Search for authentication logic
hyphae search-docs "authentication middleware"

# Limit to top 3
hyphae search-docs "database pooling" --limit 3
```

---

### `hyphae list-sources` -- List ingested sources

```
hyphae list-sources
```

Lists all ingested document sources with their file type, chunk count, and ingestion date.

```bash
hyphae list-sources
```

Example output:
```
Path                                                         Type       Chunks   Ingested
------------------------------------------------------------------------------------------
/home/user/project/src/main.rs                               Code       5        2024-03-05
/home/user/project/README.md                                 Markdown   3        2024-03-05
```

---

### `hyphae forget-source` -- Remove an ingested source

```
hyphae forget-source <path>
```

| Option | Required | Description |
|--------|----------|-------------|
| `<path>` | yes | Source path to remove (as shown by `list-sources`) |

Removes a document source and all its associated chunks from the store.

```bash
hyphae forget-source /home/user/project/src/old_module.rs
```

---

### `hyphae search-all` -- Unified cross-store search

```
hyphae search-all <query> [--limit N] [--include-docs] [--project-root <path> --worktree-id <id>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `<query>` | -- | yes | -- | Search query |
| `--limit` | `-l` | no | `10` | Maximum total results |
| `--include-docs` | -- | no | `true` | Include document chunks in results |
| `--project-root` | -- | no | -- | Identity v1 repository root used with `--worktree-id` to scope memory results to the active worktree |
| `--worktree-id` | -- | no | -- | Identity v1 worktree identifier used with `--project-root` to scope memory results to the active worktree |

Searches across both episodic memories and ingested documents, merging results using Reciprocal Rank Fusion (RRF) for unified relevance ranking.
When the identity-v1 pair is supplied, memory results are scoped to the active worktree and `_shared` memories remain visible. Document chunks remain project-scoped. The identity flags must be supplied together.

```bash
# Search everything
hyphae search-all "database connection"

# Memories only (exclude docs)
hyphae search-all "database connection" --include-docs false

# Limit results
hyphae search-all "auth" --limit 5

# Scope memory results to one worktree while keeping docs project-scoped
hyphae search-all "auth" --project-root /repo/demo --worktree-id wt-alpha
```

---

## Administration and maintenance

### `hyphae session` -- Session lifecycle tracking

```
hyphae session start --project <project> [--task <task>] [--scope <scope>]
hyphae session end --id <session-id> [--summary <text>] [--file <path> ...] [--errors <count>]
hyphae session context --project <project> [--scope <scope>] [--limit <n>]
hyphae session status --id <session-id>
```

Use these commands when you want structured session records instead of only
free-form memory entries. This is the session lifecycle surface Cortina can use
to start a work session on the first meaningful event and close it on session
stop with files changed and errors encountered. Use `--scope` when one project
can have multiple active workers or host runtimes at the same time and they
should not share a single active session.

**Examples:**

```bash
# Start a session
hyphae session start --project api --task "fix flaky auth tests"

# Start a parallel worker-scoped session
hyphae session start --project api --scope worker-a --task "parallel validation shard"

# End it with outcome metadata
hyphae session end \
  --id ses_01ABC... \
  --summary "fixed auth timing issue and stabilized tests" \
  --file src/auth.rs \
  --file tests/auth_test.rs \
  --errors 0

# Review recent sessions
hyphae session context --project api --limit 10

# Review only one worker/runtime lane
hyphae session context --project api --scope worker-a --limit 10

# Read one session as structured JSON
hyphae session status --id ses_01ABC...
```

### `hyphae feedback` -- Structured feedback signals

```
hyphae feedback signal --session-id <session-id> --type <signal-type> --value <integer> [--source <name>] [--project <project>]
```

Use this when a runtime or hook already knows the current session and wants to
record a structured signal instead of only storing a topic memory. This is the
CLI surface Cortina now uses for corrections and resolved errors, while
`hyphae session end` records the final session-success or session-failure
signal.

**Examples:**

```bash
# Record a correction
hyphae feedback signal \
  --session-id ses_01HY... \
  --type correction \
  --value -1 \
  --source cortina.post_tool_use \
  --project api

# Record a positive recovery signal
hyphae feedback signal \
  --session-id ses_01HY... \
  --type error_resolved \
  --value 1 \
  --source cortina.post_tool_use \
  --project api
```

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

### `hyphae export-training` -- Export memories as training JSONL

```
hyphae export-training --format <sft|dpo|alpaca> [-t <topic>] [-o <output>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--format` | `-f` | yes | -- | Output format: `sft`, `dpo`, `alpaca` |
| `--topic` | `-t` | no | -- | Limit export to a topic |
| `--output` | `-o` | no | stdout | Write to file instead of stdout |

Exports memories as training JSONL for supervised fine-tuning (SFT), direct preference optimization (DPO), or Alpaca format. `hyphae export-training-data` is kept as a compatibility alias.

**Format details:**

- `sft`: `{"instruction": "...", "response": "..."}` pairs from memories
- `dpo`: `{"prompt": "...", "chosen": "...", "rejected": "..."}` from corrections and errors
- `alpaca`: `{"instruction": "...", "input": "...", "output": "..."}` format

```bash
# Export decisions as SFT pairs
hyphae export-training --format sft --topic "decisions-api" -o sft_decisions.jsonl

# Export all corrections as DPO pairs (for preference training)
hyphae export-training --format dpo --topic "corrections" -o dpo_pairs.jsonl

# Export everything as Alpaca format
hyphae export-training --format alpaca -o full_training.jsonl
```

---

### `hyphae backup` -- Backup the database

```
hyphae backup [-o <output-path>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--output` | `-o` | no | `./hyphae-backup-<timestamp>.db` | Backup file path |

Creates a complete backup of the Hyphae database including all memories, memoirs, documents, and metadata.

```bash
# Automatic timestamped backup
hyphae backup

# Specific location
hyphae backup --output /backups/hyphae-2024-03.db
```

---

### `hyphae restore` -- Restore from backup

```
hyphae restore <backup-path> [--verify]
```

| Argument | Required | Description |
|----------|----------|-------------|
| `backup-path` | yes (positional) | Path to backup file |
| `--verify` | no | Verify database integrity before restoring |

Restores the database from a backup. The current database is moved to `.backup` before restoration.

```bash
# Restore from backup
hyphae restore /backups/hyphae-2024-03.db

# Verify before restoring
hyphae restore /backups/hyphae-2024-03.db --verify
```

---

### `hyphae evaluate` -- Measure agent improvement

```
hyphae evaluate [--days <N>] [--metric <metric>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--days` | `-d` | no | `14` | Time window in days |
| `--metric` | `-m` | no | all | Single metric: `recall`, `consolidation`, `lessons`, `code_graph`, `training_export`, `coverage` |

Evaluates agent performance over a time window using 6 metrics:

1. **Recall quality** — How well the agent retrieves relevant memories
2. **Consolidation efficiency** — Topic redundancy and memory merging
3. **Lesson extraction** — Patterns extracted from corrections and errors
4. **Code graph coverage** — Symbols captured via Rhizome export
5. **Training data volume** — Exportable SFT/DPO pairs
6. **Topic coverage** — Distribution across topics

```bash
# Evaluate last 14 days
hyphae evaluate

# Evaluate last 30 days
hyphae evaluate --days 30

# Check only recall quality
hyphae evaluate --metric recall
```

---

## Configuration and setup

### `hyphae init` -- Automatic configuration

```
hyphae init [-m <mode>]
```

| Option | Short | Required | Default | Description |
|--------|-------|----------|---------|-------------|
| `--mode` | `-m` | no | `mcp` | Mode: `mcp`, `hook`, `all` |

**Modes:**

| Mode | Action | Description                                                       |
|------|--------|-------------------------------------------------------------------|
| `mcp` | Configure the MCP server | Auto-detects and configures supported editor MCP settings |
| `hook` | Install Claude Code lifecycle hooks | Installs `PostToolUse`, `PreCompact`, and `SessionEnd` capture hooks |
| `all` | All of the above | Configure MCP plus Claude Code lifecycle hooks |

**Supported MCP targets:**

| Tool | Config file |
|------|-------------|
| Claude Code | `~/.claude.json` |
| Claude Desktop | `~/Library/.../claude_desktop_config.json` |
| Cursor | `~/.cursor/mcp.json` |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` |
| VS Code | `~/Library/.../Code/User/mcp.json` |
| Zed | `~/.zed/settings.json` |
| Amp | `~/.config/amp/settings.json` |
| OpenAI Codex CLI | `~/.codex/config.toml` |

For OpenAI Codex CLI, `hyphae init` also writes:

```toml
notify = ["hyphae", "codex-notify"]

[mcp_servers.hyphae]
command = "/path/to/hyphae"
args = ["serve"]
```

```bash
# Standard setup
hyphae init

# Install everything
hyphae init --mode all

# Just Claude Code lifecycle hooks
hyphae init --mode hook
```

---

### `hyphae codex-notify` -- Handle Codex notify events

```
hyphae codex-notify '<json-notification>'
```

This command is normally invoked by Codex via:

```toml
notify = ["hyphae", "codex-notify"]
```

It stores a compact session summary when Codex emits `agent-turn-complete`, and it also normalizes other Codex notify events into searchable lifecycle notes when they carry useful context.

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

- **[MCP-TOOLS.md](MCP-TOOLS.md)** — All 23 MCP tool definitions (parameters, request/response examples) for AI agent integration
- **[FEATURES.md](FEATURES.md)** — Conceptual guides: Memory vs Memoir, multi-session workflows, topic organization, consolidation, importance levels, decay model, and complete configuration reference
