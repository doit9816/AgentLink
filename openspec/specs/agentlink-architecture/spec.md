# AgentLink Architecture

## Purpose
Define the core layering and v1 priority path for AgentLink changes.

## Requirements

### Requirement: Layered connector flow
AgentLink SHALL route chat traffic through the Platform -> Engine -> AgentSession -> programming Agent architecture.

#### Scenario: Adding channel behavior
- **WHEN** a change adds or modifies a chat channel
- **THEN** it SHALL use the existing Platform abstraction rather than bypassing the Engine

#### Scenario: Adding agent behavior
- **WHEN** a change adds or modifies programming-agent execution
- **THEN** it SHALL implement or reuse Agent and AgentSession behavior rather than placing agent logic inside a channel implementation

### Requirement: v1 priority path
AgentLink SHALL prioritize a practical Feishu/Lark plus Codex workflow while keeping extension points for other platforms.

#### Scenario: Planning v1 work
- **WHEN** a change competes with Feishu/Lark plus Codex usability
- **THEN** the proposal SHALL state why the change is needed now or keep it outside the current scope
