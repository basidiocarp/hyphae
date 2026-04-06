# Hyphae Architecture

Hyphae is a 5-crate Rust workspace that compiles into a single binary. It
solves the context-loss problem with two memory models: decaying episodic
memories for day-to-day work, and memoir graphs for durable concepts. This
document covers the crate layout, storage pipeline, and the places where search
and decay behavior matter.

---

## Design Principles

- **Two memory models, not one compromise** — decay-based memories and
  permanent memoir graphs solve different problems and stay separate in code and
  schema.
- **Local-first retrieval** — SQLite, FTS5, sqlite-vec, and the default
  embedding model all run on the machine where Hyphae is installed.
- **Hybrid search over single-mode search** — keyword precision and semantic
  recall are blended instead of forcing contributors to choose one.
- **Graceful degradation** — if embeddings are unavailable, Hyphae still works
  through FTS and keyword search.
- **Durable interfaces** — CLI, MCP, and store traits all sit on shared core
  types so behavior stays aligned across surfaces.

---

## Workspace Structure

```text
hyphae-cli ──► hyphae-ingest ──► hyphae-core
   │               ▲
   ├──► hyphae-store ─────────► hyphae-core
   │       ▲
   └──► hyphae-mcp ──► hyphae-ingest
            ├──► hyphae-store
            └──► hyphae-core
```

All five crates compile into the `hyphae` binary.

- **`hyphae-core`**: Domain types, traits, and embedder abstraction. No
  database I/O. No transport code.
- **`hyphae-store`**: SQLite implementation of `MemoryStore` and `MemoirStore`,
  including FTS5, sqlite-vec, migrations, and decay bookkeeping.
- **`hyphae-ingest`**: File readers and chunking strategies for Markdown, code,
  and other indexed documents. Pure ingest logic, no persistence.
- **`hyphae-mcp`**: JSON-RPC 2.0 over stdio. Tool definitions, dispatch, and
  response shaping for agent clients.
- **`hyphae-cli`**: Operator surface for store, recall, memoir, session, init,
  benchmark, and server commands.

---

## Core Abstraction

```rust
pub trait MemoryStore {
    fn store(&self, memory: Memory) -> HyphaeResult<MemoryId>;
    fn get(&self, id: &MemoryId) -> HyphaeResult<Option<Memory>>;
    fn search_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>>;
    fn apply_decay(&self, decay_factor: f32) -> HyphaeResult<usize>;
    fn prune_expired(&self) -> HyphaeResult<usize>;
}
```

`hyphae-store` implements this trait. The CLI and MCP layers consume it through
shared core types. The invariant is simple but important: store and recall
surfaces must agree on memory semantics even when embeddings are disabled or
the caller uses compact mode.

---

## Request Flow

When a CLI command or MCP tool call arrives:

1. **Load configuration** (`hyphae_cli::config::load_config`)
   Resolves config from `HYPHAE_CONFIG`, then the platform config path, then
   built-in defaults.
   Example: if no config file exists, Hyphae still boots with local defaults.

2. **Open the store** (`SqliteStore::new` or `SqliteStore::with_dims`)
   Opens SQLite, ensures the schema, and aligns vector dimensions with the
   active embedding model.
   Example: changing from a 384d model to a 768d model recreates vector
   tables and clears stale embeddings.

3. **Choose the retrieval path** (`search_hybrid`, `search_fts`,
   keyword fallback)
   Uses hybrid search when an embedder is available, FTS when it is not, and a
   keyword fallback when the FTS path has no hits.
   Example: `hyphae_memory_recall` can still return useful results without the
   embeddings feature.

4. **Apply maintenance behaviors** (`apply_decay`, dedup on store)
   Recall may trigger decay if more than 24 hours have passed. Store may update
   an existing memory instead of creating a near-duplicate.
   Example: same-topic memories above the similarity threshold are merged.

5. **Format the response** (`hyphae-mcp` tool handlers or CLI printers)
   Returns verbose output for humans or compact output for LLM-heavy flows.
   Example: `hyphae serve --compact` shortens recall and store responses to cut
   token cost.

---

## Store and Search

File: `crates/hyphae-store/src/`

### How It Works

