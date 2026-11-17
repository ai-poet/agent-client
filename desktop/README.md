# CheapRouter MyGo desktop shell

这是 CheapRouter 工作台的 Go + MyGo v0.9.2 原生桌面壳，也是渐进迁移中的内部狗食客户端。它通过现有 Rust `client-daemon` 的 WebSocket `/v1` JSON 协议工作，不替换现网 Rust/GPUI 客户端，也不进入默认发版产物或升级通道。

## 分层与数据流

- `internal/daemon`：协议、WebSocket 连接、RPC、事件重放和本地 daemon 进程监督。协议唯一真相是 `crates/client-protocol`，当前协议版本为 6；这里不另造 wire 类型或协议。
- `internal/workbench`：纯状态投影，负责连接与会话代次、事件水位与去重、hydrate 期间的事件缓冲、按会话保存的草稿，以及发送态。它不依赖 MyGo/CrychicUI，不做 I/O，也不持有连接。
- `internal/shell`：原生视图、输入收集和 I/O 驱动。它把用户操作转成投影命令，在后台执行连接、RPC 和事件消费，再在 UI 线程应用结果并渲染内存快照。

```text
用户输入
   │
   ▼
internal/shell ── RPC / I/O ──▶ internal/daemon ── WebSocket /v1 JSON（v6）──▶ Rust client-daemon
   │                              │
   │ ◀──── 连接结果 / 事件 ────────┘
   │ 在 UI 线程应用结果
   ▼
internal/workbench（纯状态投影，无 UI、无 I/O）
   │ Snapshot
   ▼
MyGo ui + CrychicUI（原生视图）
```

目录访问、RPC 和事件消费均在后台执行，界面帧只读内存快照；过期连接或会话的结果由代次与游标隔离。

## UI 组件分工

MyGo 自带的 `ui` 是布局与渲染底座，CrychicUI 是其上的控件层；两者不是两套并行的窗口或 UI 实现。

- **MyGo `ui`**：`Column` / `Row` 负责布局，`List` 负责转录虚拟列表与滚动；流式 Markdown 使用 `markdown.Stream`，完成后切换为 `markdown.View`。
- **CrychicUI**：`data` 提供会话列表，`chat` 对应聊天气泡与 composer，`feedback` 提供空、加载、错误三态，`input` 提供输入框。

这两处看起来“可以统一”，但故意不统一，理由是：

- **转录区继续用 `ui.List`**：每个气泡的行内内容是 `markdown.Stream` 的增量渲染结果，需要和列表的行身份、滚动锚点在同一层绑定（`ListState.Key` 按消息 ID 保持滚动位置，流式追加时只增长当前行）。换成 `data` 的列表会把 Markdown 的增量状态和行状态拆到两层，滚动跟随和「流式不重复」都会退化。
- **会话列表用 `data` 包**：它只需要「虚拟化 + 选中态 + 点击」，没有富文本绑定，用 CrychicUI 的列表能直接拿到选中强调样式与键盘导航，不必自己实现。

`go.mod` 中锁定的直接依赖版本：

```text
github.com/egoist/mygo v0.9.2
github.com/ELY-Open-Source/CrychicUI v0.0.0-20261110123803-1c55e6a884bb
```

## 已具备能力

- **连接**：握手认证、daemon 版本展示、失败重连；重连携带已消费事件的 `resumeFrom` 游标。
- **会话列表与 hydrate**：展示名称、最后消息时间和选中会话，通过 `attachSession` / `hydrateSession` 在后台载入，并防止迟到结果覆盖新选择。
- **流式转录**：按持久化消息和 `after_message` 的顺序展示用户、助手、工具与系统条目；流式事件按水位去重并增量更新当前条目。列表按稳定条目 ID 保留 Markdown 解析和滚动状态；用户位于底部时自动跟随，上滑后保留视口。
- **输入与发送**：Enter 发送、Shift+Enter 换行；发送走现有 `Prompt` 封装，缺少 runtime 时先 `AttachSession`。发送期间禁用输入并显示「发送中…」，失败时保留并恢复原文；草稿按会话保存。
- **三态反馈**：会话与转录均有空态、加载态和错误态，载入失败可重试。

