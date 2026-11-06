#!/usr/bin/env bash
# Source-of-truth benchmark for graphrelay: an LLM agent answers ERPNext questions with our MCP
# server (`graphrelay` arm) or without any (`baseline`, the no-tool control), graded by a blind judge.
# Agents: claude-sonnet-5 and openrouter-glm-5-3-flash (see harness/providers.yaml).
#
#   benchmarks/dataset/run-sot.sh                       # both models, both arms, 1 repeat
#   benchmarks/dataset/run-sot.sh --repeats 3           # recommended for any real comparison
#   benchmarks/dataset/run-sot.sh --models claude-sonnet-5 --label pr-123
#   benchmarks/dataset/run-sot.sh --arms "graphrelay baseline graphify codegraph-colby"   # add competitor arms
#   benchmarks/dataset/run-sot.sh --structural --repeats 3   # the preregistered caller/impact slice (tasks/structural_tasks.yaml)
#   benchmarks/dataset/run-sot.sh --dry-run             # checks prerequisites, spends nothing
#   benchmarks/dataset/run-sot.sh --setup-only          # build + index + start daemon, then stop
#
# What it does: builds `cargo build --release`, indexes repos/erpnext into an ISOLATED $HOME (your
# real ~/.graphrelay is never touched), serves it on :3030 (the port harness/mcp/graphrelay.json uses),
# starts the LiteLLM proxy on :4000 for GLM (needs OPENROUTER_API_KEY, from the environment or the
# gitignored harness/.env.openrouter), runs every model x arm x repeat, grades, reports, and cleans up.
# Results: results/sot/<label>/<model>/rep<k>/ (results, transcripts, grades.json, RESULTS.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
REPO_NAME=erpnext; REPO_LABEL=ERPNext
PIN="1887825ce5c6fe2fbd5366ab52e4c73de24d23fa"   # keep in sync with setup/setup-erpnext.sh
MODELS="claude-sonnet-5 openrouter-glm-5-3-flash"
ARMS="graphrelay baseline"
REPEATS=1
LABEL="$(date +%Y-%m-%d)"
QUESTIONS="$(grep -v '^#' "$ROOT/tasks/sot_questions.txt" | tr -d ' \n')"
MAX_SPEND=8
BUILD=1; DRY=0; SETUP_ONLY=0
TASKS_ARGS=()
STRUCT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --models) MODELS="$2"; shift 2;;
    --arms) ARMS="$2"; shift 2;;
    --repeats) REPEATS="$2"; shift 2;;
    --label) LABEL="$2"; shift 2;;
    --questions) QUESTIONS="$2"; QUESTIONS_SET=1; shift 2;;
    --max-spend) MAX_SPEND="$2"; shift 2;;
    --structural) STRUCT=1; shift;;
    --repo) REPO_NAME="$2"; shift 2;;
    --no-build) BUILD=0; shift;;
    --dry-run) DRY=1; shift;;
    --setup-only) SETUP_ONLY=1; shift;;
    -h|--help) sed -n '2,20p' "$0"; exit 0;;
    *) echo "unknown option: $1" >&2; exit 2;;
  esac
done

case "$REPO_NAME" in
  erpnext) REPO_LABEL=ERPNext; TASKS_FILE="$ROOT/tasks/structural_tasks.yaml";;
  gitea)   REPO_LABEL=Gitea;   PIN="f35c67bd79b3bff9c28eb8171d53586cc800bd5c"; TASKS_FILE="$ROOT/tasks/structural_gitea_tasks.yaml";;
  *) echo "unknown --repo $REPO_NAME (erpnext|gitea)" >&2; exit 2;;
