#!/usr/bin/env bash
# Clone the benchmark target repo (frappe/erpnext) at the pinned commit.
#
# 2026-09-27: re-pinned from 95212e5738426203dd8284473c5cd8e9fa5dc0d8 to
# 1887825ce5c6fe2fbd5366ab52e4c73de24d23fa — found that 16 of
# llm_tasks_erpnext_90.yaml's 90 tasks (46% of the strict-scorer recall
# misses being investigated that day) referenced functions that didn't exist
# at the old pin at all: their `_source_sha` commits were dated *after* it
# (2026-07-09 through 2026-07-17), confirmed via `git merge-base
# --is-ancestor`. The new pin is the chronologically latest of all 90 tasks'
# `_source_sha` values, and all other 89 are confirmed ancestors of it — the
# minimal commit that makes every task's target function reachable, without
# pulling in more history/drift than necessary. Real effect: erpnext strict-
# scorer pass@5 23/90 -> 30/90, MRR 0.1737 -> 0.1910 (reproduced twice), a
# benchmark-hygiene fix, not a retrieval-quality one. 2 of the 90 tasks
# (erp-29, erp-86) still don't resolve even at this pin — confirmed genuine
# later renames (_query_opening_balance_duckdb -> get_opening_balances_duckdb,
# get_billed_qty_against_purchase_order -> get_billed_qty_amount_against_
# purchase_order), not a pin problem; left as-is.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PINNED_COMMIT="1887825ce5c6fe2fbd5366ab52e4c73de24d23fa"
DEST="${ROOT}/repos/erpnext"

if [[ -d "$DEST/.git" ]]; then
  CUR="$(git -C "$DEST" rev-parse HEAD)"
  if [[ "$CUR" == "$PINNED_COMMIT" ]]; then
    echo "erpnext already at pinned commit ${PINNED_COMMIT}"
    exit 0
  fi
  echo "erpnext exists at ${CUR}, fetching pinned commit..."
  git -C "$DEST" fetch --depth 1 origin "$PINNED_COMMIT"
  git -C "$DEST" checkout "$PINNED_COMMIT"
else
  git clone https://github.com/frappe/erpnext "$DEST"
  git -C "$DEST" checkout "$PINNED_COMMIT"
fi
echo "erpnext ready at $(git -C "$DEST" rev-parse HEAD)"
