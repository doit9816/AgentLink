# Desktop Client

## Purpose
Define the stable desktop-client workflow required for AgentLink configuration, channel testing, and agent setup.

## Requirements

### Requirement: Three-tab structure
The Tauri/Vue desktop client SHALL keep the 连接, Channel, and Agent tabs as its main structure.

#### Scenario: Editing the desktop app
- **WHEN** client/agentlink-desktop/src/App.vue is changed
- **THEN** the main workflow SHALL remain available through the 连接, Channel, and Agent tabs

### Requirement: Channel testing
The Channel tab SHALL keep a send-message-test area that can use discovered targets.

#### Scenario: Sending a test message
- **WHEN** discovered targets exist for a selected channel
- **THEN** the client SHALL let the user select one or more targets for test sends

### Requirement: Client-driven binding
Supported QR or binding flows SHALL be surfaced in the desktop client instead of opening command-line windows as the primary interaction.

#### Scenario: QR binding is supported
- **WHEN** a channel supports QR binding
- **THEN** the QR code SHALL be displayed inside the client UI

#### Scenario: Binding needs platform-created app fields
- **WHEN** a channel requires platform console fields instead of automatic QR authorization
- **THEN** the client SHALL clearly ask for those platform-provided fields and SHALL NOT present the platform management QR as automatic authorization
