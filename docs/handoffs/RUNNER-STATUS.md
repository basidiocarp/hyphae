# Runner Status

This file tracks the current handoff-agent state for `hyphae`-driven work.

## Status Model

- `queued`: handoff prepared, agent not yet dispatched
- `running`: agent dispatched and still active
- `completed`: agent returned a final result
- `closed`: agent explicitly closed and no longer in use

## Current

Last updated: 2026-04-06

- Active runners:
  - none

## Convention

- close stale agents before dispatching a new handoff pair
- use fresh agents for each new handoff cycle
