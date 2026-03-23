#!/usr/bin/env bash
# Hyphae SessionEnd hook for Claude Code
# Stores a compact session-end marker. Keep this lightweight because SessionEnd
# hooks have a short timeout enforced by Claude Code.

set -euo pipefail

HYPHAE_BIN=${HYPHAE_BIN:-__HYPHAE_BIN__}

INPUT=$(cat)

if ! command -v jq >/dev/null 2>&1; then
  exit 0
fi

SESSION_ID=$(printf '%s' "$INPUT" | jq -r '.session_id // empty' 2>/dev/null)
REASON=$(printf '%s' "$INPUT" | jq -r '.reason // "other"' 2>/dev/null)
TRANSCRIPT_PATH=$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty' 2>/dev/null)
CWD=$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)

if [ -z "$SESSION_ID" ] || [ -z "$CWD" ]; then
  exit 0
fi

PROJECT=$(basename "$CWD" 2>/dev/null || printf 'project')
CONTENT="Claude Code session ${SESSION_ID} ended (reason: ${REASON})."
if [ -n "$TRANSCRIPT_PATH" ]; then
  CONTENT="${CONTENT} Transcript: ${TRANSCRIPT_PATH}"
fi

"$HYPHAE_BIN" store \
  --topic "session/${PROJECT}" \
  --content "$CONTENT" \
  --importance low \
  -P "$PROJECT" >/dev/null 2>&1 &

exit 0
