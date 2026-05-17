# AgentLink Desktop

## Build

- `npm run dev`: start the Tauri desktop app in development mode
- `npm run build`: run the standard Tauri multi-platform build
- `npm run build:app`: build the macOS `.app` bundle
- `npm run build:dmg`: build the macOS `.dmg` package

Build outputs:

- `${CARGO_TARGET_DIR:-src-tauri/target}/release/bundle/macos/AgentLink.app`
- `${CARGO_TARGET_DIR:-src-tauri/target}/release/bundle/dmg/AgentLink_0.1.0_<arch>.dmg`
