# Fork Maintenance

This is a fork of [egoist/waku](https://github.com/egoist/waku) that adds managed
cloud account integration and assisted agent-CLI installation, and ships under
our own brand. Everything else tracks upstream.

## Remotes and branches

| Remote | Points at |
|---|---|
| `origin` | our fork (**must be repointed after creating the org fork** — currently still upstream) |
| `upstream` | `https://github.com/egoist/waku.git` |

| Branch | Role |
|---|---|
| `main` | mirrors `upstream/main`, never edited directly |
| `integration` | our default branch; all local work lands here |

Repoint `origin` once the org fork exists:

```bash
git remote set-url origin https://github.com/<org>/<fork>.git
```

## Weekly upstream merge

Upstream averages ~15 commits/day. Merge weekly — letting it drift makes
conflicts disproportionately worse.

```bash
git fetch upstream
git checkout main && git merge --ff-only upstream/main
git checkout integration && git merge main
```

Conflicts can only appear in the hook points listed below. If a conflict shows up
anywhere else, our change leaked outside its module — move it back into a
dedicated file.

## Design rule

**All new functionality lives in new files. Upstream files get minimal hook
points only** — ideally one to three lines each. This is the only thing keeping
the weekly merge cheap.

## Hook point register

Keep this table current. It is the checklist to walk after every upstream merge.

All fork logic lives in `crates/sub2api` (no GPUI, no upstream crates,
independently tested) plus two new view files. Upstream files carry only the
lines below.

| Upstream file | Our change | Lines |
|---|---|---|
| `Cargo.toml` (root) | `crates/sub2api` in `members`/`default-members`; `sub2api` dependency | 3 |
| `crates/waku-core/Cargo.toml` | `sub2api` dependency | 1 |
| `crates/waku-core/src/command_env.rs` | added `command_for_provider()` beside `command()` (managed Node runtime on `PATH`; routing itself is written into each CLI's own config by the desktop — `sub2api::global_config`); managed Node dirs appended to `executable_search_paths()` so a just-installed CLI is detected without a restart | +18 |
| `crates/waku-core/src/driver/claude.rs` | spawn uses `command_for_provider(.., "claude")` | 1 |
| `crates/waku-core/src/driver/codex.rs` | same, at both spawn sites (session + title turn) | 2 |
| `src/app.rs` | fork `mod` lines, `SettingsPage::{CloudAccount, ModelPlaza, CloudUsage}`, fork struct fields + initializers (cloud account, cli setup, custom API inputs, plaza, pay modal), startup refresh loop | ~70 |
| `src/app/render.rs` | pay-modal and announcements-modal composites in both render branches | 10 |
| `src/app/tests.rs` | `settings_search_filters_pages_for_arrow_cycling` expects the fork's nav pages | 3 |
| `src/assets.rs` | `bell`/`store`/`wallet` icon entries; embedded `images/logo.png` brand mark | ~11 |
| `src/app/runtime.rs` | `cloud_balance_stale` set at the turn-settlement seam, drained in the event pump | 6 |
| `src/app/sidebar.rs` | empty-state icon swapped for the brand mark; announcements bell in the window header (plus onboarding card + footer chip rows) | 6 |
| `src/app/composer.rs` | balance chip in the status strip | 3 |
| `resources/AppIcon*.icns`, `resources/windows/AppIcon.ico`, `resources/linux/` | brand artwork and desktop entry name | assets |
| `scripts/bundle-linux.sh` | installs the brand icon | 5 |
| `src/app/settings.rs` | nav entries, title arms, dispatch arms, `SETTINGS_PAGES` length (7 upstream → 10), one `.child(self.render_cli_setup_section(cx))` | ~24 |
| `src/updater.rs` | Windows appcast URL built from the brand env var | 8 |
| `src/analytics.rs` | early return unless `brand::ANALYTICS_ENABLED` | 4 |
| `build.rs` | `export_brand()`; Windows version block uses the brand | ~25 |
| `resources/Info.plist` | bundle identity + `SUFeedURL` | 6 |
| `scripts/release.ts` | `appName`/`executableName` from the brand | 6 |
| `scripts/appcast.ts` | default download prefix points at our release host | 3 |
| `scripts/delete-debug-app.ts` | branded debug data dirs added to the cleanup candidates | 4 |
| `locales/{app,ja,zh-CN}.yml` | our new `cloud.*`/`cli_setup.*` keys, plus a de-brand sweep: every user-visible "Waku" replaced (neutral wording, or `CheapRouter` where a name is load-bearing — consent prompts, hero copy, composer placeholder) | ~85 lines |
| `crates/waku-protocol/src/identity.rs` | `APP_NAME` reads `SUB2API_BRAND_NAME`; `DATA_DIR_NAME` (".cheaprouter") reads `SUB2API_DATA_DIR_NAME`; `DATA_DIRECTORY_NAME` is "CheapRouter"/"CheapRouter Debug" (defaults mirror `brand.rs` — keep in sync); `APP_ID` stays upstream | ~15 |
| `crates/waku-protocol/src/settings.rs`, `crates/waku-protocol/src/projectless.rs`, `crates/waku-protocol/src/model.rs` (test) | `.waku` literal → `identity::DATA_DIR_NAME` | 3 sites |
| `crates/waku-core/src/{persistence,projectless,worktree,computer_use,daemon}.rs` | `.waku`/"Waku" literals → `identity::DATA_DIR_NAME`/`DATA_DIRECTORY_NAME` (incl. one test and one error string) | 6 sites |
| `crates/waku-core/src/composer_complete.rs` | command dirs renamed to `.cheaprouter/commands` and `~/.config/cheaprouter/commands`; upstream's `.waku` locations still scanned as a compatibility layer | ~14 |
| `crates/waku-client/src/persistence.rs` | `.waku` literal → `identity::DATA_DIR_NAME` | 1 |
| `crates/waku-daemon/src/main.rs`, `crates/waku-daemon/Cargo.toml` | `sub2api::migrate::migrate_legacy_storage()` before path resolution (standalone daemon starts); `sub2api` dependency | 5 |
| `src/lib.rs` | same migration call at the top of `run()` | 5 |
| `crates/waku-protocol/src/i18n.rs` | Windows `system_locale()` via `GetUserDefaultLocaleName` (upstream's env-var probe always yielded English on Windows); two test expectations follow the brand | ~25 |
| `crates/waku-core/src/driver/codex.rs` | `app-server` args resolved per binary via `sub2api::codex_compat` (old Codex rejects `--stdio`) | 2 sites |
| `src/daemon.rs`, `src/driver/mod.rs`, `src/app/runtime.rs`, `src/analytics.rs`, `src/js_repl.rs`, `src/bin/waku_js_repl.rs` | user-visible "Waku" strings neutralized or branded | ~14 lines |

Rebranding later: change `brand.rs`/`SUB2API_BRAND_NAME` **and** sweep
`CheapRouter` in `locales/` and the two i18n test expectations.

### Files that are ours entirely

`crates/sub2api/**`, `src/app/cloud_account.rs`, `src/app/cli_setup.rs`,
`src/app/cloud_usage.rs`, `src/app/model_plaza.rs`, `src/app/cloud_pay.rs`,
`src/app/announcements.rs`, `assets/icons/{bell,store,wallet}.svg`,
`NOTICE.md`, `docs/FORK.md`.

### Conflict triage

- A conflict in one of the listed files: reapply the line, tick the row.
- A conflict anywhere else: our change leaked. Move it back into
  `crates/sub2api` or one of our own view files.
- `SETTINGS_PAGES` has a hard-coded length; upstream adding a page turns that
  into a type error rather than a silent break, which is the desired failure.

## What we deliberately do not touch

- `crates/waku-protocol` — the wire contract. Routing is desktop-local (the
  desktop writes each CLI's own global configuration; the daemon carries no
  routing state), so the protocol stays byte-identical to upstream and the
  browser client keeps working unchanged. (`identity.rs` constants are branded,
  but no message shape changes.)
- Provider drivers' protocol handling.

User data now lives under `~/.cheaprouter` (platform folders `CheapRouter`
/ `CheapRouter Debug`); `sub2api::migrate` renames the legacy `~/.waku` /
`Waku` directories in place at startup, from both the desktop and daemon
entry points.

## Routing contract (cc-switch model)

`sub2api::global_config` edits the live CLI configs — `~/.claude/settings.json`
(env block, deep-merged), `~/.codex/auth.json` + `config.toml` (toml_edit,
comments preserved), `~/.grok/config.toml`, `opencode.json` and Pi's
`models.json` (one additive `cheaprouter` provider entry each). Before the
first write per CLI the originals are backed up into
`~/.cheaprouter/takeover.json` and restored on sign-out / clearing the
endpoint. Pi's `auth.json` and `settings.json` are never read or written.

## License

Fork stays GPL-3.0-only. Record every modification in [NOTICE.md](../NOTICE.md)
with its date (GPL §5(a)) and keep the "Built on Waku" attribution in the README.
