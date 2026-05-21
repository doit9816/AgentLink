# Feishu/Lark Channel

## Purpose
Define the expected local-first Feishu/Lark connection behavior and diagnostics.

## Requirements

### Requirement: WebSocket default
Feishu/Lark SHALL default to `connection_mode = "websocket"` for local AgentLink use.

#### Scenario: Binding completes successfully
- **WHEN** Feishu/Lark binding succeeds
- **THEN** AgentLink SHALL write app_id, app_secret, connection_mode, api_base, and owner_open_id to the selected configuration

#### Scenario: Local setup guidance
- **WHEN** a user configures Feishu/Lark locally
- **THEN** AgentLink SHALL not require a public webhook as the primary path

### Requirement: Webhook fallback
Feishu/Lark webhook mode SHALL be treated as a fallback or platform-mandated mode.

#### Scenario: WebSocket cannot be used
- **WHEN** WebSocket mode is unavailable or explicitly unsupported
- **THEN** AgentLink MAY use webhook mode with clear configuration and diagnostics

### Requirement: Receive diagnostics
Feishu/Lark receive failures SHALL guide troubleshooting toward process state, config presence, connection mode, endpoint, token, and permission errors.

#### Scenario: Messages are not received
- **WHEN** bridge logs or client diagnostics report a Feishu/Lark receive problem
- **THEN** they SHALL make it practical to inspect whether agentlink.exe is running, the selected config exists, websocket mode is configured, and websocket endpoint/token/permission errors occurred
