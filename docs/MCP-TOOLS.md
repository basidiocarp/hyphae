# Hyphae -- MCP Tools Reference

This file documents all 23 tools exposed by the Hyphae MCP server over the JSON-RPC 2.0 protocol (stdio transport). These tools are called transparently by AI agents (Claude, Cursor, Windsurf, etc.)—no manual invocation is required once the server is configured. Tools are organized into three categories: episodic memory (9 tools), memoir knowledge graphs (9 tools), and RAG document ingestion (5 tools).

## Table of Contents

- [Overview](#overview)
- [Memory Tools (9)](#memory-tools-9)
  - [`hyphae_memory_store`](#hyphae_memory_store----store-a-memory)
  - [`hyphae_memory_recall`](#hyphae_memory_recall----search-memories)
  - [`hyphae_memory_update`](#hyphae_memory_update----update-a-memory)
  - [`hyphae_memory_forget`](#hyphae_memory_forget----delete-a-memory)
  - [`hyphae_memory_consolidate`](#hyphae_memory_consolidate----consolidate-a-topic)
  - [`hyphae_memory_list_topics`](#hyphae_memory_list_topics----list-topics)
  - [`hyphae_memory_stats`](#hyphae_memory_stats----global-statistics)
  - [`hyphae_memory_health`](#hyphae_memory_health----health-audit)
  - [`hyphae_memory_embed_all`](#hyphae_memory_embed_all----backfill-embeddings)
- [Memoir Tools (9)](#memoir-tools-9)
  - [`hyphae_memoir_create`](#hyphae_memoir_create----create-a-memoir)
  - [`hyphae_memoir_list`](#hyphae_memoir_list----list-memoirs)
  - [`hyphae_memoir_show`](#hyphae_memoir_show----show-a-memoir)
  - [`hyphae_memoir_add_concept`](#hyphae_memoir_add_concept----add-a-concept)
  - [`hyphae_memoir_refine`](#hyphae_memoir_refine----refine-a-concept)
  - [`hyphae_memoir_search`](#hyphae_memoir_search----search-within-a-memoir)
  - [`hyphae_memoir_search_all`](#hyphae_memoir_search_all----search-across-all-memoirs)
  - [`hyphae_memoir_link`](#hyphae_memoir_link----link-two-concepts)
  - [`hyphae_memoir_inspect`](#hyphae_memoir_inspect----inspect-a-concepts-neighborhood)
- [RAG Tools (5)](#rag-tools-5)
  - [`hyphae_ingest_file`](#hyphae_ingest_file----ingest-a-file-or-directory)
  - [`hyphae_search_docs`](#hyphae_search_docs----search-ingested-documents)
  - [`hyphae_list_sources`](#hyphae_list_sources----list-ingested-sources)
  - [`hyphae_forget_source`](#hyphae_forget_source----remove-a-source)
  - [`hyphae_search_all`](#hyphae_search_all----unified-cross-store-search)
- [See also](#see-also)

---

## Overview

The MCP server is started with `hyphae serve` and communicates over stdio using JSON-RPC 2.0. Enable compact mode (`hyphae serve --compact` or `compact = true` in `config.toml`) to reduce response sizes by ~40%.

---

## Memory Tools (9)

### `hyphae_memory_store` -- Store a memory

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `topic` | string | yes | -- | Category (e.g., `project-kexa`, `architecture-decisions`) |
| `content` | string | yes | -- | Information to memorize |
| `importance` | string (enum) | no | `medium` | `critical`, `high`, `medium`, `low` |
| `keywords` | string[] | no | -- | Keywords to improve search |
| `raw_excerpt` | string | no | -- | Verbatim excerpt (code, error message) |

Automatic behaviors:
- Auto-dedup: if a similar memory with >85% similarity exists in the same topic, it is updated instead of creating a duplicate
- Auto-embed: if the embedder is available, the memory is automatically vectorized
- Consolidation alert: if the topic exceeds 7 entries, a warning is added to the response

**Example request:**
```json
{
  "topic": "decisions-api",
  "content": "Using JWT for API authentication",
  "importance": "high",
  "keywords": ["jwt", "auth", "api"]
}
```

**Example response (normal mode):**
```
Stored memory: 01HWXYZ123456789ABCDEF
[Note: topic 'decisions-api' has 8 entries. Consider consolidating.]
```

**Example response (compact mode):**
```
ok:01HWXYZ123456789ABCDEF
```

---

### `hyphae_memory_recall` -- Search memories

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Natural language query |
| `topic` | string | no | -- | Filter by topic |
| `limit` | integer | no | `5` | Max results (1-20) |
| `keyword` | string | no | -- | Filter by exact keyword |
| `session_id` | string | no | -- | Explicit session id from `hyphae_session_start`; use this for scoped attribution when one project has parallel sessions |

Automatic behaviors:
- Auto-decay: applies decay if >24h since last run
- Access update: increments the access counter for each result

**Example request:**
```json
{
  "query": "database choice",
  "topic": "decisions-api",
  "limit": 3,
  "session_id": "ses_01ABC..."
}
```

**Example response (normal mode):**
```
--- 01HWXYZ123456789ABCDEF ---
  topic:      decisions-api
  importance: high
  weight:     0.950
  summary:    Using PostgreSQL for JSONB support
  keywords:   postgres, jsonb, database
```

**Example response (compact mode):**
```
[decisions-api] Using PostgreSQL for JSONB support
```

---

### `hyphae_memory_update` -- Update a memory

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `id` | string | yes | -- | ID of the memory to update |
| `content` | string | yes | -- | New content (replaces the summary) |
| `importance` | string (enum) | no | (preserved) | New importance |
| `keywords` | string[] | no | (preserved) | New keywords |

**Example request:**
```json
{
  "id": "01HWXYZ123456789ABCDEF",
  "content": "PostgreSQL for JSONB + PostGIS for geo data",
  "importance": "critical"
}
```

---

### `hyphae_memory_forget` -- Delete a memory

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | ID of the memory to delete |

**Example:**
```json
{ "id": "01HWXYZ123456789ABCDEF" }
```

---

### `hyphae_memory_consolidate` -- Consolidate a topic

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `topic` | string | yes | Topic to consolidate |
| `summary` | string | yes | Consolidated summary (replaces all memories in the topic) |

Unlike the CLI, the MCP requires the agent to provide the summary. The agent must first recall the topic's memories, then synthesize them.

**Example:**
```json
{
  "topic": "errors-resolved",
  "summary": "CORS fixed via nginx header. Memory leak fixed by closing DB connections. Rate limiting added on /api/auth."
}
```

---

### `hyphae_memory_list_topics` -- List topics

**Parameters:** None

**Example response:**
```
decisions-api: 5
errors-resolved: 12
preferences: 3
```

---

### `hyphae_memory_stats` -- Global statistics

**Parameters:** None

**Example response:**
```
Memories: 20, Topics: 3, Avg weight: 0.847, Oldest: 2024-01-15 09:30, Newest: 2024-03-05 14:22
```

---

### `hyphae_memory_health` -- Health audit

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `topic` | string | no | (all) | Specific topic to audit |

Reports per topic: entry count, average weight, stale entries, consolidation need.

**Example response:**
```
decisions-api: 5 entries, avg_weight=0.92, stale=0, needs_consolidation=false
errors-resolved: 12 entries, avg_weight=0.65, stale=3, needs_consolidation=true
```

---

### `hyphae_memory_embed_all` -- Backfill embeddings

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `topic` | string | no | (all) | Limit to a topic |

Available only if the `embeddings` feature is enabled. Generates vectors for memories that don't have one yet.

---

## Memoir Tools (9)

### `hyphae_memoir_create` -- Create a memoir

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | yes | Unique memoir name |
| `description` | string | no | Description |

**Example:**
```json
{ "name": "system-architecture", "description": "Design decisions and component relationships" }
```

---

### `hyphae_memoir_list` -- List memoirs

**Parameters:** None

Returns all memoirs with their concept counts.

---

### `hyphae_memoir_show` -- Show a memoir

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | yes | Memoir name |

Returns stats, labels, and all concepts in the memoir.

---

### `hyphae_memoir_add_concept` -- Add a concept

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `memoir` | string | yes | Memoir name |
| `name` | string | yes | Concept name (unique within the memoir) |
| `definition` | string | yes | Dense description of the concept |
| `labels` | string | no | Comma-separated labels (e.g., `domain:arch,type:decision`) |

**Example:**
```json
{
  "memoir": "system-architecture",
  "name": "auth-service",
  "definition": "Handles JWT and OAuth2 flows",
  "labels": "domain:auth,type:service"
}
```

---

### `hyphae_memoir_refine` -- Refine a concept

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `memoir` | string | yes | Memoir name |
| `name` | string | yes | Concept name |
| `definition` | string | yes | New definition (replaces the old one) |

Increments revision and increases confidence.

---

### `hyphae_memoir_search` -- Search within a memoir

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `memoir` | string | yes | -- | Memoir name |
| `query` | string | yes | -- | Search query |
| `label` | string | no | -- | Filter by label (e.g., `domain:tech`) |
| `limit` | integer | no | `10` | Max results |

---

### `hyphae_memoir_search_all` -- Search across all memoirs

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Search query |
| `limit` | integer | no | `10` | Max results |

---

### `hyphae_memoir_link` -- Link two concepts

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `memoir` | string | yes | Memoir name |
| `from` | string | yes | Source concept name |
| `to` | string | yes | Target concept name |
| `relation` | string (enum) | yes | Relation type |

**`relation` values:**
`part_of`, `depends_on`, `related_to`, `contradicts`, `refines`, `alternative_to`, `caused_by`, `instance_of`, `superseded_by`

**Example:**
```json
{
  "memoir": "system-architecture",
  "from": "api-gateway",
  "to": "auth-service",
  "relation": "depends_on"
}
```

---

### `hyphae_memoir_inspect` -- Inspect a concept's neighborhood

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `memoir` | string | yes | -- | Memoir name |
| `name` | string | yes | -- | Concept name |
| `depth` | integer | no | `1` | BFS depth |

**Example:**
```json
{
  "memoir": "system-architecture",
  "name": "auth-service",
  "depth": 2
}
```

Returns the concept and all concepts reachable in N hops, with the links between them.

---

## RAG Tools (5)

### `hyphae_ingest_file` -- Ingest a file or directory

Ingest a file or directory into Hyphae's document store for RAG search. Content is automatically chunked based on file type: markdown files are split by heading, code files by function, and other text files use a sliding window.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | yes | -- | Absolute or relative path to a file or directory to ingest |
| `recursive` | boolean | no | `false` | If path is a directory, recurse into subdirectories |

Automatic behaviors:
- Auto-detect file type: Markdown, code (14 languages), or plain text
- Auto-chunk: chooses the best chunking strategy per file type
- Auto-embed: if the embedder is available, chunks are automatically vectorized
- Skip binary files: binary files are detected and skipped

**Example request:**
```json
{
  "path": "/home/user/project/src",
  "recursive": true
}
```

**Example response (normal mode):**
```
Ingested: /home/user/project/src/main.rs (5 chunks)
Ingested: /home/user/project/src/lib.rs (3 chunks)
```

**Example response (compact mode):**
```
ok:2 files, 8 chunks
```

---

### `hyphae_search_docs` -- Search ingested documents

Search ingested documents using hybrid (vector + FTS) or FTS-only search. Returns ranked chunks with source paths and relevance scores.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Natural language search query |
| `limit` | integer | no | `10` | Maximum number of results to return (1-100) |

**Example request:**
```json
{
  "query": "authentication middleware",
  "limit": 5
}
```

**Example response (normal mode):**
```
--- chunk 01HW...C1 ---
  source: /home/user/project/src/auth.rs
  score:  0.872
  content: pub fn auth_middleware(...) { ... }
```

**Example response (compact mode):**
```
[auth.rs:0.87] pub fn auth_middleware(...) { ... }
```

---

### `hyphae_list_sources` -- List ingested sources

List all ingested document sources with their type, chunk count, and ingestion date.

**Parameters:** None

**Example response:**
```
/home/user/project/src/main.rs    Code    5 chunks    2024-03-05
/home/user/project/README.md      Markdown 3 chunks   2024-03-05
```

---

### `hyphae_forget_source` -- Remove a source

Remove an ingested document source and all its chunks from the store.

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Source path of the document to delete (as shown by `hyphae_list_sources`) |

**Example:**
```json
{ "path": "/home/user/project/src/old_module.rs" }
```

---

### `hyphae_search_all` -- Unified cross-store search

Search across both episodic memories and ingested documents in a single query. Results are merged and ranked using Reciprocal Rank Fusion (RRF) to produce a unified relevance ordering.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Natural language search query |
| `limit` | integer | no | `10` | Total results across both stores (1-50) |
| `include_docs` | boolean | no | `true` | Whether to include document chunks in results |

**Example request:**
```json
{
  "query": "database connection pooling",
  "limit": 5,
  "include_docs": true
}
```

**Example response (normal mode):**
```
--- memory 01HW...M1 (score: 0.92) ---
  topic:   decisions-api
  content: Configured PgBouncer for connection pooling

--- doc chunk 01HW...C3 (score: 0.85) ---
  source:  /home/user/project/src/db.rs
  content: pub fn create_pool(config: &DbConfig) -> Pool { ... }
```

**Example response (compact mode):**
```
[memory|decisions-api:0.92] Configured PgBouncer for connection pooling
[doc|db.rs:0.85] pub fn create_pool(config: &DbConfig) -> Pool { ... }
```

---

## See also

- **[CLI-REFERENCE.md](CLI-REFERENCE.md)** — All CLI commands with syntax, option tables, and examples
- **[FEATURES.md](FEATURES.md)** — Conceptual guides: Memory vs Memoir, multi-session workflows, topic organization, consolidation, importance levels, decay model, and complete configuration reference
