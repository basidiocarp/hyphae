# Recall-to-Action Feedback Loop

Engineering design document for automatically boosting memories that correlate with successful outcomes after recall.

**Status**: Design
**Author**: Architecture review
**Date**: 2026-03-22

## Problem

Hyphae recalls memories based on text similarity (FTS5 + cosine via sqlite-vec) and static weight. The weight decays over time but never increases based on utility. A memory recalled before a successful coding session is treated identically to one recalled before a session full of errors. Over time, all episodic memories drift toward zero weight regardless of value.

The goal is to close the loop between recall and outcome so that useful memories rise without manual curation.

## 1. Signal Collection

### What constitutes "success" after a recall?

Signals are collected within a **window** after each recall event. The window is defined as the remainder of the current MCP session (from the recall timestamp until `session_end`), capped at 60 minutes.

| Signal | Source | Weight | Interpretation |
|--------|--------|--------|---------------|
| No errors in N subsequent tool calls | MCP tool call log | +1 per 10 error-free calls, max +3 | Agent used the recalled context without hitting errors |
| Tests pass after code changes | `hyphae_memory_store` with topic `tests/passed` or cortina hook | +2 | Code changes informed by recall led to passing tests |
| No self-corrections | Absence of `corrections` topic stores in window | +1 | Agent did not need to reverse decisions |
| Session ends with errors=0 | `session_end` call with errors="0" or null | +2 | Overall session success |
| Session ends with errors>0 | `session_end` call with errors>0 | -2 | Overall session failure |
| Edit reversals in window | `hyphae_memory_store` with topic `corrections` | -1 per correction, max -3 | Agent had to undo decisions |
| Explicit positive feedback | Future: agent calls `hyphae_memory_boost(id)` | +3 | Agent explicitly marks memory as helpful |

**Error detection heuristic**: A tool call is counted as an error if the MCP response contains `isError: true`. This is already part of the JSON-RPC protocol.

### Signal collection is passive

Phase 1 collects signals without acting on them. The MCP server already sees every tool call in sequence. The required additions are:

1. Log which memory IDs were returned by each `hyphae_memory_recall` call
2. Log tool call outcomes (success/error) with timestamps
3. Correlate at session end

## 2. Schema Changes

### New table: `recall_events`

Records each recall invocation and which memories were returned.

```sql
CREATE TABLE IF NOT EXISTS recall_events (
    id TEXT PRIMARY KEY,            -- ulid
    session_id TEXT,                -- FK to sessions.id (nullable for sessionless recalls)
    query TEXT NOT NULL,            -- the recall query string
    recalled_at TEXT NOT NULL,      -- ISO 8601 timestamp
    memory_ids TEXT NOT NULL,       -- JSON array of memory IDs returned
    memory_count INTEGER NOT NULL,  -- len(memory_ids) for quick stats
    project TEXT                    -- project context at recall time
);

CREATE INDEX IF NOT EXISTS idx_recall_events_session
    ON recall_events(session_id);
CREATE INDEX IF NOT EXISTS idx_recall_events_recalled_at
    ON recall_events(recalled_at);
```

### New table: `outcome_signals`

Records success/failure signals tied to a session and time window.

```sql
CREATE TABLE IF NOT EXISTS outcome_signals (
    id TEXT PRIMARY KEY,            -- ulid
    session_id TEXT,                -- FK to sessions.id
    signal_type TEXT NOT NULL,      -- 'error_free_run', 'test_pass', 'no_corrections',
                                    -- 'session_success', 'session_failure', 'correction', 'explicit_boost'
    signal_value INTEGER NOT NULL,  -- positive = success, negative = failure (see weight table above)
    occurred_at TEXT NOT NULL,      -- ISO 8601 timestamp
    source TEXT,                    -- what generated this signal (tool name, hook name)
    project TEXT
);

CREATE INDEX IF NOT EXISTS idx_outcome_signals_session
    ON outcome_signals(session_id);
CREATE INDEX IF NOT EXISTS idx_outcome_signals_occurred_at
    ON outcome_signals(occurred_at);
```

### New table: `recall_effectiveness`

Precomputed effectiveness scores, updated by the scoring job.

