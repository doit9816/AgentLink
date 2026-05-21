## ADDED Requirements

### Requirement: Public updater feed endpoint
The AgentLink desktop updater SHALL query a public static metadata endpoint whose contents remain reachable without GitHub Release asset authentication.

#### Scenario: Desktop checks for updates
- **WHEN** the desktop client triggers a Tauri updater check
- **THEN** it SHALL request `latest.json` from the configured public static updater-feed URL rather than from GitHub's `releases/latest/download/latest.json` redirect

#### Scenario: Public metadata references downloadable artifacts
- **WHEN** the public updater feed exposes a `latest.json`
- **THEN** every artifact URL referenced by that metadata SHALL resolve to a file published under the same public updater-feed host
