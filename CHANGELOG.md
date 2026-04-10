# Changelog

All notable changes to Hyphae are documented in this file.

## [Unreleased]

### Added

- **Passive MCP resources**: Added bounded `resources/list` and `resources/read`
  surfaces for current passive context, compact summaries, and project
  understanding bundles.
- **Typed passive artifacts**: Added typed passive retrieval surfaces for
  compact summaries and project-understanding bundles in `hyphae-store`.

### Fixed

- **Context boundary redaction**: Passive resource reads and initialize-time
  preload now share boundary redaction behavior for obvious secret-bearing
  content.
- **Project-scoped current resources**: Current passive resources no longer
  guess across projects when no active project context exists.

## [0.10.8] - 2026-04-09

### Added

- **Backup and restore flow**: The CLI now supports backup creation, listing,
  restore validation, and safer path handling for local database recovery.
- **Retrieval benchmarking**: Added a Criterion bench for retrieval hot paths
  so hybrid ranking and unified search performance can be tracked directly.

### Changed

- **Release variants**: The release and install surfaces now document the slim
  and embeddings-enabled binary split more explicitly.
- **Docs structure**: Operator and maintainer docs now follow the lowercase
  path layout with a central `docs/README.md` and refreshed guide references.

## [0.10.6] - 2026-04-08

### Changed

- **Foundation alignment**: repo guidance and architecture notes now describe
  the core, store, ingest, MCP, and CLI boundaries more concretely.
- **Boundary verification**: added explicit surface checks for crate layering
  and maintainer-facing contract guidance.

### Fixed

- **Request tracing continuity**: request, session, and workspace identity now
  survive deeper into write-heavy and workflow-heavy MCP paths.
- **Runtime instrumentation depth**: CLI and tool execution paths now enter the
  shared root, workflow, and subprocess spans at the fragile boundaries that
  were still under-instrumented.

## [0.10.5] - 2026-04-08

### Fixed

- **Deeper request-local tracing context**: MCP tool handling now threads
  `session_id`, `request_id`, and better workspace scope into downstream spans
  so failures inside write-heavy paths retain useful identity.
- **Broader runtime boundary instrumentation**: CLI startup, project
  detection, doctor checks, and MCP workflow execution now enter shared root,
  workflow, and subprocess spans around long-running or fragile boundaries.
- **Docs now match the runtime surface**: README and MCP/feature guides no
  longer hard-code stale tool counts and now document `HYPHAE_LOG`, stderr
  logging, and serve/runtime behavior accurately.

## [0.10.4] - 2026-04-08

### Changed

- **Shared Spore logging rollout**: Hyphae now consumes `spore v0.4.9`,
  initializes logging with the app-aware path, and adds shared root, request,
  and tool spans around MCP serve flows.
- **Runtime compatibility with non-exhaustive Spore editors**: Init logic now
  tolerates future `spore::editors::Editor` variants instead of failing to
  compile.

### Changed

- **Docs cleanup**: README, architecture, CLI reference, embedding-model docs,
  feature docs, setup guides, and troubleshooting docs were refreshed to match
  the current product surface.

## [0.10.2] - 2026-04-01

### Fixed

- **Release-gating for macOS binaries**: Apple release builds now fail fast on
  functional smoke-test or MCP initialize regressions instead of swallowing
  those failures during packaging.
- **macOS build diagnostics**: Release workflows now capture verbose native
  build logs, pin the SDK and deployment target, re-sign the binary, and upload
  diagnostics when the Apple build path fails.

## [0.10.0] - 2026-03-31

### Added

- **Owned read surfaces**: Hyphae now exposes explicit CLI surfaces for session
  list, session timeline, activity, and context gathering so downstream tools
  no longer need private store reads.
- **Runtime session document metadata**: Command-output ingestion now persists
  `runtime_session_id` and returns it from chunk retrieval.

### Changed

- **Identity-v1 enforcement**: Session, context, and command-output flows now
  require the structured project identity contract instead of falling back to
  project-scoped legacy behavior.
- **Versioned payloads**: Cap-facing CLI and MCP payloads now emit and validate
  explicit `schema_version` fields for session, memoir, memory, and
  command-output boundaries.

### Fixed

- **Worktree-safe recall and context**: Identity-aware context gathering and
  session lookup no longer bleed through legacy `session/*` fallback memories.
- **Cross-tool contract drift**: Hyphae import, command-output, and dashboard
  read paths now validate the real contract shapes instead of optimistic ad hoc
  parsing.

## [0.9.5] - 2026-03-27

### Added

- **Explicit recall session identity**: `hyphae_memory_recall` now accepts
  `session_id` so scoped recall attribution stays attached to the right
  parallel session.
- **Structured session status**: `hyphae session status --id ...` now returns
  machine-readable session metadata for runtime integrations.

### Fixed

- **Scoped recall attribution**: Session-backed recall logging now validates
  session ownership, derives the correct project when needed, and avoids
  collapsing back to project-wide active-session inference.