```sql
CREATE TABLE IF NOT EXISTS recall_effectiveness (
    memory_id TEXT NOT NULL,         -- FK to memories.id
    recall_event_id TEXT NOT NULL,   -- FK to recall_events.id
    effectiveness REAL NOT NULL,     -- computed score [-1.0, 1.0]
    signal_count INTEGER NOT NULL,   -- number of signals that contributed
    computed_at TEXT NOT NULL,        -- when this score was computed
    PRIMARY KEY (memory_id, recall_event_id)
);

CREATE INDEX IF NOT EXISTS idx_recall_effectiveness_memory
    ON recall_effectiveness(memory_id);
```

### Migration strategy

Add these tables in `schema.rs` using the existing pattern: check `sqlite_master` for table existence, create if missing. No existing tables are altered.

```rust
// In init_db_with_dims(), after existing migrations:
tx.execute_batch("
    CREATE TABLE IF NOT EXISTS recall_events ( ... );
    CREATE TABLE IF NOT EXISTS outcome_signals ( ... );
    CREATE TABLE IF NOT EXISTS recall_effectiveness ( ... );
    -- indexes --
");
```

## 3. Scoring Algorithm

### 3.1 Per-recall effectiveness

For a given recall event `R` that returned memories `[M1, M2, ..., Mn]`:

```
window_start = R.recalled_at
window_end   = min(session.ended_at, R.recalled_at + 60min)

signals = SELECT * FROM outcome_signals
          WHERE session_id = R.session_id
            AND occurred_at BETWEEN window_start AND window_end

-- session_success/session_failure count for every recall whose window reaches
-- session_end, even though the timeline still attaches the row to the latest
-- recall for operator display

positive_sum = sum(signal_value) for signals where signal_value > 0
negative_sum = sum(signal_value) for signals where signal_value < 0

-- Normalize to [-1.0, 1.0]
raw_score = (positive_sum + negative_sum) / max(|positive_sum| + |negative_sum|, 1)

-- Position discount: first result gets full credit, later results get less
-- This reflects that earlier results are more likely to be read/used
for i, memory_id in R.memory_ids:
    position_factor = 1.0 / (1.0 + 0.3 * i)   -- [1.0, 0.77, 0.63, 0.53, ...]
    effectiveness[memory_id][R.id] = raw_score * position_factor
```

### 3.2 Aggregate effectiveness per memory

A memory may be recalled multiple times. Aggregate with exponential recency weighting:

```
-- For memory M, across all recall events where M was returned:
events = SELECT * FROM recall_effectiveness WHERE memory_id = M.id
         ORDER BY computed_at DESC

recency_half_life = 14 days
aggregate = 0.0

for event in events:
    age_days = (now - event.computed_at).days()
    recency_weight = exp(-0.693 * age_days / recency_half_life)  -- 0.693 = ln(2)
    aggregate += event.effectiveness * recency_weight

aggregate_effectiveness = clamp(aggregate, -1.0, 1.0)
```

### 3.3 Weight boosting

Apply the aggregate effectiveness to the memory's weight:

```
BOOST_FACTOR = 0.15          -- maximum boost per cycle is 15% of current weight
MAX_BOOST_MULTIPLIER = 1.5   -- weight can never exceed 1.5x its base value
MIN_WEIGHT = 0.05            -- floor to prevent memories from being zeroed out
PENALTY_FACTOR = 0.05        -- negative effectiveness penalizes less aggressively

if aggregate_effectiveness > 0:
    boost = weight * BOOST_FACTOR * aggregate_effectiveness
else:
    boost = weight * PENALTY_FACTOR * aggregate_effectiveness  -- negative, reduces weight

-- Apply with ceiling
base_weight = weight stored at memory creation (or last consolidation)
new_weight = clamp(
    weight + boost,
    MIN_WEIGHT,
    min(1.0, base_weight * MAX_BOOST_MULTIPLIER)
)
```

### 3.4 Interaction with existing decay

The existing `apply_decay` mechanism runs on recall (via `maybe_auto_decay`). The feedback boost runs after decay, so the net effect is:

```
after_decay = weight * decay_factor_adjusted_for_importance
after_boost = after_decay + boost_from_effectiveness
final = clamp(after_boost, MIN_WEIGHT, MAX_BOOST_MULTIPLIER * base_weight)
```

This means a memory that keeps being useful will resist decay. A memory that gets recalled but leads to bad outcomes will decay faster than normal.

