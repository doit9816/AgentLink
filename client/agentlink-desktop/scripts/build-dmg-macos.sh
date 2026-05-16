#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="AgentLink"
VERSION="$(node -p "require('./package.json').version")"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/src-tauri/target}"
APP_BUNDLE_DIR="$TARGET_DIR/release/bundle/macos"
APP_BUNDLE_PATH="$APP_BUNDLE_DIR/${APP_NAME}.app"
DMG_DIR="$TARGET_DIR/release/bundle/dmg"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ARCH_SUFFIX="x64" ;;
  arm64) ARCH_SUFFIX="arm64" ;;
  *) ARCH_SUFFIX="$ARCH" ;;
esac
DMG_NAME="${APP_NAME}_${VERSION}_${ARCH_SUFFIX}.dmg"
RW_DMG_PATH="$DMG_DIR/rw.${DMG_NAME}"
FINAL_DMG_PATH="$DMG_DIR/$DMG_NAME"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build:dmg is only supported on macOS." >&2
  exit 1
fi

cd "$ROOT_DIR"

echo "Building macOS app bundle..."
npm run build:app

if [[ ! -d "$APP_BUNDLE_PATH" ]]; then
  echo "Missing app bundle at $APP_BUNDLE_PATH" >&2
  exit 1
fi

mkdir -p "$DMG_DIR"
rm -f "$RW_DMG_PATH" "$FINAL_DMG_PATH"

APP_SIZE_KB="$(du -sk "$APP_BUNDLE_DIR" | awk '{print $1}')"
DMG_SIZE_MB="$(( (APP_SIZE_KB + 1023) / 1024 + 64 ))"

echo "Creating read-write disk image (${DMG_SIZE_MB} MB)..."
hdiutil create \
  -srcfolder "$APP_BUNDLE_DIR" \
  -volname "$APP_NAME" \
  -fs HFS+ \
  -format UDRW \
  -size "${DMG_SIZE_MB}m" \
  "$RW_DMG_PATH"

echo "Compressing disk image..."
hdiutil convert "$RW_DMG_PATH" -format UDZO -o "$FINAL_DMG_PATH"
rm -f "$RW_DMG_PATH"

echo "Finished DMG at: $FINAL_DMG_PATH"
