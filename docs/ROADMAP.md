# Hyphae Roadmap

This page is the Hyphae-specific backlog. The root [ROADMAP.md](../../ROADMAP.md) keeps the cross-system priorities and sequencing.

## Recently Shipped

- Shared path resolution for config and database locations instead of scattered path handling.
- More portable transcript, Codex notify, Claude import, and embedding-cache path discovery.
- Normalized `agent_session` source handling so Claude and Codex session-derived memories land in one shared model.
- Broader `hyphae init` and `doctor` host/config handling via shared editor registration helpers.
- Structured feedback-loop foundations:
  - session-linked recall events
  - session-linked outcome signals
  - active-session reuse per project
  - stronger feedback/session foreign-key integrity

## Next

These items should stay aligned with the ecosystem roadmap because they affect other tools or change cross-system sequencing.

### Recall effectiveness and outcome ranking

Implement recall effectiveness scoring from [FEEDBACK-LOOP-DESIGN.md](FEEDBACK-LOOP-DESIGN.md) so Hyphae can rank recalled context by observed usefulness instead of similarity alone.

### Context-aware recall

Infer useful memories from current repo state, open files, diffs, and active failures so agents do not need to guess the exact search query before recall becomes helpful.

### Multi-project memory quality

Deepen project and workspace separation, then add semantic deduplication, conflict detection, and stronger provenance so one Hyphae store can support multiple repos and agents without muddying recall.

### Auto-ingestion watcher

Add a watcher-driven ingest mode so long-lived worktrees stay fresh without repeated manual ingest commands.

### Shared and remote memory

Add remote encrypted sync and shared-memory modes for multi-machine and multi-agent setups.

### Training and export surfaces

Build the planned `export-training-data` command and other structured export paths that turn stored recall and outcome data into reusable evaluation input.

## Later

These are valuable Hyphae capabilities, but they do not need to drive ecosystem order.

### Memory consolidation via LLM

Add optional LLM-assisted consolidation that merges related memories into higher-level summaries.

### Cross-memory linking

Detect and link related memories, memoirs, and document chunks so recall can surface adjacent knowledge without requiring an exact repeated query.

### Temporal queries

Support point-in-time questions such as “what did I know about auth last week?”

### Conversation summarization quality

Keep improving transcript and notify summarization so session-derived memories become more structured and more useful than raw transcript import.

### Memory provenance tools

Add `trace`-style lineage views that show which conversation, file, or commit produced a given memory and how it changed over time.

## Search and Retrieval

These are retrieval-quality improvements that mostly live inside Hyphae itself.

### Query expansion

Expand search queries with related terms and memoir graph context.

### Faceted search

Support combined filters such as topic, importance, source type, and date range in one search flow.

### Reranking pipeline

Add a second-stage reranker for top-k retrieval quality.

### Graph-powered retrieval

Traverse memoir relationships during search so linked concepts can influence recall.

## Local UX and Operations

### Export/import

Provide explicit export and import commands for backup, migration, and local sharing.

### Structured output modes

Expose consistent `json`, `table`, and compact formats across CLI commands.

### Memory tags

Add lightweight tagging that cuts across topic boundaries.

### TTL and expiry

Support explicit expiration dates beyond the existing decay model.

### Bulk cleanup operations

Add batch forget, archive, and cleanup flows for large stores.

### IDE surfaces and hooks

Consider webhook/event hooks and IDE-side memory surfaces once the underlying ranking and provenance models are stronger.
