#!/usr/bin/env bash
# Fills the <!--BENCH:install-*--> markers in README.md's Comparison table
# from bench/run.sh's "Install (warm...)" table. Run after bench/run.sh,
# piping its output in:
#
#   bash bench/run.sh | tee /tmp/bench.txt
#   bash bench/sync-readme.sh /tmp/bench.txt
#
# Only touches text between the markers — everything else in README.md is
# left alone. A tool with no row in the bench output (not on PATH) leaves
# its marker untouched.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

BENCH_OUTPUT="${1:?usage: sync-readme.sh <bench-output-file>}"

python3 - "$BENCH_OUTPUT" <<'EOF'
import re, sys, pathlib

bench = pathlib.Path(sys.argv[1]).read_text()
readme = pathlib.Path("README.md")
text = readme.read_text()

section_match = re.search(r"## Install \(warm.*?\n\n(.*?)\n\n", bench, re.S)
section = section_match.group(1) if section_match else ""

values = {}
for line in section.splitlines():
    m = re.match(r"\|\s*(\S+)\s*\|\s*([\d.]+ ms)\s*\|", line)
    if m:
        values[m.group(1)] = m.group(2)

mapping = {"npm": "install-npm", "bun": "install-bun", "3va": "install-3va"}
for key, marker in mapping.items():
    if key not in values:
        continue
    text, n = re.subn(
        rf"(<!--BENCH:{marker}-->).*?(<!--/BENCH:{marker}-->)",
        lambda m: m.group(1) + values[key] + m.group(2),
        text,
    )
    if n == 0:
        print(f"warning: marker BENCH:{marker} not found in README.md", file=sys.stderr)

readme.write_text(text)
EOF
