# Hyphae Scope

Hyphae owns knowledge persistence and retrieval for the basidiocarp ecosystem.

---

## What hyphae owns

- **Episodic memory** — decay-based storage and recall of events, decisions, and corrections
- **Semantic memoirs** — permanent knowledge graphs for architecture, domain models, and durable concepts
- **Hybrid search** — FTS5 and vector retrieval blended into a single ranked result set
- **RAG pipeline** — document ingestion, chunking, and context injection
- **Session lifecycle** — session start, end, and context snapshots
- **Feedback signals** — corrections, outcomes, and recall event logging
- **Self-evaluation** — measuring the effectiveness of the knowledge system itself (`hyphae evaluate`, `hyphae extract-lessons`)

---

## What hyphae does not own

- **Lifecycle event capture** — cortina captures hook signals and writes structured events
- **Code structure intelligence** — rhizome owns code analysis and export
- **Host execution and context assembly** — volva owns the runtime orchestration layer
- **Signal routing from hooks** — cortina routes hook output to consumers
- **Operator dashboards** — cap reads and renders ecosystem data

---

## Frozen features

`hyphae export-training` exists because the training data is here, not because training export is hyphae's responsibility. It will not grow further inside hyphae. No new export formats, no fine-tuning pipeline features. If fine-tuning becomes a real workflow, it graduates to its own tool or a lamella skill that reads from hyphae.

---

## Scope test

When considering adding a feature to hyphae, ask: "Is this storing, retrieving, or evaluating knowledge?"

If yes, it belongs here. If the feature uses hyphae's data to serve a different purpose — export pipelines, UI rendering, signal routing — it belongs elsewhere.
