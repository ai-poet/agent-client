# CheapRouter

CheapRouter 把「低价模型中转站」和「原生 Agent 工作台」合成了一个桌面应用：
一端是价格实惠的 Claude / GPT / Grok 模型网关，另一端是用 Rust +
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
写的本地编码 agent 指挥台。登录即用，开箱即路由，新手不用碰任何配置文件，
老手也能精确掌控每一份 CLI 配置。

> **Built on [Waku](https://github.com/egoist/waku)** by egoist, licensed under
> GPL-3.0-only. This is a modified fork; see [NOTICE.md](NOTICE.md) for the list
> of changes and [docs/FORK.md](docs/FORK.md) for how it tracks upstream.
> Please report issues with this build here, not to the upstream project.

## 为什么选 CheapRouter

- **登录即路由，零配置。** 浏览器里登录云账号，客户端就把网关地址和密钥直接写进
  每个 CLI 自己的全局配置——Claude Code、Codex、Grok Build、OpenCode、Pi 全部对齐。
  接管前自动备份你的原配置，退出登录时原样恢复；不注入环境变量、不架本地代理、
  不需要手改任何文件。终端里直接跑 `claude` / `codex` 也同样走网关。
- **缺什么装什么。** 全新电脑没装 Node、没装任何 CLI 也没关系：设置 → Providers
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
  标注倍率（如 ×0.50）、24 小时在线率和降级/故障状态，不用打开网页就能挑出
  又便宜又稳的线路；高峰期换个分组接着跑。
- **公告直达。** 标题栏铃铛实时同步服务公告（限时活动、模型上新、维护通知），
  未读红点提醒，客户端内直接阅读。
- **完整的 Agent 工作台。** 多项目、多会话并行，消息排队与中途转向，Git
  checkpoint 会话级回滚，worktree 隔离，内置终端、diff 与技能库——上游 Waku
  的全部能力都在。

## 三步上手

1. **安装并登录。** 从 [最新 Release](../../releases/latest) 下载对应平台安装包，
   打开应用点「登录」，浏览器里完成注册——回到客户端一切就绪，路由已自动配置。
2. **补齐工具链。** 设置 → Providers 一键安装缺失的 CLI（需要的 Node 运行时
   也会自动装好）。已经装过的会被直接识别，不重复安装。
3. **开跑。** 打开一个项目文件夹，输入你想做的事，回车。余额在左下角，
   随用随充。

进阶：五个 CLI 也可以各自指定自定义 base URL / API key（设置 → Providers，
卡片式管理，带连通性检测和「打开配置文件」入口）；云路由和自定义接口共存时
云路由优先。`cargo run -p sub2api --example routing_doctor` 可以随时对照
期望路由与磁盘上的实际配置。

## 支持的 agent CLI

| CLI | 云网关路由 | 自定义接口 | 一键安装 |
|---|---|---|---|
| Claude Code | ✓ | ✓ | ✓ |
| Codex CLI | ✓ | ✓ | ✓ |
| Grok Build | ✓ | ✓ | ✓ |
| OpenCode | — | ✓ | — |
| Pi | — | ✓ | — |

会话协议层面还支持 [Amp](https://ampcode.com/)、Cursor CLI、
[Fx](https://fx.sh/)、Kimi Code 等（自带配置使用）。每个 provider 走各自的
原生结构化协议，会话可延续。

## Install

Download the installer for your platform from the
[latest release](../../releases/latest). Builds update themselves.

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

Development is supported on macOS, Linux, and Windows and requires
[Rust 1.96 or newer](https://www.rust-lang.org/tools/install) and
[Bun](https://bun.sh/). Linux supports both Wayland and X11, and Windows needs
the MSVC toolchain; install the native build prerequisites listed in
[CONTRIBUTING.md](CONTRIBUTING.md) first.

```sh
bun install
bun run dev
```

The embedded browser and experimental computer-use integration currently
remain macOS-only. Agent sessions, projects, transcripts, skills, usage,
diffs, file editing, and the terminal run natively on Linux and Windows.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and checks.
Release maintainers should also read [RELEASING.md](RELEASING.md).

## Upstream

This fork exists because of upstream Waku. You can support its development via
[GitHub Sponsors](https://github.com/sponsors/egoist).

## License

Licensed under the [GNU General Public License v3.0 only](LICENSE), the same
license as upstream Waku. Modifications are recorded in [NOTICE.md](NOTICE.md).
