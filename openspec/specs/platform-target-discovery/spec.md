# Platform Target Discovery

## Purpose
Define how AgentLink discovers chat users and sessions from inbound platform messages.

## Requirements

### Requirement: Inbound-message-first discovery
AgentLink SHALL discover users and sessions from real inbound platform messages instead of requiring users to hand-enter platform identifiers as the primary flow.

#### Scenario: User sends a message to a bot
- **WHEN** a Platform receives an inbound chat message
- **THEN** AgentLink SHALL persist a target record that can be selected by the desktop client

### Requirement: Target persistence fields
Target records SHALL preserve enough context for follow-up replies and test sends.

#### Scenario: Persisting a discovered target
- **WHEN** a target is saved
- **THEN** it SHALL include project, platform, session_key, user_id, user_name, message_id, reply_ctx, content_preview, and updated_at when available from the inbound message

### Requirement: Reply context preference
AgentLink SHALL prefer inbound reply context for real chat replies.

#### Scenario: Replying to an inbound message
- **WHEN** a reply is sent for an existing inbound conversation
- **THEN** AgentLink SHALL use the stored reply_ctx before falling back to manually supplied target fields
