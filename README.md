# CheapRouter

CheapRouter is a fast, native desktop app for working with local coding agents.
It is built in Rust with
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and keeps
projects, sessions, and transcripts on your machine.

> **Built on [Waku](https://github.com/egoist/waku)** by egoist, licensed under
> GPL-3.0-only. This is a modified fork; see [NOTICE.md](NOTICE.md) for the list
> of changes and [docs/FORK.md](docs/FORK.md) for how it tracks upstream.
> Please report issues with this build here, not to the upstream project.

## What this fork adds

- **Managed cloud account.** Sign in through your browser and route agents
  through the hosted gateway. Settings → CheapRouter Account carries the balance,
  top-up, model group selection, gateway pricing per model, code redemption,
  and your invite link; the balance also sits beside the plan usage meter while
  routing is on. Your own `~/.claude` and `~/.codex` are never rewritten:
  routing is applied as process environment at launch, so signing out needs no
  cleanup.
- **Assisted CLI setup.** Settings → Providers detects missing agent CLIs and
  an outdated or absent Node, then installs them for you. Node installs
  unattended with live progress — a portable runtime into an app-managed
  directory on Windows (falling back to the silent MSI, then winget) and the
  official tarball on macOS; on Linux your package manager owns Node and the
  app only reports it. On Windows the app also installs Git unattended
  (portable, per-user installer, winget) — task checkpoints shell out to
  `git`, and Claude Code's mature shell tool is Git Bash. npm installs try the
  mainland mirror first on Windows with the official registry as fallback. A failed install shows the
  installer's own output instead of a generic error, and the command is one
  click away on the clipboard if you would rather run it yourself.

Everything else tracks upstream Waku.

## Install

Download the installer for your platform from the
[latest release](../../releases/latest). Builds update themselves.

## Supported agents

CheapRouter works with:

- [Amp](https://ampcode.com/)
- Claude Code
- Codex CLI
- Cursor CLI
- [Fx](https://fx.sh/)
- Grok Build
- Kimi Code
- OpenCode
- Pi

Settings → Providers tells you what is missing and how to install it. Detection
is automatic, and each provider uses its native structured protocol and session
continuity.

## Highlights

- Keep projects and independent agent sessions in one native app.
- Switch models, reasoning effort, and access modes from a shared interface.
- Queue or steer follow-up messages while an agent is working.
- Rewind Git-backed tasks with conversation-aware checkpoints.
- Store app state locally; the managed cloud account is optional.

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

The browser client lives at [`apps/web`](apps/web) and uses the generated
browser transport in [`packages/waku-client`](packages/waku-client). Its
checked-in types are generated directly from the Rust protocol, while its
WebSocket client implements the same handshake, request IDs, subscriptions,
sequence deduplication, and replay cursors as the Rust client. Run
`bun run protocol:generate` after changing a wire type and
`bun run protocol:check` to verify that generated files are current.

Projectless task workspaces live on the daemon host under
`~/.waku/projects/<date>/<slug>`. The daemon moves workspaces created by the
older `~/.waku/<date>/<slug>` layout on first load.

Configuration ownership is separate too: the Release desktop writes
`~/.waku/app.json`, while Debug stays isolated at `temp/app.json`. Daemon
provider and Computer Use settings live in `~/.waku/settings.json`. The
desktop's Settings → Daemon page can explicitly
expose the child daemon on a fixed port, configure exact browser origins, and
copy its stable authentication token. It remains loopback-only by default.

When connected to a daemon managed outside the desktop process, Waku never
interprets daemon paths on the client machine. The local folder picker and PTY
are therefore unavailable until the protocol gains daemon-host picker and
terminal-stream endpoints; files, diffs, Git, skills, usage, task state, and
attachments already use daemon RPC.

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
