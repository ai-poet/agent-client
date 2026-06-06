# CLAUDE.md

Agent Client 是一款用于随时随地监控和控制本地 AI 编码 Agent 的移动应用。您的开发环境，尽在口袋中。直接连接到您的实际开发环境——代码始终保留在您的机器上。

**支持的 Agent：** Claude Code、Codex 和 OpenCode。

## 仓库结构

这是一个 npm workspace 单体仓库：

- `packages/server` — 守护进程：Agent 生命周期、WebSocket API、MCP 服务器
- `packages/app` — 移动端 + Web 客户端（Expo）
- `packages/cli` — Docker 风格 CLI（`agent-client run/ls/logs/wait`，也支持 `paseo` 别名）
- `packages/relay` — 端到端加密中继，用于远程访问
- `packages/desktop` — Electron 桌面端封装
- `packages/website` — 官网

## 文档

| 文档 | 内容 |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 系统设计、包分层、WebSocket 协议、Agent 生命周期、数据流 |
| [docs/CODING_STANDARDS.md](docs/CODING_STANDARDS.md) | 类型规范、错误处理、状态设计、React 模式、文件组织 |
| [docs/TESTING.md](docs/TESTING.md) | TDD 工作流、确定性、优先使用真实依赖而非 mock、测试组织 |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | 开发服务器、构建同步注意事项、CLI 参考、Agent 状态、Playwright MCP |
| [docs/RELEASE.md](docs/RELEASE.md) | 发布手册、草稿发布、完成检查清单 |
| [docs/CUSTOM-PROVIDERS.md](docs/CUSTOM-PROVIDERS.md) | 自定义提供商配置：Z.AI、阿里云/通义千问、ACP Agent、配置文件、自定义二进制文件 |
| [docs/ANDROID.md](docs/ANDROID.md) | 应用变体、本地/云构建、EAS 工作流 |
| [docs/DESIGN.md](docs/DESIGN.md) | 如何在实现前设计功能 |
| [SECURITY.md](SECURITY.md) | 中继威胁模型、端到端加密、DNS 重绑定、Agent 认证 |

## 快速开始

```bash
npm run dev                          # 在 Tmux 中启动守护进程 + Expo
npm run cli -- ls -a -g              # 列出所有 Agent
npm run cli -- daemon status         # 检查守护进程状态
npm run typecheck                    # 每次修改后务必运行
npm run format                       # 使用 Biome 自动格式化
npm run format:check                 # 检查格式化，不写入
```

完整设置、构建同步要求和调试信息请参阅 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## 关键规则

- **未经批准，切勿重启主 Agent Client 守护进程（端口 6767）** — 它管理所有运行中的 Agent。如果您是 Agent，重启它会终止您自己的进程。
- **切勿认为超时意味着服务需要重启** — 超时可能是暂时的。
- **切勿在测试中添加认证检查** — Agent 提供商自行处理认证。
- **切勿在本地运行完整测试套件。** 测试套件很重，会冻结机器，尤其是多个 Agent 并行运行时。规则：
  - 仅运行您修改的特定测试文件：`npx vitest run <文件> --bail=1`
  - 除非明确要求，否则不要运行 `npm run test` 整个工作区。
  - 如果必须运行大范围套件，将输出重定向到文件后读取：`npx vitest run <文件> --bail=1 > /tmp/test-output.txt 2>&1`，然后读取文件。
  - 切勿重新运行另一个 Agent 已经运行并报告通过的测试套件——信任结果。
  - 完整套件验证请推送到 CI，查看 GitHub Actions。
- **每次修改后务必运行 typecheck。**
- **提交前运行 `npm run format`。** 本仓库使用 Biome 进行格式化。不要手动修复格式问题——让格式化工具处理。
- **品牌构建必须设置并验证托管云登录端点。** 构建任何品牌/白标应用时，不要止步于应用名称、图标、包名/应用 ID 或更新 URL。品牌包装脚本还必须设置 `EXPO_PUBLIC_MANAGED_SERVICE_URL`，并且打包前必须检查构建的 Web 包中是否包含预期的登录/服务域名。已知的品牌端点：
  - CyberAICoding: `https://ai-coding.cyberspirit.io`
  - CheapRouter: `https://cheaprouter.org`