### 3.5 Concrete parameter table

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `WINDOW_MAX_MINUTES` | 60 | Most agent sessions complete within 60 min; longer windows dilute correlation |
| `BOOST_FACTOR` | 0.15 | Conservative: 6-7 successful recall cycles to go from 0.5 to ~0.75 |
| `PENALTY_FACTOR` | 0.05 | Penalties are 3x weaker than boosts to avoid punishing memories unfairly |
| `MAX_BOOST_MULTIPLIER` | 1.5 | Prevents runaway boosting; a memory cannot exceed 150% of its base weight |
| `MIN_WEIGHT` | 0.05 | Keeps memories discoverable even with negative effectiveness |
| `RECENCY_HALF_LIFE_DAYS` | 14 | Old recall events fade; a 30-day-old event has ~25% influence |
| `POSITION_DISCOUNT` | 0.3 | Result at position 3 gets 63% credit vs position 0 |
| `MIN_SIGNALS_FOR_BOOST` | 2 | Require at least 2 signals before applying any boost (noise reduction) |

## 4. MCP Integration

### 4.1 Recall event logging

In `tool_recall` (crates/hyphae-mcp/src/tools/memory.rs), after the search results are collected and before formatting output:

```rust
// After: for (mem, _) in &scored_results { store.update_access(&mem.id); }
// Add:
let memory_ids: Vec<String> = scored_results.iter()
    .map(|(mem, _)| mem.id.to_string())
    .collect();
if !memory_ids.is_empty() {
    let _ = store.log_recall_event(
        session_id.as_deref(),  // from MCP server state
        query,
        &memory_ids,
        project,
    );
}
```

The same pattern applies to the FTS fallback path lower in `tool_recall`.

### 4.2 Outcome signal collection

Signals are recorded by observing MCP tool call results. In the MCP server's dispatch loop (`crates/hyphae-mcp/src/server.rs` or equivalent), after each tool call completes:

```rust
// Pseudocode for the MCP dispatch loop
fn handle_tool_result(&self, tool_name: &str, result: &ToolResult, session_id: &str) {
    if result.is_error {
        // Don't log individual errors as outcome signals yet.
        // Errors are counted by the error-free-run tracker.
        self.error_free_count = 0;
    } else {
        self.error_free_count += 1;
        if self.error_free_count % 10 == 0 {
            let _ = store.log_outcome_signal(
                session_id,
                "error_free_run",
                1,  // +1 per 10 clean calls
                "mcp_dispatch",
                project,
            );
        }
    }
}
```

### 4.3 Session end signals

`store.session_end()` now records `session_success` or `session_failure` directly when the structured session row is completed. `tool_session_end` and `hyphae session end` only add the compatibility `session/{project}` memory afterward, on a best-effort basis.

```rust
let error_count = errors
    .and_then(|value| value.parse::<i64>().ok())
    .unwrap_or(0);
let signal_type = if error_count > 0 {
    "session_failure"
} else {
    "session_success"
};
let signal_value = if error_count > 0 { -2 } else { 2 };
let _ = store.log_outcome_signal(
    Some(session_id),
    signal_type,
    signal_value,
    Some("hyphae.session_end"),
    Some(&project),
);
```

### 4.4 Cortina hook signals

Cortina's PostToolUse hooks already capture errors and corrections. These write to `outcome_signals` via Hyphae's structured surfaces:

- tool error: `signal_type = "tool_error"`, `signal_value = -1`
- correction: `signal_type = "correction"`, `signal_value = -1`
- test pass: `signal_type = "test_passed"`, `signal_value = 2`

Cortina writes to the same hyphae SQLite DB (path from `HYPHAE_DB` env var or config).

### 4.5 Effectiveness computation trigger

**Phase 2 (offline)**: CLI command `hyphae feedback compute` iterates over all `recall_events` where `recalled_at > last_computation_at`, computes effectiveness, and writes to `recall_effectiveness`. Then aggregates per memory and adjusts weights.

**Phase 3 (online)**: On each `hyphae_memory_recall`, after returning results, spawn a lightweight background computation for recall events in the current session that have enough signals. Uses the existing `hyphae_metadata` table to track `last_feedback_compute_at`.

```rust
// On recall, after logging the event:
if let Ok(true) = store.should_compute_feedback(session_id) {
    // Only compute if there are >=2 signals for at least one recall event
    let _ = store.compute_session_effectiveness(session_id);
}
```

## 5. Risks and Mitigations

### 5.1 False positives: unrelated success boosts recalled memories

An agent recalls a memory about "database indexing", then successfully fixes a CSS bug. The database memory gets falsely boosted.

