## 1. 更新后端基础

- [x] 1.1 检查现有 Tauri 应用结构，并确定升级偏好的本地 settings/config 保存位置。
- [x] 1.2 在 `client/agentlink-desktop/package.json` 增加 `@tauri-apps/plugin-updater`，如需安装后重启则增加 `@tauri-apps/plugin-process`。
- [x] 1.3 在 `client/agentlink-desktop/src-tauri/Cargo.toml` 增加 `tauri-plugin-updater`，如需重启支持则增加 `tauri-plugin-process`。
- [x] 1.4 在 Tauri builder 中注册 updater/process 插件，并在 capabilities 中加入 `updater:default` 和必要的 `process:default` 权限。
- [x] 1.5 增加升级偏好数据结构，包含自动检查、最近检查时间和最近一次非敏感失败状态。
- [x] 1.6 为已签名的 Windows 桌面端发布配置 Tauri updater，且不在仓库文件中保存私钥或真实 secret。

## 2. Bridge 安全安装流程

- [x] 2.1 复用 `连接` Tab 使用的桌面进程状态，检测 `agentlink.exe` 是否正在运行。
- [x] 2.2 当 bridge 正在运行时，阻止或延后安装操作，直到用户确认停止/安装行为。
- [x] 2.3 确保更新失败不会修改通道凭据、SQLite 已发现目标或 Agent 设置。

## 3. 桌面端 UI

- [x] 3.1 在 `App.vue` 中使用 `@tauri-apps/api/app` 获取当前版本，并使用 updater 插件 `check({ timeout: 15000 })` 查询 stable 更新。
- [x] 3.2 在现有三 Tab 桌面工作流中增加紧凑的升级状态和控制。
- [x] 3.3 展示当前版本、可用最新版本、发布时间/release notes、检查中/下载中/可安装状态、下载进度，以及清晰的失败信息。
- [x] 3.4 增加手动检查、重试、下载安装、安装后重启和自动检查偏好控制。
- [x] 3.5 保留 Channel 的发送消息测试区域和多目标选择行为。

## 4. 发布元数据支持

- [x] 4.1 在 `tauri.conf.json` 中开启 `bundle.createUpdaterArtifacts`，配置 updater `pubkey` 和 stable release `latest.json` endpoint。
- [x] 4.2 更新 `.github/workflows/release.yml`，为 tauri-action 注入 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，并确认 `updaterJsonPreferNsis: true`。
- [x] 4.3 增加或更新 release 辅助代码/脚本，使显式打包可以输出更新元数据和签名产物。
- [x] 4.4 增加校验，确认更新元数据引用已生成的 Windows 产物，并包含必需签名数据。
- [x] 4.5 保持常规开发检查与 release 打包、更新产物生成相互独立。

## 5. 验证

- [x] 5.1 Rust 修改后运行 `cargo fmt`。
- [x] 5.2 桌面端 Tauri 修改后，使用 `CARGO_TARGET_DIR=H:\agentlink-desktop-target` 运行合适的 Rust 检查。
- [x] 5.3 如果全局 Node 版本过旧，使用配置的 Node 路径运行 `npm run build:ui`。
- [x] 5.4 为更新检查成功、元数据失败、下载失败、bridge 运行时安装门控增加聚焦测试或手动验证记录。