1. **Persist entities** — memories, memoirs, concepts, links, documents, and
   chunks are stored in regular SQLite tables.
2. **Mirror searchable text** — FTS5 virtual tables index topic, summary,
   keywords, concept text, and chunk content.
3. **Mirror vectors** — sqlite-vec virtual tables hold normalized embeddings
   for memories and chunks.
4. **Blend scores** — BM25 contributes keyword precision and cosine similarity
   contributes semantic recall.
5. **Deduplicate or decay when needed** — store and recall operations keep the
   database from filling with stale or redundant rows.

### Search Matrix

| Mode | Input | Behavior |
|------|-------|----------|
| Hybrid | Query plus embedding | 30% FTS and 70% cosine similarity, merged by memory ID |
| FTS only | Query text | BM25 ranking through FTS5 tables |
| Keyword fallback | Topic and keywords | Used when richer search paths are unavailable or empty |

### Adding a New Stored Surface

File: `crates/hyphae-store/src/`

1. Add the core type or trait method in `hyphae-core`.
2. Extend the SQLite schema and migration path in `hyphae-store`.
3. Thread the behavior through CLI or MCP handlers.
4. Add roundtrip and edge-case tests before shipping the new surface.

---

## Data Model

### Memory

```rust
pub struct Memory {
    pub id: String,                     // ULID
    pub topic: String,
    pub summary: String,
    pub weight: f32,                    // decays over time
    pub importance: Importance,         // decay and pruning behavior
    pub keywords: Vec<String>,
    pub embedding: Option<Vec<f32>>,    // 384d, 768d, or 1024d
}
```

### Importance

```rust
pub enum Importance {
    Critical,   // decay: 0.0, prune: never
    High,       // decay: 0.5x rate, prune: never
    Medium,     // decay: 1.0x rate, prune below threshold
    Low,        // decay: 2.0x rate, prune below threshold
    Ephemeral,  // decay: 5.0x rate, auto-expires
}
```

### Schema

12 tables. Key invariants:

- Deleting a memoir cascades to its concepts and links.
- Concept names are unique within a memoir, not globally.
- Self-links are rejected with a database check constraint.
- FTS tables stay synchronized through triggers.
- Vector tables are rebuilt when embedding dimensions change.

---

## Configuration

Config file: `~/.config/hyphae/config.toml`

```toml
[memory]
default_importance = "medium"
decay_rate = 0.95
prune_threshold = 0.1

[embeddings]
model = "BAAI/bge-small-en-v1.5"

[recall]
enabled = true
limit = 15

[mcp]
transport = "stdio"
compact = true
```

Environment variables override config:

- `HYPHAE_CONFIG` — use a non-default config file path
- `HYPHAE_DB` — point Hyphae at a different SQLite database
- `HYPHAE_LOG` — raise or lower log verbosity

---

## Security

- FTS queries are sanitized before they reach SQLite search syntax.
- Store queries use parameterized statements instead of string-built SQL.
- Storage stays local by default: no database server, no remote vector store.
- The default embedding model runs locally through `fastembed`.
- Tested against SQL injection, FTS injection, null bytes, unicode boundaries,
  and large payloads.

---

## Testing

```bash
cargo test --all
cargo test -p hyphae-store
cargo test --ignored
```

| Category | Count | What's Tested |
|----------|-------|---------------|
| Unit | 300+ | CRUD, schema migrations, decay math, memoir graph behavior, chunking helpers |
| Integration | 100+ | CLI roundtrips, MCP tool dispatch, store and recall flows, dedup behavior |
| Security and edge cases | 50+ | Injection attempts, null bytes, unicode, empty states, oversized payloads |
| Performance | 10+ | Store, search, decay, and benchmark thresholds on larger in-memory datasets |

Fixtures are mostly synthetic and live beside crate tests. The important review
step is to keep benchmark and search assertions aligned with the current schema
and embedding dimensions.

---

## Key Dependencies

- **`rusqlite`** — local relational storage with bundled SQLite support.
- **`sqlite-vec`** — vector similarity search without introducing an external
  vector database.
- **`fastembed`** — local embedding generation behind the optional
  `embeddings` feature.
- **`spore`** — shared ecosystem transport and config-loading helpers.