Mitigations:
- Position discount: memories lower in the result list get less credit. If the agent recalled 5 memories and only used one, the others are naturally discounted.
- MIN_SIGNALS_FOR_BOOST = 2: requires multiple signals, not just one session success.
- Recency half-life = 14 days: a single false-positive event fades to 25% influence within a month.
- Aggregate across events: one false positive is diluted by later negative evidence and fades as its recency weight decays.
- Phase 2 analysis: before enabling online boosting, inspect data manually via `hyphae feedback inspect` to validate correlation quality.

### 5.2 Runaway boosting: popular memories dominate

A frequently recalled memory accumulates effectiveness score and drowns out newer, potentially better memories.

Mitigations:
- MAX_BOOST_MULTIPLIER = 1.5: hard ceiling. Weight can never exceed 150% of base.
- Weight is clamped to [0.0, 1.0] by the existing `Weight` type. The boost multiplier applies to the base weight, so even 1.5 * 1.0 = 1.5 is clamped to 1.0 by `Weight::new_clamped`.
- Decay still applies: even boosted memories decay if not accessed. The boost slows decay but does not exempt from it.
- Existing search uses RRF: the hybrid search (30% FTS + 70% cosine) already considers text relevance. Weight is used as a tiebreaker in `ORDER BY weight DESC`, not as the primary ranking signal. A boosted irrelevant memory still won't surface for unrelated queries.

### 5.3 Cold start: new memories have no recall history

A freshly stored memory has effectiveness = 0, competing against well-established memories.

Mitigations:
- No penalty for no data: `aggregate_effectiveness = 0.0` when there are no recall events, so the memory's weight is unmodified by the feedback system.
- Importance as prior: new memories start with `weight = 1.0` (the maximum). The feedback loop only adjusts weight downward via decay or slows that decay via positive effectiveness.
- MIN_SIGNALS_FOR_BOOST = 2: recalls with fewer than 2 contributing signals persist a `0.0` effectiveness row, so they do not get boosted or penalized from sparse evidence.

### 5.4 Stale correlations persist

A memory was useful 6 months ago but the codebase has changed. Its boosted weight keeps it surfacing.

Mitigations:
- Recency half-life = 14 days: after 28 days, an event has ~25% influence; after 56 days, ~6%.
- Existing decay runs continuously: `maybe_auto_decay` runs on every recall, applying importance-weighted decay. Even a boosted memory eventually drops below the prune threshold.
- Stale indicator at 30 days: the existing `age_indicator` in `tool_recall` already warns agents about stale memories.

### 5.5 Storage growth

High-volume usage creates many recall_events and outcome_signals rows.

Mitigations:
- Prune old data: `hyphae feedback prune --older-than 90d` deletes recall_events and outcome_signals older than the retention period. Can be added to the existing `hyphae prune` command.
- Estimated size: each recall event is ~200 bytes, each signal ~150 bytes. At 100 recalls/day and 500 signals/day, that is ~70 KB/day, ~25 MB/year.

### 5.6 Feedback loop instability

Boosted memories get recalled more, generating more positive signals, getting boosted further.

Mitigations:
- MAX_BOOST_MULTIPLIER = 1.5 with weight capped at 1.0: the ceiling prevents exponential growth.
- BOOST_FACTOR = 0.15: conservative step size. Even with perfect effectiveness (+1.0), each cycle adds at most 15% of current weight.
- Penalty factor: bad outcomes reduce weight, providing a counterbalancing force.
- Weight is one of many ranking signals: hybrid search uses text similarity and embedding distance as primary signals. Weight breaks ties, it does not dominate ranking.

## 6. Implementation Phases

### Phase 1: Signal Collection (no boosting)

**Goal**: Instrument recall and outcome logging. Collect data for offline analysis.

**Changes**:
1. **Schema**: Add `recall_events` and `outcome_signals` tables to `schema.rs`
2. **Store methods**: Add `log_recall_event()` and `log_outcome_signal()` to `SqliteStore`
3. **MCP recall**: In `tool_recall`, log recall events after search results are collected
4. **MCP dispatch**: Track error-free call streaks, log `error_free_run` signals
5. **Session end**: Log `session_success` / `session_failure` signals
6. **CLI**: Add `hyphae feedback stats` to inspect collected data

**Files modified**:
- `crates/hyphae-store/src/schema.rs` -- add tables
- `crates/hyphae-store/src/store/mod.rs` -- add `feedback` module
- `crates/hyphae-store/src/store/feedback.rs` -- NEW: `log_recall_event`, `log_outcome_signal`, query helpers
- `crates/hyphae-mcp/src/tools/memory.rs` -- instrument `tool_recall`
- `crates/hyphae-mcp/src/server.rs` -- track error-free streaks (if centralized dispatch exists)
- `crates/hyphae-mcp/src/tools/session.rs` -- log session outcome signals
- `crates/hyphae-cli/src/commands/mod.rs` -- add `feedback` subcommand

