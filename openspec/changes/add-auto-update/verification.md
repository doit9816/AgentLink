## 验证记录

- 更新检查成功：通过前端 `check({ timeout: 15000 })` 路径实现。需要在发布端存在有效 `latest.json` 后，用“连接”页“检查更新”按钮确认展示最新版本、发布时间和 release notes。
- 更新元数据失败：`checkForUpdates()` 捕获 updater 异常，写入 `updatePreferences.lastUpdateError`，仅显示错误提示，不修改 Channel 凭据、SQLite targets 或 Agent 设置。
- 下载失败：`installAvailableUpdate()` 捕获 `downloadAndInstall()` 异常，保留可重试状态并显示失败信息，不要求重启桌面端。
- Bridge 运行中安装门控：安装前调用 `bridge_runtime_status` 刷新状态；若 bridge 正在运行，必须确认停止，停止失败或仍在运行时拒绝安装。
- 发布产物校验：`scripts/validate-updater-artifacts.ps1` 检查 `latest.json`、签名字段、bundle 产物和 `.sig` 文件；`scripts/build-updater-artifacts.ps1` 仅在显式执行且环境提供 `TAURI_SIGNING_PRIVATE_KEY` 时生成 updater artifacts。
