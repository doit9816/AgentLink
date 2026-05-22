# Design

## Overview

Use Tauri bundle resources to carry a staged copy of the root `agentlink` executable inside packaged desktop apps. Keep the existing CLI-driven desktop logic intact by making path resolution discover the bundled binary through the same `resolve_existing_path` flow that already supports repository-relative development paths.

## Staging flow

1. `beforeBuildCommand` runs a desktop packaging helper instead of the plain UI build.
2. The helper still builds the frontend, then builds the root `agentlink` binary for the active Tauri target triple.
3. The helper copies that binary to `client/agentlink-desktop/src-tauri/resources-generated/target/release/agentlink[.exe]`.
4. `tauri.conf.json` bundles `resources-generated/` into the application resources root.

This preserves the familiar `target/release/agentlink` relative shape inside the bundle, which means the desktop resolver can find it without inventing a second path convention.

## Runtime discovery

Desktop path discovery already checks:

- the current working directory
- the current executable directory
- repository-relative development roots

Extend that root set with packaged resource neighbors derived from `current_exe()`:

- `<exe dir>/Resources`
- `<exe dir>/resources`
- `<exe dir>/../Resources`
- `<exe dir>/../resources`

For macOS app bundles, this covers `AgentLink.app/Contents/Resources`, where Tauri places bundled resources.

## UI default

Packaged builds should default the CLI field to `agentlink` / `agentlink.exe` instead of an empty string. The resolver can map that logical name onto the staged bundle resource, while advanced users can still choose an external binary manually.
