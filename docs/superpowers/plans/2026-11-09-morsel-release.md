# Morsel release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Ship Morsel as one universal macOS app (Apple Silicon and Intel), signed and notarized, that updates itself.

**Architecture:** A release script and a GitHub Actions workflow build the universal app with the Tauri CLI. Signing and notarization read the Apple credentials from environment variables only. The updater plugin checks a `latest.json` file on the GitHub releases page and verifies each update with a minisign public key.

**Tech Stack:** Tauri 2 CLI, tauri-plugin-updater, tauri-plugin-process, tauri-apps/tauri-action, React 19, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-08-morsel-v1-design.md` (sections 3, 13, 16).

## Global Constraints

- macOS 14.2 or later, Apple Silicon and Intel (spec §3).
- No secret goes into the repository: the Apple certificate, the Apple ID password and the updater private key stay in the Keychain, in `~/.tauri`, or in GitHub secrets.
- The hardened runtime needs the entitlement `com.apple.security.device.audio-input` for the microphone.
- Morsel sends no analytics (spec §13). The update check sends only the normal HTTP request for `latest.json`.

## Review Focus

1. An update check with no network must not show an error to the user. Test: `update_check_failure_shows_nothing`.
2. The user must decide when the app restarts. Test: `update_bar_installs_only_after_a_click`.
3. A build with no Apple certificate must still give a working unsigned app for local use. Check: `scripts/release.sh` with no `APPLE_*` variables.

---

### Task 1: Universal build, entitlements and a content security policy

**Files:** `src-tauri/tauri.conf.json`, `src-tauri/Entitlements.plist`, `scripts/release.sh`, `package.json`

- [ ] `pnpm tauri build --target universal-apple-darwin --bundles app` works, and `lipo -archs` shows `x86_64 arm64`.
- [ ] Add the entitlements file (audio input) and `bundle.macOS.entitlements`, and set a strict CSP.
- [ ] `scripts/release.sh` builds the universal app and the DMG; it signs and notarizes only when `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` are set.
- [ ] Check: `pnpm run release` with no Apple variables, then `codesign -dv` and `lipo -archs`.

### Task 2: In-app updates

**Files:** `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json`, `src-tauri/tauri.conf.json`, `src/app/UpdateBar.tsx`, `src/app/AppShell.tsx`

- [ ] Tests: `update_bar_shows_a_new_version`, `update_bar_installs_only_after_a_click`, `update_check_failure_shows_nothing`.
- [ ] Add the updater and process plugins, the public key, the endpoint `https://github.com/alienstro/morsel/releases/latest/download/latest.json`, and `createUpdaterArtifacts: true`.
- [ ] Check: `pnpm vitest run src/app` and `cargo test --lib app_builds`.

### Task 3: Release workflow and the release guide

**Files:** `.github/workflows/release.yml`, `docs/RELEASE.md`

- [ ] The workflow runs on a `v*` tag on `macos-latest`, builds the universal app with tauri-action, signs and notarizes with GitHub secrets, and makes a draft release with `latest.json`.
- [ ] The guide lists each secret, how to make the Developer ID certificate, and how to publish a release.
- [ ] Check: `actionlint` or a YAML parse of the workflow.
