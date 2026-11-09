# CheapRouter

CheapRouter 把「低价模型网关」和「原生 Agent 工作台」合成了一个桌面应用：
一端是 Claude / GPT / Grok，以及订阅分组里的 DeepSeek / Kimi / GLM 等模型的
网关，另一端是用 Rust + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
写的本地编码 agent 工作台，自带一个编译在客户端里的内置 Agent。登录即用，
模型按当前源站自动拉取，不需要安装或配置任何外部 CLI。

> **Built on [Waku](https://github.com/egoist/waku)** by egoist, licensed under
> GPL-3.0-only. This is a modified fork; see [NOTICE.md](NOTICE.md) for the list
> of changes and [docs/FORK.md](docs/FORK.md) for how it tracks upstream.
> Please report issues with this build here, not to the upstream project.

## 为什么选 CheapRouter

- **内置 Agent，装好就能用。** 引擎直接编译在客户端里，不需要 Node、npm 或任何
  外部 CLI。网关上能对话的 Claude、GPT、Grok、DeepSeek、Kimi、GLM 等模型都在
  它的模型选择器里，每个模型自动走对应的接口格式、用提供它的那个分组的密钥，
  推理强度按模型家族给出可选档位。详见下文 [内置 Agent](#内置-agent)。
- **源站一切，全局联动。** 底部源站菜单切换后，内置 Agent 的模型请求、模型目录、
  余额、用量、订阅、公告和充值等业务 API 都使用新源站；旧源站尚未返回的请求不会
  覆盖新数据。登录和令牌刷新仍固定在完成登录的认证域名，避免认证会话跨节点漂移。
- **订阅一目了然。** 设置 → CheapRouter 账号列出你的每个有效订阅：剩余天数、日 /
  周 / 月的已用额度与上限；左下角账号菜单也有一行摘要。订阅里的模型按周期计费，
  不扣余额，内置 Agent 会让 DeepSeek、Kimi、GLM 这类模型自动走提供它的订阅。
- **按厂商选模型。** 模型入口位于输入区的一级控件里，打开后直接按厂商分类，
  只列出当前厂商的可用模型；也可按名称、厂商或自定义接口跨分类搜索，不用再进入
  服务商二级菜单。
- **余额、充值、用量原生内置。** 左下角实时余额（每个任务结束自动刷新，低于 $5
  变黄、$1 变红），原生充值弹窗支持扫码 / Stripe / 兑换码，用量页带模型与分组
  筛选、分页、逐请求的 token / 缓存 / 费用 / 延迟明细——不用再开网页后台。
- **模型广场，先比价再干活。** 全部在售模型的官方价与网关价对照（划线价一目了然）、
  分组倍率与健康度、缓存 / 长上下文 / 阶梯计费标注，一眼挑出最划算的分组。
- **分组一键切换。** 左下角账号菜单按模型平台选择计费分组，内置 Agent 会立即拿到
  新的分组密钥；进行中的单次请求保持在原线路，新请求使用新分组。菜单里每个分组旁
  直接标注倍率（如 ×0.50）、24 小时在线率和降级/故障状态；也可以按平台开启自动
  故障转移，分组故障时切到健康分组，恢复后自动切回。
- **画图，文生图和图生图都在一个框里。** 侧栏「搜索」下面点「画图」：只写提示词
  就出图，拖入、粘贴或选几张图片就在它们的基础上改图。模型、尺寸（每档标好单价）、
  质量、张数随手选，预计花费和余额就在「生成」旁边；画好的图存进系统「图片」
  文件夹，也留在图库里随时预览、再改、发到对话。详见下文 [画图](#画图)。
- **电脑操作。** 在 macOS 和 Windows 上，agent 可以直接操作桌面应用（Windows 在
  设置 → 电脑操作里一键下载驱动），也能在对话里调用 `generate_image` 生成图片。
  驱动没装时只是不开放桌面控制，对话照常进行。
- **公告直达。** 标题栏铃铛实时同步服务公告（限时活动、模型上新、维护通知），
  未读红点提醒，客户端内直接阅读。
- **看得清 agent 在做什么。** 翻代码的动作合成一行（如「已探索 · 2 次搜索，3 个
  文件」），命令合成另一行，每一步都写明在做什么、做完了什么；失败的命令标红，
  悬停看输出结尾。思考只占一行，中途发的转向消息立刻出现，前后的工作各自折叠、
  各自计时，「工作了多久」不含等你批准的时间。转录区右上角的状态胶囊随时告诉你
  目标、当前步骤、计划进度和后台任务。
- **出错有交代，改错能撤回。** 一轮失败时输入框上方出现错误条：完整报错、一键
  复制、一键重试（重试不会删文件），不再编一段假回复。每轮的改动卡片可以只撤销
  这一轮改过的文件——之后又被改过的文件原样保留并列出来，文件夹里的其他东西一概
  不碰；点卡片上的文件直接看 diff。
- **批准不打断你。** 多个权限请求排队显示、带计数，按数字键就能作答，Esc 不会
  误停整个任务；「拒绝并说明」直接告诉 agent 该怎么做。应用在后台时有待你处理的
  请求会弹系统通知，侧栏也标出等待数。快捷键：⇧⌘M 切换权限模式、⇧⌘P 切换
  Plan 模式、⌘T 切换推理强度（Windows 用 Ctrl）。
- **完整的 Agent 工作台。** 多项目、多会话并行，消息排队与中途转向，Git
  checkpoint 会话级回滚，worktree 隔离，内置终端、diff 与技能库——上游 Waku
  的全部能力都在。

## 两步上手

1. **安装并登录。** 从 [最新 Release](../../releases/latest) 下载对应平台安装包，
   打开应用点「登录」，浏览器里完成注册——回到客户端即可拉取当前源站的模型目录。
2. **开跑。** 打开一个项目文件夹，输入你想做的事，回车。默认且唯一使用的 Agent
   就是内置 Agent；余额在左下角，随用随充。

进阶：设置 → 模型接口 统一登记你自己的接口——地址、密钥、接口格式、模型列表，
以及可测速自动选优的备用域名。内置 Agent 的 Messages、Responses、Chat Completions
三种接口格式可分别绑定自定义接口；未绑定的格式使用底部当前源站。输入框只展示内置
Agent 实际支持的 `/` 命令、共享技能和自定义命令，不再发现或透传外部 CLI 命令。

## 内置 Agent

内置 Agent 是默认且唯一可启动的 provider。它是编译进客户端的一个完整编码 agent：读写文件、
执行命令、搜索代码、调用 MCP 工具、启动子 agent。会话记录由应用自己保存，回滚、
分支、续聊都不依赖任何外部进程。

**模型与接口格式。** 模型选择器列出网关目录里能对话的模型（Gemini 系列目前只接入 Gemini 4 Argon），
同一个模型只列一次。接口格式由模型家族决定，不需要手动选：

| 模型家族 | 接口格式 | 推理强度 |
|---|---|---|
| Claude | Anthropic Messages | low 至 max，最新型号另有 ultracode |
| GPT / o 系列、Grok | OpenAI Responses | low 至 max |
| DeepSeek、GLM 4.5 及以后、Kimi K3 | OpenAI Chat Completions | low / high / max |
| Kimi K2、MiniMax | OpenAI Chat Completions | 由模型自己决定（接口不提供档位） |
| Gemini 4 Argon | Google Gemini（网关 v1beta） | low / medium / high |

**每个模型走对的分组。** 一个密钥只属于一个分组，而每个分组只提供它支持的那些
模型。内置 Agent 按模型挑分组：Claude、GPT、Grok 使用账号菜单里为对应模型平台
绑定的分组；其余模型优先走提供它的有效订阅，没有订阅时走提供它的、倍率最低的
分组。需要的密钥会自动准备好（优先复用账号里已有的密钥），选择器里走订阅的模型
会注明订阅名。

**Claude 路线省钱又听话。** 走 Claude 时按 Claude Code 的方式打提示缓存断点，
直连 Anthropic 的分组（如 Claude Max）每一步都从缓存读前面的对话，不再整段重付；
推理强度也按 Claude Code 的方式发送（新一代模型用 adaptive thinking 加 effort，
老模型仍用思考预算），网关的用量日志里能看到所选档位，Opus 5.5 选任何档位都
不会报错。

**先规划，再动手。** Plan 模式下 agent 只读代码、写计划，计划经你批准后才开始
修改；它也可以在对话中自己进入 Plan 模式，界面上的模式标签会跟着变。

**目标模式。** `/goal` 给任务定一个目标，agent 一轮接一轮地做下去，直到它确认
目标完成；停止当前一轮会暂停目标，每轮的 token 都计入目标。

**上下文心里有数。** 所有路线（Claude、GPT、Grok、DeepSeek、Kimi、GLM……）都会
按设置里的阈值自动压缩上下文，压缩前后的 token 数留在转录里；`/compact` 手动
压缩，`/context` 看上下文占用，`/cost` 看本次用量。用量面板显示每次请求的输入、
缓存命中和输出。各家 agent 的待办清单统一显示成 ○ / → / ✓，重新打开任务也还在。

**跨平台的命令行。** macOS 上用 bash；Windows 上优先用 Git Bash，没有安装时改用
PowerShell（有 7 就用 7），工作目录在多次调用之间保持，中文输出不乱码，超时会
结束整棵进程树。

**项目指令。** 自动读取项目里的 `AGENTS.md` / `CLAUDE.md`（从全局配置目录到当前
目录逐级读取），与你在设置里写的系统提示一起生效。

**不设步数上限。** 一条消息可以连续执行任意多步，长任务不会被中途截断；想停的时候
随时取消。

**可配置项。** 设置 → 内置 Agent：三种接口格式各用哪个接口、系统提示、自动压缩
与自定义百分比阈值、内置工具开关、MCP 服务器和权限规则。内置 Agent 始终启用。

内置 Agent 的执行内核移植自 [Sailry](https://github.com/sailry/sailry-harness)，
现有 Provider、工具、权限和历史适配继续复用 Claurst 层。代码统一位于
[`crates/agent-core`](crates/agent-core)，其中 `engine` 保留 Sailry 的
`agent/engine`、`agent/model`、`agent/canonical` 结构；所做的修改逐条记录在
[NOTICE.md](NOTICE.md)。

## 画图

侧栏「搜索」下面的「画图」打开一个独立页面，和任务并排，不遮住侧栏。

- **一个输入框，两种画法。** 只写提示词就是文生图；把图片拖到页面上、粘贴截图，
  或点回形针选文件，就变成图生图（PNG / JPEG / WebP，单张不超过 20 MB）。回车或
  点「生成」提交。
- **选项都在手边。** 模型（网关目录里的 gpt-image、Grok 生图模型）、分组（默认
  自动）、尺寸（每档标出 1K / 2K / 4K 和单价）、质量、一次几张，旁边显示预计花费
  和余额。
- **图库。** 最新的在前；生成中的卡片会计时，失败的卡片写明原因（没开生图、没过
  审核、余额不足……），余额不足时直接给出充值入口。每张图可以预览、用作参考图、
  改提示词再画、重新生成、发到对话、复制提示词、在文件夹中显示。
- **存在哪。** 图片在系统「图片」文件夹下的 `CheapRouter/年-月/`，图库记录与参考图
  在 `~/.cheaprouter/image-studio/`。
- **不怕长任务。** 网关支持异步生图时，图在网关上排队生成，关掉窗口也不丢，下次
  打开画图页接着取结果；不支持时自动改用流式请求，不会被代理的超时掐断。
- **自动找能画的分组。** 生图权限按分组单独开放，遇到没开的分组会自动换下一个，
  画成功的分组会被记住；agent 在对话里调用 `generate_image` 时也走这个分组。

## 支持的 agent

只支持 **内置 Agent**。它直接运行在 `client-daemon` 内，支持云网关和自定义模型接口，
无需安装任何外部程序。Claude Code、Codex、Grok Build、OpenCode、Pi、Amp、Cursor、
Fx、Kimi Code 等外部 CLI 不再被检测、配置、导入或启动；已有本地会话镜像仍可查看，
但不能续跑外部 CLI。

## Install

macOS 一键安装（推荐，绕开 Gatekeeper 弹窗）：

```bash
curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
```

Windows 从 [最新 Release](../../releases/latest) 下载 `CheapRouter-*-Setup.exe`
安装（x64 与 arm64 均有）。安装后应用内自动更新。目前不提供 Linux 安装包。

## Architecture

The native desktop is an RPC client of the standalone `client-daemon` process.
Provider sessions run in [`client-core`](crates/client-core), behind the
authenticated, versioned WebSocket contract in
[`client-protocol`](crates/client-protocol). The desktop depends on
[`client`](crates/client), not on the daemon implementation. The
daemon owns task SQLite data, uploaded attachments, provider-native session
forks, and all workspace filesystem and Git operations; paths returned by it
always refer to the daemon host. The desktop retains only presentation state
and a disposable preview cache.

The built-in agent (`ProviderKind::Native`) runs inside the daemon rather than
as a child process: Sailry's execution state machine in
[`crates/agent-core/engine`](crates/agent-core/engine), with the existing
provider, tool, permission, and history adapters in `crates/agent-core/query`,
driven through [`crates/agent-bridge`](crates/agent-bridge) and translated into
the provider contract by `crates/client-core/src/driver/native.rs`. Its transcript is owned by the
daemon, which is what makes rewind, branch and resume plain truncations.

All fork functionality lives in [`crates/sub2api`](crates/sub2api) plus a
handful of view files; see [docs/FORK.md](docs/FORK.md) for the hook-point
register. Routing is desktop-local: the desktop writes only the built-in agent's
own settings (with backups in `~/.cheaprouter/takeover.json`), never an external
CLI's configuration. Business API clients resolve the currently selected gateway
origin; login, token refresh and logout stay on the credential's authentication
origin. Switching origin invalidates origin-scoped caches and rejects late
responses from the previous origin.

The browser client lives at [`apps/web`](apps/web) and uses the generated
browser transport in [`packages/client`](packages/client). Its
checked-in types are generated directly from the Rust protocol. Run
`bun run protocol:generate` after changing a wire type and
`bun run protocol:check` to verify that generated files are current.

App data lives under `~/.cheaprouter` (projectless task workspaces at
`~/.cheaprouter/projects/<date>/<slug>`; the Release desktop writes
`app.json` and daemon settings `settings.json` there, while Debug stays
isolated at `temp/`). Legacy `~/.waku` directories from older builds are
renamed in place at startup.

Release apps bundle and sign `client-daemon`. Development keeps the daemon at
`target/debug/waku-debug-daemon`, allowing provider-only edits to rebuild and
replace the daemon without relaunching the debug build.

## Development

Development is supported on macOS and Windows and requires
[Rust 1.96 or newer](https://www.rust-lang.org/tools/install) and
[Bun](https://bun.sh/). Windows needs the MSVC toolchain; install the native
build prerequisites listed in [CONTRIBUTING.md](CONTRIBUTING.md) first. The
customized GPUI workspace is vendored under `vendor/gpui/`, and reusable UI
components live in `vendor/gpui-kit/` (the `client-ui` crate); no
separate checkout or submodule initialization is needed. Linux
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
