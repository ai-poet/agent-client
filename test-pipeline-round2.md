# Tests 第二轮修复报告

基线：[`5160b9e44`](https://github.com/ai-poet/agent-client/commit/5160b9e44)，[Tests run 44913928322](https://github.com/ai-poet/agent-client/actions/runs/44913928322)。未改 tag、`.github/workflows/release.yml`、`CHANGELOG.md` 或原生依赖。

## 超时

- `.github/workflows/test.yml` 将 `go test` 的每包超时从默认 10 分钟提高到 30 分钟；macOS、Windows 矩阵、90 分钟作业上限和原生库步骤不变。
- Shell 在两个平台都触及默认上限。Windows 停在执行了 23 秒的 `TestProjectEditorAndRemoval`，栈在 MyGo 软件光栅化；macOS 停在刚开始的 `TestSidebarSessions/SidebarRenameKeepsEmptyTitleOut`，栈在布局。两者都是可运行的渲染栈，没有显示等待锁。
- Windows Node 到时仍在执行 `TestBackupRoundTrip`、`TestRestoreIntoEmptyProfile`、`TestSwapRestoredProfile` 与 `TestFreezeBackupReopensAfterFailure`。前三个有对应的备份恢复文件创建或 `FlushFileBuffers` 栈；最后一个刚进入取消子用例，正在打开 Turso 库。没有看到循环等待。Linux 定向复跑中，两个 Shell 测试及四个 Node 测试均完成；这不能替代 Windows/macOS 对新预算的验证。

## Desktop 投影和交互

| 失败 | 处理 |
| --- | --- |
| `TestProviderRouteProjection`、`TestLegacyCompatibleProviderRouteProjection` | 保留内建 `/0`、`/1` 路由、API、配置和模型元数据检查；兼容方式使用可选密钥，非 Azure 路由没有 API 版本。仍覆盖旧方式在替代路由不可用时保留。 |
| `TestConversationProjection/context_estimate_and_bound_mentions` | 用有效容量区分估算上下文和实测 token，保留不同分母及引用断言。 |
| `TestCheapSignInAddsProviderAndSignOutDisablesOnlyIt`、`TestCheapGatewayWriteSurvivesRevisionRace` | 点击退出登录前滚动账号页。保留其他提供商不变和修订冲突后的写入验证；Windows 仍检查凭据文件为普通文件，仅不比较 POSIX 模式。 |
| `TestCheapSidebarAccountAndGroupSwitchEndToEnd` | 侧栏一级菜单可显示当前已绑定组，但不会展开未选组。通过通道菜单检查切换、创建及复用密钥、模型刷新、其他通道不变、拒绝后的回滚、本地保存失败时不写入 Node。 |
| `TestComposerSendRouting/composer_parity_commands`、`TestCloseShortcut/ConversationHeaderAndKeys` | 用实际可访问的重置按钮检查 ⇧⌘M 打开模型控制面板，不依赖当前模型是否渲染推理档位。 |
| `TestComposerSendRouting/failure_retains_draft` | 在空编辑器上检查明确拒绝后自动恢复草稿，无需点击已不存在的提示按钮。 |
| `TestPanelProjection/panel_tabs_close,_stay_closed_and_never_show_a_blank_body` | 让脚本模型完成首轮，在 Client 确认完成后再归档。保留归档结束终端和清除面板的检查。 |

## Node、Store 和知识库

| 失败 | 处理 |
| --- | --- |
| `TestCompactionHandoff/failures_are_recorded/budget` | 用受支持的最小上下文及不可拆分的超长最新输入触发真正的预算拒绝；验证未请求模型且仅写一条失败记录。旧的超小上下文会被视为未知并使用估算值。 |
| `TestCompactionHandoff/overflow_continuation` | 检查压缩后继续调用工具并保留最新原始任务；已有摘要和重新注入的上下文仍不当作用户输入。 |
| `TestChildSettlementRefreshesAgedDelegation` | 验证过页子任务通过父会话的 `child` 增量帧报告确切任务及取消状态，不再期待整页失效。 |
| `TestGuidanceQueue/queue_commands` | 确认有等待项时删除和归档都被拒绝，队列不变；完成执行并移除剩余队列项后再检查可归档。 |
| `TestGuidanceQueue/other_settings_stay_ordinary` | 区分沿用运行中设置的直接引导和保留原设置的已排队输入，验证投递对象和最终领取顺序。 |
| `TestStalledDelivery` | 每条命令只报告一次停滞；用第二条命令验证自动跟进计数、停止后的保留证据和用户输入恢复。 |
| `TestBackgroundNoticeEvidence/.../reserved` | 已收集的结果替代尚未投递的停滞证据（包括已预留的），但不改写已投递的记录；验证结果已消费，不重复提醒。 |
| `TestCollectionLifecycle` | 模型选择只影响索引绑定，不上调集合修订或代次；在受控索引扫描下验证未安装时的关键词检索、安装后的嵌入索引及后续更新。 |
| `TestCommandLogClassification` | 总数计入 3 条工作区读取命令，覆盖全部 334 条注册命令；原有 16 条摘要记录规则和检查保留。 |

## Windows 专有失败

| 失败 | 处理 |
| --- | --- |
| `TestPKCEExchangePersistsOwnerOnlyCredentials` | Windows 不比较合成的 POSIX mode bits；仍验证凭据存在、可以重新读取以及登录状态。不是 ACL 验证。 |
| `TestFirstLaunchSilentAutoMigrationAndIdempotency` | 扫描整个 profile 前关闭窗口和 Node，先释放 `node.lock`；同时改用原生绝对项目路径并明确核对首次导入的项目、会话和消息数。 |
| `TestRunMigratesBacksUpAndSkipsCompleted`、`TestRunFillMissingOnlyAddsAbsentRows`、`TestRunSupportsSplitReleaseDatabase`、`TestInterruptedMigrationRecoversWithoutDuplicates`、`TestDetectAndRunWithSpacesAndNonASCIIPath` | 用 `t.TempDir()` 构造项目路径。Windows 没有卷名的 `/tmp/...` 不算绝对路径，迁移器原本拒绝该项目和关联数据是正确行为。 |
| `TestNativeOutputEncoding/{UTF-16,OEM,UTF-8_BOM}` | 通过实际 retained stream 写入字节，验证解码文本、原始字节数、转码标志及 UTF-16 拆分偏移的拒绝结果。OEM 用固定的 CP437，避免依赖 CI 的控制台设置。 |

## 验证与剩余风险

- **Linux，Go 1.27.3 / `CGO_ENABLED=0`**：复用已存在的 Turso 动态库，按包运行本报告列出的既有回归测试。通过受影响的 Desktop live 投影、发送、面板和 CheapRouter 交互，Shell 快捷键和超时样本，Node 命令分类、四个备份恢复超时样本、Engine、Store、知识库，以及 `cmd/sailry-desktop`、`internal/migrate`、`internal/cheapauth`、`internal/node/process` 的包测试。没有运行本机 `go test ./...`，没有重编原生库。
- **中间运行**：并行冷编译时，压缩套件有一个子用例触发既有的 5 秒夹具期限，单独重跑整个压缩套件通过。侧栏分组测试在夹具修改期间曾失败一次，随后连续通过六次，最终定向测试集也通过。
- **Windows amd64**：用 `GOOS=windows GOARCH=amd64 go test -c` 成功编译上述四个 Windows 专有失败对应的测试包；未在本机运行 `.exe`，不能据此确认编码转换、文件锁、ACL 或 Windows UI 行为。
- **静态检查**：变更的 Go 文件通过 `gofmt` 检查，`git diff --check` 通过，Tests YAML 可解析。
- **待 GitHub Actions 确认**：macOS 和 Windows 上的全套测试、实际原生库加载，以及原超时包是否在新预算内跑完。Linux 无头测试不等同于原生窗口验收。未新增测试门禁。
