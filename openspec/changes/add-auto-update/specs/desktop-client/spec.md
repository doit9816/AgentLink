## MODIFIED Requirements

### Requirement: Three-tab structure
Tauri/Vue 桌面客户端 SHALL 保持 `连接`、Channel 和 Agent 三个 Tab 作为主结构，升级控制 SHALL 在不增加第四个主 Tab 的前提下展示。

#### Scenario: Editing the desktop app
- **WHEN** `client/agentlink-desktop/src/App.vue` 被修改
- **THEN** 主工作流 SHALL 仍可通过 `连接`、Channel 和 Agent 三个 Tab 使用

#### Scenario: Displaying update controls
- **WHEN** 展示桌面端升级状态或操作
- **THEN** 它们 SHALL 出现在现有桌面工作流中，且不替代 `连接`、Channel 或 Agent Tab
