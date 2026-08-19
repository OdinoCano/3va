#!/usr/bin/env bash
# Fills the <!--BENCH:install-*-->, <!--BENCH:http-*--> and <!--BENCH:mem-*-->
# markers in README.md's Comparison table from bench/run.sh's "Install (warm
# ...)" and "HTTP throughput ... and memory" tables. Run after bench/run.sh,
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

def fill(marker_value_map, text):
    for marker, value in marker_value_map.items():
        text, n = re.subn(
            rf"(<!--BENCH:{marker}-->).*?(<!--/BENCH:{marker}-->)",
            lambda m: m.group(1) + value + m.group(2),
            text,
        )
        if n == 0:
            print(f"warning: marker BENCH:{marker} not found in README.md", file=sys.stderr)
    return text

values = {}
section_match = re.search(r"## Install \(warm.*?\n\n(.*?)\n\n", bench, re.S)
section = section_match.group(1) if section_match else ""
for line in section.splitlines():
    m = re.match(r"\|\s*(\S+)\s*\|\s*([\d.]+ ms)\s*\|", line)
    if m:
        values[m.group(1)] = m.group(2)
install_map = {f"install-{k}": v for k, v in values.items()}
text = fill(install_map, text)

http_values = {}
section_match = re.search(
    r"## HTTP throughput \(100k requests, 1,000 concurrent\) and memory\n\n(.*?)\n\n",
    bench,
    re.S,
)
section = section_match.group(1) if section_match else ""
for line in section.splitlines():
    m = re.match(
        r"\|\s*(\S+)\s*\|\s*([\d,]+)\s*\|\s*[\d.]+%\s*\|\s*([\d.]+ MB)\s*\|\s*([\d.]+ MB)\s*\|",
        line,
    )
    if m:
        name, rps, idle, loaded = m.group(1), m.group(2), m.group(3), m.group(4)
        idle_val = idle.replace(" MB", "")
        http_values[name] = {"http": f"{rps} req/s", "mem": f"{idle_val} → {loaded}"}

for kind, suffix in (("http", "http"), ("mem", "mem")):
    text = fill(
        {f"{suffix}-{name}": vals[kind] for name, vals in http_values.items()},
        text,
    )

readme.write_text(text)
EOF