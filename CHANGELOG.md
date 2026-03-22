# Changelog

## v0.7.1 - 2026-03-22

### Fixed

- **Tool definitions cache**: Removed OnceLock that cached stale `has_embedder` state on first call. Tool lists now reflect actual embedder availability.
- **Vector search data loss**: Replaced `filter_map(|r| r.ok())` with proper error propagation in 4 KNN query sites. Corrupted rows now surface as errors instead of silently disappearing.
- **Unicode panic**: String slicing in MCP server and memory tools now uses `truncate_str()` with UTF-8 boundary checking. Multi-byte characters in memory summaries no longer crash the server.
- **Spore migration**: Updated for spore v0.4.0 `SporeError` return types.

### Added

- SAFETY comments on all 11 `unchecked_transaction` sites documenting why nested transactions cannot occur.

## v0.7.0 - 2026-03-21

### Added

- **`hyphae purge` command**: Delete data by project (`--project`) or age (`--before`). Supports `--dry-run` and `--force` flags. Covers memories, sessions, chunks, and documents.
- **`hyphae audit-secrets` command**: Scans existing memories for API keys, tokens, passwords, and private keys. Reports matches by memory ID and pattern type.
- **`hyphae changelog` command**: Summarizes activity since a given date. Aggregates sessions, memories by topic, resolved errors, and lessons. Supports `--since yesterday/today/last-week` or ISO dates.
- **Secrets rejection mode**: `reject_secrets = true` in config blocks storage of memories containing detected secrets. Shared `detect_secrets()` in hyphae-core.
- **Relation normalization**: Canonical relation types (calls, imports, implements, extends, contains, references, tests) with synonym mapping on write. Case-insensitive.

### Changed

- **FTS5 project column**: Added `project UNINDEXED` to `memories_fts` virtual table. Project-scoped FTS queries no longer require a JOIN. Auto-migration for existing databases.
- **search_all optimization**: Reduced overfetch multiplier from 4x to 1.5x in hybrid search, cutting intermediate memory allocation by ~50%.
- **Sessions table**: Moved from lazy creation to main schema init. Removed `ensure_sessions_table()` calls.
- **Spore v0.3.0**: Self-update, logging, and config now use shared spore modules.

## v0.3.7

### Added

- **`hyphae import-claude-memory` CLI**: Imports Claude Code conversation memories from JSONL files, with `--watch` mode for continuous monitoring of new exports.
- **`hyphae ingest-sessions` CLI**: Indexes Claude Code conversation transcripts for full-text search across past sessions.
- **`hyphae project list/link/search/share` CLI**: Project management commands for listing linked projects, linking new ones, searching within project scope, and sharing memories across projects.
- **`hyphae doctor` diagnostic command**: Health check that validates database integrity, embedder status, MCP connectivity, and configuration.
- **`hyphae prune` CLI command**: Manual pruning of expired and low-importance memories with configurable thresholds.
- **`hyphae_gather_context` MCP tool**: Aggregates relevant memories, code context, and session history for a given task description.
- **`hyphae_recall_global` MCP tool**: Cross-project memory recall that searches the `_shared` pool for knowledge applicable across all projects.
- **`hyphae_session_start/end/context` MCP tools**: Session lifecycle management for tracking conversation boundaries and retrieving session-scoped context.
- **`hyphae_onboard` MCP tool**: Guided onboarding flow that detects the project environment and returns relevant memories and configuration hints.
- **`hyphae_import_code_graph` and `hyphae_code_query` MCP tools**: Receive symbol graphs from Rhizome and query them with 5 query types (`symbols`, `callers`, `callees`, `implementors`, `structure`).
- **Cross-project knowledge sharing**: `_shared` memory pool for facts and patterns that apply across all projects, with automatic promotion of frequently-accessed cross-project memories.
- **Context-aware recall with code expansion**: `hyphae_memory_recall` expands queries with symbol names from code memoirs when the query appears code-related.
- **HTTP embedder**: Ollama and OpenAI-compatible embedding endpoint support via configurable HTTP embedder, as an alternative to local FastEmbed.
- **Lazy FastEmbed model download**: Embedding model is downloaded on first use rather than at startup, reducing cold-start time for non-embedding workflows.

## v0.3.2

### Added
- `hyphae_memory_recall` gains `code_context` parameter: expands search queries with symbol names from code memoirs (0.5 RRF weight)
- `is_code_related` heuristic detects CamelCase, snake_case, file extensions, and path separators to gate expansion
- `expand_with_code_context` searches code memoirs for matching concepts and adds them as FTS boost terms

## v0.3.1

