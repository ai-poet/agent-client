# CheapRouter Desktop

基于 [Sailry](https://github.com/sailry/sailry-harness) 开发的 CheapRouter 桌面客户端：Go + [MyGo](https://github.com/egoist/mygo) 原生 GPU UI，无 Electron 壳。集成低价多模型网关、内置编码 Agent、创作工作台（AI 画图）与插件生态。

> 仓库派生自 [sailry/sailry-harness](https://github.com/sailry/sailry-harness)（Apache-2.0），归属声明见 [NOTICE](NOTICE) 与 [LICENSE](LICENSE)。

## 特性

- **原生轻量界面**：纯 Go + MyGo 自绘，长会话流畅；无 GPU 时自动回退 CPU 渲染。
- **CheapRouter 网关（可选）**：主站 / CDN 线路切换、PKCE 本地回跳登录、按 lane（Claude / Codex / Gemini / 国模 / Grok）绑定分组、在线率与福利/常驻标识、余额与套餐同步。未登录也可使用本地项目、会话与自定义模型服务。
- **Agent 工作台**：原生执行循环、多文件编辑、Git / Worktree、内置终端、代码审查与插件；可通过 iroh P2P 加密链路连接远程 Host。
- **创作工作台**：文生图 / 图生图，自动识别网关画图模型，也支持自定义画图接口（Base URL / Key / 模型）。
- **两跳热更新**：兼容旧版客户端原地升级，之后的版本由 MyGo 内置更新器接管。
- **旧版数据无损迁移**：首次启动只读检测旧版数据目录，先备份再按稳定 ID 导入。

## 本地开发

环境要求：

- **Go 1.27.3**（`GOTOOLCHAIN=go1.27.3`，默认 `CGO_ENABLED=0`）
- **Rust 工具链**（仅用于首次编译原生库 `native/iroh`、`native/turso`，版本由各自 `rust-toolchain.toml` 固定）
- 可选：`just`、`jq`、Node.js ≥ 22.13（插件测试）

启动开发客户端：

```sh
./dev-desktop.sh [dev|release|<profile>] [额外参数]
```

- `dev`（默认）：数据目录 `$HOME/.sailry-dev`
- `release`：`$HOME/.sailry`
- `<profile>`：`.runtime/profiles/<profile>`

仅看 UI（不打开 profile、不启动 Node）：

```sh
unset GOROOT && export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0
go run ./cmd/sailry-desktop --preview
```

常用环境变量：

| 变量 | 说明 |
| --- | --- |
| `SAILRY_IROH_LIB` | `libsailry_iroh.{dylib,so}` / `sailry_iroh.dll` 路径 |
| `SAILRY_TURSO_LIB` | `libturso_sync_sdk_kit.{dylib,so}` / `turso_sync_sdk_kit.dll` 路径 |
| `SAILRY_HELPERS_DIR` | 辅助工具目录 |
| `SAILRY_ARCH` | 打包/交叉编译目标架构（`arm64` / `amd64`） |
| `SAILRY_TELEMETRY` | `off` 或 `DO_NOT_TRACK=1` 关闭匿名遥测 |
| `MYGO_UPDATE_FEED_URL` | 打包期注入 MyGo 更新清单基址 |
| `MYGO_UPDATE_PUBLIC_KEY` | 打包期注入 MyGo 清单验签公钥（base64） |

原生库（按需构建一次，产物缓存在 `.runtime/native/<os>-<arch>/`）：

```sh
sh native/iroh/build.sh
sh native/turso/build.sh
sh native/capture/build.sh
```

## 构建与打包

```sh
unset GOROOT && export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0
go build ./...
```

发行版打包（支持 `--dry-run` 在 Linux 上预检）：

```sh
# macOS：.app / .dmg / 第一跳更新 zip / 第二跳 MyGo tar.gz
./cmd/sailry-desktop/macos/package.sh -arch arm64 --require-update-key

# Windows：Inno Setup 安装器 / 便携 zip / 第二跳 MyGo tar.gz
./cmd/sailry-desktop/windows/package.sh -arch amd64 --require-update-key
```

CI 流水线见 [`.github/workflows/release.yml`](.github/workflows/release.yml) 与 [`.github/workflows/sync-release.yml`](.github/workflows/sync-release.yml)，发布手册见 [docs/RELEASING.zh.md](docs/RELEASING.zh.md)。

## 热更新

对象存储桶 `cheaprouter-releases`，两把独立 Ed25519 密钥：

1. **第一跳（旧版 v0.3.x → MyGo v0.4.0+）**：macOS 保持 Bundle ID `sh.waku` 与递增 `CFBundleVersion`，由 `scripts/appcast.ts` 签发 `appcast.xml`；Windows 保持 Inno Setup `AppId`，由 `scripts/appcast-windows.ts` 签发 `appcast-windows-*.xml`。
2. **第二跳（MyGo → 后续版本）**：`cmd/mygo-manifest` 对四端 `.tar.gz` 的 SHA-256 做 Ed25519 签名，产出 `mygo/update-*.json`；客户端经 `internal/update` 拉取、验签并原子替换。上传顺序为先传不可变归档，再更新短缓存清单。

## 架构

五层单向依赖，详见 [ARCHITECTURE.md](ARCHITECTURE.md)：

```text
Desktop UI (internal/desktop, MyGo)
        │  视图投影 / 动作分发 (internal/desktop/live)
        ▼
Client (internal/client)      命令发送 / 事件归约 / 断线恢复
        ▼
Link (internal/link)          对等认证 / 帧传输 / 回执（iroh P2P 或本地）
        ▼
Node (internal/node)          Agent 引擎 / Turso 存储 / 工具与插件运行时
        ▲
Protocol (internal/protocol)  命令 / 事件 / 快照 / 错误码 / 线协议
```

| 模块 | 职责 |
| --- | --- |
| `internal/protocol` | 命令、事件、ID、快照、错误码、能力声明与 `wire` 编解码 |
| `internal/link` | 节点身份、配对、本地传输与 iroh 端到端加密通信、回执与重连 |
| `internal/client` | 类型化命令调用、有序事件投影、去重与请求恢复 |
| `internal/node` | Agent 执行引擎、模型适配、工具、Git/文件/终端服务、Turso 持久化、插件宿主 |
| `internal/desktop` | MyGo 桌面壳、`kit` 控件库、主题、i18n、侧栏/会话/设置/创作工作台与 `live` 绑定 |
| `internal/cheapauth` | CheapRouter 登录、PKCE 回调、lane 分组绑定、网关与余额同步 |
| `internal/imagestudio` | 创作工作台后端客户端、任务存储与画图接口配置 |
| `internal/update` | MyGo 热更新配置、检查与安装 |
| `internal/migrate` | 旧版项目、会话与历史的只读备份与迁移 |

## 目录

```text
.
├── go.mod / go.sum
├── bundled.go                 # 内置插件与主题资源嵌入
├── dev-desktop.sh             # 跨平台开发启动脚本
├── cmd/
│   ├── sailry-desktop/        # 桌面主入口、mygo.json、macOS/Windows 打包脚本
│   ├── sailry-host/           # 无头 Host 服务
│   ├── mygo-manifest/         # 第二跳更新归档与清单签名工具
│   ├── gen-win-icon/          # Windows 多尺寸 .ico 生成
│   └── update-model-catalog/  # 模型目录快照更新
├── internal/                  # protocol / link / client / node / desktop 等
├── native/                    # 原生库源码与构建脚本（iroh / turso / capture）
├── assets/                    # 品牌图标与内置资源
├── locales/                   # 多语言源文件（go generate 生成内嵌词典）
├── plugins/ themes/ schemas/ sdk/
├── scripts/ docs/ .github/workflows/
├── LICENSE / NOTICE / TRADEMARKS.md
└── third_party_licenses/
```

## 许可证与归属

- Apache License 2.0，全文见 [LICENSE](LICENSE)。
- 基于 [Sailry](https://github.com/sailry/sailry-harness) 派生；MyGo、CrychicUI、Ghostty 等第三方组件的著作权与来源声明保留于 [NOTICE](NOTICE) 与 [third_party_licenses/](third_party_licenses/)。
- Apache-2.0 不授予 Sailry 商标使用权，详见 [TRADEMARKS.md](TRADEMARKS.md)。
