# Hyphae Roadmap

## High Impact — Core Differentiation

### Auto-ingestion watcher
File system watcher (notify crate) that auto-re-ingests changed files. Keeps the knowledge base fresh without manual `hyphae ingest --force`.

### Memory consolidation via LLM
Optional LLM-powered consolidation that merges related memories into higher-level summaries. Currently `extract.rs` is rule-based; an LLM pass could produce much richer consolidation.

### Cross-memory linking
Automatically detect and link related memories, memoirs, and document chunks. Surface "you stored something related 3 weeks ago" during recall.

### Multi-project support
Namespace memories by project/workspace. One hyphae instance serving multiple codebases without cross-contamination. `hyphae --project myapp search "auth flow"`.

### Context-aware recall
Automatically infer what memories are relevant based on the current git diff, open files, or recent errors. MCP tool that takes a "situation" and returns the most useful context without the agent needing to know what to search for.

### Semantic deduplication
Detect when a new memory is semantically equivalent to an existing one and merge them instead of creating duplicates. Keeps the store clean over time.

### Conflict detection
Flag when a new memory contradicts an existing one. "You stored 'API uses JWT' but also 'API uses session cookies' — which is current?"

Partially shipped: ordinary memories can now be invalidated with a reason and optional replacement memory, and invalidated entries are hidden from default recall while remaining reviewable.

## Medium Impact — Developer Experience

### `hyphae init` command
Auto-detect the user's editor (Claude Code, Cursor, Zed, etc.) and write the correct MCP config. Zero-friction onboarding.

Partially shipped: `hyphae init` now supports lifecycle hook installation for Claude Code, including `PostToolUse`, `PreCompact`, and `SessionEnd`.

### Git-aware ingestion
Respect `.gitignore`, auto-ingest on commit hooks, track file hashes to skip unchanged files. `hyphae ingest . --git-aware`.

### Export/import
`hyphae export memories.json` / `hyphae import memories.json` for backup, migration, and sharing knowledge bases between machines.

### Structured output modes
`--format json|table|compact` on all CLI commands for scripting and piping into other tools.

### Conversation summarizer
Ingest an entire agent conversation transcript and extract the key decisions, discoveries, and action items as structured memories. "What did we learn last session?"

### Memory provenance
Track where each memory came from (which conversation, which file, which commit). `hyphae trace <memory-id>` shows the full lineage.

Partially shipped: memories now persist project, branch, and worktree metadata. Traceability to commit and richer provenance queries are still open.

### Temporal queries
"What did I know about auth as of last Tuesday?" Point-in-time snapshots of the knowledge base. Useful for understanding how understanding evolved.

### Memory importance auto-scoring
Analyze access patterns to auto-promote frequently recalled memories and auto-demote never-accessed ones. Learn what the agent actually finds useful.

### Retrieval feedback loop
Track which search results the agent actually used vs ignored. Use this signal to improve ranking over time (learned relevance weights per project).

## Integration & Ecosystem

### Remote sync
Optional encrypted sync between machines via S3/R2/git. Work on laptop, recall on desktop.

### Multi-agent shared memory
Multiple agents (code review bot, CI bot, planning agent) read/write to the same hyphae instance with agent-scoped namespaces.

Partially shipped: memories are now branch/worktree-aware, which reduces cross-branch contamination and lays groundwork for agent-scoped memory.

### Webhook/event hooks
Trigger external actions on memory events. "When a critical memory is stored, post to Slack." Extensibility point for workflows.

### IDE sidebar
VS Code / JetBrains extension that shows relevant memories for the currently open file. Passive knowledge surfacing without explicit search.

## Search & Retrieval

### Query expansion
Automatically expand search queries with synonyms and related terms from the memoir graph. Searching "auth" also finds "authentication", "login", "JWT".

### Faceted search
Filter by topic + importance + date range + source type in a single query. `hyphae search "error handling" --topic backend --since 2026-01 --importance high`.

### Reranking pipeline
After initial retrieval, apply a cross-encoder reranker for higher precision on the top-K results. Pluggable reranker trait like the embedder.

### Graph-powered retrieval
Use memoir concept links to traverse related concepts during search. Finding "payment" also pulls in linked concepts like "Stripe", "refund policy", "PCI compliance".

## Quick Wins

### Memory tags
Lightweight tagging (`hyphae store --tags "auth,security"`) orthogonal to topics, enabling cross-cutting queries.

### TTL/expiry on memories
Explicit expiration dates beyond the decay model. "Remember this for 30 days" for temporary context like sprint goals.

### Bulk operations
`hyphae forget --topic "old-project"`, `hyphae decay --dry-run` to preview what would be pruned.

### Shell completions
Generate completions for bash/zsh/fish via clap's built-in support.
