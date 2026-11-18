# 已删除能力与测试审计

本文记录 `feat/mygo-desktop` 上 SSH 连接资源、本地数据库、Office 文档解析和本地推理（含 OCR）的删除边界，以及后续测试清理和生产侧残留清理。删除实现的原始提交分别为 `379c1e0b`、`8c8998c8` 和 `ba05c6a9`；本文重点审计 `bbaa29b6`、`cacb9e2b`、`9ec058c9` 三个测试清理提交。

## 1. 能力删除范围

### SSH 连接资源

- **生产包与界面**：删除 `internal/ssh`、Node 的 SSH 资源/服务、SSH 文件传输与 SSH 终端实现、相关 store 读写、桌面 SSH 主机/凭据/传输界面，以及内置 `plugins/ssh` 包。
- **运行期命令**：`list_ssh`、`save_ssh`、`remove_ssh`、`check_ssh`、`trust_ssh`、`run_ssh`、`browse_ssh_directory`、`download_ssh_file`、`modify_ssh_file`、`transfer_ssh`、`stage_ssh_upload`、`finish_ssh_upload`、`update_ssh_host`、`cancel_ssh`、`list_ssh_terminals`、`open_ssh_terminal`、`close_ssh_terminal` 不再进入授权白名单或日志分类，也不再被运行期接受。
- **工具与提示词**：SSH 执行、传输和连接管理工具不再注册；助手资源绑定和普通工具上下文不再包含 SSH 特例。
- **设置项**：删除 SSH 主机、凭据和连接资源编辑入口。
- **本次残留清理**：移除 `authorization.go` 和 `logging_commands.go` 中整组旧命令；删除 store 中无调用方的 `bindSshAuthentication` 和 `ListSshTerminals`；改写 Host/WSL 注释中已过时的 SSH 安装、授权描述。

### 本地数据库连接与查询

- **生产包与界面**：删除 `internal/databases`（SQLite/MySQL/PostgreSQL 驱动与传输）、Node 数据库服务和 profile/credential 操作、桌面连接界面中的数据库部分，以及内置 `plugins/databases` 包。
- **运行期命令**：`list_databases`、`save_database`、`remove_database`、`check_database`、`test_database`、`browse_database`、`query_database`、`cancel_database` 不再进入授权白名单或日志分类。
- **工具与提示词**：数据库查询、执行和连接管理工具不再注册；助手和插件运行输入不再暴露数据库能力。
- **设置项**：删除数据库 profile、连接测试和查询入口。
- **本次残留清理**：移除授权与日志清单中的旧命令；删除无调用方的 `databasePath`、`bindDatabasePath` 及只服务于旧连接资源的 host-path 辅助函数。

### Office 文档解析

- **生产包与界面**：删除 `internal/office`、Node 的 Office/PDF 文档运行桥接、知识读取中的 Office 提取路径，以及内置 `plugins/office` 包和 Word/Excel/PowerPoint/PDF 技能。
- **运行期命令与工具**：`read_office` 不再进入授权白名单或日志分类；`office.read` 不再注册为可用工具操作。
- **设置项**：无独立保留的 Office 设置页；相关文档解析运行时入口随能力一并删除。
- **提示词**：附件水合提示不再要求模型调用 `read_office`，改为使用支持附件的工具和通用 attachment source。

### 本地推理（OCR、embedding、decision 与语音）

- **生产包与界面**：删除 `internal/localmodels`（含 OCR、ONNX、embedding、decision、模型资产与安装）、`internal/speech`、Node 本地模型服务/运行时、模型与听写设置界面，以及相关 SDK 和插件脚本。
- **运行期命令**：`read_local_models`、`read_local_model_settings`、`save_local_model_settings`、`register_local_model`、`install_local_model`、`refresh_local_model`、`delete_local_model`、`cancel_local_model` 不再进入授权白名单或日志分类。
- **工具**：`list_ocr_models`、`install_ocr_model`、`uninstall_ocr_model` 等 OCR 包工具不再注册；本地模型、安装和决策脚本接口一并移除。
- **设置项与预览**：设置模型分组不再提及 OCR；安装队列预览不再展示 OCR 或其他已删除的本地模型安装项。
- **知识与提示词**：本地语义索引及模型提取路径删除；知识 source 的 `Extractor` 注释改为通用提取修订标识，不再声称它标识 OCR 模型。

## 2. 对应测试用例审计

### `bbaa29b6`：OCR 工具残留

| 用例名 | 已删能力 | 处理方式 |
| --- | --- | --- |
| `TestAgentManagementTools`（含 `full permission`、权限拒绝和 planning mode 场景） | OCR 模型管理 | 删除 `list_ocr_models` / `install_ocr_model` / `uninstall_ocr_model` 的发现、调用、结果和审批断言；审批数由 21 调整为 19；权限拒绝场景改用仍存在的 `roles.drop`，planning mode 删除 OCR 安装断言。 |

### `cacb9e2b`：本地语义索引集成测试

| 用例名 | 已删能力 | 处理方式 |
| --- | --- | --- |
| `TestKnowledgeStorage`（含 `memory tools feed an automatic collection`） | 本地 embedding / 语义索引 | 整项删除，`knowledge_storage_test.go` 文件随实现移除。 |
| `TestKnowledgeUserAndProjectStorage`（含 `storage collections are scoped to their owner`、`collections leave with their owner`） | 本地 embedding / 语义索引 | 整项删除，`knowledge_users_test.go` 文件随实现移除。 |

