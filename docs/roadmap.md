# Hyphae Roadmap

This page is the Hyphae-specific backlog. The workspace [ROADMAP.md](../../docs/workspace/ROADMAP.md) keeps the ecosystem sequencing and cross-repo priorities.

## Recently Shipped

- Path handling is much more consistent than the early releases. Config, database, transcript, notify, import, and embedding-cache locations now resolve through shared logic instead of scattered per-command code.
- Hyphae now treats Claude and Codex session-derived memories as one normalized `agent_session` model. That gives the ecosystem a shared source of truth instead of separate host-specific ingestion paths.
- `hyphae init` and `doctor` now ride the shared editor-registration and host-discovery work. The result is better config handling and fewer host-specific edge cases.
- The feedback-loop foundation is in place. Recalls and outcomes can now link back to scoped sessions with stronger integrity checks instead of living as loosely related events.

## Next

### Recall effectiveness and outcome ranking

Hyphae should rank recalled context by observed usefulness, not just similarity. The work in [feedback-loop-design.md](feedback-loop-design.md) is the immediate path to making memory selection smarter across the ecosystem.

### Context-aware recall

Agents should not need to guess the perfect query before Hyphae becomes helpful. The next step is using repo state, open files, diffs, and active failures to infer which memories matter before the next tool call.

### Multi-project memory quality

One Hyphae store needs stronger project and workspace separation than it has today. Better deduplication, conflict detection, and provenance are the difference between a shared memory surface and a noisy pile of vaguely related notes.

### Auto-ingestion watcher

Long-lived worktrees should stay fresh without repeated manual ingest commands. A watcher-driven mode matters because the ecosystem is moving toward persistent local context, not one-shot imports.

### Shared and remote memory

Remote encrypted sync and shared-memory modes are the next step once the local single-machine path is reliable. This work needs to stay aligned with the ecosystem roadmap because it changes how multiple hosts and agents share context.

### Training and export surfaces

Hyphae is accumulating useful outcome data. The next priority is turning that stored history into structured export paths that can feed evaluation, training, and review workflows without custom one-off scripts.

## Later

### Memory consolidation

Optional LLM-assisted consolidation should merge related memories into better summaries once the ranking and provenance layers are stronger. Doing it earlier would polish recall output before the underlying trust signals are ready.

### Search ergonomics

Query expansion, faceted search, and a reranking pipeline all belong here. They matter, but they should follow the higher-leverage recall and provenance work that improves the base quality of what Hyphae stores and returns.

### Provenance and lineage tools

Hyphae should eventually make it easy to trace where a memory came from, how it changed, and what conversation, file, or commit produced it. That is valuable once the store has enough history to make lineage worth inspecting.

### Local operations

Export and import, structured output modes, tags, TTL controls, and bulk cleanup all improve day-to-day operations. They are useful, but they do not need to drive ecosystem order ahead of ranking and retrieval quality.

## Research

### Graph-powered retrieval

Hyphae can probably get better by traversing memoir relationships during search, but the open question is how much graph context improves retrieval before it starts surfacing loosely related noise.

### Temporal queries

Point-in-time questions such as "what did I know about this last week?" are clearly useful. The unresolved part is how to answer them cleanly when memories decay, merge, or get superseded over time.

### IDE surfaces and hooks

IDE-side memory surfaces and event hooks may become valuable once ranking, provenance, and session attribution are solid. Until then, they risk exposing a rough backend through a polished shell.
