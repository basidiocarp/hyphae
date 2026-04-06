# Hyphae -- Feature Guides

This file covers the conceptual and operational guides for Hyphae: when to use each memory system, how to structure multi-session workflows, best practices for topic hygiene, and the full configuration reference. For command syntax and tool definitions, see the linked reference documents below.

## Table of Contents

- [Overview](#overview)
- [Memory vs Memoir: when to use which](#memory-vs-memoir-when-to-use-which)
- [Multi-session workflow](#multi-session-workflow)
- [Topic organization](#topic-organization)
- [Consolidation guide](#consolidation-guide)
- [Importance levels guide](#importance-levels-guide)
- [Decay model explained](#decay-model-explained)
- [Document Memory (RAG)](#document-memory-rag)
- [Complete configuration](#complete-configuration)
- [See also](#see-also)

---

## Overview

Hyphae offers two complementary memory systems. Memories are episodic: temporal storage with decay, organized by topic. Important memories persist, trivial ones fade naturally. Memoirs are semantic: permanent knowledge graphs organized by memoir, containing concepts linked by typed relations. Concepts are refined, never deleted.

The CLI offers 29 commands. The MCP server exposes 23 tools (9 memory + 9 memoir + 5 RAG). Both access the same SQLite database.

---

## Memory vs Memoir: when to use which

### Quick comparison

| Aspect | Memory (episodic) | Memoir (semantic) |
|--------|-------------------|-------------------|
| **Lifespan** | Temporary (decay) | Permanent |
| **Organization** | By topic (flat) | By memoir (graph) |
| **Granularity** | One fact, one decision | One concept with relations |
| **Search** | FTS + vector | FTS + labels + graph BFS |
| **Evolution** | Memory decays or is pruned | Concept is refined (revision++) |
| **Best for** | Events, errors, one-off decisions | Architecture, domain models, structured knowledge |

### Concrete examples

**Use Memory when...**

```bash
# An error was just resolved
hyphae store -t "errors" -c "Timeout fixed by increasing pool_max_size to 20" -i medium -k "timeout,pool"

# A preference is discovered
hyphae store -t "preferences" -c "User prefers absolute imports" -i high

# A temporary fact
hyphae store -t "sprint-context" -c "Current sprint is focused on billing" -i low
```

**Use Memoir when...**

```bash
# Model architecture as a graph
hyphae memoir create -n "arch" -d "System architecture"
hyphae memoir add-concept -m "arch" -n "api-gateway" -d "Single entry point, routing, rate limiting" -l "type:service"
hyphae memoir add-concept -m "arch" -n "user-db" -d "PostgreSQL 16, schema users/sessions" -l "type:database"
hyphae memoir link -m "arch" --from "api-gateway" --to "user-db" -r depends-on

# Document a business domain
hyphae memoir create -n "billing-domain" -d "Billing domain concepts"
hyphae memoir add-concept -m "billing-domain" -n "invoice" -d "Invoice with line items, VAT, due date"
hyphae memoir add-concept -m "billing-domain" -n "payment" -d "Payment linked to one or more invoices"
hyphae memoir link -m "billing-domain" --from "payment" --to "invoice" -r related-to
```

### Combined workflow: from memory to graph

1. **Sessions 1-5**: Store decisions as memories
   ```bash
   hyphae store -t "decisions-api" -c "Chose PostgreSQL for JSONB" -i high
   hyphae store -t "decisions-api" -c "Redis for session cache" -i high
   hyphae store -t "decisions-api" -c "JWT for API auth" -i high
   ```

2. **Session 6**: When patterns emerge, distill into a memoir
   ```bash
   hyphae memoir create -n "api-arch" -d "API v1 Architecture"
   hyphae memoir distill --from-topic "decisions-api" --into "api-arch"
   ```

3. **Sessions 7+**: Refine the graph and continue storing events
   ```bash
   hyphae memoir refine -m "api-arch" -n "postgres" -d "PostgreSQL 16 + JSONB + PostGIS"
   hyphae store -t "decisions-api" -c "Added PostGIS for geolocation" -i high
   ```

---

## Multi-session workflow

### Concrete scenario: API project over 3 days

#### Session 1 -- Project start

The agent starts with no context.

```
Agent: hyphae_memory_recall("API project setup")
HYPHAE: No memories found.

Agent: (reads code, makes decisions)

Agent: hyphae_memory_store({
  topic: "api-project",
  content: "New REST API project with Actix-web. Structure: src/routes/, src/models/, src/db/",
  importance: "high",
  keywords: ["actix", "rust", "api", "structure"]
})
HYPHAE: Stored memory: 01HW...A1

Agent: hyphae_memory_store({
  topic: "api-project",
  content: "PostgreSQL chosen for JSONB support and Rust ecosystem maturity",
  importance: "high",
  keywords: ["postgres", "database", "decision"]
})
HYPHAE: Stored memory: 01HW...A2
```

#### Session 2 -- Resuming the next day

The agent starts and recalls context.

```
Agent: hyphae_memory_recall("API project")
HYPHAE:
  [api-project] New REST API project with Actix-web. Structure: src/routes/, src/models/, src/db/
  [api-project] PostgreSQL chosen for JSONB support and Rust ecosystem maturity

Agent: (doesn't need to re-read code to understand the context)
Agent: (fixes a bug and stores the resolution)

Agent: hyphae_memory_store({
  topic: "errors-resolved",
  content: "Auth middleware was blocking public routes -- added exclusion filter by prefix",
  importance: "medium",
  keywords: ["auth", "middleware", "routing", "fix"]
})
```

#### Session 3 -- Same problem recurs

```
Agent: hyphae_memory_recall("auth middleware issue")
HYPHAE:
  [errors-resolved] Auth middleware was blocking public routes -- added exclusion filter by prefix

Agent: (applies the known solution directly without re-debugging)
```

### Key points of multi-session workflow

1. At session start: always `hyphae_memory_recall` with project context
2. After each important decision: `hyphae_memory_store` with importance `high`
3. After each bug fix: `hyphae_memory_store` with specific keywords
4. Periodically: `hyphae_memory_health` to check topic hygiene
5. When a topic grows: `hyphae_memory_consolidate` to densify

### Context-aware recall

`hyphae_memory_recall` now makes a few query-context decisions explicitly:

- Session/history queries such as "previous session login flow" boost `session/*` memories ahead of general matches.
- When `code_context: true` and the query looks code-related, recall gathers matching concepts in `code:{project}` using extracted code terms before finalizing results, then merges globally visible `_shared` memories as fallback.
- Identity-v1 worktree scoping still applies to the primary recall pass and any code-context expansion, while `_shared` fallback memories remain visible.

---

## Topic organization

### Best practices

#### Naming

| Pattern | Example | When to use |
|---------|---------|-------------|
| `{project}` | `my-api` | General project context |
| `decisions-{project}` | `decisions-api` | Architecture and design decisions |
| `errors-resolved` | `errors-resolved` | Fixed bugs and their solutions |
| `preferences` | `preferences` | Code style, tool preferences |
| `conventions-{project}` | `conventions-api` | Code conventions, naming, structure |
| `infra` | `infra` | URLs, ports, server configuration |
| `context-{sprint}` | `context-sprint-3` | Temporary sprint context |

#### Basic rules

1. One topic per concern -- don't mix decisions and errors in the same topic
2. Prefix by project when working on multiple projects
3. Use `critical` for invariant facts (ports, URLs, credentials)
4. Use `low` for temporary notes that don't need to persist
5. Consolidate regularly when a topic exceeds 7-10 entries
6. Don't create overly granular topics -- `cors-errors` is too narrow, `errors-resolved` is sufficient

#### Anti-patterns

- `todo` -- HYPHAE is not a task manager
- `misc` / `miscellaneous` -- Too vague, impossible to recall efficiently
- One topic per file -- Excessive granularity, use keywords instead
- Everything as `critical` -- Defeats the purpose of decay, everything will be kept indefinitely

---

## Consolidation guide

### When to consolidate?

- **HYPHAE says so**: the MCP warns when a topic exceeds 7 entries
- **The audit shows it**: `hyphae health` reports `needs_consolidation=true`
- **Manually**: when you feel a topic has become noisy

### How to consolidate?

#### Via CLI (automatic consolidation)

```bash
# See the state
hyphae health

# Consolidate by replacing all memories
hyphae consolidate --topic "errors-resolved"

# Or keep originals (summary is added, not replaced)
hyphae consolidate --topic "errors-resolved" --keep-originals
```

The CLI merges automatically: it concatenates summaries with ` | `, merges keywords, and takes the highest importance.

#### Via MCP (agent-guided consolidation)

The agent does a better job because it understands the content:

```
Agent: hyphae_memory_recall("errors-resolved" topic, limit 20)
HYPHAE: (returns 12 memories)

Agent: (synthesizes an intelligent summary)

Agent: hyphae_memory_consolidate({
  topic: "errors-resolved",
  summary: "Main resolved errors: 1) CORS fixed via nginx proxy_set_header 2) DB memory leak fixed by closing connections with defer 3) Rate limiting added on /api/auth to counter brute force 4) Actix timeout increased to 30s for uploads"
})
```

### Impact of consolidation

| Before | After |
|--------|-------|
| 12 entries in the topic | 1 dense entry |
| Search returns noise | Search returns the relevant summary |
| Variable weights (some decayed) | Weight = 1.0 (fresh) |
| Mixed importance | Importance = the highest of the originals |

### When NOT to consolidate

- Topic with <5 entries -- not yet necessary
- Topic whose entries are very different (not consolidable into a coherent summary)
- When individual memories have important keywords for specific searches

---

## Importance levels guide

### The 4 levels

#### `critical` -- Never forgotten, never pruned

**Decay:** 0 (none)
**Pruning:** never

**Use for:**
- Production ports, URLs and credentials
- Absolute security constraints
- Invariant project facts

**Examples:**
```bash
hyphae store -t "infra" -c "Prod DB on port 5433, not 5432" -i critical
hyphae store -t "security" -c "Never store PII in logs" -i critical
hyphae store -t "infra" -c "Prod API is behind Cloudflare, direct IP is blocked" -i critical
```

#### `high` -- Slow decay, never pruned

**Decay:** 0.5x the normal rate
**Pruning:** never

**Use for:**
- Architecture decisions
- Recurring patterns
- Confirmed user preferences

**Examples:**
```bash
hyphae store -t "decisions" -c "REST over GraphQL for v1" -i high
hyphae store -t "preferences" -c "Always use absolute imports" -i high
hyphae store -t "conventions" -c "File names in kebab-case" -i high
```

#### `medium` -- Normal decay, can be pruned

**Decay:** 1.0x (standard rate)
**Pruning:** yes, when weight < threshold (default 0.1)

Default value if not specified.

**Use for:**
- One-off configurations
- Session context
- Standard bug fixes

**Examples:**
```bash
hyphae store -t "errors" -c "CORS fixed by adding header in nginx" -i medium
hyphae store -t "config" -c "REDIS_URL variable configured in .env.local" -i medium
```

#### `low` -- Fast decay, pruned quickly

**Decay:** 2.0x the normal rate
**Pruning:** yes, when weight < threshold

**Use for:**
- Temporary exploration notes
- Unconfirmed hypotheses
- Ephemeral context

**Examples:**
```bash
hyphae store -t "exploration" -c "Testing XYZ lib -- doesn't seem compatible" -i low
hyphae store -t "context" -c "Currently debugging the auth module" -i low
```

### Summary table

| Level | Decay rate | Prune | Typical lifespan | Usage |
|-------|-----------|-------|-----------------|-------|
| `critical` | 0 | never | infinite | Invariant facts |
| `high` | 0.5x | never | months | Important decisions |
| `medium` | 1.0x | yes | weeks | Standard context |
| `low` | 2.0x | yes | days | Temporary notes |

---

## Decay model explained

### The principle

Each memory has a weight that starts at 1.0 and decreases over time. The lower the weight, the less relevant the memory. When the weight drops below a threshold (default 0.1), the memory can be automatically pruned.

### The formula

```
effective_rate = base_rate x importance_multiplier / (1 + access_count x 0.1)

new_weight = weight x (1 - effective_rate)
```

Where:
- `base_rate` = configured decay rate (default 0.95, meaning 5% loss per cycle)
- `importance_multiplier` = see table below
- `access_count` = number of times the memory has been recalled

### Importance multipliers

| Importance | Multiplier | Effect |
|-----------|-----------|--------|
| `critical` | 0.0 | **No decay** -- weight stays at 1.0 forever |
| `high` | 0.5 | Decay at half speed |
| `medium` | 1.0 | Normal decay |
| `low` | 2.0 | Decay at double speed |

### The effect of access

The more a memory is recalled, the more it resists decay. The denominator `(1 + access_count x 0.1)` reduces the effective rate:

| Access count | Divisor | Effective rate (medium) |
|--------------|---------|------------------------|
| 0 | 1.0 | 5.0% per cycle |
| 5 | 1.5 | 3.3% per cycle |
| 10 | 2.0 | 2.5% per cycle |
| 20 | 3.0 | 1.7% per cycle |

A memory recalled 10 times decays 2x slower than a memory never recalled.

### When decay runs

- **Automatically**: on each `hyphae recall` or `hyphae_memory_recall`, if >24h since last run
- **Manually**: via `hyphae decay`
- The last decay timestamp is stored in `hyphae_metadata.last_decay_at`

### Concrete example

A `medium` memory, never recalled, with the default decay rate (0.95):

| Day | Weight | Status |
|-----|--------|--------|
| 0 | 1.000 | Fresh |
| 7 | 0.698 | Still relevant |
| 14 | 0.488 | Starting to age |
| 21 | 0.341 | Aged |
| 30 | 0.214 | Nearly pruned |
| 46 | 0.099 | **Pruned** (< 0.1) |

The same memory at `high` (0.5x decay):

| Day | Weight | Status |
|-----|--------|--------|
| 0 | 1.000 | Fresh |
| 30 | 0.463 | Still solid |
| 60 | 0.214 | Starting to age |
| 90 | 0.099 | Never pruned (high = no pruning) |

And at `critical`: weight = 1.000 forever.

### Protection against data loss

- `critical` memories never decay
- `high` memories are never pruned (even if their weight drops)
- Decay is access-aware: recalling a memory reinforces it
- Pruning only removes `medium` and `low` below the threshold

---

## Document Memory (RAG)

Hyphae ingests files and directories into a searchable vector store. Files are chunked, embedded, and stored in the same SQLite database. When the agent searches, relevant chunks are retrieved as context. This gives agents access to project source code, documentation, and configuration without re-reading files each session.

### When to use document memory

| Use case | Better approach |
|----------|----------------|
| Agent needs to reference a specific codebase | `hyphae ingest src/ --recursive` |
| Project docs should persist across sessions | `hyphae ingest docs/` |
| One-off decision or fact | Use episodic memory (`hyphae store`) |
| Permanent concept with relations | Use memoir (`hyphae memoir add-concept`) |

### Chunking strategies

Hyphae automatically selects the best chunking strategy based on file type:

| File type | Strategy | Behavior |
|-----------|----------|----------|
| **Markdown** (`.md`, `.mdx`) | By Heading | Splits at `#` headings, max 500 tokens per chunk. Each chunk tagged with its heading. |
| **Code** (`.rs`, `.py`, `.js`, `.ts`, `.go`, `.java`, etc.) | By Function | Splits at function/method boundaries using language-aware regex patterns. |
| **Text** (`.txt`, `.log`, `.json`, `.toml`, `.yaml`, etc.) | Sliding Window | 500-word windows with 50-word overlap for context continuity. |

Binary files are automatically detected and skipped. Hidden files and build directories (`target/`, `node_modules/`, `.git/`) are excluded during directory ingestion.

### Hybrid search over document chunks

Document search follows the same pipeline as memory search:

1. **Has embedder?** → Hybrid search: 30% FTS5 BM25 + 70% cosine similarity via sqlite-vec
2. **No embedder?** → FTS5 full-text search
3. **No FTS results?** → Keyword LIKE fallback

### Unified cross-store search with RRF

The `search-all` command (and `hyphae_search_all` MCP tool) searches across **both** episodic memories and document chunks in a single query. Results are merged using **Reciprocal Rank Fusion (RRF)**, which combines rankings from different sources into a single relevance order without requiring score normalization.
When the identity-v1 pair is present, memory results are scoped to the active worktree and `_shared` memories remain visible. Document chunks stay project-scoped because the chunk store does not currently track worktree identity.

```bash
# Search everything at once
hyphae search-all "database connection pooling"

# Agent (via MCP) gets unified results:
# [memory|decisions-api:0.92] Configured PgBouncer for connection pooling
# [doc|src/db.rs:0.85]        pub fn create_pool(config: &DbConfig) -> Pool { ... }
```

This means the agent can find relevant context whether it was stored as a memory, added to a knowledge graph, or ingested from a source file.

### Supported file types

**Code** (14 languages): `.rs`, `.py`, `.js`, `.ts`, `.tsx`, `.go`, `.java`, `.c`, `.cpp`, `.h`, `.cs`, `.rb`, `.swift`, `.kt`

**Markdown**: `.md`, `.mdx`

**Text**: `.txt`, `.log`, `.csv`, `.json`, `.toml`, `.yaml`, `.yml`

Unknown extensions default to text. Binary files are rejected.

---

## Complete configuration

### Configuration file

Location: `~/.config/hyphae/config.toml` (or `$HYPHAE_CONFIG`)

```toml
[store]
# Path to the SQLite database (default: platform path)
# path = "~/Library/Application Support/dev.hyphae.hyphae/memories.db"

[memory]
# Default importance if not specified
default_importance = "medium"

# Decay rate per day (0.95 = loses 5% per day)
decay_rate = 0.95

# Automatic pruning threshold
prune_threshold = 0.1

[embeddings]
# Embedding model (fastembed code)
model = "BAAI/bge-small-en-v1.5"

# Alternatives:
# "BAAI/bge-base-en-v1.5"                       # 768d, English, more accurate
# "BAAI/bge-large-en-v1.5"                      # 1024d, English, best accuracy
# "jinaai/jina-embeddings-v2-base-code"         # 768d, optimized for code

[extraction]
# Layer 0: rule-based fact extraction (zero LLM cost)
enabled = true

# Minimum score to keep a fact
min_score = 3.0

# Maximum facts per extraction pass
max_facts = 10

[recall]
# Layer 2: context injection before sessions
enabled = true

# Maximum memories to inject
limit = 15

[mcp]
# MCP server transport
transport = "stdio"

# Compact mode: short responses to save tokens
compact = true

# Custom instructions added to the MCP server description
# instructions = "Always recall before starting work"
```

### Environment variables

| Variable        | Description |
|-----------------|-------------|
| `HYPHAE_CONFIG` | Path to the configuration file |
| `HYPHAE_DB`     | Path to the SQLite database |
| `HYPHAE_LOG`    | Log level (`debug`, `info`, `warn`, `error`) |

### Database location

| Platform | Path                                                          |
|----------|---------------------------------------------------------------|
| macOS | `~/Library/Application Support/dev.hyphae.hyphae/memories.db` |
| Linux | `~/.local/share/dev.hyphae.hyphae/memories.db`                |

Can be overridden via `--db <path>` or `HYPHAE_DB`.

### Changing the embedding model

When changing the model in `config.toml`:
1. On next startup, HYPHAE detects the dimension change
2. The `vec_memories` table is dropped and recreated
3. All existing embeddings are cleared
4. Regenerate with `hyphae embed --force`

---

## See also

- **[CLI-REFERENCE.md](CLI-REFERENCE.md)** — All CLI commands with syntax, option tables, and examples
- **[MCP-TOOLS.md](MCP-TOOLS.md)** — All 23 MCP tool definitions (parameters, request/response examples) for AI agent integration
