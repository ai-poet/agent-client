<h1 align="center">Agent Client</h1>

<p align="center">AI Agent 客户端 —— 专为 Claude Code CLI 与 Codex CLI 打造</p>

---

Agent Client 是一个针对 **Claude Code CLI** 和 **Codex CLI** 深度定制的 AI Agent 客户端，提供更强大的 Agent 管理、编排和远程控制能力。

## 核心特性

- **深度集成 Claude Code & Codex**: 针对两大主流 AI 编码 Agent 进行专项优化，支持高级参数配置、自定义工作流和深度交互
- **多 Agent 并行编排**: 在本地机器上并行运行多个 Agent，支持跨 Agent 任务协调与结果聚合
- **自托管架构**: Agent 运行在您的本地环境中，使用您的工具、配置和技能，代码始终留在您的机器上
- **全平台覆盖**: iOS、Android、桌面端、Web 和 CLI，随时随地管理您的 Agent
- **隐私优先**: 无遥测、无追踪、无强制登录，您的数据完全由您掌控
- **语音控制**: 语音模式下达任务指令，解放双手

## 系统要求

| 平台 | 最低版本 | 备注 |
|---|---|---|
| Node.js | 18.x | 守护进程运行所需 |
| iOS | 15.0+ | 移动端 App |
| Android | 8.0+ | 移动端 App |
| macOS | 12+ | 桌面端 |
| Windows | 10+ | 桌面端 |

## 快速开始

Agent Client 运行一个本地守护进程（daemon）来管理您的编码 Agent。桌面应用、移动应用、Web 应用和 CLI 都通过它进行连接。

### 前置要求

您需要至少安装并配置好一个 Agent CLI：

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) — Anthropic 的 Claude 编码助手
- [Codex](https://github.com/openai/codex) — OpenAI 的 Codex 编码 Agent

### 桌面应用（推荐）

从 releases 页面下载。打开应用后守护进程自动启动，无需额外安装。

手机连接：在设置中扫描二维码即可配对。

### CLI / 无头模式

安装 CLI 并启动 Agent Client：

```bash
npm install -g @getpaseo/cli
agent-client
```

终端会显示二维码，使用任意客户端扫描连接。适用于服务器和远程机器场景。

## CLI 使用示例

应用中的所有操作都可以通过终端完成：

```bash
# 使用 Claude Code 运行任务
agent-client run --provider claude/opus-4.6 "实现用户认证系统"

# 使用 Codex 在独立工作区运行
agent-client run --provider codex/gpt-5.4 --worktree feature-x "实现功能 X"

# 列出运行中的 Agent
agent-client ls

# 实时连接 Agent 输出流
agent-client attach abc123

# 向运行中的 Agent 追加指令
agent-client send abc123 "顺便加上单元测试"

# 连接远程守护进程
agent-client --host workstation.local:6767 run "运行完整测试套件"
```

## Agent 编排技能（实验性）

通过技能系统教 Agent 如何使用 CLI 编排其他 Agent。

## 开发

Monorepo 包结构：

| 包 | 说明 |
|---|---|
| `packages/server` | 守护进程（Agent 进程编排、WebSocket API、MCP 服务器） |
| `packages/app` | Expo 客户端（iOS、Android、Web） |
| `packages/cli` | CLI（守护进程与 Agent 工作流） |
| `packages/desktop` | Electron 桌面应用 |
| `packages/relay` | 远程连接中继包 |
| `packages/website` | 官网与文档 |

常用命令：

```bash
# 启动所有本地开发服务
npm run dev

# 单独启动各端
npm run dev:server
npm run dev:app
npm run dev:desktop

# 构建守护进程
npm run build:daemon

# 全仓库类型检查
npm run typecheck
```

## 许可证

AGPL-3.0
