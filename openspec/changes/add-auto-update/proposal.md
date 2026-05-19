## Why

AgentLink 桌面端用户目前在发布新版本后需要手动替换二进制文件，这会让通道连通性、Codex 集成、桌面绑定流程等修复较慢到达真实安装环境。增加自动升级路径后，可以降低支持成本，并让应用在不打断正在运行的 bridge 的前提下，安全交付已签名的版本更新。

## What Changes

- 增加桌面端自动升级能力，支持检查、展示、下载和安装新的 AgentLink 桌面端版本。
- 增加升级策略相关配置和 UI，包括手动检查、更新可用状态、关闭自动检查。
- 增加后端/Tauri 命令来处理升级状态和升级操作，让 Vue 客户端不把 shell 命令作为主要流程。
- 定义 Windows 更新包所需的发布元数据、签名和产物要求。
- 参考本地 `codex-account-switcher` 的 Tauri v2 updater + GitHub Releases `latest.json` 流程，适配到 AgentLink 的 Vue 桌面端和 bridge 运行状态。
- 增加失败处理，包括元数据不可用、下载错误、签名失败，以及 AgentLink 正在运行时延后安装。
- 非目标：本变更不为任意通道插件增加自更新，不要求常规开发检查时执行打包，也不发布外部参考项目说明。

## Capabilities

### New Capabilities
- `desktop-auto-update`: 覆盖更新发现、用户可见的升级流程、发布元数据校验、下载/安装行为，以及相关桌面端设置。

### Modified Capabilities
- `desktop-client`: 在保留现有三 Tab 工作流的同时，增加升级控制和状态展示。
- `build-test-workflow`: 增加更新发布元数据和校验要求，但不改变“只有明确请求时才打包”的规则。

## Impact

- `client/agentlink-desktop/src/App.vue`: 在现有桌面端工作流中增加升级状态和控制。
- `client/agentlink-desktop/package.json`: 增加前端 updater/process 相关 Tauri 插件依赖。
- `client/agentlink-desktop/src-tauri/`: 增加 `tauri-plugin-updater`，配置 `createUpdaterArtifacts`、updater endpoint/pubkey、capability 权限，并处理 Windows 安装流程。
- `.github/workflows/release.yml`: 复用 tauri-action 的 updater artifact 输出能力，注入 updater 私钥 secret 并上传 `latest.json`/签名产物。
- `src/` 或共享配置模块：如果复用现有配置形态，则保存升级偏好。
- `scripts/`: 增加或调整发布/更新元数据生成与校验脚本。
- `dist/`: 未来发布包包含更新元数据和已签名产物。
- 测试/检查：为升级状态处理增加聚焦的 Rust 和前端验证；打包仍保持显式触发。
