# CheapRouter MyGo desktop shell

这是渐进迁移中的 MyGo 原生工作台：`package ui` 绘制原生 GPU 界面，Go
客户端沿用现有 Rust `client-daemon` 的 WebSocket `/v1` JSON 协议（协议版本 6）。
MyGo 固定为 `github.com/egoist/mygo v0.9.2`，没有新增协议、wire 类型或登录 /
充值 / 设置接口。它仍是内部狗食客户端，不是默认发版客户端。

## 已具备能力

- 连接与身份状态：握手认证、daemon 版本、失败重连；重连携带已消费事件的
  `resumeFrom` 游标。
- 会话列表：名称、最后消息时间、选中会话，并通过 `attachSession` /
  `hydrateSession` 在后台载入。
- 转录区：按持久化消息和 `after_message` 的顺序展示用户、助手、工具与系统
  条目；流式事件增量更新当前条目。文本使用 MyGo 官方 `markdown.Stream`，完成
  后切换为 `markdown.View`，列表按稳定条目 ID 保留解析与滚动状态。
- 转录状态：空态「这个会话还没有消息」、加载态「载入会话…」、错误态
  「载入失败，可重试」及重试按钮。
- 滚动：用户位于底部时随新消息和当前条目增长自动跟随；上滑后保留视口，
  回到底部后恢复跟随。
- 输入框：Enter 发送、Shift+Enter 换行；发送使用现有 `Prompt` 封装，缺少
  runtime 时先 `AttachSession`。发送期间输入被禁用并显示「发送中…」；失败显示
  「发送失败，已保留输入」并恢复原文。

目录、RPC 和事件消费均在后台执行，界面帧只读内存快照。

## 本地运行

要求 Go 1.27.3。构建和测试均使用 `CGO_ENABLED=0`，不需要 Bun、webview 或显示器。

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

### 连接已有 daemon

沿用现有 Rust 桌面的环境变量。`WAKU_DAEMON_ADDRESS` 与
`WAKU_DAEMON_TOKEN` 必须成对设置：

```sh
WAKU_DAEMON_ADDRESS=127.0.0.1:4312 \
WAKU_DAEMON_TOKEN=... \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

未设置地址和 token 时，壳会启动并监督本地 daemon：使用 `127.0.0.1:0`、
`--parent-pid`、自动生成的 `WAKU_DAEMON_TOKEN` 和 `WAKU_APP_EXECUTABLE`，读取
首行 readiness JSON，并在退出时请求 shutdown。daemon 默认取桌面可执行文件旁的
`client-daemon`（Windows 为 `client-daemon.exe`）；开发时可用
`WAKU_DAEMON_PATH` 指定现有构建产物：

```sh
WAKU_DAEMON_PATH=/path/to/client-daemon \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

## 测试与构建

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test -count=1 ./...
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go build ./...
```

协议测试使用进程内假 WebSocket 服务器；进程约定测试使用测试二进制的 helper
process；界面测试使用 `ui.NewTester`，覆盖转录、滚动、输入和发送状态，不会打开
真实窗口，因此也能在无显示器的 Linux 环境运行。

GitHub Actions 的 **Tests** workflow 在 macOS 和 Windows 上分别运行上述 Go 测试及
构建；原有 Rust、protocol 和 browser client 检查保持不变。Go job 失败会使 Tests
失败。与现有发布矩阵一致，这里没有新增 Linux 发布。

## 尚未迁移与发布边界

终端、diff、设置和自动更新器仍未迁移；充值 / 扫码、公告、模型广场、teams、
画图和电脑操作入口也属于后续阶段。本阶段不嵌 WebView，不把 daemon 换成 Go
Node，也不做旧数据迁移。

当前自动发版仍只发布现有 Rust/GPUI 客户端。`release.yml`、`sync-release.yml`、
产物、升级通道和 appcast 均未改；Go 壳只随 Tests 自动测试和构建，不进入发布产物。

接手时请先读 [HANDOFF.md](HANDOFF.md)：里面写了迁移背景与分期、协议 / 现有代码 /
MyGo API 的参考路径，以及不要提前实施的后续阶段。
