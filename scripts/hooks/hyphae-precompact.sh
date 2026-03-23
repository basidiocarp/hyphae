#!/usr/bin/env bash
# Hyphae PreCompact hook for Claude Code
# Captures a lightweight marker before compaction so the session has a breadcrumb
# even when full summarization is deferred to later tooling.

set -euo pipefail

HYPHAE_BIN=${HYPHAE_BIN:-__HYPHAE_BIN__}

INPUT=$(cat)

if ! command -v jq >/dev/null 2>&1; then
  exit 0
fi

SESSION_ID=$(printf '%s' "$INPUT" | jq -r '.session_id // empty' 2>/dev/null)
TRIGGER=$(printf '%s' "$INPUT" | jq -r '.trigger // "unknown"' 2>/dev/null)
CUSTOM_INSTRUCTIONS=$(printf '%s' "$INPUT" | jq -r '.custom_instructions // empty' 2>/dev/null)
CWD=$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)

if [ -z "$SESSION_ID" ] || [ -z "$CWD" ]; then
  exit 0
fi

PROJECT=$(basename "$CWD" 2>/dev/null || printf 'project')
CONTENT="Context compaction requested (${TRIGGER}) for session ${SESSION_ID}."
if [ -n "$CUSTOM_INSTRUCTIONS" ]; then
  CONTENT="${CONTENT} Instructions: ${CUSTOM_INSTRUCTIONS}"
fi

"$HYPHAE_BIN" store \
  --topic "session/${PROJECT}" \
  --content "$CONTENT" \
  --importance low \
  -P "$PROJECT" >/dev/null 2>&1 &

exit 0
