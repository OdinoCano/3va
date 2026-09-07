#!/usr/bin/env bash
# Downloads the official tc39/test262 suite into tests/test262 (gitignored).
# Not a submodule: test262 is ~70k files with a large history, and we only
# ever need the current tree, so a shallow clone keeps the main repo clean.
set -euo pipefail

DIR="tests/test262"

if [ -d "$DIR/harness" ]; then
  echo "test262 already present at $DIR"
  exit 0
fi

echo "Cloning tc39/test262 (shallow)..."
rm -rf "$DIR"
mkdir -p tests
git clone --depth=1 https://github.com/tc39/test262.git "$DIR"

echo "test262 ready at $DIR"
