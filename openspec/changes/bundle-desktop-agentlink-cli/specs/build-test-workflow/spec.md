## ADDED Requirements

### Requirement: Desktop packaging stages bridge CLI
Desktop packaging SHALL stage the root AgentLink CLI into bundle resources before creating packaged desktop artifacts.

#### Scenario: Building a packaged desktop app
- **WHEN** a packaged desktop build is requested
- **THEN** the build workflow SHALL compile the root `agentlink` executable for the active target triple
- **AND** it SHALL copy that executable into the desktop bundle resources before Tauri creates platform bundles
