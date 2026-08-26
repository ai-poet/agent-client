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
| `crates/waku-core/src/command_env.rs` | added `command_for_provider()` beside `command()` (gateway env + managed Node runtime on `PATH`) | +17 |
| `crates/waku-core/src/driver/claude.rs` | spawn uses `command_for_provider(.., "claude")` | 1 |
| `crates/waku-core/src/driver/codex.rs` | same, at both spawn sites (session + title turn) | 2 |
| `src/app.rs` | `mod cli_setup; mod cloud_account;`, `SettingsPage::CloudAccount`, three struct fields + their initializers, startup refresh loop | ~40 |
| `src/app/composer.rs` | balance chip in the status strip | 3 |
| `resources/AppIcon*.icns`, `resources/windows/AppIcon.ico`, `resources/linux/` | brand artwork and desktop entry name | assets |
| `scripts/bundle-linux.sh` | installs the brand icon | 5 |
| `src/app/settings.rs` | nav entry, title arm, dispatch arm, `SETTINGS_PAGES` length 7→8, one `.child(self.render_cli_setup_section(cx))` | ~12 |
| `src/updater.rs` | Windows appcast URL built from the brand env var | 8 |
| `src/analytics.rs` | early return unless `brand::ANALYTICS_ENABLED` | 4 |
| `build.rs` | `export_brand()`; Windows version block uses the brand | ~25 |
| `resources/Info.plist` | bundle identity + `SUFeedURL` | 6 |
| `scripts/release.ts` | `appName`/`executableName` from the brand | 6 |
| `locales/{app,ja,zh-CN}.yml` | our new `cloud.*`/`cli_setup.*` keys, plus a de-brand sweep: every user-visible "Waku" replaced (neutral wording, or `CheapRouter` where a name is load-bearing — consent prompts, hero copy, composer placeholder) | ~85 lines |
| `crates/waku-protocol/src/identity.rs` | `APP_NAME` reads `SUB2API_BRAND_NAME` (same default as `brand.rs` — keep in sync); `APP_ID`/data dir stay upstream | 8 |
| `crates/waku-protocol/src/i18n.rs` | Windows `system_locale()` via `GetUserDefaultLocaleName` (upstream's env-var probe always yielded English on Windows); two test expectations follow the brand | ~25 |
| `crates/waku-core/src/driver/codex.rs` | `app-server` args resolved per binary via `sub2api::codex_compat` (old Codex rejects `--stdio`) | 2 sites |
| `src/daemon.rs`, `src/driver/mod.rs`, `src/app/runtime.rs`, `src/analytics.rs`, `src/js_repl.rs`, `src/bin/waku_js_repl.rs` | user-visible "Waku" strings neutralized or branded | ~14 lines |

Rebranding later: change `brand.rs`/`SUB2API_BRAND_NAME` **and** sweep
`CheapRouter` in `locales/` and the two i18n test expectations.

### Files that are ours entirely

`crates/sub2api/**`, `src/app/cloud_account.rs`, `src/app/cli_setup.rs`,
`NOTICE.md`, `docs/FORK.md`.

### Conflict triage

- A conflict in one of the listed files: reapply the line, tick the row.
- A conflict anywhere else: our change leaked. Move it back into
  `crates/sub2api` or one of our own view files.
- `SETTINGS_PAGES` has a hard-coded length; upstream adding a page turns that
  into a type error rather than a silent break, which is the desired failure.

## What we deliberately do not touch

- `crates/waku-protocol` — the wire contract. Cloud account state is
  desktop-local in v1, so the daemon protocol stays byte-identical to upstream
  and the browser client keeps working unchanged.
- Provider drivers' protocol handling — we only inject environment at spawn.
- User data layout (`~/.waku/`) — no migration story in v1.

## License

Fork stays GPL-3.0-only. Record every modification in [NOTICE.md](../NOTICE.md)
with its date (GPL §5(a)) and keep the "Built on Waku" attribution in the README.
