#!/usr/bin/env bash
# Usage: ./scripts/release.sh v0.1.3
#
# Bumps version in all crate Cargo.toml files, commits, tags, and optionally pushes.
# Prevents the "forgot to update Cargo.toml" problem.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

die() { echo -e "${RED}error:${RESET} $1" >&2; exit 1; }
info() { echo -e "${GREEN}=>${RESET} $1"; }
warn() { echo -e "${YELLOW}warn:${RESET} $1"; }

# ── Validate input ──────────────────────────────────────────────────────────

TAG="${1:-}"
[ -z "$TAG" ] && die "Usage: ./scripts/release.sh v0.1.3"

# Accept both v0.1.3 and 0.1.3
VERSION="${TAG#v}"

# Validate semver format
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    die "Invalid version format: $VERSION (expected semver like 0.1.3 or 0.1.3-rc1)"
fi

# ── Pre-flight checks ──────────────────────────────────────────────────────

# Must be in repo root
[ -f Cargo.toml ] || die "Run from the repository root (where Cargo.toml is)"

# Working tree must be clean
if [ -n "$(git status --porcelain)" ]; then
    die "Working tree is dirty. Commit or stash changes first."
fi

# Tag must not already exist
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    die "Tag v$VERSION already exists"
fi

# All workspace crate Cargo.toml files
CRATE_TOMLS=(
    crates/hyphae-core/Cargo.toml
    crates/hyphae-store/Cargo.toml
    crates/hyphae-mcp/Cargo.toml
    crates/hyphae-cli/Cargo.toml
)

CURRENT_VERSION=$(grep '^version' "${CRATE_TOMLS[0]}" | head -1 | sed 's/.*"\(.*\)"/\1/')
info "Current version: ${BOLD}$CURRENT_VERSION${RESET}"
info "New version:     ${BOLD}$VERSION${RESET}"

# ── Bump version ────────────────────────────────────────────────────────────

for toml in "${CRATE_TOMLS[@]}"; do
    if [ ! -f "$toml" ]; then
        die "Crate Cargo.toml not found: $toml"
    fi
    # Replace only the first version = "..." line (in [package] section)
    awk -v old="$CURRENT_VERSION" -v new="$VERSION" '
        done == 0 && /^version = "/ { sub(old, new); done=1 }
        { print }
    ' "$toml" > "${toml}.tmp" && mv "${toml}.tmp" "$toml"
    info "Updated ${toml}"
done

# Update Cargo.lock
cargo check --quiet 2>/dev/null
info "Updated Cargo.lock"

# ── Quality gate ────────────────────────────────────────────────────────────

info "Running quality checks..."
cargo fmt --all --check || die "cargo fmt failed — run 'cargo fmt --all' first"
cargo clippy --all-targets --quiet 2>/dev/null || die "cargo clippy failed"
cargo test --quiet 2>/dev/null || die "cargo test failed"
info "All checks passed"

# ── Generate changelog ─────────────────────────────────────────────────────

PREV_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")

if [ -n "$PREV_TAG" ]; then
    LOG_RANGE="${PREV_TAG}..HEAD"
    info "Generating changelog from ${BOLD}$PREV_TAG${RESET} to HEAD"
else
    LOG_RANGE=""
    info "Generating changelog from initial commit"
fi

# Collect all commits into a temp file to avoid subshell issues
COMMITS_FILE=$(mktemp)
trap 'rm -f "$COMMITS_FILE"' EXIT
if [ -n "$LOG_RANGE" ]; then
    git log "$LOG_RANGE" --pretty=format:"%s|%h" --no-merges > "$COMMITS_FILE"
else
    git log --pretty=format:"%s|%h" --no-merges > "$COMMITS_FILE"
fi

# Group commits by conventional commit type
CHANGELOG=""

add_section() {
    local title="$1" prefix="$2"
    local commits=""
    while IFS='|' read -r msg hash; do
        if echo "$msg" | grep -qE "^${prefix}"; then
            desc=$(echo "$msg" | sed -E "s/^${prefix}:?\s*//")
            commits="${commits}- ${desc} (\`${hash}\`)\n"
        fi
    done < "$COMMITS_FILE"
    if [ -n "$commits" ]; then
        CHANGELOG="${CHANGELOG}### ${title}\n${commits}\n"
    fi
}

add_section "Features" "feat"
add_section "Bug Fixes" "fix"
add_section "Performance" "perf"
add_section "Refactoring" "refactor"
add_section "Documentation" "docs"
add_section "Tests" "test"
add_section "Chores" "chore"
add_section "CI" "ci"

# Catch any commits that don't match conventional format
OTHER=""
while IFS='|' read -r msg hash; do
    if ! echo "$msg" | grep -qE "^(feat|fix|perf|refactor|docs|test|chore|ci)"; then
        OTHER="${OTHER}- ${msg} (\`${hash}\`)\n"
    fi
done < "$COMMITS_FILE"
if [ -n "$OTHER" ]; then
    CHANGELOG="${CHANGELOG}### Other\n${OTHER}\n"
fi

if [ -z "$CHANGELOG" ]; then
    CHANGELOG="No changes since last release.\n"
fi

RELEASE_NOTES="## v${VERSION}\n\n${CHANGELOG}"

info "Changelog:"
echo ""
echo -e "$RELEASE_NOTES"

# Write CHANGELOG.md (prepend to existing or create new)
CHANGELOG_FILE="CHANGELOG.md"
FORMATTED_NOTES=$(echo -e "$RELEASE_NOTES")

if [ -f "$CHANGELOG_FILE" ]; then
    # Prepend new release notes after the header
    if head -1 "$CHANGELOG_FILE" | grep -q "^# Changelog"; then
        # Has header — insert after it
        {
            head -1 "$CHANGELOG_FILE"
            echo ""
            echo "$FORMATTED_NOTES"
            tail -n +2 "$CHANGELOG_FILE"
        } > "${CHANGELOG_FILE}.tmp" && mv "${CHANGELOG_FILE}.tmp" "$CHANGELOG_FILE"
    else
        # No header — prepend with header
        {
            echo "# Changelog"
            echo ""
            echo "$FORMATTED_NOTES"
            cat "$CHANGELOG_FILE"
        } > "${CHANGELOG_FILE}.tmp" && mv "${CHANGELOG_FILE}.tmp" "$CHANGELOG_FILE"
    fi
else
    {
        echo "# Changelog"
        echo ""
        echo "$FORMATTED_NOTES"
    } > "$CHANGELOG_FILE"
fi
info "Updated ${BOLD}$CHANGELOG_FILE${RESET}"

# ── Commit & tag ────────────────────────────────────────────────────────────

git add "${CRATE_TOMLS[@]}" Cargo.lock "$CHANGELOG_FILE"
git commit -m "chore: bump version to v$VERSION"
git tag -a "v$VERSION" -m "$(echo -e "Release v$VERSION\n\n$FORMATTED_NOTES")"

info "Created commit and tag ${BOLD}v$VERSION${RESET}"

# ── Push prompt ─────────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}Ready to push. Run:${RESET}"
echo ""
echo "  git push origin main --tags"
echo ""
