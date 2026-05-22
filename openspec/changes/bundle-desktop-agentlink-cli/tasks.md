## 1. Bundled CLI packaging

- [x] 1.1 Add a desktop packaging helper that builds and stages the root `agentlink` executable into Tauri bundle resources.
- [x] 1.2 Update desktop Tauri config and package scripts to bundle the staged CLI resource during packaged builds.

## 2. Runtime discovery and defaults

- [x] 2.1 Extend desktop executable path discovery to search packaged resource roots, including macOS app bundle resources.
- [x] 2.2 Default packaged desktop connections to the bundled AgentLink CLI name instead of an empty executable path.

## 3. Validation

- [ ] 3.1 Run focused validation for the touched surfaces (`cargo fmt`, desktop `cargo check`, `npm run build:ui`, and the staging helper). `cargo fmt`, `cargo check`, the targeted path-resolution test, and the staging helper passed; `npm run build:ui` is still blocked locally by the current Vite/Node runtime mismatch.
