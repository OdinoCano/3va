#!/usr/bin/env bash
# update-version.sh — Bump all version references across the 3va project
#
# Usage:
#   ./scripts/update-version.sh 2.1.0 2.2.0   # from 2.1.0 to 2.2.0
#   ./scripts/update-version.sh 2.2.0           # shorthand (prompts for current)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

CURRENT="${1:-}"
NEXT="${2:-}"

if [[ -z "$CURRENT" ]]; then
    echo "Usage: $0 <current-version> <next-version>"
    echo "       $0 <next-version>   (auto-detect current from Cargo.toml)"
    echo ""
    echo "Detecting current version from Cargo.toml..."
    CURRENT=$(grep '^version = ' "$ROOT_DIR/crates/js/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "Detected current version: $CURRENT"
    if [[ -z "$CURRENT" ]]; then
        echo "Error: Could not detect current version"
        exit 1
    fi
    echo ""
    echo "Please provide the new version:"
    echo "  $0 $CURRENT <next-version>"
    exit 1
fi

if [[ -z "$NEXT" ]]; then
    NEXT="$CURRENT"
    CURRENT=$(grep '^version = ' "$ROOT_DIR/crates/js/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
    if [[ -z "$CURRENT" ]]; then
        echo "Error: Could not detect current version"
        exit 1
    fi
    echo "Please provide the new version:"
    echo "  $0 $CURRENT <next-version>"
    exit 1
fi

echo "Updating version: $CURRENT → $NEXT"

# ── Rust crates ───────────────────────────────────────────────────────────────
echo "  • Updating crates/*/Cargo.toml..."
for f in "$ROOT_DIR"/crates/*/Cargo.toml; do
    if grep -q "version = \"$CURRENT\"" "$f" 2>/dev/null; then
        sed -i "s/version = \"$CURRENT\"/version = \"$NEXT\"/g" "$f"
        echo "    updated $f"
    fi
done

# ── Rust crate internal dependencies (dev-dependencies with path) ────────────────
echo "  • Updating crate internal dependency versions..."
for f in "$ROOT_DIR"/crates/*/Cargo.toml; do
    if grep -q "vvva_.*path" "$f" 2>/dev/null; then
        sed -i "s/vvva_\([^ ]*\) = { version = \"$CURRENT\", path/vvva_\1 = { version = \"$NEXT\", path/g" "$f" 2>/dev/null || true
    fi
done

# ── process.rs runtime version strings ─────────────────────────────────────────
echo "  • Updating crates/js/src/builtins/process.rs..."
sed -i "s/\"3va\/\$CURRENT\"/\"3va\/$NEXT\"/g" "$ROOT_DIR/crates/js/src/builtins/process.rs"

# ── CHANGELOG.md ──────────────────────────────────────────────────────────────
echo "  • Updating docs/CHANGELOG.md..."
if grep -q "## \[${CURRENT}\]" "$ROOT_DIR/docs/CHANGELOG.md" 2>/dev/null; then
    sed -i "s/## \[${CURRENT}\]/## [${NEXT}]/" "$ROOT_DIR/docs/CHANGELOG.md"
fi

# ── Roadmap ────────────────────────────────────────────────────────────────────
echo "  • Updating docs/12-roadmap/01-roadmap.md..."
sed -i "s/Current Status (v${CURRENT}/Current Status (v${NEXT}/" "$ROOT_DIR/docs/12-roadmap/01-roadmap.md" 2>/dev/null || true

# ── Distribution files ───────────────────────────────────────────────────────
DIST_FILES=(
    "bucket/3va.json"
    "Formula/3va.rb"
    "dist/cargo-wrapper/three-va/Cargo.toml"
    "dist/chocolatey/3va.nuspec"
    "dist/flatpak/com.github.OdinoCano.3va.metainfo.xml"
    "dist/flatpak/com.github.OdinoCano.3va.yml"
    "dist/homebrew/Formula/3va.rb"
    "dist/nix/default.nix"
    "dist/nix/flake.nix"
    "dist/scoop/3va.json"
    "dist/snap/snapcraft.yaml"
)

for f in "${DIST_FILES[@]}"; do
    FULL_PATH="$ROOT_DIR/$f"
    if [[ -f "$FULL_PATH" ]]; then
        if grep -q "$CURRENT" "$FULL_PATH" 2>/dev/null; then
            sed -i "s/$CURRENT/$NEXT/g" "$FULL_PATH"
            echo "    updated $f"
        fi
    fi
done

# ── Verify ─────────────────────────────────────────────────────────────────────
echo ""
echo "Verification (looking for remaining '$CURRENT' references)..."
REMAINING=$(cd "$ROOT_DIR" && grep -rn "$CURRENT" \
    --include="*.json" --include="*.toml" --include="*.rb" \
    --include="*.ps1" --include="*.xml" --include="*.nix" \
    --include="*.yaml" --include="*.yml" --include="*.nuspec" \
    --include="*.rs" \
    bucket/ Formula/ dist/ crates/*/Cargo.toml crates/js/src/ \
    docs/ 2>/dev/null | grep -v target | grep -v vendor | grep -v ".git" || true)

if [[ -n "$REMAINING" ]]; then
    echo "WARNING: Some references to $CURRENT remain:"
    echo "$REMAINING" | head -20
else
    echo "All references updated."
fi

echo ""
echo "Done! Version is now $NEXT"
echo ""
echo "Next steps:"
echo "  1. Review changes: git diff"
echo "  2. Update docs/CHANGELOG.md with release notes"
echo "  3. Commit and tag: git add -A && git commit -m 'Bump version to $NEXT'"
echo "  4. Create tag: git tag v$NEXT"
