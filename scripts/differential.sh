#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Differential suite: runs the same scenario with `node` and with `3va` and
# fails when the output differs. See tests/differential/README.md and
# docs/09-testing/07-compatibilidad-node.md (§7.2, §7.4 step 1).
#
# Each scenario is a directory under tests/differential/scenarios/ with a
# scenario.json manifest. client-server scenarios have a server.js and a
# client.js; single-process scenarios have a main.js. Server and client always
# run in separate OS processes (spec §7.3.2); the runner picks the port and
# passes it on the command line so it never appears in the compared output.
#
# Usage:
#   scripts/differential.sh [scenario-substring ...]
#
# Environment:
#   THREEVA_BIN            3va binary (default: target/release/3va, else `3va`)
#   NODE_BIN               node binary (default: `node`, must be Node 24)
#   DIFFERENTIAL_FILTER    comma-separated substrings; only matching scenarios run
#   DIFFERENTIAL_OUT       output directory (default: target/differential)
#   DIFFERENTIAL_REFRESH   1 to force `npm ci` even if node_modules exists
#   DIFFERENTIAL_KEEP      1 to keep previous per-run result directories
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="$ROOT/tests/differential"
NODE_BIN="${NODE_BIN:-node}"
OUT_DIR="${DIFFERENTIAL_OUT:-$ROOT/target/differential}"
WORK_DIR="$OUT_DIR/work"
RESULTS_DIR="$OUT_DIR/results"
KNOWN_FILE="$SRC_DIR/known-differences.json"
ALL_COMBOS="node/node 3va/node node/3va 3va/3va"

if [ -z "${THREEVA_BIN:-}" ]; then
  if [ -x "$ROOT/target/release/3va" ]; then THREEVA_BIN="$ROOT/target/release/3va"; else THREEVA_BIN=3va; fi
fi

FILTER="${DIFFERENTIAL_FILTER:-}"
if [ "$#" -gt 0 ]; then FILTER="$(printf '%s,' "$@")"; fi

C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_DIM=$'\033[2m'; C_OFF=$'\033[0m'
if [ ! -t 1 ]; then C_RED=; C_GREEN=; C_YELLOW=; C_DIM=; C_OFF=; fi

PASS=0; KNOWN=0; DIFF=0; STALE=0
declare -a DIFF_LINES=() STALE_LINES=() KNOWN_LINES=()

say() { printf '%s\n' "$*"; }

free_port() {
  "$NODE_BIN" -e 'const s=require("net").createServer();s.listen(0,"127.0.0.1",()=>{process.stdout.write(String(s.address().port));s.close();});'
}

# Print one field of a scenario JSON file (arrays space-separated).
scen_field() {
  "$NODE_BIN" -e '
    const fs=require("fs");
    const s=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));
    let v=s[process.argv[2]];
    if(v===undefined||v===null) v="";
    if(Array.isArray(v)) v=v.join(" ");
    process.stdout.write(String(v));
  ' "$1" "$2"
}

known_reason() { # <scenario> <combo>
  [ -f "$KNOWN_FILE" ] || { printf ''; return; }
  "$NODE_BIN" -e '
    const fs=require("fs");
    let k={}; try{k=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));}catch(e){}
    const s=k[process.argv[2]]||{};
    const v=s[process.argv[3]]!==undefined?s[process.argv[3]]:(s["*"]!==undefined?s["*"]:"");
    process.stdout.write(String(v));
  ' "$KNOWN_FILE" "$1" "$2"
}

known_keys() {
  [ -f "$KNOWN_FILE" ] || return 0
  "$NODE_BIN" -e '
    const fs=require("fs");
    let k={}; try{k=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));}catch(e){}
    for(const [s,combos] of Object.entries(k))
      for(const c of Object.keys(combos)) console.log(s+"\t"+c);
  ' "$KNOWN_FILE"
}