**Estimated effort**: 2-3 days

### Phase 2: Offline Analysis

**Goal**: CLI command to compute effectiveness scores and inspect correlations. No automatic weight changes.

**Changes**:
1. **Schema**: Add `recall_effectiveness` table
2. **Scoring**: Implement the per-recall and aggregate scoring algorithms
3. **CLI commands**:
   - `hyphae feedback compute` -- compute effectiveness for all unscored recall events
   - `hyphae feedback inspect [--memory-id ID]` -- show effectiveness history for a memory
   - `hyphae feedback top [--limit N]` -- show memories with highest aggregate effectiveness
   - `hyphae feedback prune --older-than DURATION` -- clean up old events and signals
4. **Dry-run boost**: `hyphae feedback simulate` -- show what weight changes would be applied without actually applying them

**Files modified**:
- `crates/hyphae-store/src/schema.rs` -- add `recall_effectiveness` table
- `crates/hyphae-store/src/store/feedback.rs` -- add scoring functions
- `crates/hyphae-cli/src/commands/feedback.rs` -- NEW: CLI subcommands

**Estimated effort**: 3-4 days

### Phase 3: Online Boosting

**Goal**: Automatically adjust memory weights based on computed effectiveness.

**Changes**:
1. **On-recall trigger**: After `tool_recall`, compute effectiveness for prior recall events in the same session that now have enough signals
2. **Weight adjustment**: Apply the boost formula to memory weights via `MemoryStore::update`
3. **Base weight tracking**: Add `base_weight` column to `memories` table (set to weight at creation time or last consolidation) to enforce `MAX_BOOST_MULTIPLIER` relative to the original weight
4. **Config**: Add `[feedback]` section to `config.toml`:
   ```toml
   [feedback]
   enabled = true
   boost_factor = 0.15
   penalty_factor = 0.05
   max_boost_multiplier = 1.5
   min_signals = 3
   window_max_minutes = 60
   recency_half_life_days = 14
   ```
5. **Feature flag**: Gate behind `feedback` feature flag (default off) until validated

**Files modified**:
- `crates/hyphae-store/src/schema.rs` -- add `base_weight` column migration
- `crates/hyphae-store/src/store/feedback.rs` -- add `apply_feedback_boost`, `compute_session_effectiveness`
- `crates/hyphae-mcp/src/tools/memory.rs` -- trigger effectiveness computation on recall
- `crates/hyphae-core/src/memory.rs` -- add `base_weight` field to `Memory`
- `crates/hyphae-cli/src/config.rs` -- add feedback config section

**Estimated effort**: 4-5 days

## 7. Store API

New methods on `SqliteStore` (in `crates/hyphae-store/src/store/feedback.rs`):

```rust
impl SqliteStore {
    // ─────────────────────────────────────────────────────────────────────────
    // Phase 1: Logging
    // ─────────────────────────────────────────────────────────────────────────

    /// Log a recall event with the memory IDs that were returned.
    pub fn log_recall_event(
        &self,
        session_id: Option<&str>,
        query: &str,
        memory_ids: &[String],
        project: Option<&str>,
    ) -> HyphaeResult<String>;  // returns recall_event_id

    /// Log an outcome signal for a session.
    pub fn log_outcome_signal(
        &self,
        session_id: &str,
        signal_type: &str,
        signal_value: i32,
        source: &str,
        project: Option<&str>,
    ) -> HyphaeResult<String>;  // returns signal_id

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 2: Scoring
    // ─────────────────────────────────────────────────────────────────────────

    /// Compute effectiveness for all unscored recall events.
    /// Returns the number of recall events processed.
    pub fn compute_effectiveness(&self) -> HyphaeResult<usize>;

    /// Get aggregate effectiveness for a memory across all its recall events.
    pub fn aggregate_effectiveness(
        &self,
        memory_id: &MemoryId,
    ) -> HyphaeResult<f32>;  // returns recency-weighted effectiveness [-1.0, 1.0]

    /// Get feedback stats: total recall events, total signals, top memories.
    pub fn feedback_stats(
        &self,
        project: Option<&str>,
    ) -> HyphaeResult<FeedbackStats>;

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 3: Boosting
    // ─────────────────────────────────────────────────────────────────────────

    /// Compute effectiveness for a session's recall events and apply weight boosts.
    /// Only processes events with >= min_signals signals.
    pub fn compute_session_effectiveness(
        &self,
        session_id: &str,
        config: &FeedbackConfig,
    ) -> HyphaeResult<usize>;  // returns number of memories boosted

    /// Check if enough signals have accumulated to justify computation.
    pub fn should_compute_feedback(
        &self,
        session_id: &str,
        min_signals: usize,
    ) -> HyphaeResult<bool>;

    /// Prune recall events and signals older than the given duration.
    pub fn prune_feedback_data(
        &self,
        older_than: chrono::Duration,
    ) -> HyphaeResult<(usize, usize)>;  // (events_pruned, signals_pruned)
}
```

