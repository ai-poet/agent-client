# MyGo 工作台交接说明

本文档说明 `feat/mygo-desktop` 上工作台核心做到了哪、还差什么，方便接手继续做。不要把 Go 桌面设为默认发版客户端，也不要合入 `test`/`main`，除非老板另有原话。

## 仓库与提交

- 仓库：`ai-poet/agent-client`，本地路径 `/workspace/repos/agent-client`
- 分支：`feat/mygo-desktop`（跟踪 `origin/feat/mygo-desktop`）
- 已推到远程的最新提交：`d47fc2b7`（第一期薄壳 + Tests 里的 `go-desktop` job）
- 本机后续提交（交接时可能尚未 push）：
  - `40157d34` `feat(desktop): 补齐协议客户端与转录投影（工作台 UI 未完成）`
  - 本说明文档的提交

先 `git status` / `git log` 确认本地是否仍 ahead，需要从 GitHub 拉时再 push，不要 force。

## 已完成

第一期（`d47fc2b7`）已具备：

- `desktop/` 下 MyGo v0.9.2 原生 GPU 壳，窗口标题 CheapRouter，文案中文
- 沿用现有 Rust `client-daemon`：环境变量 `WAKU_DAEMON_ADDRESS` / `WAKU_DAEMON_TOKEN`（必须成对）、`WAKU_DAEMON_PATH`、`WAKU_APP_EXECUTABLE`
- 未设地址/token 时监督启动本地 daemon：`127.0.0.1:0`、`--parent-pid`、读首行 readiness JSON、退出时 shutdown
- Tests 在 macOS / Windows 上跑 `go test ./...` 与 `go build ./...`；`release.yml` / `sync-release.yml` 未改，现网发版仍是 GPUI 客户端
- 远程开发环境用 `WAKU_DAEMON_PATH` 指向 `target/debug/client-daemon` 后，窗口能显示「已连接」

`40157d34` 已补上协议客户端（均在 `desktop/internal/daemon`，不要把 UI 写进这个包）：

- `ConnectWithResume`、按 `requestId` 的 RPC 路由、RPC 错误与超时、连接关闭时未完成请求立即失败
- sequenced `event` 按 `session/runtime/epoch/sequence` 去重，重连 hello 带 `resumeFrom`
- 工作台封装：`LoadTaskState` / `HydrateSession` / `AttachSession` / `Prompt`，UI 不要自己拼 JSON
- 单测：`replay_test.go`、`client_rpc_test.go`、`workbench_test.go`；`go test ./internal/daemon` 通过

`desktop/internal/shell` 已有转录内存投影（`transcript.go` + `transcript_test.go`）：hydrate 时按 `after_message` 恢复顺序，流式事件追加或更新对应条目。尚未接到界面，也还没用 MyGo 官方流式 Markdown（`Stream`）渲染。

## 必须先修的编译断裂

当前 `desktop/` 下 `go test ./...` 与 `go build ./...` **不能通过**。根因是协议客户端改了接口，薄壳 UI 还没跟上：

1. `daemon.Supervisor.Connect` 已变为 `Connect(ctx, resume []ReplayCursor)`，`shell.Connector` 仍是 `Connect(ctx)`，`cmd/cheaprouter-desktop` 因此编不过。
2. `daemon.Connection` 已增加 `LoadTaskState` / `HydrateSession` / `AttachSession` / `Prompt` / `Events` / `ReplayCursors` 等，`shell/model_test.go` 里的 `fakeConnection` 未实现这些方法。

建议：重连时把上一连接的 `ReplayCursors()` 传给 `Connect`；网络/进程工作仍离开 UI 线程，帧只读内存快照。修完后这两条命令都必须通过：

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test -count=1 ./...
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go build ./...
```

## 本阶段还要做的事（按优先级）

先修编译，再按这个顺序，不要跳到后面阶段。

1. **把协议客户端接到壳上**
   - 更新 `shell.Connector` / `Model` / 假对象，使重连带上 `resumeFrom`
   - 连接断开后用已记录的游标恢复；迟到的 RPC/事件不要覆盖新连接或新会话代次
   - 身份状态：现有协议没有独立账号登录命令，只展示 daemon 握手认证与连接身份，不要另造登录协议，也不要做网页登录或充值扫码

2. **工作台核心 UI**（`desktop/internal/shell`，MyGo 原生 `package ui`，不要用 HTML/webview）
   - 会话列表：列出 / 选中 / hydrate（走已有封装）
   - 转录区：用 MyGo v0.9.2 官方流式 Markdown（`Stream`）渲染投影后的会话消息与事件
   - 输入框：走 `Prompt`（以及发送前需要的 `AttachSession`），支持基本发送和失败提示
   - 界面测试用 `ui.NewTester`，不要依赖真实窗口或显示器
   - 更新 `desktop/README.md`：写清已具备的能力、运行方式、如何连接已有 daemon，以及仍未迁移的部分

3. **若 1–2 可测且稳定，再补**
   - 内置终端：官方终端 / Ghostty 插件；必须在 `CGO_ENABLED=0` 下能编过、能测
   - diff 查看：官方 diff / 编辑器能力
   - 若终端或 diff 在无 cgo / 无显示器环境下无法稳定测试，先做成可编译模块并写清限制，不要引入 cgo，也不要因此把 1–2 搞坏

## 硬约束

- 协议唯一真相：`crates/client-protocol`（`PROTOCOL_VERSION=6`）。JSON 字段名必须与现有 Serde/TS 契约一致；持久化结构（如 `Project` / `AgentSession` / `Message`）是 snake_case。不要发明新的 type，不要改 Rust 协议。
- MyGo 钉死 `github.com/egoist/mygo v0.9.2`，Go `1.27.3`，构建和测试一律 `CGO_ENABLED=0`、`GOTOOLCHAIN=go1.27.3`。不要把 lobe-mygo 当地基或加进依赖。API 以模块源码和文档为准，不要臆造。
- 只改 `desktop/` 下的 Go 代码、测试和 `desktop/` 文档。可以小幅补充 `.github/workflows/test.yml` 里已有的 `go-desktop` job，但不要改 Rust 测试矩阵，不要改 `release.yml`、`sync-release.yml`、`claude.yml`，不要新增 Linux 官方发版矩阵。
- 不要改 `crates/`、`src/`、现有 GPUI 客户端。尤其不要修 `origin/test` 上已有的 Windows Rust 失败（`js_repl::tests::file_urls_use_windows_and_posix_path_rules`），那不是这期的范围。
- 界面文案中文，窗口标题仍是 CheapRouter。这仍是内部狗食客户端。

## 不要在本轮做

这些是更后面的阶段，不要一并实施：

- 设置页全文、充值/扫码、公告、模型广场、teams UI、画图、电脑操作工作台、自动更新器
- 把 daemon 换成 Go Node，或分批替换执行层
- `~/.cheaprouter` 数据迁移器（会话、设置、登录态、自定义接口、技能、项目配置）
- 把该分支合入 `test`/`main`，或把 Go 桌面设为默认安装包 / 发版产物

## 本地运行（编译修复之后）

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

连已有 daemon：

```sh
WAKU_DAEMON_ADDRESS=127.0.0.1:4312 \
WAKU_DAEMON_TOKEN=... \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

开发时指定刚编译的 daemon：

```sh
WAKU_DAEMON_PATH=/path/to/client-daemon \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

默认不 push。完成并核对测试后，等老板原话批准再推。