# Launch one side of a combo in the current directory, bounded by
# LAUNCH_TIMEOUT seconds. Reads SERVER_PERMS / CLIENT_PERMS / extra_env globals.
#   launch <runtime> <role: server|client|single> <script> [args...]
launch() {
  local rt="$1" role="$2" script="$3"; shift 3
  local -a perms=()
  if [ "$role" = client ]; then
    read -r -a perms <<<"$CLIENT_PERMS"
  else
    read -r -a perms <<<"$SERVER_PERMS"
  fi
  local -a cmd=()
  if [ "$rt" = node ]; then
    cmd=("$NODE_BIN" "$script" "$@")
  else
    cmd=("$THREEVA_BIN" run "${perms[@]}" "$script" -- "$@")
  fi
  if [ -n "$extra_env" ]; then
    timeout --signal=KILL "$LAUNCH_TIMEOUT" env $extra_env "${cmd[@]}"
  else
    timeout --signal=KILL "$LAUNCH_TIMEOUT" "${cmd[@]}"
  fi
}

# ── Prepare the working tree ─────────────────────────────────────────────────
mkdir -p "$OUT_DIR"
[ "${DIFFERENTIAL_KEEP:-}" = "1" ] || rm -rf "$RESULTS_DIR"
mkdir -p "$RESULTS_DIR" "$WORK_DIR"
rm -rf "$WORK_DIR/scenarios.new"
cp -r "$SRC_DIR/scenarios" "$WORK_DIR/scenarios.new"
rm -rf "$WORK_DIR/scenarios"
mv "$WORK_DIR/scenarios.new" "$WORK_DIR/scenarios"
rm -rf "$WORK_DIR/lib"
cp -r "$SRC_DIR/lib" "$WORK_DIR/lib"
cp "$SRC_DIR/package.json" "$SRC_DIR/package-lock.json" "$WORK_DIR/"

if [ ! -d "$WORK_DIR/node_modules" ] || [ "${DIFFERENTIAL_REFRESH:-}" = "1" ]; then
  say "${C_DIM}Installing pinned npm dependencies (npm ci)…${C_OFF}"
  ( cd "$WORK_DIR" && npm ci --no-audit --no-fund >/dev/null 2>&1 )
fi

# A private CA for the tls-* scenarios; NODE_EXTRA_CA_CERTS points at it.
CERTS_DIR="$WORK_DIR/certs"
if [ ! -f "$CERTS_DIR/ca.pem" ]; then
  mkdir -p "$CERTS_DIR"
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$CERTS_DIR/ca-key.pem" -out "$CERTS_DIR/ca.pem" \
    -days 2 -subj "/CN=3va-differential-ca" >/dev/null 2>&1 || true
  openssl req -newkey rsa:2048 -nodes -keyout "$CERTS_DIR/server-key.pem" -out "$CERTS_DIR/server.csr" \
    -subj "/CN=localhost" >/dev/null 2>&1 || true
  printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > "$CERTS_DIR/san.ext"
  openssl x509 -req -in "$CERTS_DIR/server.csr" -CA "$CERTS_DIR/ca.pem" -CAkey "$CERTS_DIR/ca-key.pem" \
    -CAcreateserial -out "$CERTS_DIR/server-cert.pem" -days 2 -extfile "$CERTS_DIR/san.ext" >/dev/null 2>&1 || true
