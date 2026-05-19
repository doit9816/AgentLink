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
   The repository already uses GitHub Actions, and a branch published through `raw.githubusercontent.com` provides a simple anonymous static endpoint under `https://raw.githubusercontent.com/doit9816/AgentLink/updater-feed/`. This avoids depending on release-asset access policy and keeps the updater feed under the same repository ownership without requiring GitHub Pages. The desktop updater endpoint in `tauri.conf.json` will move to `.../windows/latest.json` on that branch.

2. Stage the updater feed from signed bundle output after `tauri build`.
   A new PowerShell script under `scripts/` will validate the generated bundle, copy updater-compatible installer artifacts plus `.sig` files into a feed directory, and rewrite the `latest.json` download URLs to the public base URL. This keeps the feed generation logic local, testable, and reusable from both CI and explicit local release work.

3. Publish the Windows updater feed from the Windows tagged-release job.
   The current desktop updater flow is Windows-first, and the local install mode already contains Windows-specific behavior. The release workflow will stage the feed on the Windows matrix entry and publish it to the `updater-feed` branch on tagged releases. Other desktop artifacts can continue to ship through GitHub Releases without blocking the updater path.

4. Keep updater verification layered.
   `scripts/validate-updater-artifacts.ps1` remains the bundle-level validation for generated updater artifacts. The new staging script adds feed-level validation by ensuring the rewritten `latest.json` still points at files that exist in the public feed directory.

## Risks / Trade-offs

- [Risk] The repository or branch is not publicly readable -> Mitigation: document that the repo must stay public so clients can fetch the updater feed from `raw.githubusercontent.com`.
- [Risk] Rewritten metadata could point to the wrong file names if the bundle layout changes -> Mitigation: derive URLs from the actual copied file names and fail the script when referenced files are missing.
- [Risk] Windows-only public feed leaves non-Windows desktop builds without an updater path -> Mitigation: keep non-Windows bundles in GitHub Releases and scope the feed path/documentation to the current Windows desktop updater support.

## Migration Plan

1. Change the desktop updater endpoint to the `updater-feed` branch URL.
2. Add and verify the feed-staging script locally against generated bundle output when release artifacts exist.
3. Extend the release workflow so tagged releases publish the staged updater feed to the `updater-feed` branch.
4. After the next tagged release, verify `latest.json` from the public raw branch URL and confirm the desktop client can check updates against it.

## Open Questions

- Whether future macOS/Linux desktop updater support should publish platform-specific feeds under separate subpaths or merge into one multi-platform feed.
