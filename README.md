# AgentLink

<div align="center">

**🚀 Connect Any Chat Channel to Any Programming Agent**

[![Build Status](https://github.com/doit9816/AgentLink/actions/workflows/release.yml/badge.svg)](https://github.com/doit9816/AgentLink/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/doit9816/AgentLink/releases)

</div>

AgentLink is a **high-performance Rust bridge** that seamlessly connects chat channels to AI programming agents, enabling natural language coding workflows across your favorite messaging apps.

## ✨ Key Features

### 🌐 Universal Chat Channel Support
Connect to **15+ chat channels** with native adapters:
- **Enterprise**: Feishu/Lark (WebSocket), DingTalk, Slack (Socket Mode), WeCom
- **Social**: Telegram (Long Polling), Discord (Gateway), LINE, Weibo
- **Messaging**: WeChat Personal (iLink), QQ/OneBot, QQ Official Bot, MAX
- **Extensible**: HTTP Webhook & WebSocket Bridge for custom integrations

### 🤖 Multi-Agent Architecture
Unified interface for popular programming agents:
- **Codex** (exec & app-server modes)
- **Claude Code** / **Cursor** / **Gemini CLI**
- **OpenCode** / **Kimi** / **Qoder** / **iFlow**
- **Generic CLI adapter** for any command-line agent

### 🖥️ Beautiful Desktop Client
**Tauri 2.0 + Vue 3** cross-platform GUI:
- 🎨 Modern, responsive interface
- 🔐 QR code scanning for instant platform setup
- ⚙️ Visual configuration editor with validation
- 🧪 Built-in message testing with rich media support
- 📊 Real-time service status monitoring

### 🔄 Automated Cross-Platform Builds
**GitHub Actions powered CI/CD**:
- ✅ Automatic builds on every push to `main`
- 📦 Platform-specific installers (MSI, DMG, AppImage, DEB)
- 🏷️ Tagged releases with semantic versioning
- 🔧 Optimized icon configuration for all platforms

### 🔒 Production-Ready Features
- **Session Management**: SQLite-backed persistent sessions
- **Approval Workflow**: Built-in `/allow` and `/deny` commands
- **Rich Media**: Images, files, audio, video, location, cards
- **Security**: Token authentication, signature verification
- **Reliability**: Message queuing, error recovery, timeout handling

## 📚 Documentation

- [中文文档 (Chinese README)](README.zh-CN.md)
- [协议说明 (Protocol Documentation)](docs/agentlink-protocol.zh-CN.md)

## 🚀 Quick Start

### Installation

Download the latest release for your platform:

**Windows**
```powershell
# Download from GitHub Releases
# Extract and run agentlink.exe
```

**macOS**
```bash
# Download the DMG from GitHub Releases
# Drag AgentLink.app to Applications
```

**Linux**
```bash
# Download AppImage or DEB package
chmod +x AgentLink-*.AppImage
./AgentLink-*.AppImage
```

### First Run

1. **Launch the desktop client** to configure your first chat channel
2. **Scan QR code** for instant Feishu/Lark/WeChat setup
3. **Configure your agent** (Codex, Claude Code, etc.)
4. **Start chatting** with your AI programming assistant!

## 🛠️ Development

### Prerequisites
- Rust 1.70+
- Node.js 18+
- Platform-specific dependencies (see below)

### Build from Source

```bash
# Clone the repository
git clone https://github.com/doit9816/AgentLink.git
cd AgentLink

# Build the core bridge
cargo build --release

# Validate a config file
./target/release/agentlink validate-config examples/agentlink.1.toml

# Build the desktop client
cd client/agentlink-desktop
npm install
npm run build

# Outputs:
# - CLI: ../target/release/agentlink
# - Desktop bundles: src-tauri/target/release/bundle/
```

### Platform-Specific Dependencies

**Linux**
```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

**macOS**
```bash
# Xcode Command Line Tools required
xcode-select --install
```

**Windows**
```powershell
# Visual Studio Build Tools or MSVC required
# WebView2 runtime (usually pre-installed on Windows 10/11)
```

## 📦 Packaging & Distribution

### Local Packaging

**Windows (All-in-One)**
```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\auto-package.ps1
```

This script:
- ✅ Runs all tests
- 🔨 Builds core bridge in release mode
- 🖥️ Builds Tauri desktop client
- 📦 Generates platform installers (MSI, NSIS)
- 🗜️ Creates distribution archive

**macOS/Linux**
```bash
bash ./scripts/package-unix.sh <version> <target-name>
```

### Automated GitHub Releases

Our CI/CD pipeline automatically builds and publishes releases:

**On manual dispatch**
- Triggers the cross-platform desktop build workflow without changing the published updater feed

**On Version Tag (`v*`)**
```bash
git tag v0.2.0
git push origin v0.2.0
```
- Creates stable release
- Generates release notes
- Publishes platform-specific installers:
  - **Windows**: `.zip`, `.msi`, `.exe` (NSIS)
  - **macOS**: `.tar.gz`, `.dmg`
  - **Linux**: `.tar.gz`, `.AppImage`, `.deb`
- Publishes the Windows desktop updater feed to the `updater-feed` branch:
  - `https://raw.githubusercontent.com/doit9816/AgentLink/updater-feed/windows/latest.json`

### Release Artifacts

Each release includes:
- 📦 **Core Bridge**: Standalone CLI executable
- 🖥️ **Desktop Client**: Platform-specific installer
- 📄 **Documentation**: README and examples
- 🔧 **Configuration Templates**: Sample TOML files

## Local Packaging

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\auto-package.ps1
```

The package script builds the core bridge, the desktop client, and the Windows release archive.

## Desktop Updater Feed

The desktop client's Tauri updater reads from a public feed in the `updater-feed` branch instead of `releases/latest/download/latest.json`:

```text
https://raw.githubusercontent.com/doit9816/AgentLink/updater-feed/windows/latest.json
```

Tagged Windows releases stage `latest.json`, the referenced installer, and the matching `.sig` file before publishing them to the `updater-feed` branch.

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Chat Channels                             │
│  Feishu │ Slack │ Telegram │ Discord │ WeChat │ QQ │ ...   │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                  Channel Adapters                            │
│         (Native WebSocket/HTTP/Long Polling)                 │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                    AgentLink Engine                          │
│  • Session Management  • Message Routing                     │
│  • Approval Workflow   • Rich Media Handling                 │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                   Agent Adapters                             │
│    Codex │ Claude Code │ Gemini │ Cursor │ Generic CLI      │
└─────────────────────────────────────────────────────────────┘
```

## 🗂️ Code Layout

The Rust workspace has been refactored from a flat `src/*.rs` layout into feature-oriented modules:

```text
src/
  agents/      Codex integration and generic CLI agent presets
  app/         config loading and registry assembly
  channels/    native chat channels plus Bridge/HTTP adapters
  core/        shared traits, messages, approvals, and session types
  engine/      routing, session serialization, approval flow, final replies
  setup/       setup commands such as Feishu/Lark bootstrap helpers
  store/       SQLite-backed persistence
  testing/     mock platform/agent helpers for e2e and integration tests
  lib.rs       public exports for embedding and tests
  main.rs      CLI entrypoint and default runtime wiring
```

`src/lib.rs` still re-exports the main public modules, so existing imports can keep working while new code moves to the namespaced layout.

## 🎯 Use Cases

- **Team Collaboration**: Let your team interact with AI agents through familiar chat apps
- **Code Review**: Get instant code suggestions and reviews in Slack/Discord channels
- **DevOps Automation**: Trigger deployments and infrastructure changes via chat commands
- **Learning & Onboarding**: Help new developers with interactive coding assistance
- **Multi-Channel Support**: Reach users on their preferred messaging channel

## 🔧 Configuration

AgentLink uses TOML configuration files for maximum flexibility:

```toml
[[projects]]
name = "my-project"
default_platforms = ["feishu", "slack"]

[[projects.platforms]]
id = "feishu"
type = "feishu"
connection_mode = "websocket"  # No public IP required!

[projects.platforms.options]
app_id = "cli_xxx"
app_secret = "xxx"

[projects.agent]
type = "codex"

[projects.agent.options]
work_dir = "/path/to/project"
backend = "exec"
mode = "suggest"
model = "gpt-5.2"
```

See `examples/` directory for complete configuration templates.

## 🤝 Contributing

We welcome contributions! Here's how you can help:

1. 🐛 **Report Bugs**: Open an issue with reproduction steps
2. 💡 **Suggest Features**: Share your ideas in discussions
3. 🔧 **Submit PRs**: Fix bugs or add new features
4. 📖 **Improve Docs**: Help others understand AgentLink better
5. 🌍 **Add Channels**: Implement adapters for new chat channels

## 📊 Project Status

- ✅ **Core Engine**: Stable
- ✅ **Desktop Client**: Stable (Tauri 2.0)
- ✅ **CI/CD Pipeline**: Fully automated
- ✅ **15+ Chat Channels**: Production ready
- 🚧 **Rich Media**: Expanding channel support
- 🚧 **Mobile Clients**: Planned

## 🙏 Acknowledgments

Built with amazing open-source technologies:
- [Rust](https://www.rust-lang.org/) - Systems programming language
- [Tauri](https://tauri.app/) - Desktop application framework
- [Vue 3](https://vuejs.org/) - Progressive JavaScript framework
- [Tokio](https://tokio.rs/) - Async runtime for Rust

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

<div align="center">

**⭐ Star us on GitHub if AgentLink helps your workflow!**

[Report Bug](https://github.com/doit9816/AgentLink/issues) · [Request Feature](https://github.com/doit9816/AgentLink/issues) · [Documentation](README.zh-CN.md)

</div>
