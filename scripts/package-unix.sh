#!/usr/bin/env bash

set -euo pipefail

VERSION="${1:-0.1.0}"
TARGET_NAME="${2:-linux-x64}"
CORE_TARGET_DIR="${3:-}"
DESKTOP_TARGET_DIR="${4:-}"
SKIP_DESKTOP_ARTIFACTS="${5:-0}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
STAGE="$DIST/agentlink-v${VERSION}-${TARGET_NAME}"
ARCHIVE="${STAGE}.tar.gz"

if [[ -z "$CORE_TARGET_DIR" ]]; then
  CORE_TARGET_DIR="$ROOT/target"
fi

if [[ -z "$DESKTOP_TARGET_DIR" ]]; then
  DESKTOP_TARGET_DIR="$ROOT/target/agentlink-desktop"
fi

RELEASE_BIN="$CORE_TARGET_DIR/release/agentlink"
DESKTOP_BIN="$DESKTOP_TARGET_DIR/release/agentlink-desktop"
DESKTOP_BUNDLE_DIR="$DESKTOP_TARGET_DIR/release/bundle"

rm -rf "$STAGE"
mkdir -p "$STAGE/examples" "$STAGE/docs" "$STAGE/client/agentlink-desktop"

cp "$RELEASE_BIN" "$STAGE/"
cp "$ROOT/README.zh-CN.md" "$STAGE/"
cp -R "$ROOT/examples/." "$STAGE/examples/"
cp "$ROOT/docs/agentlink-protocol.zh-CN.md" "$STAGE/docs/"

if [[ "$SKIP_DESKTOP_ARTIFACTS" != "1" ]]; then
  if [[ -f "$DESKTOP_BIN" || -d "$DESKTOP_BUNDLE_DIR" ]]; then
    mkdir -p "$STAGE/desktop"
    if [[ -f "$DESKTOP_BIN" ]]; then
      cp "$DESKTOP_BIN" "$STAGE/desktop/"
    fi
    if [[ -d "$DESKTOP_BUNDLE_DIR" ]]; then
      cp -R "$DESKTOP_BUNDLE_DIR" "$STAGE/desktop/bundle"
    fi
  fi
fi

rsync -a \
  --exclude node_modules \
  --exclude target \
  --exclude dist \
  --exclude data \
  --exclude '*.log' \
  --exclude '*.sqlite3' \
  --exclude '*.sqlite3-shm' \
  --exclude '*.sqlite3-wal' \
  "$ROOT/client/agentlink-desktop/" \
  "$STAGE/client/agentlink-desktop/"

rm -f "$ARCHIVE"
tar -C "$DIST" -czf "$ARCHIVE" "$(basename "$STAGE")"
echo "Package written: $ARCHIVE"