- **Parallel session context filtering**: CLI and MCP session context can now
  filter by scope so worker-specific inspection does not bleed across active
  sessions.

## [0.9.4] - 2026-03-27

### Added

- **Scoped session starts**: `hyphae session start` and
  `hyphae_session_start` now accept an optional scope so parallel workers in one
  project can keep distinct active sessions.
- **Learned recall effectiveness**: Structured outcome signals now roll up into
  `recall_effectiveness`, and hybrid recall ranking uses that learned feedback
  as a small bias.

### Changed

- **Session context visibility**: CLI and MCP session-context output now
  surface scope when present so parallel session debugging is less opaque.

### Fixed

- **Parallel session attribution**: Scoped sessions stop Cortina and other
  runtimes from collapsing concurrent work in one project into a single active
  session.
- **MCP session contract drift**: The MCP schema and session-context output now
  advertise session scope so schema-driven clients can use scoped sessions
  safely.

## [0.9.3] - 2026-03-27

### Added

- **Feedback loop signals**: `hyphae session end` now records structured
  success and failure outcome signals, and recall paths can log structured
  recall events for later evaluation.
- **Feedback CLI surface**: Added `hyphae feedback signal` for writing
  structured outcome signals against an existing session.

### Changed

- **Structured recall tracking**: Active-session recall calls, including
  empty-result recalls, now emit structured recall events instead of relying on
  inferred memory writes.
- **Active session reuse**: Hyphae now reuses the current active session per
  project instead of opening competing active sessions that make attribution
  ambiguous.
- **Host-neutral setup guidance**: Setup docs and `hyphae init` messaging now
  describe supported editors and runtimes more generally.

### Fixed

- **Feedback session integrity**: `recall_events` and `outcome_signals` now
  enforce real session foreign keys, with migration logic that preserves valid
  rows and nulls orphaned legacy ids.

## [0.9.2] - 2026-03-26

### Changed

- **Host-neutral session sources**: Claude Code and Codex session-derived
  memories now share one `agent_session` source model while older rows continue
  to load correctly.
- **Centralized path resolution**: CLI data and config path resolution now go
  through one shared resolver instead of being recomputed across commands.
- **Clearer import guidance**: `hyphae import-claude-memory` and
  `hyphae ingest-sessions` now explain Claude memory import versus general agent
  transcript ingestion more clearly.

### Fixed

- **Configured DB path handling**: `hyphae doctor`, `backup`, `restore`, and
  normal startup now all honor `store.path` and CLI `--db` overrides
  consistently.
- **Embedding model cache portability**: FastEmbed downloads now use the
  platform cache directory instead of a Unix-shaped path.

## [0.9.1] - 2026-03-23

### Fixed

- **Older DB migration failure**: Hyphae now adds new `memories` columns before
  creating related indexes, so older databases without `branch`, `worktree`, or
  invalidation columns no longer fail to open.
- **Codex notify storage on existing installs**: `hyphae codex-notify` can now
  write memories into upgraded pre-0.8 databases instead of crashing during
  schema initialization.

## [0.9.0] - 2026-03-23

### Added

- **Codex lifecycle breadcrumbs**: `hyphae codex-notify` now stores
  lighter-weight lifecycle memories for non-`agent-turn-complete` Codex
  notifications.

### Changed

- **Richer Codex transcript reconciliation**: Codex session ingestion now
  preserves lifecycle payload context such as approvals and status messages.
- **Codex integration docs**: CLI and setup docs now explain the notify adapter
  as a durable turn-summary path plus lighter lifecycle breadcrumb capture.

## [0.8.0] - 2026-03-23

### Added

- **Codex notify adapter**: Added `hyphae codex-notify` and `hyphae init`
  support for Codex `notify = ["hyphae", "codex-notify"]`.
- **Codex session transcript ingestion**: `hyphae ingest-sessions` now
  understands real Codex session event streams in addition to Claude transcripts
  and legacy Codex text history.

### Changed

- **Normalized host session ingestion**: Codex notify handling and transcript
  parsing now accumulate through a shared normalized session model.
- **Codex integration docs**: CLI and setup docs now describe Codex as a
  first-class integration path alongside Claude Code.

## [0.7.1] - 2026-03-22

### Added

- **Unsafe transaction docs**: Added safety comments on all
  `unchecked_transaction` sites to document why nested transactions cannot
  occur.

### Fixed

- **Tool definitions cache**: Removed an `OnceLock` path that cached stale
  `has_embedder` state on first call.
- **Vector search data loss**: KNN query sites now propagate errors instead of
  silently dropping corrupted rows.
- **Unicode panic**: String slicing in the MCP server and memory tools now uses
  UTF-8-safe truncation.
- **Spore migration**: Updated for the shared `SporeError` return surface.

## [0.7.0] - 2026-03-21

### Added

