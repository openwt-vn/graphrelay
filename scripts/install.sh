#!/usr/bin/env sh
# GraphRelay installer
# Usage: curl -fsSL https://raw.githubusercontent.com/openwt-vn/graphrelay/main/scripts/install.sh | sh
set -e

REPO="openwt-vn/graphrelay"
BIN_NAME="gr"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# ── Parse arguments ───────────────────────────────────────────────────────────
# Supports:
#   GITHUB_TOKEN=xxx curl ... | sh
#   curl ... | sh -s -- --token=xxx
#   curl ... | sh -s -- --version=v0.1.0 --dir=/usr/local/bin
for arg in "$@"; do
  case "$arg" in
    --token=*)   GITHUB_TOKEN="${arg#--token=}" ;;
    --version=*) VERSION="${arg#--version=}" ;;
    --dir=*)     INSTALL_DIR="${arg#--dir=}" ;;
    *) echo "Unknown argument: $arg" && exit 1 ;;
  esac
done

# ── Detect OS + arch ──────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64)  TARGET="aarch64-apple-darwin" ;;
      x86_64) TARGET="x86_64-apple-darwin"  ;;
      *)      echo "Unsupported macOS arch: $ARCH" && exit 1 ;;
    esac
    ;;
  Linux)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-unknown-linux-gnu"  ;;
      aarch64) TARGET="aarch64-unknown-linux-gnu"  ;;
      *)       echo "Unsupported Linux arch: $ARCH" && exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

# ── Auth header (token or gh CLI) ────────────────────────────────────────────
AUTH_HEADER=""
if [ -n "$GITHUB_TOKEN" ]; then
  AUTH_HEADER="Authorization: Bearer ${GITHUB_TOKEN}"
elif command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  GITHUB_TOKEN="$(gh auth token 2>/dev/null)"
  AUTH_HEADER="Authorization: Bearer ${GITHUB_TOKEN}"
fi

_curl() {
  if [ -n "$AUTH_HEADER" ]; then
    curl -fsSL -H "$AUTH_HEADER" "$@"
  else
    curl -fsSL "$@"
  fi
}

# ── Resolve version ───────────────────────────────────────────────────────────
if [ -z "$VERSION" ]; then
  VERSION="$(_curl "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | sed 's/.*"tag_name": *"\(.*\)".*/\1/')"
fi

if [ -z "$VERSION" ]; then
  echo "Could not fetch latest release."
  echo "  • Check releases: https://github.com/${REPO}/releases"
  echo "  • Pass a token:   curl ... | sh -s -- --token=YOUR_TOKEN"
  echo "  • Pin a version:  curl ... | sh -s -- --version=v0.1.0 --token=YOUR_TOKEN"
  exit 1
fi

ARCHIVE="graphrelay-${VERSION}-${TARGET}.tar.gz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Installing graphrelay ${VERSION} (${TARGET})..."

# ── Download ──────────────────────────────────────────────────────────────────
# For private repos the direct github.com/releases/download URL returns 404.
# Resolve the asset's API URL via the releases endpoint, then download with
# Accept: application/octet-stream.
# Note: in GitHub's JSON, "url" appears BEFORE "name" in each asset object,
# so we track the last seen asset URL with awk.
if [ -n "$AUTH_HEADER" ]; then
  RELEASE_JSON="$(_curl "https://api.github.com/repos/${REPO}/releases/tags/${VERSION}")"

  ASSET_URL="$(printf '%s' "$RELEASE_JSON" | awk '
    /"url":.*releases\/assets/ {
      u = $0; gsub(/.*"url": *"/, "", u); gsub(/".*/, "", u); last = u
    }
    index($0, "\"'"${ARCHIVE}"'\"") { print last; exit }
  ')"

  if [ -z "$ASSET_URL" ]; then
    echo "Error: asset '${ARCHIVE}' not found in release ${VERSION}."
    echo "Assets in this release:"
    printf '%s' "$RELEASE_JSON" | grep '"name":' | grep '\.tar\.gz' | sed 's/.*"name": *"//;s/".*//'
    exit 1
  fi

  # GitHub redirects to S3 — use --location-trusted only for the initial
  # request; on the S3 redirect we must NOT forward the Authorization header
  # (S3 rejects requests with a Bearer token it doesn't recognize).
  # Strategy: fetch the redirect Location first, then download from S3 clean.
  S3_URL="$(curl -sI \
    -H "$AUTH_HEADER" \
    -H "Accept: application/octet-stream" \
    "$ASSET_URL" | grep -i '^location:' | tr -d '\r' | sed 's/location: *//i')"

  if [ -n "$S3_URL" ]; then
    curl -fsSL "$S3_URL" -o "$TMP/$ARCHIVE"
  else
    # No redirect — download directly (token already accepted)
    curl -fsSL \
      -H "$AUTH_HEADER" \
      -H "Accept: application/octet-stream" \
      "$ASSET_URL" -o "$TMP/$ARCHIVE"
  fi