### Added
- `hyphae_import_code_graph` MCP tool receives symbol graphs from Rhizome as memoirs with idempotent upsert (create/update/skip unchanged)
- `hyphae_code_query` MCP tool with 5 query types: `symbols`, `callers`, `callees`, `implementors`, `structure` (BFS depth 2)
- `upsert_concepts`, `upsert_links`, `prune_concepts` on `MemoirStore` for batch code graph imports with transactional atomicity
- Symbol pruning: removes concepts and cascades link cleanup when symbols leave the codebase

## v0.3.0

### Added
- `hyphae completions <bash|zsh|fish|powershell>` generates shell completions via clap_complete
- `hyphae init` auto-detects editors (Claude Code, Cursor, VS Code, Zed, Windsurf, Amp, Claude Desktop, Codex CLI) and writes MCP config with backup and merge
- Multi-project support: namespace memories/documents by project via `--project` flag, `store.default_project` config, or git repo auto-detection
- `hyphae watch <path>` monitors filesystem and auto-re-ingests changed files with debounced events and graceful shutdown
- Project filtering on all search/list operations; `None` returns all (backward compatible)
- `hyphae serve --project <name>` scopes MCP tool operations to a project namespace
- `expires_at` field with auto-expiry for ephemeral importance (default 4 hours); `prune_expired()` cleans up
- `Importance::Ephemeral` variant for short-lived context like sprint goals
- `ByStructuredOutput` chunking strategy with auto-detection for test results, build errors, diffs, and log output
- `hyphae_store_command_output` stores chunked command output with ephemeral TTL; `hyphae_get_command_chunks` retrieves chunks with pagination
- `offset` parameter on all search methods for paginated retrieval

### Changed
- CLI restructured: early-return commands (completions, config, init) skip store/embedder initialization
- `MemoryStore` and `ChunkStore` traits now accept `project: Option<&str>` and `offset: usize` on search methods
- Schema auto-migrates `project` and `expires_at` columns on `memories`, `project` on `documents`
- Hybrid search FTS query includes all columns (`updated_at`, `project`, `expires_at`) for correct row mapping
- Removed cargo audit workflow; unmaintained transitive deps from fastembed are not actionable

### CI/CD
- Concurrency groups on all workflows cancel stale runs on new pushes
- MSRV (1.85) check job
- Removed duplicate security-audit job from CI
- Coverage workflow runs tests once via `--json` instead of twice
- Binary-size and startup-time combined into single performance job
- `taiki-e/install-action` replaces `cargo install` for hyperfine and cross
- `rust-cache` and `--locked` on release builds

## v0.2.0

### Added
- `hyphae ingest <path>` ingests files/directories into a searchable vector store
- Chunking: Sliding Window (text), By Heading (markdown), By Function (code)
- `hyphae search-all <query>` searches memories and document chunks with Reciprocal Rank Fusion
- MCP RAG tools: `hyphae_ingest_file`, `hyphae_search_docs`, `hyphae_list_sources`, `hyphae_forget_source`, `hyphae_search_all` (23 tools total)
- `hyphae-ingest` crate: file readers + chunking logic, no database dependency

## v0.1.0

### Added
- Two memory models: episodic (temporal, decay-based) and semantic memoirs (permanent knowledge graphs)
- 18 MCP tools over stdio: 9 memory + 9 memoir for any MCP-compatible agent
- 29 CLI commands for storing, recalling, searching, and managing memories and memoirs
- Hybrid search: 30% BM25 (FTS5) + 70% cosine similarity (sqlite-vec)
- Local embeddings via BGE-small-en-v1.5 (fastembed). No API calls.
- Rule-based fact extraction from conversation text. No LLM cost.
- Importance-based decay: critical memories never fade, low-importance notes decay naturally
- Permanent knowledge graphs with typed relations, labels, and BFS traversal
- `hyphae init` auto-detects and configures Claude Code, Cursor, VS Code, Windsurf, Zed, Amp, and more
- Single SQLite database: portable, backupable, no external services

### Architecture
- 4-crate workspace: hyphae-core (types/traits), hyphae-store (SQLite), hyphae-mcp (JSON-RPC), hyphae-cli (commands)
- Feature-gated embeddings: `--no-default-features` for fast iteration without the embedding model
- Auto-migrations on startup
- Compact MCP mode saves ~40% tokens on recall output

### Quality
- 211 tests (28 core, 93 store, 69 MCP, 21 CLI)
- Input validation on all MCP tool parameters
- Transaction safety for multi-table operations
- NaN-safe numeric types (Weight, Confidence)
- CI: fmt, clippy, cross-platform tests (Linux/macOS/Windows), coverage, performance guards, security audit
- Multi-target releases: linux musl, macOS, Windows
