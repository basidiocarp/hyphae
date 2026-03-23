#!/usr/bin/env bash
# Hyphae PostToolUse hook for Claude Code
# Counts tool calls and auto-extracts context every N calls.
# Install: hyphae init --mode hook (or manually add to ~/.claude/settings.json)
#
# Input (stdin): JSON with tool_name, tool_input, tool_output, etc.
# Output: nothing (PostToolUse hooks are fire-and-forget)

set -euo pipefail

# Config
EXTRACT_EVERY=15           # Extract every N tool calls
COUNTER_FILE="${HYPHAE_HOOK_COUNTER:-${TMPDIR:-${TEMP:-${TMP:-.}}}/hyphae-hook-counter}"
HYPHAE_BIN=${HYPHAE_BIN:-__HYPHAE_BIN__}

# Read hook input
INPUT=$(cat)
if ! command -v jq >/dev/null 2>&1; then
  exit 0
fi
TOOL_NAME=$(printf '%s' "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)

# Skip Hyphae's own tools (avoid infinite loop)
case "$TOOL_NAME" in
  hyphae_*|mcp__hyphae__*) exit 0 ;;
esac

# Increment counter
COUNT=0
if [ -f "$COUNTER_FILE" ]; then
  COUNT=$(cat "$COUNTER_FILE" 2>/dev/null || echo "0")
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"

# Reset counter on hyphae_memory_store (agent stored voluntarily)
if [ "$TOOL_NAME" = "hyphae_memory_store" ] || [ "$TOOL_NAME" = "mcp__hyphae__hyphae_memory_store" ]; then
  echo "0" > "$COUNTER_FILE"
  exit 0
fi

# Not time to extract yet
if [ "$COUNT" -lt "$EXTRACT_EVERY" ]; then
  exit 0
fi

# Reset counter
echo "0" > "$COUNTER_FILE"

# Extract from tool output if available
TOOL_OUTPUT=$(
  printf '%s' "$INPUT" | jq -r '
    (.tool_response // .tool_output // empty)
    | if type == "string" then . else tojson end
  ' 2>/dev/null
)
if [ -z "$TOOL_OUTPUT" ]; then
  exit 0
fi

# Get project name from cwd
PROJECT_CWD=$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null)
if [ -n "$PROJECT_CWD" ]; then
  PROJECT=$(basename "$PROJECT_CWD" 2>/dev/null || echo "project")
else
  PROJECT=$(basename "$(pwd)" 2>/dev/null || echo "project")
fi

# Extract facts and store (async, don't block the agent)
echo "$TOOL_OUTPUT" | "$HYPHAE_BIN" extract -p "$PROJECT" 2>/dev/null &

exit 0
