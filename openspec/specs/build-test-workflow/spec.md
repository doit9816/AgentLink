# Build And Test Workflow

## Purpose
Define the build, test, and packaging constraints for AgentLink development.

## Requirements

### Requirement: Target directory discipline
AgentLink builds SHALL avoid writing large build artifacts to the repository D: drive target directory by default.

#### Scenario: Running Rust checks for the main program
- **WHEN** cargo check or cargo test is run for the main AgentLink program
- **THEN** CARGO_TARGET_DIR SHOULD be set to `H:\agentlink-target`

#### Scenario: Running Rust checks for the desktop client
- **WHEN** cargo check is run for the desktop client
- **THEN** CARGO_TARGET_DIR SHOULD be set to `H:\agentlink-desktop-target`

### Requirement: Requested packaging only
AgentLink SHALL not be packaged unless packaging is explicitly requested.

#### Scenario: User requests tests only
- **WHEN** the user asks to run tests without packaging
- **THEN** the workflow SHALL avoid release packaging commands

### Requirement: Change-specific validation
Changes SHALL include validation that matches their touched surfaces.

#### Scenario: Rust code changes
- **WHEN** Rust source files are modified
- **THEN** the workflow SHALL run `cargo fmt` and an appropriate cargo check

#### Scenario: Frontend code changes
- **WHEN** frontend source files are modified
- **THEN** the workflow SHALL run `npm run build:ui`
