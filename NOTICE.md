# Modification Notice

This repository is a modified fork of [Waku](https://github.com/egoist/waku) by
egoist, licensed under the GNU General Public License v3.0 only (see
[LICENSE](LICENSE)).

Per GPL-3.0 section 5(a), the modifications below are recorded with their dates.
This fork remains licensed under GPL-3.0-only.

## Modifications

| Date | Change |
|---|---|
| 2026-08-24 | Forked from upstream `egoist/waku` at commit `d82304a`. |
| 2026-08-24 | Added `NOTICE.md` (this file) and `docs/FORK.md` (fork maintenance workflow). |
| 2026-08-24 | Added `src/brand.rs`: single-source product branding (name, bundle id, update feed, telemetry opt-out). |
| 2026-08-24 | Added the `sub2api` crate (`crates/sub2api`): managed cloud account integration (browser sign-in over a loopback redirect, credential storage, token refresh), gateway routing configuration, and agent CLI detection with install-command generation. |
| 2026-08-24 | Added `src/app/cloud_account.rs`: Settings → Cloud Account page. |
| 2026-08-24 | Added `src/app/cli_setup.rs`: missing-agent setup section on Settings → Providers. |
| 2026-08-24 | `crates/waku-core`: added `command_env::command_for_provider()` and used it at the Claude and Codex spawn sites, so agents inherit gateway routing as process environment. |
| 2026-08-24 | `src/analytics.rs`: telemetry disabled unless the build opts in, so this fork does not report to upstream's endpoint. |
| 2026-08-24 | `src/updater.rs`, `resources/Info.plist`, `build.rs`, `scripts/release.ts`: retargeted updates and bundle identity at our own release channel. |
| 2026-08-24 | Replaced the application icons (`resources/AppIcon.icns`, `resources/AppIconDev.icns`, `resources/windows/AppIcon.ico`, `resources/linux/icons/`) and the Linux desktop entry name with our own brand artwork. |
| 2026-08-24 | Added hosted top-up entry and agent CLI installation to the settings pages. |
| 2026-08-24 | Added model group selection, gateway model pricing, code redemption, and referral details to Settings → Cloud Account, and a balance chip to the composer status strip. |
| 2026-08-24 | Added `crates/sub2api/src/node_install.rs`: unattended Node 22 installation (portable zip / MSI / winget on Windows, mirror tarball on macOS) with staged progress, and injected the managed runtime into the `PATH` of spawned agent processes. |
| 2026-08-25 | Added `crates/sub2api/src/git_install.rs`: unattended Git for Windows installation (PortableGit / silent per-user installer / winget / GitHub direct), `CLAUDE_CODE_GIT_BASH_PATH` export for spawned Claude sessions, and Git on the spawned-process `PATH` so task checkpoints work. |
| 2026-08-25 | Added `crates/sub2api/src/codex_compat.rs` and used it at the Codex spawn sites: `app-server` arguments resolved from the binary's own `--help`, restoring compatibility with Codex versions that reject `--stdio`. |
| 2026-08-25 | Replaced every user-visible "Waku" in the UI strings (locales and hardcoded messages) with neutral wording or this fork's brand; `APP_NAME` now carries the brand. Upstream attribution remains in the README, NOTICE, and source comments. |
| 2026-08-25 | `crates/waku-protocol/src/i18n.rs`: fixed "follow the system language" on Windows by querying `GetUserDefaultLocaleName` — upstream read Unix env vars (`LANG` etc.), which Windows does not set, so the app always started in English there. |
| 2026-08-25 | Added a cloud sign-in card to the welcome screen and an account chip to the sidebar footer. |

## Upstream attribution

Waku is developed by egoist and contributors. Upstream source, issue tracker,
and releases: <https://github.com/egoist/waku>.

The GPUI framework is developed by Zed Industries:
<https://github.com/zed-industries/zed>.
