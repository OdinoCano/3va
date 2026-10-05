#!/usr/bin/env bash
# Downloads the official tc39/test262 suite into tests/test262 (gitignored).
# Not a submodule: test262 is ~70k files with a large history, and we only
# ever need one tree, so a shallow fetch keeps the main repo clean. The commit
# is pinned (override with TEST262_REF) so conformance numbers are comparable
# between machines and over time; bump it deliberately.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

DIR="tests/test262"
REF="${TEST262_REF:-419d3e0a2273ba01a3bfcbec423f2801425b8e93}"

if [ -d "$DIR/harness" ]; then
  echo "test262 already present at $DIR"
  exit 0
fi

echo "Fetching tc39/test262 @ ${REF:0:10} (shallow)..."
rm -rf "$DIR"
mkdir -p "$DIR"
git -C "$DIR" init -q
git -C "$DIR" fetch -q --depth=1 https://github.com/tc39/test262.git "$REF"
git -C "$DIR" checkout -q FETCH_HEAD

echo "test262 ready at $DIR"
