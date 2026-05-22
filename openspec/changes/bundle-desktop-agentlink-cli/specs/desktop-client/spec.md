## ADDED Requirements

### Requirement: Packaged desktop CLI discovery
Packaged desktop builds SHALL discover a bundled AgentLink CLI without requiring the user to browse for a separate executable on first launch.

#### Scenario: Opening a packaged desktop build
- **WHEN** the desktop client starts from a packaged application bundle that contains a staged `agentlink` executable resource
- **THEN** the connection workflow SHALL resolve that bundled executable for default CLI operations
- **AND** the executable field SHALL not default to an empty value
