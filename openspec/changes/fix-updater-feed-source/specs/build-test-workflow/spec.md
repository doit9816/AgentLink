## MODIFIED Requirements

### Requirement: Requested packaging only
AgentLink SHALL not be packaged unless packaging is explicitly requested.

#### Scenario: User requests tests only
- **WHEN** the user asks to run tests without packaging
- **THEN** the workflow SHALL avoid release packaging commands and SHALL not generate or publish updater-feed artifacts

### Requirement: Change-specific validation
Changes SHALL include validation that matches their touched surfaces.

#### Scenario: Rust code changes
- **WHEN** Rust source files are modified
- **THEN** the workflow SHALL run `cargo fmt` and an appropriate cargo check

#### Scenario: Frontend code changes
- **WHEN** frontend source files are modified
- **THEN** the workflow SHALL run `npm run build:ui`

### Requirement: Public updater-feed publishing
Tagged desktop releases SHALL publish updater metadata and referenced artifacts to a public static feed in addition to generating signed release bundles.

#### Scenario: Tagged Windows desktop release
- **WHEN** the release workflow builds the Windows desktop bundle for a `v*` tag
- **THEN** it SHALL stage `latest.json`, the updater-compatible installer artifact, and the matching signature file into a public updater-feed directory

#### Scenario: Deploying the updater feed
- **WHEN** the Windows tagged-release feed directory has been staged successfully
- **THEN** the release workflow SHALL publish that directory to the configured public static host used by the desktop updater endpoint
