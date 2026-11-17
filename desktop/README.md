# CheapRouter MyGo desktop shell

这是渐进迁移中的 MyGo 原生工作台：`package ui` 绘制原生 GPU 界面，Go
客户端沿用现有 Rust `client-daemon` 的 WebSocket `/v1` JSON 协议。当前窗口具备
连接状态、失败重连、会话列表（名称与最后消息时间）、选中会话，以及
`attachSession` / `hydrateSession` 后的消息数占位；重连会携带已消费事件的
`resumeFrom` 游标。**它还不是完整工作台**。

## 本地运行

要求 Go 1.27.3。构建和测试均使用 `CGO_ENABLED=0`，不需要 Bun、webview 或显示器。

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

连接已经运行的 daemon 时，沿用现有 Rust 桌面的两个环境变量（必须成对设置）：

```sh
WAKU_DAEMON_ADDRESS=127.0.0.1:4312 \
WAKU_DAEMON_TOKEN=... \
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go run ./cmd/cheaprouter-desktop
```

未设置这两个变量时，壳会启动自己监督的本地 daemon：使用 `127.0.0.1:0`、
`--parent-pid`、`WAKU_DAEMON_TOKEN` 和 `WAKU_APP_EXECUTABLE`，读取首行 readiness
JSON，并在退出时请求 shutdown。daemon 默认取桌面可执行文件旁的
`client-daemon`（Windows 为 `client-daemon.exe`）；开发时也可用
`WAKU_DAEMON_PATH=/path/to/client-daemon` 指定现有构建产物。

## 测试与构建

```sh
cd desktop
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test ./...
GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go build ./...
```

协议测试使用进程内假 WebSocket 服务器；进程约定测试使用测试二进制的 helper
process；界面测试使用 `ui.NewTester`，不会打开真实窗口，因此也能在无显示器的
Linux 环境运行。MyGo 固定为 `github.com/egoist/mygo v0.9.2`。

GitHub Actions 的 **Tests** workflow 在 macOS 和 Windows 上分别运行上述 Go 测试及
构建；原有 Rust、protocol 和 browser client 检查保持不变。Go job 失败会使 Tests
失败。与现有发布矩阵一致，这里没有新增 Linux 发布。

## 发布边界与后续

当前自动发版仍只发布现有 Rust/GPUI 客户端。`release.yml`、`sync-release.yml`、
产物、升级通道和 appcast 均未改；Go 壳只随 Tests 自动测试和构建，不进入发布产物。

当前已接通连接/重连、握手、协议错误识别、daemon 生命周期、会话目录和会话
hydrate；目录与 RPC 都在后台执行，界面帧只读内存快照。右侧暂只显示所选会话的
消息数，流式 Markdown 转录渲染、输入框、终端和 diff 仍未迁移。没有新增协议或
wire 类型，会话列表直接复用 `internal/daemon` 暴露的持久化结构。

接手时请先读 [HANDOFF.md](HANDOFF.md)：里面写了迁移背景与分期、协议 / 现有代码 /
MyGo API 的参考路径，以及不要提前实施的后续阶段。
