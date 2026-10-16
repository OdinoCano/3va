#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Downloads the official Node.js test suite (nodejs/node) into tests/node
# (gitignored), sparse-checked-out to test/common, test/fixtures and
# test/parallel. The tag is pinned (override with NODE_TESTS_REF) so the
# compatibility baseline is comparable over time; bump it deliberately.
# Sister script of scripts/setup-test262.sh.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

DIR="tests/node"
REF="${NODE_TESTS_REF:-v24.22.0}"

if [ -d "$DIR/test/parallel" ]; then
  echo "node tests already present at $DIR"
  exit 0
fi

echo "Fetching nodejs/node ${REF} (sparse: test/common, test/fixtures, test/parallel)..."
rm -rf "$DIR"
mkdir -p "$DIR"
git -C "$DIR" init -q
git -C "$DIR" remote add origin https://github.com/nodejs/node.git
git -C "$DIR" config core.sparseCheckout true
printf 'test/common/\ntest/fixtures/\ntest/parallel/\n' > "$DIR/.git/info/sparse-checkout"
git -C "$DIR" fetch -q --depth=1 origin "refs/tags/${REF}"
git -C "$DIR" checkout -q FETCH_HEAD

echo "$REF" > "$DIR/REF"
git -C "$DIR" rev-parse HEAD > "$DIR/COMMIT"
echo "node tests ready at $DIR ($(git -C "$DIR" rev-parse --short HEAD))"
