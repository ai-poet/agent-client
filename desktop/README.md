# CheapRouter MyGo desktop shell

这是渐进迁移的第一期脚手架：MyGo `package ui` 绘制原生 GPU 界面，Go
客户端沿用现有 Rust `client-daemon` 的 WebSocket `/v1` JSON 协议。当前窗口只展示
未连接、连接中、已连接或失败原因，并支持失败后重新连接；**它不是完整工作台**。

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

本期只实现连接、握手、协议错误识别和 daemon 生命周期边界。后续还需补全：

- `request` / `response` 路由、RPC 错误和超时；
- sequenced `event` 订阅、去重、断线恢复与 `resumeFrom`；
- task/project 状态、settings 等消息以及完整工作台 UI；
- 执行层向 Go 的迁移，以及升级后对 `~/.cheaprouter` 数据的迁移器。
