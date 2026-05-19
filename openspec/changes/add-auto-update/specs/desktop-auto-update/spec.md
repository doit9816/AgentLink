## ADDED Requirements

### Requirement: 更新可用性检查
桌面客户端 SHALL 通过 Tauri v2 updater 检查是否存在更新的、已签名的 AgentLink 桌面端版本。

#### Scenario: 手动更新检查
- **WHEN** 用户从桌面客户端触发手动更新检查
- **THEN** 客户端 SHALL 报告当前版本是否已是最新、是否存在新版本，或检查是否失败

#### Scenario: 自动更新检查
- **WHEN** 已启用自动更新检查，并且配置的检查间隔已经到期
- **THEN** 桌面客户端 SHALL 在启动后检查更新，且不阻塞主 UI 工作流

### Requirement: 更新偏好
桌面客户端 SHALL 将非敏感更新偏好与通道凭据分开持久化。

#### Scenario: 自动检查已关闭
- **WHEN** 用户关闭自动更新检查
- **THEN** 后续启动检查 SHALL 被跳过，直到用户重新启用或运行手动检查

#### Scenario: 现有配置缺少更新设置
- **WHEN** 桌面客户端加载不含更新设置的现有配置
- **THEN** 客户端 SHALL 应用默认更新设置，且不改变平台凭据或已发现目标

### Requirement: 签名更新校验
updater SHALL 在安装更新前校验发布元数据和更新产物。

#### Scenario: 签名校验失败
- **WHEN** 更新元数据或下载产物校验失败
- **THEN** updater SHALL 拒绝安装并显示失败状态，且不修改已安装应用

#### Scenario: 不支持的平台产物
- **WHEN** 发布元数据不包含受支持的 Windows 桌面端产物
- **THEN** updater SHALL 报告没有兼容更新可用

### Requirement: 受控更新安装
桌面客户端 SHALL 在安装已下载更新前要求用户执行明确操作。

#### Scenario: Bridge 进程已停止
- **WHEN** 更新已下载，并且 AgentLink bridge 进程未运行
- **THEN** 客户端 SHALL 允许用户安装并重启到新版本

#### Scenario: Bridge 进程正在运行
- **WHEN** 更新已可安装，并且 AgentLink bridge 进程正在运行
- **THEN** 客户端 SHALL 要求用户在安装开始前停止或确认停止 bridge

#### Scenario: 安装后重启
- **WHEN** 更新安装完成并等待重启生效
- **THEN** 客户端 SHALL 提供明确的重启操作

### Requirement: 更新失败处理
桌面客户端 SHALL 将更新失败与通道和 Agent 工作流隔离。

#### Scenario: 更新检查失败
- **WHEN** 更新 endpoint 不可用或返回无效数据
- **THEN** 客户端 SHALL 展示更新失败，同时保留现有连接、Channel、Agent 和已发现目标状态

#### Scenario: 下载失败
- **WHEN** 更新下载失败
- **THEN** 客户端 SHALL 允许用户重试更新，且不要求重启桌面端
