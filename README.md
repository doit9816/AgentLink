# AgentLink

AgentLink is a local bridge that connects chat channels to programming agents.

It provides:

- Native channel adapters for Feishu/Lark, DingTalk, Telegram, Slack, Discord, LINE, WeCom, Weixin personal iLink, QQ/OneBot, QQBot, Weibo, MAX, HTTP and WebSocket bridge.
- Agent adapters for Codex and generic CLI-style programming agents.
- A Tauri + Vue desktop client for configuring channels, agents, QR binding, local state and test messages.
- Automated Windows, macOS and Linux packaging through GitHub Actions.

Chinese documentation:

- [README.zh-CN.md](README.zh-CN.md)
- [docs/agentlink-protocol.zh-CN.md](docs/agentlink-protocol.zh-CN.md)

## Local Packaging

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\auto-package.ps1
```

The package script builds the core bridge, the desktop client, and the Windows release archive.

## GitHub Packaging

The workflow in `.github/workflows/release.yml` publishes:

- `latest` prerelease on every push to `main`
- Stable release artifacts on `v*` tags

Release artifacts include platform-specific desktop bundles plus a packaged bridge archive:

- Windows: zip archive, MSI installer and NSIS installer
- macOS: tar.gz archive and DMG bundle
- Linux: tar.gz archive, AppImage and DEB bundle