## 8. Data Types

```rust
// In crates/hyphae-store/src/store/feedback.rs

#[derive(Debug, Clone)]
pub struct FeedbackConfig {
    pub boost_factor: f32,           // 0.15
    pub penalty_factor: f32,         // 0.05
    pub max_boost_multiplier: f32,   // 1.5
    pub min_signals: usize,          // 3
    pub window_max_minutes: i64,     // 60
    pub recency_half_life_days: f64, // 14.0
    pub position_discount: f32,      // 0.3
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            boost_factor: 0.15,
            penalty_factor: 0.05,
            max_boost_multiplier: 1.5,
            min_signals: 3,
            window_max_minutes: 60,
            recency_half_life_days: 14.0,
            position_discount: 0.3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeedbackStats {
    pub total_recall_events: usize,
    pub total_signals: usize,
    pub total_scored: usize,
    pub avg_effectiveness: f32,
    pub top_memories: Vec<(MemoryId, f32)>,  // top N by aggregate effectiveness
}

#[derive(Debug, Clone)]
pub struct RecallEvent {
    pub id: String,
    pub session_id: Option<String>,
    pub query: String,
    pub recalled_at: String,
    pub memory_ids: Vec<String>,
    pub memory_count: usize,
    pub project: Option<String>,
}
```

## 9. Testing Strategy

### Unit tests (in `feedback.rs`)

- `test_log_recall_event_stores_correctly` -- verify insertion and JSON serialization of memory_ids
- `test_log_outcome_signal_stores_correctly` -- verify insertion with all signal types
- `test_compute_effectiveness_positive` -- 3 positive signals in window produce effectiveness > 0
- `test_compute_effectiveness_negative` -- 3 negative signals produce effectiveness < 0
- `test_compute_effectiveness_mixed` -- mixed signals produce intermediate score
- `test_compute_effectiveness_empty_window` -- no signals produce effectiveness = 0
- `test_position_discount` -- first result gets higher effectiveness than last
- `test_aggregate_with_recency` -- recent events weighted more than old events
- `test_boost_clamped_to_max` -- weight never exceeds MAX_BOOST_MULTIPLIER * base_weight
- `test_boost_floor_at_min_weight` -- weight never goes below MIN_WEIGHT
- `test_min_signals_gate` -- no boost applied with fewer than MIN_SIGNALS signals
- `test_prune_old_data` -- events older than threshold are deleted

### Integration tests

- `test_recall_logs_event` -- full recall flow logs to recall_events table
- `test_session_lifecycle_with_feedback` -- start session, recall, tool calls, end session, verify signals
- `test_feedback_compute_cli` -- `hyphae feedback compute` processes all pending events

### Snapshot tests

- Snapshot the output of `hyphae feedback stats` and `hyphae feedback inspect`

## 10. Open Questions

1. **Should cortina write to the hyphae DB directly or via MCP?** Direct SQLite writes are simpler and faster. MCP would require adding new tools. Recommendation: direct writes, since cortina already knows the DB path.

2. **Should the feedback loop apply to memoir concept lookups too?** Memoirs are permanent knowledge graphs, not episodic. Their relevance is more stable. Recommendation: defer to a separate design.

3. **What is the right recency half-life?** 14 days is a guess. Phase 2 analysis should include a sensitivity analysis: compute aggregate effectiveness with half-lives of 7, 14, 28, and 56 days and compare rank stability.

4. **Should we track which specific memory the agent "used"?** If the agent quotes a memory ID in a subsequent tool call, that is a stronger signal than just "it was in the recall results". This is hard to detect reliably. Recommendation: defer, use position discount as a proxy.