## 本地运行

要求 Go 1.27.3。正常构建和测试使用 `CGO_ENABLED=0`，不需要 Bun、webview 或 cgo。

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

### 连接已有 daemon

沿用现有 Rust 桌面的环境变量。`WAKU_DAEMON_ADDRESS` 与 `WAKU_DAEMON_TOKEN` 必须成对设置：

```sh
WAKU_DAEMON_ADDRESS=127.0.0.1:4312 \
WAKU_DAEMON_TOKEN=... \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

### 由壳拉起本地 daemon

未设置地址和 token 时，壳会启动并监督本地 daemon：使用 `127.0.0.1:0`、`--parent-pid`、自动生成的 `WAKU_DAEMON_TOKEN` 和 `WAKU_APP_EXECUTABLE`，读取首行 readiness JSON，并在退出时请求 shutdown。

daemon 默认取桌面可执行文件旁的 `client-daemon`（Windows 为 `client-daemon.exe`）；开发时可用 `WAKU_DAEMON_PATH` 指定现有构建产物：

```sh
WAKU_DAEMON_PATH=/path/to/client-daemon \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

## 测试与构建

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test ./...
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go build ./...
```

对 shell 和纯状态投影运行 race detector：

```sh
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=1 go test -race ./internal/shell/ ./internal/workbench/
```

这里的 `CGO_ENABLED=1` 只用于 Go race detector，不代表桌面壳引入了 cgo 依赖。

协议测试使用进程内假 WebSocket 服务器；进程约定测试使用测试二进制的 helper process；界面测试使用 `ui.NewTester`，覆盖转录、滚动、输入和发送状态，不会打开真实窗口，因此也能在无显示器的 Linux 环境运行。

帧截图同样由无头测试生成，不是默认测试产物：

- `TestCrychicFrame`：设置 `CHEAPROUTER_FRAME_PATH` 后输出完整工作台帧，未设置时跳过。
- `TestCrychicTranscriptLongStringWrapping`：始终执行长串换行断言；设置 `CHEAPROUTER_LONGSTRING_FRAME_PATH` 后额外输出对应 PNG。例如：

  ```sh
  CHEAPROUTER_LONGSTRING_FRAME_PATH=/tmp/cheaprouter-longstring.png \
  GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test ./internal/shell/ \
    -run TestCrychicTranscriptLongStringWrapping -count=1 -v
  ```

GitHub Actions 的 **Tests** workflow 在 macOS 和 Windows 上运行 Go 测试及构建；原有 Rust、protocol 和 browser client 检查保持不变。Go job 失败会使 Tests 失败。与现有发布矩阵一致，这里没有新增 Linux 发布。

## 尚未迁移

按 [HANDOFF.md](HANDOFF.md) 的分期，以下能力仍未迁移，不属于本轮交付范围：

- 内置终端、diff 查看；
- 设置页、充值扫码、公告、模型广场、teams、画图、电脑操作；
- 更新器；
- `~/.cheaprouter` 数据迁移器。

也不在本阶段把 daemon 换成 Go Node，或把 Go 桌面切为默认客户端。

## 当前限制与发布边界

- 不使用 cgo，不嵌入 webview；界面由 MyGo 原生渲染。
- 当前验证以假服务器、进程 helper 和无头 UI 测试为主，尚未完成真实后端联调，也未做真实窗口中的交互点击测试。
- 当前自动发版仍只发布现有 Rust/GPUI 客户端。`release.yml`、`sync-release.yml`、发布产物、升级通道和 appcast 均未改；Go 壳只随 Tests 自动测试和构建。

接手时请先读 [HANDOFF.md](HANDOFF.md)：其中记录了迁移背景、后续分期，以及协议、现有代码和 MyGo API 的参考路径。