### `9ec058c9`：SSH、数据库与 Office 残留

| 用例名 | 已删能力 | 处理方式 |
| --- | --- | --- |
| `TestAssistantPreferencesKeepOrdinaryToolDiscovery` | SSH | SSH 绑定助手 fixture 改为普通 `notes` 助手；删除 bound-resource 提示断言。 |
| `TestChildStandardTools` | 数据库 | 两个“非允许操作”fixture 从 `QueryDatabase` 改为仍存在的 `ApplyRoles`。 |
| `TestCommandLogClassification` | SSH、数据库 | 删除 `save_ssh` / `save_database` 的 secret 类型与日志标识符 fixture，保留其他命令的日志覆盖检查。 |
| `TestAdmissionSameRequestID` | SSH、数据库 | 对已删除但仍可解码的 `save_ssh` / `save_database` 跳过运行期 ledger fixture 对照。 |
| `TestOutcomeUnknownAfterCrash` | SSH | 未决请求 fixture 从 `save_ssh` 改为 `save_search_settings`，继续覆盖崩溃后的 unknown 状态。 |
| `secretCommands`（测试 fixture） | SSH、数据库 | 删除 SSH/数据库凭据 payload；调度回调改用 `save_provider`。 |
| `pluginAssistantAcceptance`（验收辅助函数） | SSH | 删除 SSH 会话和 `save_ssh` fixture；普通本地工具与审批改用插件助手会话验证。 |
| `TestUnassignedReminderToolKeepsDiscoveryLabels` | SSH | 删除通过 SSH resource 验证上下文准入的尾部断言，保留发现标签测试。 |
| `TestAssistantToolsAddOrdinaryContext`（`ssh`、`database` 子测试） | SSH、数据库 | 整项及 `plugin_tools_context_test.go` 删除。 |
| `TestPluginTurnInput` | SSH、数据库 | flow 操作 fixture 从数据库查询/SSH 传输改为文件读/写。 |
| `testToolDeclarations`（测试 fixture） | SSH、数据库 | 删除 `ExecuteDatabase → ControlDatabases` 和 `RunSsh → ControlSsh` 映射样例。 |
| `TestBundledPackageContracts` | SSH、数据库、Office | 内置包数量从 27 改为 24，对应三个退役包。 |
| `TestFailureOverridesSuccessPreview` | SSH | 工具展示 fixture 从 `ssh_save_connection` 改为 `write_file`，继续验证错误优先于成功预览。 |
| `TestTelemetryOffSendsNothing` | SSH、数据库 | 删除 `save_database` / `save_ssh` 秘密遥测样例，保留 provider 等仍有效命令的覆盖。 |
| `TestTerminalInventory` | SSH 终端 | 删除 SSH 终端循环、`list_ssh_terminals` / `close_ssh_terminal` 调用；本地终端的结束状态、快照和关闭行为覆盖保留。 |

### 本次运行期清单清理的同步测试调整

| 用例名 | 处理方式 |
| --- | --- |
| `removedCapabilityCommand`（测试辅助函数） | 集中列出仍保留 wire 声明、但运行期已删除的 SSH、数据库、Office 和本地模型命令。 |
| `TestCommandAuthorizationCoverage` | 跳过协议保留命令后再检查授权覆盖；同时仍拒绝白名单中的额外/未知命令。 |
| `TestCommandLogClassification` | 在命令分类和 secret 推导循环中跳过协议保留命令，避免把 wire 兼容误认为运行期支持。 |
| `TestAdmissionSameRequestID` | 复用统一排除函数，不再只特判 `save_ssh` / `save_database`。 |

## 3. 协议保留项

`internal/protocol` 是共享 wire 契约，本分支不删除其中的旧类型或枚举，包括：

- `message.go` / `requests.go` 中旧 command kind、payload/output 注册和 durable 元数据；
- `tool.go` 中 SSH、数据库、`office.read` 等 `ToolOperation`；
- `plugin_action.go` 中 `ssh.*`、`databases.*` 等 `PluginAction`；
- `models.go` 的 `ModelsKindOcr`、`feature_use.go` 的 `FeatureUseSettingsOCR`；
- `SshProfile`、`DatabaseProfile`、`SshTrust`、终端的 SSH 字段及关联 ID/union 类型。

保留的原因是让旧客户端请求、历史请求账本、存储事件和升级数据仍可解码；**可解码不代表可执行**。`authorization.go` 已不再接纳这些命令，插件 manifest 的 `retiredActions` 和 `plugins.RetiredBundled` 也继续作为显式退役防线保留。

同理，store 中旧表名、升级/校验逻辑和历史用量名称解析仍可读取既有数据；`admission.DigestOnly` 对 `save_ssh` / `save_database` 的摘要规则保留，避免旧秘密请求在兼容路径中泄露原文。这些不是恢复能力。

以下 SSH 字样也与已删除的 SSH 连接资源无关，明确保留：

- `process/environment.go` 的 `SSH_AUTH_SOCK` 环境变量透传；
- `process/sandbox.go` 对 `.ssh` 目录的屏蔽；
- `git/worktrees.go` 和插件仓库地址校验中的 `ssh` URL scheme。