esac
ERPNEXT="$ROOT/repos/$REPO_NAME"   # the repo under test (name kept for the competitor/judge arms, which are ERPNext-only)
export BENCH_REPO="$REPO_NAME" BENCH_REPO_LABEL="$REPO_LABEL" BENCH_REPO_PIN="$PIN"
case "$REPO_NAME" in gitea) export BENCH_REPO_SLUG="go-gitea/gitea";; *) export BENCH_REPO_SLUG="frappe/erpnext";; esac
if [[ $STRUCT -eq 1 ]]; then
  TASKS_ARGS=(--tasks-file "$TASKS_FILE")
  ALL_IDS="$(python3 -c "import yaml,sys; print(','.join(t['id'] for t in yaml.safe_load(open(sys.argv[1]))['tasks']))" "$TASKS_FILE")"
  if [[ ${QUESTIONS_SET:-0} -eq 1 ]]; then
    # A subset of the tasks file (exploratory re-runs); every id must exist in it.
    for q in ${QUESTIONS//,/ }; do [[ ",$ALL_IDS," == *",$q,"* ]] || { echo "--questions: $q is not in $(basename "$TASKS_FILE")" >&2; exit 2; }; done
  else
    QUESTIONS="$ALL_IDS"
  fi
elif [[ "$REPO_NAME" != erpnext ]]; then echo "--repo $REPO_NAME needs --structural (the 30 questions are ERPNext-only)" >&2; exit 2; fi
if [[ "$REPO_NAME" != erpnext ]]; then for a in $ARMS; do [[ "$a" == graphrelay || "$a" == baseline ]] || { echo "arm $a is ERPNext-only" >&2; exit 2; }; done; fi

say() { printf '[sot] %s\n' "$*"; }
die() { printf '[sot] ERROR: %s\n' "$*" >&2; exit 1; }

# ---- prerequisites --------------------------------------------------------------------------
[[ -d "$ERPNEXT/.git" ]] || die "repos/$REPO_NAME missing (ERPNext: setup/setup-erpnext.sh)"
HEAD_SHA="$(git -C "$ERPNEXT" rev-parse HEAD)"
[[ "$HEAD_SHA" == "$PIN" ]] || die "$REPO_NAME is at $HEAD_SHA, expected the pinned $PIN"
command -v claude >/dev/null || die "the claude CLI is not on PATH (it is the agent runner)"
python3 -c "import yaml" 2>/dev/null || die "python3 needs PyYAML (pip install pyyaml)"
if [[ -f "$ROOT/harness/.env.openrouter" ]]; then set -a; . "$ROOT/harness/.env.openrouter"; set +a; fi

RUN_MODELS=""
for m in $MODELS; do
  if [[ "$m" == openrouter-* && -z "${OPENROUTER_API_KEY:-}" ]]; then
    say "SKIPPING $m: OPENROUTER_API_KEY is not set (export it, or put it in benchmarks/dataset/harness/.env.openrouter, which is gitignored)"
  else
    RUN_MODELS="$RUN_MODELS $m"
  fi
done
[[ -n "${RUN_MODELS// /}" ]] || die "no runnable model"

say "label=$LABEL models=[$RUN_MODELS ] arms=[$ARMS] repeats=$REPEATS questions=$QUESTIONS max_spend=\$$MAX_SPEND/arm-run"
if [[ $DRY -eq 1 ]]; then say "dry run: prerequisites OK, nothing was built, started or spent"; exit 0; fi

# ---- build the graphrelay under test ---------------------------------------------------------
CG="$REPO/target/release/gr"
if [[ $BUILD -eq 1 ]]; then say "cargo build --release"; (cd "$REPO" && cargo build --release -q); fi
[[ -x "$CG" ]] || die "$CG not found (run without --no-build)"
GIT_DESC="$(git -C "$REPO" rev-parse --short HEAD)$(git -C "$REPO" diff --quiet HEAD -- 2>/dev/null || echo '+dirty')"
say "graphrelay under test: release build of $GIT_DESC"

# ---- isolated daemon ------------------------------------------------------------------------
PORT_BUSY() { curl -s -o /dev/null -m 2 "http://localhost:$1/" 2>/dev/null; }
if PORT_BUSY 3030; then die "port 3030 is in use (a graphrelay daemon of yours?). Stop it first: gr stop"; fi
BENCH_HOME="$(mktemp -d "${TMPDIR:-/tmp}/graphrelay-sot-home.XXXXXX")"
PROXY_PID=""; GFY_PID=""; LAYA_PID=""; JUDGE_HOMES=""
cleanup() {
  for h in $JUDGE_HOMES; do HOME="$h" "$CG" stop >/dev/null 2>&1 || true; rm -rf "$h"; done
  [[ -n "$LAYA_PID" ]] && kill "$LAYA_PID" 2>/dev/null || true
  HOME="$BENCH_HOME" "$CG" stop >/dev/null 2>&1 || true
  [[ -n "$PROXY_PID" ]] && kill "$PROXY_PID" 2>/dev/null || true
  [[ -n "$GFY_PID" ]] && kill "$GFY_PID" 2>/dev/null || true
  [[ -n "$BENCH_HOME" && -d "$BENCH_HOME" ]] && rm -rf "$BENCH_HOME"
}
trap cleanup EXIT

say "indexing $REPO_NAME into an isolated HOME ($BENCH_HOME)"
(cd "$ERPNEXT" && HOME="$BENCH_HOME" "$CG" init >/dev/null 2>&1 || true)
(cd "$ERPNEXT" && HOME="$BENCH_HOME" "$CG" index | tail -1)
(cd "$ERPNEXT" && HOME="$BENCH_HOME" "$CG" start --port 3030 >/dev/null 2>&1)
for _ in $(seq 1 60); do
  HOME="$BENCH_HOME" "$CG" status 2>/dev/null | grep -E "^${REPO_NAME}\s" | grep -q fresh && break
  sleep 2
done
HOME="$BENCH_HOME" "$CG" status | sed -n '1,6p'
CODE="$(curl -s -o /dev/null -m 5 -w '%{http_code}' http://localhost:3030/$REPO_NAME/mcp || true)"
[[ "$CODE" == "405" || "$CODE" == "406" ]] || die "graphrelay MCP endpoint not up (HTTP $CODE)"

# ---- optional competitor arms ---------------------------------------------------------------
NEED=""
for a in $ARMS; do [[ "$a" == graphify || "$a" == codegraph-colby ]] && NEED="$NEED $a"; done
if [[ -n "$NEED" ]]; then
  say "preparing competitor arms:$NEED"
  "$ROOT/setup/setup-arms.sh" $NEED > "$BENCH_HOME/setup-arms.log" 2>&1 || { tail -20 "$BENCH_HOME/setup-arms.log" >&2; die "setup-arms.sh failed"; }
  if [[ " $NEED " == *" graphify "* ]]; then
    PORT_BUSY 8090 && die "port 8090 is in use"
    "$HOME/.local/share/uv/tools/graphifyy/bin/python" -m graphify.serve "$ROOT/repos/erpnext-graphify/graphify-out/graph.json" \
      --transport http --port 8090 > "$BENCH_HOME/graphify-http.log" 2>&1 &
    GFY_PID=$!
    for _ in $(seq 1 30); do PORT_BUSY 8090 && break; sleep 1; done
    PORT_BUSY 8090 || die "graphify MCP daemon did not start (see $BENCH_HOME/graphify-http.log)"
  fi
fi

# ---- optional judge arms (own graphrelay daemon + checkout each, judge enabled in its config) ---
judge_arm() {  # name port provider
  local name="$1" port="$2" provider="$3" repo="$ROOT/repos/erpnext-$1" h
  PORT_BUSY "$port" && die "port $port is in use"
  [[ -d "$repo/.git" ]] || git clone -q --local --no-checkout "$ERPNEXT" "$repo"
  git -C "$repo" checkout -q "$PIN"
  # mirror the main project's .graphrelay (ignore list + pre-built index.scip) so the ONLY difference
  # from the `graphrelay` arm is the judge
  mkdir -p "$repo/.graphrelay"
  cp "$ERPNEXT/.graphrelay/config.toml" "$repo/.graphrelay/config.toml"
  [[ -f "$ERPNEXT/.graphrelay/index.scip" ]] && cp "$ERPNEXT/.graphrelay/index.scip" "$repo/.graphrelay/index.scip"
  if [[ "$provider" == jev ]]; then
    [[ -n "${OPENROUTER_API_KEY:-}" ]] || die "graphrelay-jev needs OPENROUTER_API_KEY (Jev is routed through OpenRouter)"
    export GRAPHRELAY_JEV_API_KEY="$OPENROUTER_API_KEY"
    printf '\n[retrieval.jev]\nenabled = true\nbase_url = "https://openrouter.ai/api/v1/systemone"\nmodel = "typesafe/jev-1.13"\n' >> "$repo/.graphrelay/config.toml"
  else
    printf '\n[retrieval.laya]\nenabled = true\n' >> "$repo/.graphrelay/config.toml"
  fi
  h="$(mktemp -d "${TMPDIR:-/tmp}/graphrelay-sot-$name.XXXXXX")"; JUDGE_HOMES="$JUDGE_HOMES $h"
  say "indexing erpnext-$name ($provider judge) on :$port"
  (cd "$repo" && HOME="$h" "$CG" init >/dev/null 2>&1 || true)
  (cd "$repo" && HOME="$h" "$CG" index | tail -1)
  (cd "$repo" && HOME="$h" "$CG" start --port "$port" >/dev/null 2>&1)
  for _ in $(seq 1 60); do HOME="$h" "$CG" status 2>/dev/null | grep -E "^erpnext-$name\s" | grep -q fresh && break; sleep 2; done
  # a config that silently did not apply would make this arm a copy of `graphrelay`
  curl -s -m 5 "http://localhost:$port/erpnext-$name/api/status" | grep -q "\"judge_provider\":\"$provider\"" \
    || die "erpnext-$name: judge_provider is not $provider (config not applied / key missing)"
  say "erpnext-$name: judge_provider=$provider confirmed"
}
for a in $ARMS; do
  case "$a" in
    graphrelay-jev) judge_arm jev 3031 jev;;
    graphrelay-laya)
      LAYA_BIN="$ROOT/harness/laya-venv/bin/laya-serve"
      [[ -x "$LAYA_BIN" ]] || die "laya-serve missing: python3 -m venv harness/laya-venv && harness/laya-venv/bin/pip install 'laya[serve]'"
      PORT_BUSY 8000 && die "port 8000 is in use"
      say "starting laya-serve (english checkpoint) on :8000"
      LAYA_MODELS=english LAYA_HOST=127.0.0.1 LAYA_PORT=8000 "$LAYA_BIN" > "$BENCH_HOME/laya.log" 2>&1 &
      LAYA_PID=$!
      for _ in $(seq 1 180); do curl -s -m 2 http://127.0.0.1:8000/health >/dev/null && break; sleep 2; done
      curl -s -m 2 http://127.0.0.1:8000/health >/dev/null || die "laya-serve did not come up (see $BENCH_HOME/laya.log)"
      judge_arm laya 3032 laya;;
  esac
