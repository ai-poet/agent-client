# CheapRouter Desktop：三项修复验收

## 1. macOS 图标

macOS 和 Windows 使用同一份 `assets/branding/cheaprouter.png`。在 Linux 上运行
`go generate ./assets/branding` 即可重建两种格式（macOS 生成器需要 Python 3 和
Pillow）。ICNS 含 16、32、64、128、256、512、1024 像素的图像和 Retina 条目；
1024 像素条目直接保留原始 PNG。`mygo.json` 已切换到新图标，旧的 ICNS 已移除。

验证：重新生成后 Git 无差异；Pillow 识别出 11 个条目。新增的品牌契约测试检查配置路径和
ICNS 中的原图。既有的产品标识测试、MyGo 的 ICNS 和 Info.plist 测试，以及发布脚本的
`--dry-run` 均通过。另用产品的原始 `mygo.json` 和一个最小可执行文件，在 Linux 上用
固定版本的 MyGo 生成了 arm64 `.app`：实读 `CFBundleIconFile` 为 `AppIcon.icns`，
`Contents/Resources/AppIcon.icns` 与仓库里的 `cheaprouter.icns` 逐字节一致。

人工确认：在 macOS 上安装正式签名的包，在 Finder 和 Dock 查看图标，尤其是覆盖旧版后的图标缓存。

## 2. 按绑定分组使用原生协议

每个绑定 lane 独立保存 provider、组密钥和模型列表。两个组即使提供同名模型，
也可通过各自的 provider ID 分别选择。切换分组用绑定的组 ID 查平台；
解绑或退出时停用过期记录，不把旧会话保存的账户 ID 改派给其他组。

| 分组 `platform` | 模型请求路径（保留网关代理前缀） |
| --- | --- |
| `anthropic` / `claude` | `/v1/messages` |
| `openai` / `gpt` / `codex` | 仅在组返回 `supports_responses: true` 时用 `/v1/responses`，否则用 `/v1/chat/completions` |
| `gemini` / `google` | 基址 `/v1beta`，流式请求发往 `/v1beta/models/{model}:streamGenerateContent` |
| 未知、复合平台，或未绑定的账户默认路由 | `/v1/chat/completions` |

原有分组平台和模型目录都不能证明网关支持 Responses，因此客户端只接受组响应中明确的布尔能力标志，
不按模型名猜测，也不通过重试另一种协议重复发送请求。

验证：新增的本地 HTTP 测试通过真实模型适配器捕获请求，核对路径、认证头、请求体；
覆盖协议别名、非标准代理前缀、未知平台，以及 lane 名与分组平台不一致的情况。
新增的持久化集成测试在真实本地 Node 中保存三个共享同一模型名的 provider，验证选择器的三条记录、
各自的协议、端点和密钥，以及解绑、退出后的停用行为。

人工确认：在预发布网关核对 `/api/v1/groups/available` 的能力字段，特别是希望启用 Responses 的组；
未提供能力字段的旧网关会继续走 Chat Completions。

## 3. 自动更新弹窗

启动时的自动检查发现新版后，只在一个主窗口弹出「发现新版本」，显示版本号及清单的
`notes` 原文。点「稍后」把版本记在该配置目录的 `update.json`；设置页的手动检查会
重置这条记录，但自身不会弹窗。「下载并安装」复用现有下载、验签和安装流程，重启仍由用户决定。

验证：新增的 MyGo 界面测试覆盖多窗口去重、版本与日志、延期、手动重置及安装按钮。
另在 Linux 图形环境中打开真实 MyGo 主窗口，用隔离配置注入模拟检查结果：
`1.2.0` 显示多行日志，点击「稍后」后重开同版本不再弹；
`1.3.0` 再次弹出，点击安装后观察到该版本进入下载状态。
测试没有连接发布服务、替换二进制或重启用户的应用。

人工确认：用已签名的 macOS 和 Windows 测试包走完一次真实升级，核对安装后的重启、版本及用户数据。

## 最终验证

Linux amd64，`GOTOOLCHAIN=go1.27.3`、`CGO_ENABLED=0`，已取消 `GOROOT` 覆盖。
`go build ./...` 和 `go vet ./...` 均成功，无诊断。仅运行定向测试，未运行 `go test ./...`。

| 定向测试 | 结果 |
| --- | --- |
| `internal/cheapauth`、`internal/node/providers`、`internal/node/agent/model` | 全包通过，包括回环 HTTP 协议测试 |
| `internal/update`、`internal/desktop/i18n`、`internal/desktop/settings` | 全包通过 |
| `internal/desktop/live` | CheapRouter 登录、分组切换、同名模型投影、退出，以及 composer 和原生更新弹窗的定向用例通过 |
| `internal/desktop/shell`、`cmd/sailry-desktop` | 既有更新状态、产品标识及新增 macOS 图标契约用例通过 |

首次数据库集成测试因缺少项目专用的 Turso 动态库未能启动。使用仓库锁定的 Rust 1.98.0
和 `native/turso/build.sh` 构建后重跑通过，未修改原生依赖，也未将库或工具链纳入提交。
