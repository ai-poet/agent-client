# CheapRouter

CheapRouter 把「低价模型网关」和「原生 Agent 工作台」合成了一个桌面应用：
一端是 Claude / GPT / Grok，以及订阅分组里的 DeepSeek / Kimi / GLM 等模型的
网关，另一端是用 Rust + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
写的本地编码 agent 工作台，自带一个编译在客户端里的内置 Agent。登录即用，
开箱即路由，新手不用碰任何配置文件，老手也能精确掌控每一份 CLI 配置。

> **Built on [Waku](https://github.com/egoist/waku)** by egoist, licensed under
> GPL-3.0-only. This is a modified fork; see [NOTICE.md](NOTICE.md) for the list
> of changes and [docs/FORK.md](docs/FORK.md) for how it tracks upstream.
> Please report issues with this build here, not to the upstream project.

## 为什么选 CheapRouter

- **内置 Agent，装好就能用。** 引擎直接编译在客户端里，不需要 Node、npm 或任何
  外部 CLI。网关上能对话的 Claude、GPT、Grok、DeepSeek、Kimi、GLM 等模型都在
  它的模型选择器里，每个模型自动走对应的接口格式、用提供它的那个分组的密钥，
  推理强度按模型家族给出可选档位。详见下文 [内置 Agent](#内置-agent)。
- **登录即路由，零配置。** 浏览器里登录云账号，客户端就把网关地址和密钥直接写进
  每个 CLI 自己的全局配置——Claude Code、Codex、Grok Build、OpenCode、Pi 全部对齐。
  接管前自动备份你的原配置，退出登录时原样恢复；不注入环境变量、不架本地代理、
  不需要手改任何文件，Codex 的 ChatGPT 登录（`auth.json`）也不会被改动。终端里
  直接跑 `claude` / `codex` 同样走网关。
- **订阅一目了然。** 设置 → CheapRouter 账号列出你的每个有效订阅：剩余天数、日 /
  周 / 月的已用额度与上限；左下角账号菜单也有一行摘要。订阅里的模型按周期计费，
  不扣余额，内置 Agent 会让 DeepSeek、Kimi、GLM 这类模型自动走提供它的订阅。
- **缺什么装什么。** 全新电脑没装 Node、没装任何 CLI 也没关系：设置 → 服务商
  会检测缺失项并一键补齐。Node 22 无人值守安装（Windows 便携包 → 静默 MSI →
  winget 三级回退，npm 国内镜像优先），Claude Code / Codex / Grok Build 一键安装，
  安装失败直接显示安装器的原始输出，命令也可以一键复制自己跑。
- **余额、充值、用量原生内置。** 左下角实时余额（每个任务结束自动刷新，低于 $5
  变黄、$1 变红），原生充值弹窗支持扫码 / Stripe / 兑换码，用量页带模型与分组
  筛选、分页、逐请求的 token / 缓存 / 费用 / 延迟明细——不用再开网页后台。
- **模型广场，先比价再干活。** 全部在售模型的官方价与网关价对照（划线价一目了然）、
  分组倍率与健康度、缓存 / 长上下文 / 阶梯计费标注，一眼挑出最划算的分组。
- **分组一键切换。** 每个 CLI（Claude Code / Codex / Grok）独立绑定计费分组，
  左下角账号菜单里两次点击就能换线路——不进设置页、不重启：切换即时重写该 CLI
  的全局配置并重新绑定密钥，新启动的任务立刻走新分组。菜单里每个分组旁直接
  标注倍率（如 ×0.50）、24 小时在线率和降级/故障状态；也可以按平台开启自动
  故障转移，分组故障时切到健康分组，恢复后自动切回。
- **电脑操作与生图。** 在 macOS 和 Windows 上，agent 可以直接操作桌面应用
  （Windows 在设置 → 电脑操作里一键下载驱动），也能通过网关生成图片。驱动没装时
  只是不开放桌面控制，对话照常进行。
- **公告直达。** 标题栏铃铛实时同步服务公告（限时活动、模型上新、维护通知），
  未读红点提醒，客户端内直接阅读。
- **完整的 Agent 工作台。** 多项目、多会话并行，消息排队与中途转向，Git
  checkpoint 会话级回滚，worktree 隔离，内置终端、diff 与技能库——上游 Waku
  的全部能力都在。

## 两步上手

1. **安装并登录。** 从 [最新 Release](../../releases/latest) 下载对应平台安装包，
   打开应用点「登录」，浏览器里完成注册——回到客户端一切就绪，路由已自动配置。
2. **开跑。** 打开一个项目文件夹，输入你想做的事，回车。默认用的就是内置 Agent；
   余额在左下角，随用随充。

外部 agent CLI 依然可以接（设置 → 服务商），但那是可选项，不再是跑起来的前提。

进阶：设置 → 模型接口 统一登记你自己的接口——地址、密钥、接口格式、模型列表，
以及可测速自动选优的备用域名；内置 Agent 的三种接口格式和各个 CLI 各自选用其中
一个。已登录时 CLI 优先走云路由，内置 Agent 的某种接口格式指向了自定义接口时
则以自定义接口为准。`cargo run -p sub2api --example routing_doctor` 可以随时对照
期望路由与磁盘上的实际配置。

## 内置 Agent

内置 Agent 是默认 provider。它是编译进客户端的一个完整编码 agent：读写文件、
执行命令、搜索代码、调用 MCP 工具、启动子 agent。会话记录由应用自己保存，回滚、
分支、续聊都不依赖任何外部进程。

**模型与接口格式。** 模型选择器列出网关目录里能对话的模型（Gemini 暂未接入），
同一个模型只列一次。接口格式由模型家族决定，不需要手动选：

| 模型家族 | 接口格式 | 推理强度 |
|---|---|---|
| Claude | Anthropic Messages | low 至 max，最新型号另有 ultracode |
| GPT / o 系列、Grok | OpenAI Responses | low 至 max |
| DeepSeek、GLM 4.5 及以后、Kimi K3 | OpenAI Chat Completions | low / high / max |
| Kimi K2、MiniMax | OpenAI Chat Completions | 由模型自己决定（接口不提供档位） |

**每个模型走对的分组。** 一个密钥只属于一个分组，而每个分组只提供它支持的那些
模型。内置 Agent 按模型挑分组：Claude、GPT、Grok 走你为 Claude Code、Codex、
通用线路绑定的分组；其余模型优先走提供它的有效订阅，没有订阅时走提供它的、倍率
最低的分组。需要的密钥会自动准备好（优先复用账号里已有的密钥），选择器里走订阅
的模型会注明订阅名。

**先规划，再动手。** Plan 模式下 agent 只读代码、写计划，计划经你批准后才开始
修改；它也可以在对话中自己进入 Plan 模式，界面上的模式标签会跟着变。

**跨平台的命令行。** macOS 上用 bash；Windows 上优先用 Git Bash，没有安装时改用
PowerShell（有 7 就用 7），工作目录在多次调用之间保持，中文输出不乱码，超时会
结束整棵进程树。

**项目指令。** 自动读取项目里的 `AGENTS.md` / `CLAUDE.md`（从全局配置目录到当前
目录逐级读取），与你在设置里写的系统提示一起生效。

**不设步数上限。** 一条消息可以连续执行任意多步，长任务不会被中途截断；想停的时候
随时取消。

**可配置项。** 设置 → 内置 Agent：三种接口格式各用哪个接口、系统提示、自动压缩
与阈值、内置工具开关、MCP 服务器、权限规则，以及启用开关。

内置 Agent 的引擎基于 [Claurst](https://github.com/kuberwastaken/claurst)
（GPL-3.0），以 vendored 形式放在 [`crates/waku-agent`](crates/waku-agent)，
所做的修改逐条记录在 [NOTICE.md](NOTICE.md)。

## 支持的 agent

| Agent | 云网关路由 | 自定义接口 | 一键安装 |
|---|---|---|---|
| **内置 Agent**（默认） | ✓ | ✓ | 无需安装 |
| Claude Code | ✓ | ✓ | ✓ |
| Codex CLI | ✓ | ✓ | ✓ |
| Grok Build | ✓ | ✓ | ✓ |
| OpenCode | — | ✓ | — |
| Pi | — | ✓ | — |

会话协议层面还支持 [Amp](https://ampcode.com/)、Cursor CLI、
[Fx](https://fx.sh/)、Kimi Code 等（自带配置使用）。每个 provider 走各自的
原生结构化协议，会话可延续。

## Install

macOS 一键安装（推荐，绕开 Gatekeeper 弹窗）：

```bash
curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
```

Windows 从 [最新 Release](../../releases/latest) 下载 `CheapRouter-*-Setup.exe`
安装（x64 与 arm64 均有）。安装后应用内自动更新。目前不提供 Linux 安装包。

## Architecture

The native desktop is an RPC client of the standalone `waku-daemon` process.
Provider sessions run in [`waku-core`](crates/waku-core), behind the
authenticated, versioned WebSocket contract in
[`waku-protocol`](crates/waku-protocol). The desktop depends on
[`waku-client`](crates/waku-client), not on the daemon implementation. The
daemon owns task SQLite data, uploaded attachments, provider-native session
forks, and all workspace filesystem and Git operations; paths returned by it
always refer to the daemon host. The desktop retains only presentation state
and a disposable preview cache.

The built-in agent (`ProviderKind::Native`) runs inside the daemon rather than
as a child process: a vendored copy of the Claurst engine in
[`crates/waku-agent`](crates/waku-agent), driven through
[`crates/waku-agent-bridge`](crates/waku-agent-bridge) (engine lifecycle,
permissions, history, MCP, routing) and translated into the provider contract
by `crates/waku-core/src/driver/native.rs`. Its transcript is owned by the
daemon, which is what makes rewind, branch and resume plain truncations.

All fork functionality lives in [`crates/sub2api`](crates/sub2api) plus a
handful of view files; see [docs/FORK.md](docs/FORK.md) for the hook-point
register. Routing is desktop-local: the desktop writes each CLI's own global
configuration (cc-switch model, with pre-takeover backups in
`~/.cheaprouter/takeover.json`), and the daemon carries no routing state, so
the wire protocol stays byte-identical to upstream.

The browser client lives at [`apps/web`](apps/web) and uses the generated
browser transport in [`packages/waku-client`](packages/waku-client). Its
checked-in types are generated directly from the Rust protocol. Run
`bun run protocol:generate` after changing a wire type and
`bun run protocol:check` to verify that generated files are current.

App data lives under `~/.cheaprouter` (projectless task workspaces at
`~/.cheaprouter/projects/<date>/<slug>`; the Release desktop writes
`app.json` and daemon settings `settings.json` there, while Debug stays
isolated at `temp/`). Legacy `~/.waku` directories from older builds are
renamed in place at startup.

Release apps bundle and sign `waku-daemon`. Development keeps the daemon at
`target/debug/waku-debug-daemon`, allowing provider-only edits to rebuild and
replace the daemon without relaunching the debug build.

## Development

Development is supported on macOS and Windows and requires
[Rust 1.96 or newer](https://www.rust-lang.org/tools/install) and
[Bun](https://bun.sh/). Windows needs the MSVC toolchain; install the native
build prerequisites listed in [CONTRIBUTING.md](CONTRIBUTING.md) first. Linux
still builds from source, but it is not tested in CI and no Linux package is
published.

```sh
bun install
bun run dev
```

The embedded browser remains macOS-only. Computer Use runs on macOS and on
Windows, where it drives the desktop through `cua-driver` (downloaded from
Settings → Computer Use). Agent sessions, projects, transcripts, skills,
usage, diffs, file editing, and the terminal run natively on both.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and checks.
Release maintainers should also read [RELEASING.md](RELEASING.md).

## Upstream

This fork exists because of upstream Waku. You can support its development via
[GitHub Sponsors](https://github.com/sponsors/egoist).

## License

Licensed under the [GNU General Public License v3.0 only](LICENSE), the same
license as upstream Waku. Modifications are recorded in [NOTICE.md](NOTICE.md).
