#!/usr/bin/env bash
# Prepare the two competitor arms (both optional; run-sot.sh calls this when they are requested).
#   graphify:        AST-only graph (`graphify update`, API keys unset: zero credits) of its own byte-identical
#                    checkout repos/erpnext-graphify, so the 100 MB graphify-out/ never sits inside repos/erpnext
#                    where the baseline and graphrelay agents could Grep it and graphrelay would index it
#   codegraph-colby: github.com/colbymchenry/codegraph, installed at ~/.colby-codegraph (never on PATH,
#                    it collides with our binary name), indexing its own byte-identical checkout
#                    repos/erpnext-colby so its own `.codegraph/` index dir stays separate from ours.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ERPNEXT="$ROOT/repos/erpnext"; COLBY_REPO="$ROOT/repos/erpnext-colby"; GRAPHIFY_REPO="$ROOT/repos/erpnext-graphify"
COLBY_BIN="$HOME/.colby-codegraph/bin/codegraph"   # colby's own binary name (third-party, not renamed)
WANT="${*:-graphify codegraph-colby}"
[[ -d "$ERPNEXT/.git" ]] || { echo "Run setup/setup-erpnext.sh first" >&2; exit 1; }
PIN="$(git -C "$ERPNEXT" rev-parse HEAD)"

if [[ " $WANT " == *" graphify "* ]]; then
  command -v graphify >/dev/null || { command -v uv >/dev/null && uv tool install "graphifyy[mcp]" || pipx install "graphifyy[mcp]"; }
  if [[ ! -d "$GRAPHIFY_REPO/.git" ]]; then
    git clone -q --local --no-checkout "$ERPNEXT" "$GRAPHIFY_REPO"
  fi
  git -C "$GRAPHIFY_REPO" checkout -q "$PIN"
  [[ "$(git -C "$GRAPHIFY_REPO" rev-parse HEAD)" == "$PIN" ]] || { echo "graphify checkout not at pin" >&2; exit 1; }
  if [[ -f "$GRAPHIFY_REPO/graphify-out/graph.json" ]]; then echo "graphify graph already built"; else
    echo "building graphify graph (AST-only)"
    (unset OPENAI_API_KEY ANTHROPIC_API_KEY MOONSHOT_API_KEY GEMINI_API_KEY OPENROUTER_API_KEY; time graphify update "$GRAPHIFY_REPO")
  fi
  ls -lh "$GRAPHIFY_REPO/graphify-out/graph.json"
fi

if [[ " $WANT " == *" codegraph-colby "* ]]; then
  [[ -x "$COLBY_BIN" ]] || { echo "colby codegraph is not installed at $COLBY_BIN (install it from github.com/colbymchenry/codegraph with an isolated prefix)" >&2; exit 1; }
  if [[ ! -d "$COLBY_REPO/.git" ]]; then
    git clone -q --local --no-checkout "$ERPNEXT" "$COLBY_REPO"
  fi
  git -C "$COLBY_REPO" checkout -q "$PIN"
  [[ "$(git -C "$COLBY_REPO" rev-parse HEAD)" == "$PIN" ]] || { echo "colby checkout not at pin" >&2; exit 1; }
  if [[ -d "$COLBY_REPO/.codegraph" ]]; then echo "colby index already built"; else
    echo "indexing erpnext-colby"
    (cd "$COLBY_REPO" && time "$COLBY_BIN" init -y)
  fi
  (cd "$COLBY_REPO" && "$COLBY_BIN" status | head -12)
fi