- **默认数据目录：** `~/.agent-client`（旧版本为 `~/.paseo`）。
- **切勿对 WebSocket 或消息模式进行破坏性变更。** 主要兼容路径是旧版移动应用客户端连接新更新的守护进程。用户先更新桌面端和守护进程，然后继续使用旧应用一段时间。每次模式变更必须向后兼容旧客户端对新守护进程：
  - 新字段：始终使用 `.optional()` 并设置合理的默认值，或使用 `.transform()` 回退。
  - 切勿将字段从可选改为必填。
  - 切勿移除字段——将其弃用（继续接受，停止发送）。
  - 切勿缩小字段类型（例如 `string` → `enum`，`nullable` → 非空）。
  - 测试时思考："6 个月前的客户端还能解析这个吗？"以及"6 个月前的守护进程发送的内容这个客户端还能接受吗？"

## 平台隔离

应用运行在 iOS、Android、Web（浏览器）和 Web（Electron 桌面端）上。代码默认跨平台。仅在必要时进行隔离。从 `@/constants/platform` 导入隔离条件。

### 四种隔离条件

| 条件 | 类型 | 使用场景 |
|---|---|---|
| `isWeb` | 常量 | DOM API — `document`、`window`、`<div>`、`addEventListener`、`ResizeObserver`。这是**例外**，不是默认。 |
| `isNative` | 常量 | 原生专用 API — Haptics、`StatusBar.currentHeight`、推送令牌、相机/扫码、`expo-av`。 |
| `getIsElectron()` | 缓存函数 | 桌面端封装功能 — 文件对话框、标题栏拖拽区域、守护进程管理、应用更新、Dock 角标。 |
| `useIsCompactFormFactor()` | Hook | 布局决策 — 侧边栏覆盖 vs 固定、模态框 vs 全屏、单面板 vs 分屏。来自 `@/constants/layout`。 |

### 决策矩阵

| 我需要... | 使用 |
|---|---|
| 访问 DOM（`document`、`window`、`<div>`、`addEventListener`） | `if (isWeb)` |
| 使用原生专用 API（Haptics、推送令牌、相机） | `if (isNative)` |
| 使用 Electron 桥接（文件对话框、标题栏、更新） | `if (getIsElectron())` |
| 在手机和平板/桌面之间切换布局 | `useIsCompactFormFactor()` |
| 悬停显示，原生端始终可见 | `isHovered \|\| isNative \|\| isCompact`（悬停仅在 Web 上有效） |
| 专门隔离 iOS 或 Android | `Platform.OS === "ios"` / `Platform.OS === "android"`（少用，保持内联） |

### 规则

- **默认跨平台。** 没有特定原因不要隔离。
- **优先使用 Metro 文件扩展名而非 `if` 语句。** 当模块在不同平台有根本不同的实现时，使用 `.web.ts` / `.native.ts` 文件扩展名替代运行时 `if (isWeb)` 分支。Metro 在构建时解析正确的文件——未使用的平台代码不会被打包。将 `if (isWeb)` 保留给小型的内联检查（单行或几个属性）。如果您发现自己写了一个大的 `if (isWeb) { ... } else { ... }` 块，请拆分为独立文件。
  ```
  hooks/
    use-audio-recorder.web.ts    ← 使用 Web Audio API
    use-audio-recorder.native.ts ← 使用 expo-audio
  ```
  以 `@/hooks/use-audio-recorder` 导入 — Metro 自动选择正确的文件。
- **切勿在无 `isWeb` 保护的情况下使用原始 DOM API。** DOM API 会在原生端崩溃。将 RN ref 强制转换为 `HTMLElement` 是危险信号——确保该块仅限 Web。
- **切勿使用 `onPointerEnter`/`onPointerLeave`。** 它们在原生 iOS 上不会触发。
- **悬停仅在 Web 上有效。** React Native 的 `Pressable` 上的 `onHoverIn`/`onHoverOut` 在原生 iOS/iPad 上不会触发——底层 W3C 指针事件位于禁用的实验性标志后面。对于悬停显示 UI（竖排菜单、操作按钮），使用 `isHovered || isNative || isCompact`，以便控件在原生端始终可见，在 Web 上悬停显示。
- **不要将 Platform.OS 用作布局能力的代理。** 布局决策使用断点，而非平台检查。
- **从 `@/constants/platform` 导入 `isWeb`/`isNative`。** 切勿在本地写 `const isWeb = Platform.OS === "web"`。

## 调试

完整的守护进程日志和跟踪信息位于 `~/.agent-client/daemon.log`
