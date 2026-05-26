#!/usr/bin/env bash
# dev-setup.sh — Run once after cloning to prepare your dev environment.
# Installs git hooks and verifies required tools are present.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOOKS_SRC="$REPO_ROOT/.cargo-husky/hooks"
HOOKS_DST="$REPO_ROOT/.git/hooks"

echo "=== 3va dev setup ==="

# ── 1. Verify required tools ─────────────────────────────────────────────────
need() {
    if ! command -v "$1" &>/dev/null; then
        echo "ERROR: '$1' not found. $2"
        exit 1
    fi
}

need cargo    "Install Rust via https://rustup.rs"
need git      "Install git"

# Optional but recommended
for tool in cargo-deny cargo-audit; do
    if ! cargo "$tool" --version &>/dev/null 2>&1; then
        echo "INFO: '$tool' not installed. Run: cargo install $tool --locked"
    fi
done

# ── 2. Install git hooks from .cargo-husky/hooks/ ────────────────────────────
# cargo-husky installs these automatically on `cargo test`, but installing
# them here ensures they are active before the first commit.
if [ ! -d "$HOOKS_SRC" ]; then
    echo "ERROR: .cargo-husky/hooks/ not found. Is the repo intact?"
    exit 1
fi

mkdir -p "$HOOKS_DST"
for hook in "$HOOKS_SRC"/*; do
    name="$(basename "$hook")"
    install -m 755 "$hook" "$HOOKS_DST/$name"
    echo "  installed .git/hooks/$name"
done

# ── 3. Confirm toolchain ─────────────────────────────────────────────────────
rustup component add rustfmt clippy 2>/dev/null || true

echo ""
echo "Setup complete. Every commit will run: cargo fmt --check + cargo clippy"
echo "Every push will run: cargo test"
echo ""
echo "The CI pipeline runs the same checks and MUST pass before any PR is merged."
