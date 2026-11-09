#!/bin/sh
# Builds the universal Morsel app (Apple Silicon and Intel) and the DMG.
# Signing, notarization and update files are made only when their variables are set:
#   APPLE_SIGNING_IDENTITY                  "Developer ID Application: <name> (<team id>)"
#   APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID  notarization (APPLE_PASSWORD is an app-specific password)
#   TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]    the updater key, for the update files and latest.json
set -eu
cd "$(dirname "$0")/.."

# A universal build needs the standard library of both chips. Only rustup gives both,
# so its tools go before another Rust (such as the Homebrew one) in PATH.
if [ -x "$HOME/.cargo/bin/rustup" ]; then
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
fi

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "Signing: $APPLE_SIGNING_IDENTITY"
else
  echo "Signing: off. Other Macs will block this app. See docs/RELEASE.md."
fi
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "Notarization: on"
else
  echo "Notarization: off"
fi

EXTRA=""
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  echo "Update files: on"
  EXTRA="--config src-tauri/tauri.release.conf.json"
else
  echo "Update files: off (no TAURI_SIGNING_PRIVATE_KEY)"
fi

# shellcheck disable=SC2086
pnpm tauri build --target universal-apple-darwin --bundles app,dmg $EXTRA

BUNDLE=src-tauri/target/universal-apple-darwin/release/bundle
echo "Architectures: $(lipo -archs "$BUNDLE/macos/Morsel.app/Contents/MacOS/morsel")"
codesign -dv "$BUNDLE/macos/Morsel.app" 2>&1 | grep -E "^(Authority|Signature|TeamIdentifier)" || true
ls "$BUNDLE/dmg"
