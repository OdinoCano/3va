#!/usr/bin/env bash
# Technical-debt snapshot: deliberate shortcuts and the usual smells, one count each.
cd "$(dirname "${BASH_SOURCE[0]}")/.."
n() { grep -rnE "$1" "${@:2}" 2>/dev/null | wc -l; }
echo "ponytail: shortcuts        $(n 'ponytail:' crates --include=*.rs --include=*.js)"
echo "TODO/FIXME/XXX/HACK        $(n 'TODO|FIXME|XXX|HACK' crates --include=*.rs)"
echo ".unwrap() in src           $(n '\.unwrap\(\)' crates/*/src --include=*.rs)"
echo "unsafe in src              $(n 'unsafe ' crates/*/src --include=*.rs)"
echo "#[allow(..)] in src        $(n '#\[allow\(' crates/*/src --include=*.rs)"
echo "#[ignore]d tests           $(n '#\[ignore' crates --include=*.rs)"
echo "debug eprintln! leftovers  $(n 'eprintln!\("\[(TLA-CF|MMARK|ZOD-CORE|DEVALUE|CLACK|DEBUG)' crates --include=*.rs)"
echo "package-name special cases $(n 'source\.contains\("(assertWranglerVersion|asciiAtext|@clack|@exodus)' crates --include=*.rs)"
echo "--- largest source files"
find crates -path '*/src/*' -name '*.rs' -print0 | xargs -0 wc -l | sort -rn | sed -n 2,6p
echo "--- ponytail: ledger"
grep -rn 'ponytail:' crates --include=*.rs --include=*.js | cut -c1-140
