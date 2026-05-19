## MODIFIED Requirements

### Requirement: Requested packaging only
除非明确请求打包或 release 输出，否则 AgentLink SHALL 不执行打包，也不生成 release 更新产物。

#### Scenario: User requests tests only
- **WHEN** 用户要求只运行测试、不打包
- **THEN** 工作流 SHALL 避免执行 release 打包命令

#### Scenario: User requests packaging
- **WHEN** 用户明确请求桌面客户端 release 包
- **THEN** 工作流 SHALL 生成 Windows release 包、updater 签名，以及 stable 更新通道使用的 `latest.json`

## ADDED Requirements

### Requirement: 更新产物校验
AgentLink release 打包 SHALL 在发布或交付 release 输出前，校验桌面端更新元数据是否指向已签名、兼容的 Windows 产物，并且不泄露 updater 私钥。

#### Scenario: 更新元数据校验成功
- **WHEN** 桌面端 release 包生成时包含更新元数据
- **THEN** 工作流 SHALL 验证元数据引用了已生成的 Windows 产物，并包含必需的签名信息

#### Scenario: updater 私钥由 CI secret 提供
- **WHEN** release workflow 生成 updater 签名
- **THEN** 工作流 SHALL 从 CI secret 读取私钥，而不是从仓库文件读取私钥

#### Scenario: 更新元数据校验失败
- **WHEN** 更新元数据缺失、未签名，或引用了不存在的产物
- **THEN** 工作流 SHALL 让 release 打包步骤失败，而不是产出可用于更新的 release
