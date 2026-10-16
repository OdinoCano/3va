#!/usr/bin/env bash
# dev-setup.sh — Run once after cloning to prepare your dev environment.
# Installs git hooks, checks the required tools and reports the optional ones
# each verification layer needs (docs/09-testing/07-compatibilidad-node.md).
#
#   ./scripts/dev-setup.sh                  # hooks + tool check
#   ./scripts/dev-setup.sh --with-test262   # + ECMAScript suite (tests/test262)
#   ./scripts/dev-setup.sh --with-node-tests # + Node's own test suite (tests/node)
#   ./scripts/dev-setup.sh --all            # both suites
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOOKS_SRC="$REPO_ROOT/.cargo-husky/hooks"
HOOKS_DST="$REPO_ROOT/.git/hooks"

WITH_TEST262=0
WITH_NODE_TESTS=0
for arg in "$@"; do
    case "$arg" in
        --with-test262) WITH_TEST262=1 ;;
        --with-node-tests) WITH_NODE_TESTS=1 ;;
        --all) WITH_TEST262=1; WITH_NODE_TESTS=1 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

echo "=== 3va dev setup ==="

# ── 1. Required tools ────────────────────────────────────────────────────────
need() {
    if ! command -v "$1" &>/dev/null; then
        echo "ERROR: '$1' not found. $2"
        exit 1
    fi
}
need cargo "Install Rust via https://rustup.rs"
need git   "Install git"

# ── 2. Optional tools, grouped by what needs them ────────────────────────────
missing=0
want() { # want <command> <what it is for> <how to install>
    if ! command -v "$1" &>/dev/null; then
        echo "  - $1: needed for $2. Install: $3"
        missing=1
    fi
}
want_cargo() { # cargo subcommands
    if ! cargo "$1" --version &>/dev/null; then
        echo "  - cargo-$1: needed for $2. Install: cargo install cargo-$1 --locked"
        missing=1
    fi
}
echo "Optional tools:"
# Node compatibility checks compare 3va against the real node (same major as tests/node).
want node   "the differential suite and Node's test suite (node 24)" "https://nodejs.org or nvm install 24"
want npm    "scenario dependencies (tests/differential, compat-check)" "comes with node"
want docker "the client scenario release gate and Postgres tests" "https://docs.docker.com/engine/install/"
want jq     "perf-check and the result scripts" "apt install jq"
want curl   "perf-check and setup scripts" "apt install curl"
want hyperfine "scripts/perf-check.sh (1.x; 2.0 changed its JSON)" "cargo install hyperfine@1.20.0 --locked"
want oha    "scripts/perf-check.sh" "cargo install oha --locked"
want gitleaks "the pre-commit/pre-push secret scan" "https://github.com/gitleaks/gitleaks"
want_cargo deny  "the pre-push hook and CI (advisories, licenses)"
want_cargo audit "security scans"
want_cargo vet   "CI supply-chain audits (run: cargo vet --locked)"
want_cargo fuzz  "the pre-push fuzz smoke test (needs nightly)"
if command -v node &>/dev/null; then
    major="$(node -p 'process.versions.node.split(".")[0]')"
    [ "$major" = "24" ] || echo "  - node $major found; the differential suite and tests/node are pinned to Node 24"
fi
[ "$missing" = 0 ] && echo "  all present"

# ── 3. Git hooks from .cargo-husky/hooks/ ────────────────────────────────────
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

# ── 4. Conformance suites (large downloads, gitignored, pinned) ──────────────
# test262: ECMAScript (V8). tests/node: Node's own test/parallel, the measure of
# the Node API layer 3va reimplements. Neither is needed to build or run `cargo test`.
[ "$WITH_TEST262" = 1 ] && "$REPO_ROOT/scripts/setup-test262.sh"
[ "$WITH_NODE_TESTS" = 1 ] && "$REPO_ROOT/scripts/setup-node-tests.sh"

# ── 5. Toolchain components ──────────────────────────────────────────────────
rustup component add rustfmt clippy 2>/dev/null || true

cat <<'EOF'

Setup complete.
  Every commit runs: cargo fmt --check, cargo clippy, gitleaks
  Every push runs:   cargo test, cargo deny, fuzz smoke, gitleaks
  CI runs the same checks plus cargo vet and MUST pass before a PR is merged.

Before changing the Node API layer (http, stream, Buffer, fs, net, fetch, module
loading), read docs/09-testing/07-compatibilidad-node.md: changes need a
differential test against node and the whole API matrix, not just one case.
  scripts/differential.sh       node vs 3va, same scenarios, outputs compared
  scripts/test262.sh            ECMAScript conformance (needs --with-test262)
  cargo test -p vvva_test --test node_tests -- --ignored   (needs --with-node-tests)
  scripts/client-scenario.sh    release gate in debian:bookworm-slim (needs docker)

Memory: building and testing 3va is heavy. The git hooks already cap themselves
(scripts/hook-limits.sh); run manual cargo/test commands the same way, e.g.
  systemd-run --user --scope -q -p MemoryMax=14G -p TasksMax=4096 cargo test -j 4
TasksMax (threads count too) stops a test that spawns processes in a loop from
filling the machine, as a test-child-process-* case did with the Node suite.
EOF
