#!/usr/bin/env bash
# Run the test262 gate the way CI does (TZ=UTC, full failure list) and diff it
# against the previous run, so a regression hidden in module-code/ (which the
# gate counter ignores) still shows up.
#
#   scripts/test262.sh            # run, write target/test262-failures.txt, diff vs the last run
#   scripts/test262.sh <subdir>   # narrower slice, e.g. language/expressions
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
[ -d tests/test262/harness ] || scripts/setup-test262.sh
OUT=target/test262-failures.txt
PREV=target/test262-failures.prev.txt
[ -f "$OUT" ] && cp "$OUT" "$PREV"
TZ=UTC TEST262_FAILURES_OUT="$OUT" cargo test --release -p vvva_test --test test262 -- --ignored --nocapture "$@" || true
echo "failures: $(wc -l < "$OUT")  (list: $OUT)"
if [ -f "$PREV" ]; then
  echo "--- newly failing:"; comm -13 "$PREV" "$OUT" | sed 's/^/  /'
  echo "--- newly passing: $(comm -23 "$PREV" "$OUT" | wc -l)"
fi
