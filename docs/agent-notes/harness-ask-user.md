# Ask User 选项面板验收

## 改动与焦点策略

带选项的待答问题现在按会话、按问题 ID 自动展开一次，包括首次打开会话时已经在等待的问题。live 层收到会话变更后沿用现有的窗口 `Invalidate`；composer 在读取新投影的这一帧构建 Kit 面板，不额外等待点击、定时器或下一次刷新。

| 用户正在做什么 | 面板行为 |
| --- | --- |
| 焦点不在可编辑区 | 展开并聚焦第一个可用选项，不勾选、不提交。 |
| 在 composer 或其他可编辑区输入 | 展开，但保留文本焦点。选项可直接点击；不按停顿时长猜测用户是否结束输入。 |
| 单选 | ↑/↓ 选择，Enter 确认。 |
| 多选 | ↑/↓ 移动焦点，空格勾选或取消，Enter 提交已勾选的集合。 |
| 选择「其他」 | 焦点移入自定义输入框。Enter 提交，Shift+Enter 保留换行。 |
| 按 Esc | 仅收起，不取消问题。点击「等待回答」可重开；同一 ID 不因重绘或重连再次自动展开。 |

键盘与「提交」按钮共用 `Actions.Answer`。新问题使用独立的控件身份；检查状态和未决 `QuestionReceipt` 不触发自动回答或重新提交。审批仍须明确决定，超时仍由 Node 处理。未修改协议、Node 或外部服务。

## 验证

- 修复前用新的 MyGo 用例复现了两个问题：初始待答问题没有显示选项，手动重开后首项也未获得焦点。修复后两项通过。
- 新增 composer 用例覆盖到达帧展开、连续问题、保持输入、单选与多选提交、自定义输入、Esc 后重开、重放和回执。
- Linux/X11 隔离预览：在已完成欢迎引导的测试配置中，无点击即看到选项，初始「提交」不可用；发送 ↓ 后第二项被勾选，Enter 后面板消失，原来的两个待审批请求保留。未调用真实模型。
- Linux amd64，取消 `GOROOT` 覆盖，使用 `GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。`go build ./...`、`go vet ./...` 均通过，无诊断。未运行 `go test ./...`。
- 仅对 `internal/desktop/{composer,shell,live}` 运行包测试。composer 全包通过。部分原有视觉测试假设初始没有弹层；改为先按 Esc 收起，再执行原来的断言。重跑 shell 的提问、审批、工具提示、加载标记及分栏用例，以及 live 的 `TestDemoMode`、`TestComposerSendRouting` 和 `TestQuestionReceiptChecksOriginalRequest`，均通过。
- **shell 全包并非全绿**：`TestSingleWorkspaceTitleAndTabTransitions`（单项标题仍有标签）与 `TestWorkspaceTerminalSameConversationSelection`（点击位置在窗口外）失败。用 Go overlay 换回修改前的两个 composer 源文件，两项在同一断言复现。未改动工作区行为或放宽断言。

## 仍需人工确认

在 macOS 和 Windows 实机使用中文输入法，在预编辑文字尚未上屏时接收问题，确认候选窗、插入点和后续文字都留在原输入框。Linux 预览与无头测试不能替代该项验收。
