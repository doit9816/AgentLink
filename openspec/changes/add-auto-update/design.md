## Context

AgentLink 桌面端是 Tauri + Vue 客户端，负责协调本地配置、通道绑定、bridge 启动、目标发现和 Agent 设置。当前更新仍是手动分发问题：用户拿到新的压缩包或安装包后自行替换桌面应用。这对早期开发可接受，但会拖慢通道修复的交付速度，也会在 Feishu/Lark + Codex 路径变化时增加支持步骤。

升级流程应位于桌面客户端界面中，而不是通道代码或 Engine/AgentSession 路径里。它必须保留现有 Channel 行为、已发现目标、入站回复上下文和三 Tab 布局。发布构建和更新元数据生成仍然是显式打包动作；常规检查必须继续使用已配置的 target 目录，并避免意外执行 release 打包。

本变更参考本地 `D:\go\src\cmsCloud\tools\codex-account-switcher` 的升级实现：Tauri v2 配置 `tauri-plugin-updater`，`tauri.conf.json` 中开启 `bundle.createUpdaterArtifacts`，在 `plugins.updater` 配置公钥和 GitHub Releases `latest.json` endpoint，前端使用 `@tauri-apps/plugin-updater` 的 `check({ timeout })` 与 `downloadAndInstall()`，release workflow 通过 `tauri-apps/tauri-action` 上传 installer、updater 签名和 `latest.json`。AgentLink 应复用这种成熟路径，但增加 bridge 正在运行时的安装门控。

## Goals / Non-Goals

**Goals:**
- 增加由 Tauri 支撑的升级工作流，用于检查、展示、下载和安装新的桌面端版本。
- 保存自动检查、最近检查状态等升级偏好，但不保存凭据。
- 在 `client/agentlink-desktop/src/App.vue` 中展示升级状态和操作，同时保留 `连接`、Channel、Agent 三个 Tab。
- 确保升级不会在没有用户确认的情况下打断正在运行的 AgentLink bridge。
- 在打包流程中增加发布元数据/签名校验要求。

**Non-Goals:**
- 更新任意编程 Agent 二进制、通道插件或外部平台凭据。
- 修改 Platform -> Engine -> AgentSession -> Agent 架构。
- 修改 SQLite 目标发现语义，或要求用户手动输入聊天标识。
- 把打包作为常规 `cargo check`、`cargo test` 或 `npm run build:ui` 的一部分。

## Decisions

1. 使用 Tauri updater 作为主要更新传输机制。

   桌面端已经优先通过 Tauri 作为后端边界，因此更新检查和安装应使用 Tauri v2 updater 插件，而不是从 CLI shell-out。具体实现可优先采用前端插件 API：`getVersion()` 获取当前版本，`check({ timeout: 15000 })` 检查更新，`downloadAndInstall()` 接收下载事件并更新进度，安装后通过 process 插件 `relaunch()` 重启。Rust 侧注册 `tauri_plugin_updater::Builder::new().build()`，并在 capability 中授予 `updater:default` 与必要的 `process:default` 权限。考虑过的替代方案：自定义 Tauri command 包装全部 updater 行为；保留为 fallback，仅在 Vue 直接调用插件无法满足 bridge 门控或错误归一化时使用。

2. 将更新状态保存在桌面端 settings/config 中，而不是通道配置中。

   更新偏好作用于整个桌面应用，不属于某个具体平台或项目。实现时应优先复用 `client/agentlink-desktop/src-tauri/` 中已有的配置/设置模式；如果没有，则增加一个小型桌面设置记录，包含 `auto_check_updates`、`last_update_check_at` 和最近一次非敏感错误等字段。考虑过的替代方案：把更新状态存入 SQLite；拒绝原因是目标发现表应继续聚焦平台会话和回复上下文。

3. bridge 进程运行时对安装进行门控。

   更新检查和下载可以在应用运行时发生，但如果 `agentlink.exe` 正在运行，安装/重启必须要求用户确认。桌面端已经拥有启动/停止控制，所以可以展示“停止后安装”操作，或先要求用户停止 bridge。考虑过的替代方案：自动强制停止 bridge；拒绝原因是可能打断正在进行的编码会话或消息回复。

4. 仅在显式打包/发布步骤中生成签名更新元数据。

   当用户请求打包时，打包脚本应生成 Windows 压缩包/安装包以及更新元数据和签名。AgentLink 当前已经有 `.github/workflows/release.yml` 使用 `tauri-apps/tauri-action`，应补充 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret，并保持 `updaterJsonPreferNsis: true`。`tauri.conf.json` 中应开启 `bundle.createUpdaterArtifacts`，配置 `plugins.updater.pubkey` 和 stable release 的 `latest.json` endpoint。常规验证仍是 `cargo fmt`、带 `H:\agentlink-target` 或 `H:\agentlink-desktop-target` 的 cargo check/test，以及 `npm run build:ui`。考虑过的替代方案：每次构建都生成更新元数据；拒绝原因是会混淆开发检查和发布产物，并产生不必要的文件。

5. 保持 UI 紧凑且偏操作型。

   升级控制应放入现有桌面工作流中，优先放在 `连接` Tab 的应用/运行状态附近，或一个紧凑的设置区域。UI 应展示当前版本、已知最新版本、发布时间/release notes、检查/下载/可安装状态、下载进度、手动检查、下载安装和重启操作。它不应变成落地页，也不能移除必需的 Channel 发送消息测试区域。

## Risks / Trade-offs

- [Risk] 早期版本的更新签名或元数据配置不完整 -> Mitigation: 失败时默认拒绝安装，在客户端显示清晰状态，并保留手动替换压缩包作为 fallback。
- [Risk] bridge 正在运行时安装会丢失进行中的工作 -> Mitigation: 检测 bridge 运行状态，并在重启前要求停止/确认。
- [Risk] 更新元数据 endpoint 变化或不可达 -> Mitigation: 手动检查报告失败，但不改变已保存凭据或通道状态。
- [Risk] Windows 文件锁阻止替换 -> Mitigation: 依赖 Tauri 的 Windows 更新/安装流程，并在安装前显式关闭 AgentLink 进程。
- [Risk] 自动检查增加启动延迟 -> Mitigation: UI 加载后异步检查，并尊重禁用/间隔偏好。

## Migration Plan

1. 增加默认更新设置：在有更新 endpoint 的 release build 中使用保守间隔启用自动检查；如果构建没有配置 endpoint，则默认禁用。
2. 现有用户继续正常打开桌面端；缺失的更新设置由默认值补齐。
3. 发布打包时为后续版本增加已签名的更新产物和元数据。
4. 回滚仍通过手动安装上一个已签名发布包完成；更新检查或下载失败不会修改现有 bridge/channel 配置。

## Open Questions

- 第一个公开更新通道是否直接使用 GitHub Releases `releases/latest/download/latest.json`，以及对应仓库地址是什么？
- 自动检查应在开发构建中默认开启，还是只在配置了 endpoint 的签名 release build 中开启？
- 第一版实现是否只支持 stable 发布，还是需要包含 preview channel 设置？