- **Purge command**: Added `hyphae purge` for deleting data by project or age,
  with `--dry-run` and `--force`.
- **Secret audit command**: Added `hyphae audit-secrets` to scan stored memories
  for API keys, tokens, passwords, and private keys.
- **Changelog command**: Added `hyphae changelog` to summarize activity since a
  relative or absolute date.
- **Secrets rejection mode**: `reject_secrets = true` can now block storage of
  memories that contain detected secrets.
- **Relation normalization**: Canonical relation types now normalize synonyms on
  write.

### Changed

- **FTS5 project column**: Project-scoped FTS queries no longer require a join.
- **Search-all optimization**: Hybrid search now overfetches less, cutting
  intermediate allocation substantially.
- **Sessions table initialization**: Session tables now initialize with the main
  schema instead of lazy creation.
- **Shared Spore runtime**: Self-update, logging, and config now use shared
  Spore modules.

## [0.3.7] - 2026-03-18

### Added

- **Import and ingest commands**: Added `hyphae import-claude-memory`,
  `hyphae ingest-sessions`, project-management commands, `hyphae doctor`, and
  `hyphae prune`.
- **Cross-project retrieval**: Added `hyphae_gather_context`,
  `hyphae_recall_global`, and related session lifecycle MCP tools for broader
  context work.
- **Code graph intake**: Added `hyphae_import_code_graph` and
  `hyphae_code_query` for Rhizome-driven code memoirs.
- **Cross-project sharing**: Added the `_shared` memory pool for knowledge that
  applies across projects.
- **HTTP embedder option**: Added Ollama and OpenAI-compatible embedding
  endpoints alongside local FastEmbed.
- **Lazy embedding downloads**: FastEmbed model download now happens on first
  use instead of startup.

## [0.3.2] - 2026-03-16

### Added

- **Code-context recall boost**: `hyphae_memory_recall` gained `code_context`
  expansion so code-related queries can pull symbol names from code memoirs.

## [0.3.1] - 2026-03-16

### Added

- **Code graph import tools**: Added `hyphae_import_code_graph`,
  `hyphae_code_query`, and the supporting batch upsert and prune behavior on
  `MemoirStore`.

## [0.3.0] - 2026-03-16

### Added

- **CLI completions and init**: Added shell completions and a broader
  `hyphae init` flow for supported editors and clients.
- **Multi-project support**: Memories and documents can now be namespaced by
  project, with repo auto-detection when possible.
- **Watch mode**: Added `hyphae watch <path>` for debounced re-ingestion of
  changed files.
- **Ephemeral memories**: Added `Importance::Ephemeral`, `expires_at`, and
  `prune_expired()` for short-lived context.
- **Structured command-output storage**: Added `hyphae_store_command_output`
  and `hyphae_get_command_chunks`.
- **Pagination**: Added `offset` across search methods for paginated retrieval.

### Changed

- **Early-return CLI setup**: `completions`, `config`, and `init` now skip
  store and embedder initialization when it is not needed.
- **Project-aware traits**: Store traits now accept `project` and `offset`
  parameters across search methods.
- **Schema migrations**: The schema now auto-migrates `project` and
  `expires_at` columns.
- **Workflow cleanup**: Cargo audit was removed where transitive findings were
  not actionable.

## [0.2.0] - 2026-03-11

### Added

- **Document ingestion**: Added `hyphae ingest <path>` plus file and directory
  ingestion into a searchable vector store.
- **Chunking strategies**: Added sliding-window, heading-based, and
  function-based chunking.
- **Search-all surface**: Added `hyphae search-all <query>` with Reciprocal Rank
  Fusion across memories and document chunks.
- **RAG MCP tools**: Added document-ingest, search, list-source, forget-source,
  and cross-store search tools.
- **Ingest crate**: Added `hyphae-ingest` as a dedicated file-reader and
  chunking crate.

## [0.1.0] - 2026-03-11

### Added

- **Dual memory model**: Hyphae shipped with episodic memories and permanent
  memoir knowledge graphs.
- **MCP-first surface**: The initial release exposed 18 MCP tools over stdio
  for memory and memoir operations.
- **CLI surface**: Added 29 CLI commands for storing, recalling, searching, and
  managing memories and memoirs.
- **Hybrid search**: Shipped with BM25 plus cosine similarity over sqlite-vec.
- **Local embeddings**: The initial release used local BGE-small embeddings
  through FastEmbed, with no API calls.
- **Rule-based extraction**: Added zero-LLM fact extraction from conversation
  text.
- **Importance-based decay**: Critical memories stay durable while lower-value
  notes fade naturally.
- **Typed relation graphs**: Memoirs shipped with typed relations, labels, and
  BFS traversal.
- **Local-first storage**: The first release ran from one portable SQLite
  database with no external services.
- **Quality baseline**: The initial release shipped with cross-platform tests,
  input validation, transaction safety, NaN-safe numeric types, and multi-target
  releases.