done

probe_ms() {  # port project -> wall ms of one search_code call
  local t0 t1
  t0=$(python3 -c 'import time;print(int(time.time()*1000))')
  curl -s -m 60 -o /dev/null -X POST "http://localhost:$1/$2/mcp" -H 'content-type: application/json' -H 'accept: application/json, text/event-stream' \
    -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"intent":"validate and merge general ledger entries before saving"}}}' || true
  t1=$(python3 -c 'import time;print(int(time.time()*1000))')
  echo $((t1 - t0))
}
for a in $ARMS; do
  case "$a" in
    graphrelay-jev|graphrelay-laya)
      n="${a#graphrelay-}"; port=3031; [[ "$n" == laya ]] && port=3032
      probe_ms 3030 erpnext >/dev/null; probe_ms "$port" "erpnext-$n" >/dev/null   # warm both
      base=$(probe_ms 3030 erpnext); judged=$(probe_ms "$port" "erpnext-$n")
      say "latency probe: plain ${base} ms, $a ${judged} ms"
      (( judged > base + 300 )) || say "WARNING: $a is not measurably slower than plain graphrelay — the judge may be failing silently (it falls back to base ranking on any error)";;
  esac
done

[[ $SETUP_ONLY -eq 1 ]] && { say "setup OK (daemons answer, judges confirmed); stopping"; exit 0; }

