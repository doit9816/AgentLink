## Context

AgentLink's desktop updater currently points at `https://github.com/doit9816/AgentLink/releases/latest/download/latest.json`. In local verification on 2026-05-19, that endpoint returned `404`, and the repository's release API also returned `404` while git tags remained visible. That leaves the desktop app in a state where update checks are implemented but not operational.

The desktop updater already works through Tauri's built-in updater flow in `client/agentlink-desktop/src/App.vue`, and none of the Channel, Agent, target-discovery, reply-context, SQLite, or bridge lifecycle semantics need to change. The fix belongs in release metadata distribution and the updater endpoint configuration rather than in `Platform -> Engine -> AgentSession -> Agent`.

## Goals / Non-Goals

**Goals:**
- Make the desktop updater consume a public static `latest.json` endpoint that stays reachable without depending on GitHub's latest-release redirect semantics.
- Publish the updater metadata and every referenced Windows installer artifact to the same public feed path on tagged releases.
- Keep explicit packaging and updater-artifact generation separate from day-to-day `cargo check`, `cargo test`, and `npm run build:ui`.

**Non-Goals:**
- Rework the desktop updater UI or bridge install gating.
- Change Channel credentials, discovered targets, SQLite schema, or inbound reply-context handling.
- Add packaging to normal validation commands.

## Decisions

1. Use a dedicated `updater-feed` branch as the public updater-feed host.
   The repository already uses GitHub Actions, and a branch published through `raw.githubusercontent.com` provides a simple anonymous static endpoint under `https://raw.githubusercontent.com/doit9816/AgentLink/updater-feed/`. This avoids depending on release-asset access policy and keeps the updater feed under the same repository ownership without requiring GitHub Pages. The desktop updater endpoint in `tauri.conf.json` will move to `.../{{target}}/{{arch}}/latest.json` on that branch.

2. Stage the updater feed from signed bundle output after `tauri build`.
   A new PowerShell script under `scripts/` will validate the generated bundle, copy updater-compatible installer artifacts plus `.sig` files into a feed directory, and rewrite the `latest.json` download URLs to the public base URL. This keeps the feed generation logic local, testable, and reusable from both CI and explicit local release work.

3. Publish per-platform updater feeds from the tagged-release matrix.
   Tauri's updater endpoints support `{{target}}` and `{{arch}}`, so each matrix entry can stage a platform-specific feed under `updater-feed/<target>/<arch>/latest.json`. A final publish job will gather those staged feeds and push them to the `updater-feed` branch in one commit, which avoids cross-job branch push conflicts while keeping Windows, Linux, and macOS updates aligned with the same release tag.

4. Keep updater verification layered.
   `scripts/validate-updater-artifacts.ps1` remains the bundle-level validation for generated updater artifacts. The new staging script adds feed-level validation by ensuring the rewritten `latest.json` still points at files that exist in the public feed directory.

## Risks / Trade-offs

- [Risk] The repository or branch is not publicly readable -> Mitigation: document that the repo must stay public so clients can fetch the updater feed from `raw.githubusercontent.com`.
- [Risk] Rewritten metadata could point to the wrong file names if the bundle layout changes -> Mitigation: derive URLs from the actual copied file names and fail the script when referenced files are missing.
- [Risk] One platform build fails and blocks publication of the shared updater-feed branch -> Mitigation: keep the publish job dependent on the full build matrix so the branch only advances when every targeted desktop updater feed is available for the same tag.

## Migration Plan

1. Change the desktop updater endpoint to the `updater-feed` branch URL.
2. Add and verify the feed-staging script locally against generated bundle output when release artifacts exist.
3. Extend the release workflow so tagged releases publish the staged updater feed to the `updater-feed` branch.
4. After the next tagged release, verify `latest.json` from each public raw branch URL and confirm desktop clients on Windows, Linux, and macOS can check updates against their own feed paths.

## Open Questions

- Whether future ARM Linux targets should get their own updater feed path once those builds are added to the release matrix.
