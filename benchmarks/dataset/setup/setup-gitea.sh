#!/usr/bin/env bash
# Clone the Gitea benchmark repo (go-gitea/gitea) at the pinned commit used by `run-sot.sh --repo gitea`.
# Keep PINNED_COMMIT in sync with run-sot.sh. Shallow fetch of that one commit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PINNED_COMMIT="f35c67bd79b3bff9c28eb8171d53586cc800bd5c"
DEST="${ROOT}/repos/gitea"

if [[ -d "$DEST/.git" ]]; then
  CUR="$(git -C "$DEST" rev-parse HEAD)"
  if [[ "$CUR" == "$PINNED_COMMIT" ]]; then
    echo "gitea already at pinned commit ${PINNED_COMMIT}"
    exit 0
  fi
  echo "gitea is at ${CUR}, not the pin; refusing to move it. Remove ${DEST} and re-run." >&2
  exit 1
fi

mkdir -p "$DEST"
git -C "$DEST" init -q
git -C "$DEST" remote add origin https://github.com/go-gitea/gitea.git
git -C "$DEST" fetch -q --depth 1 origin "$PINNED_COMMIT"
git -C "$DEST" checkout -q FETCH_HEAD
echo "gitea at ${PINNED_COMMIT}"
