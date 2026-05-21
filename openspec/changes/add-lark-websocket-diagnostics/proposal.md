## Why

Feishu/Lark websocket receive failures are currently hard to diagnose because users must infer whether the bridge is running, the selected config is correct, websocket mode is active, or the platform rejected endpoint/token/permission setup. This change makes the v1 Feishu/Lark + Codex path easier to support without pushing local users toward public webhook setup.

## What Changes

- Add explicit Feishu/Lark websocket diagnostics in bridge logs for connection startup, endpoint acquisition, token/auth failures, permission errors, reconnect attempts, and inbound-message receipt.
- Surface a concise client-side diagnostic summary for the selected project/config so users can tell whether `agentlink.exe` is running, the config exists, and `connection_mode = "websocket"` is active.
- Preserve websocket as the default local connection mode; webhook remains fallback only.
- Do not change binding credentials, target discovery behavior, or packaging.

## Non-goals

- No new Feishu/Lark binding flow.
- No public webhook-first setup.
- No changes to other channels.
- No release packaging as part of this change.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `feishu-lark-channel`: strengthen receive diagnostics requirements for websocket startup, authorization, permissions, reconnects, and inbound-message visibility.

## Impact

- `src/`: Feishu/Lark platform and bridge logging/diagnostic paths.
- `client/agentlink-desktop/src/App.vue`: Channel diagnostics display or refresh behavior, if the existing command surface supports it.
- `tests/`: focused checks for diagnostic classification or log/status output where practical.
- Config shape: no breaking changes expected.