else
  curl -fsSL \
    "https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}" \
    -o "$TMP/$ARCHIVE"
fi

# ── Verify checksum ───────────────────────────────────────────────────────────
# release.yml publishes ARCHIVE.sha256 alongside every archive — download and
# check it before extracting instead of trusting the transport unconditionally.
# Best-effort: a private-repo asset lookup would need the same auth dance as the
# archive itself, so on failure to fetch the checksum we warn and continue
# rather than hard-fail an otherwise-working install.
SHA_FILE="${ARCHIVE}.sha256"
if [ -n "$AUTH_HEADER" ]; then
  SHA_ASSET_URL="$(printf '%s' "$RELEASE_JSON" | awk '
    /"url":.*releases\/assets/ {
      u = $0; gsub(/.*"url": *"/, "", u); gsub(/".*/, "", u); last = u
    }
    index($0, "\"'"${SHA_FILE}"'\"") { print last; exit }
  ')"
  if [ -n "$SHA_ASSET_URL" ]; then
    curl -fsSL -H "$AUTH_HEADER" -H "Accept: application/octet-stream" \
      "$SHA_ASSET_URL" -o "$TMP/$SHA_FILE" 2>/dev/null || true
  fi
else
  curl -fsSL \
    "https://github.com/${REPO}/releases/download/${VERSION}/${SHA_FILE}" \
    -o "$TMP/$SHA_FILE" 2>/dev/null || true
fi

if [ -s "$TMP/$SHA_FILE" ]; then
  EXPECTED="$(awk '{print $1}' "$TMP/$SHA_FILE")"
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "$TMP/$ARCHIVE" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "$TMP/$ARCHIVE" | awk '{print $1}')"
  else
    echo "Warning: no sha256sum/shasum available — skipping checksum verification." >&2
    ACTUAL=""
  fi
  if [ -n "$ACTUAL" ]; then
    if [ "$EXPECTED" != "$ACTUAL" ]; then
      echo "Error: checksum mismatch for ${ARCHIVE}" >&2
      echo "  expected: $EXPECTED" >&2
      echo "  actual:   $ACTUAL" >&2
      exit 1
    fi
    echo "✓ Checksum verified"
  fi
else
  echo "Warning: could not fetch ${SHA_FILE} — skipping checksum verification." >&2
fi

tar -xzf "$TMP/$ARCHIVE" -C "$TMP"

# ── Install ───────────────────────────────────────────────────────────────────
mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"

echo "✓ Installed to $INSTALL_DIR/$BIN_NAME"

# ── PATH check ────────────────────────────────────────────────────────────────
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "  $INSTALL_DIR is not on your PATH. Add this to your shell profile:"
    echo '    export PATH="'"$INSTALL_DIR"':$PATH"'
    echo ""
    ;;
esac

echo "  Get started:"
echo "    gr init     # register current project"
echo "    gr start    # start daemon (downloads model on first run)"