# ---- GLM proxy ------------------------------------------------------------------------------
if [[ "$RUN_MODELS" == *openrouter-* ]]; then
  export LITELLM_MASTER_KEY="${LITELLM_MASTER_KEY:-sk-local-benchmark-only}"
  LITELLM="$ROOT/harness/proxy/.venv/bin/litellm"
  [[ -x "$LITELLM" ]] || die "$LITELLM missing — see README 'Source-of-truth benchmark' for the proxy venv"
  say "starting the LiteLLM proxy on :4000"
  "$LITELLM" --config "$ROOT/harness/proxy/config.yaml" --port 4000 >"$BENCH_HOME/litellm.log" 2>&1 &
  PROXY_PID=$!
  for _ in $(seq 1 60); do curl -s -o /dev/null -m 2 http://localhost:4000/health/liveliness && break; sleep 2; done
fi

# ---- run, grade, report ---------------------------------------------------------------------
cd "$ROOT"
for model in $RUN_MODELS; do
  for rep in $(seq 1 "$REPEATS"); do
    out="results/sot/$LABEL/$model/rep$rep"
    mkdir -p "$out"
    printf 'graphrelay: release build of %s\nrepo: %s pin: %s\nquestions: %s\nmodel: %s\narms: %s\n' \
      "$GIT_DESC" "$REPO_NAME" "$PIN" "$QUESTIONS" "$model" "$ARMS" > "$out/RUN.txt"
    for arm in $ARMS; do
      say "$model rep$rep arm=$arm"
      python3 harness/run.py --arm "$arm" --task "$QUESTIONS" --provider "$model" \
        --max-spend "$MAX_SPEND" --out-dir "$out" ${TASKS_ARGS[@]+"${TASKS_ARGS[@]}"} > "$out/run-$arm.log" 2>&1 || say "  (run.py exited non-zero; see $out/run-$arm.log)"
    done
    for try in 1 2 3; do  # grade.py resumes where it stopped, so a judge timeout only costs a retry
      python3 harness/grade.py --results-dir "$out" ${TASKS_ARGS[@]+"${TASKS_ARGS[@]}"} > "$out/grade$try.log" 2>&1 && break || say "  grading attempt $try failed; see $out/grade$try.log"
    done
    python3 harness/report.py --results-dir "$out" ${TASKS_ARGS[@]+"${TASKS_ARGS[@]}"} > "$out/report.log" 2>&1 || true
    say "== $model rep$rep =="
    grep -A6 '^## Summary' "$out/RESULTS.md" 2>/dev/null | tail -4 || true
  done
done
python3 harness/sot_summary.py "results/sot/$LABEL" > /dev/null 2>&1 && say "summary: results/sot/$LABEL/SUMMARY.md" || true
say "done: results under benchmarks/dataset/results/sot/$LABEL/"
