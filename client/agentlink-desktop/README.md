# AgentLink Desktop

The desktop app is a Tauri shell around the Rust `agentlink` CLI. It now defaults to the repo-local CLI layout used by the refactored project:

- executable: `target/release/agentlink` on macOS/Linux, `target/release/agentlink.exe` on Windows
- sample config: `examples/agentlink.<n>.toml`
- working directory: project root

The desktop path picker uses native OS dialogs on Windows, macOS, and Linux.

## Build

- `npm run dev`: start the Tauri desktop app in development mode
- `npm run build`: run the standard Tauri multi-platform build
- `npm run build:app`: build the macOS `.app` bundle
- `npm run build:dmg`: build the macOS `.dmg` package

Build outputs:

- `${CARGO_TARGET_DIR:-src-tauri/target}/release/bundle/macos/AgentLink.app`
- `${CARGO_TARGET_DIR:-src-tauri/target}/release/bundle/dmg/AgentLink_0.1.0_<arch>.dmg`