fi
cp "$CERTS_DIR"/*.pem "$WORK_DIR/" 2>/dev/null || true

: > "$OUT_DIR/known-keys.txt"
known_keys >> "$OUT_DIR/known-keys.txt" || true

scenario_matches() {
  [ -z "$FILTER" ] && return 0
  local f
  local IFS=','
  for f in $FILTER; do
    [ -n "$f" ] && case "$1" in *"$f"*) return 0 ;; esac
  done
  return 1
}

# ── Run ──────────────────────────────────────────────────────────────────────
mapfile -t SCENARIOS < <(find "$SRC_DIR/scenarios" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort)

say "Differential suite"
say "  3va:  ${THREEVA_BIN} $("$THREEVA_BIN" --version 2>/dev/null | head -1)"
say "  node: ${NODE_BIN} $("$NODE_BIN" --version)"
say "  work: $WORK_DIR"
say "──────────────────────────────────────────────────────────"

cd "$WORK_DIR"

for name in "${SCENARIOS[@]}"; do
  manifest="$SRC_DIR/scenarios/$name/scenario.json"
  [ -f "$manifest" ] || continue
  scenario_matches "$name" || continue

  kind="$(scen_field "$manifest" kind)"; [ -n "$kind" ] || kind="client-server"
  timeout_s="$(scen_field "$manifest" timeout_s)"; [ -n "$timeout_s" ] || timeout_s=30
  combos="$(scen_field "$manifest" combos)"
  if [ -z "$combos" ]; then
    if [ "$kind" = single ]; then combos="node 3va"; else combos="$ALL_COMBOS"; fi
  fi
  SERVER_PERMS="$(scen_field "$manifest" server_perms)"; [ -n "$SERVER_PERMS" ] || SERVER_PERMS="--allow-net=127.0.0.1 --allow-read=."
  CLIENT_PERMS="$(scen_field "$manifest" client_perms)"; [ -n "$CLIENT_PERMS" ] || CLIENT_PERMS="--allow-net=127.0.0.1 --allow-read=."
  extra_env="$(scen_field "$manifest" env)"
  single_entry="$(scen_field "$manifest" main)"; [ -n "$single_entry" ] || single_entry="main.js"

  outdir="$RESULTS_DIR/$name"; mkdir -p "$outdir"
  baseline_combo="${combos%% *}"
  scenario_ok=1

  for combo in $combos; do
    tag="${combo//\//-}"
    slog="$outdir/$tag.server.out"; serr="$outdir/$tag.server.err"
    clog="$outdir/$tag.client.out"; cerr="$outdir/$tag.client.err"
    : > "$slog"; : > "$serr"; : > "$clog"; : > "$cerr"
    combo_error=0

    if [ "$kind" = single ]; then
      LAUNCH_TIMEOUT="$timeout_s"
      set +e
      launch "$combo" single scenarios/"$name"/"$single_entry" >"$clog" 2>"$cerr"
      cli_status=$?
      set -e
      case "$cli_status" in 124|137) combo_error=1 ;; esac
    else
      IFS='/' read -r srv_rt cli_rt <<<"$combo"
      port="$(free_port)"
      LAUNCH_TIMEOUT="$((timeout_s + 5))"
      launch "$srv_rt" server scenarios/"$name"/server.js "$port" >"$slog" 2>"$serr" &
      srv_pid=$!
      ready=0
      for _ in $(seq 1 $((timeout_s * 10))); do
        if grep -q '^READY$' "$slog" 2>/dev/null; then ready=1; break; fi
        kill -0 "$srv_pid" 2>/dev/null || break
        sleep 0.1
      done
      if [ "$ready" = 1 ]; then
        LAUNCH_TIMEOUT="$timeout_s"
        set +e
        launch "$cli_rt" client scenarios/"$name"/client.js "$port" >"$clog" 2>"$cerr"
        cli_status=$?
        set -e
      else
        printf '<<SERVER-NOT-READY>>\n' > "$clog"
        cli_status=99
        combo_error=1
      fi
      kill -TERM "$srv_pid" 2>/dev/null || true
      for _ in $(seq 1 20); do kill -0 "$srv_pid" 2>/dev/null || break; sleep 0.1; done
      kill -KILL "$srv_pid" 2>/dev/null || true
      wait "$srv_pid" 2>/dev/null || true
      case "$cli_status" in 124|137) combo_error=1 ;; esac
    fi
    printf '%s' "$cli_status" > "$outdir/$tag.status"

    if [ "$combo" = "$baseline_combo" ]; then
      cp "$clog" "$outdir/baseline.client.out"
      printf '%s' "$cli_status" > "$outdir/baseline.status"
      if [ "$combo_error" = 1 ]; then
        DIFF=$((DIFF + 1))
        DIFF_LINES+=("$name [$combo] baseline did not run (status=$cli_status; see $outdir/$tag.server.err)")
        scenario_ok=0
      fi
      continue
    fi

    base_out="$outdir/baseline.client.out"
    base_status="$(cat "$outdir/baseline.status")"
    reason="$(known_reason "$name" "$combo")"
    if cmp -s "$clog" "$base_out" && [ "$cli_status" = "$base_status" ] && [ "$combo_error" = 0 ]; then
      if [ -n "$reason" ]; then
        STALE=$((STALE + 1))
        STALE_LINES+=("$name [$combo]: known difference no longer reproduces — remove it from known-differences.json")
        scenario_ok=0
      else
        PASS=$((PASS + 1))
      fi
    elif [ -n "$reason" ]; then
      KNOWN=$((KNOWN + 1))
      KNOWN_LINES+=("$name [$combo]: $reason")
    else
      DIFF=$((DIFF + 1))
      if [ "$combo_error" = 1 ]; then
        DIFF_LINES+=("$name [$combo] did not run (status=$cli_status; see $outdir/$tag.server.err)")
      else
        DIFF_LINES+=("$name [$combo]  →  $outdir/$tag.client.out vs $outdir/baseline.client.out")
      fi
      scenario_ok=0
    fi
  done

  if [ "$scenario_ok" = 1 ]; then say "  ${C_GREEN}✓${C_OFF} $name"; else say "  ${C_RED}✗${C_OFF} $name"; fi
done

# Stale known keys: authored for a scenario/combo we do not run.
if [ -s "$OUT_DIR/known-keys.txt" ]; then
  while IFS=$'\t' read -r ks kc; do
    [ -n "$ks" ] || continue
    if [ ! -f "$SRC_DIR/scenarios/$ks/scenario.json" ]; then
      STALE=$((STALE + 1)); STALE_LINES+=("$ks [$kc]: scenario does not exist"); continue
    fi
    m="$SRC_DIR/scenarios/$ks/scenario.json"
    k="$(scen_field "$m" kind)"; [ -n "$k" ] || k="client-server"
    cs="$(scen_field "$m" combos)"
    [ -z "$cs" ] && { if [ "$k" = single ]; then cs="node 3va"; else cs="$ALL_COMBOS"; fi; }
    found=0
    for c in $cs; do [ "$c" = "$kc" ] && found=1; done
    [ "$found" = 1 ] || { STALE=$((STALE + 1)); STALE_LINES+=("$ks [$kc]: combo is not run by that scenario"); }
  done < "$OUT_DIR/known-keys.txt"
fi

say "──────────────────────────────────────────────────────────"
if [ "${#KNOWN_LINES[@]}" -gt 0 ]; then
  say "${C_YELLOW}Known differences (accepted, listed):${C_OFF}"
  for l in "${KNOWN_LINES[@]}"; do say "  ${C_YELLOW}~${C_OFF} $l"; done
fi
if [ "${#DIFF_LINES[@]}" -gt 0 ]; then
  say "${C_RED}Unexpected differences:${C_OFF}"
  for l in "${DIFF_LINES[@]}"; do say "  ${C_RED}✗${C_OFF} $l"; done
fi
if [ "${#STALE_LINES[@]}" -gt 0 ]; then
  say "${C_RED}Stale known-difference markers:${C_OFF}"
  for l in "${STALE_LINES[@]}"; do say "  ${C_RED}!${C_OFF} $l"; done
fi
say "matched=$PASS  known=$KNOWN  unexpected=$DIFF  stale=$STALE"

if [ "$DIFF" -gt 0 ] || [ "$STALE" -gt 0 ]; then exit 1; fi
say "${C_GREEN}Differential suite OK${C_OFF}"
