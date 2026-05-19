## Why

AgentLink desktop auto-update is wired up in code, but the configured updater endpoint currently returns `404`, so real update checks fail even when the UI and Tauri integration compile cleanly. We need a public, stable feed that is independent from GitHub's `releases/latest/download/latest.json` behavior and does not depend on anonymous access to release assets.

## What Changes

- Switch the desktop updater endpoint from the GitHub Release `latest.json` URL to a public static feed hosted in the repository's `updater-feed` branch.
- Add a release-time staging step that copies each platform's `latest.json`, installer artifacts, and signature files into a public updater-feed directory and rewrites metadata URLs to that public base URL.
- Extend the release workflow to publish staged updater feeds for Windows, Linux, and macOS to the `updater-feed` branch on tagged releases.
- Update release and updater documentation so maintainers know the new public feed path and deployment expectations.

## Capabilities

### New Capabilities
- `desktop-updater-feed`: Desktop auto-update uses per-platform public static feeds whose metadata and installer URLs stay reachable without GitHub Release asset access.

### Modified Capabilities
- `build-test-workflow`: Release publishing now includes deployment of per-platform updater metadata and referenced artifacts to a public static host in addition to producing signed bundles.

## Impact

- `client/agentlink-desktop/src-tauri/tauri.conf.json`
- `.github/workflows/release.yml`
- `scripts/`
- `README.md`
- `README.zh-CN.md`
