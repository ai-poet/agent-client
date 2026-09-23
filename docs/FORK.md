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
| `Cargo.toml` (root) | `crates/sub2api` and `crates/workflow-engine` in `members`/`default-members`; `sub2api` and `workflow-engine` dependencies | 6 |
| `crates/waku-core/Cargo.toml` | `sub2api` dependency | 1 |
| `crates/waku-core/src/command_env.rs` | added `command_for_provider()` beside `command()` (calls `sub2api::cli_install::apply_provider_launch_env`: managed Node runtime on `PATH`, Claude's nonessential-traffic switch off; routing itself is written into each CLI's own config by the desktop — `sub2api::global_config`); `sub2api::cli_detect::detection_dirs()` (managed runtime, version-manager dirs, remembered npm prefixes) appended to `executable_search_paths()` so a just-installed CLI is detected without a restart | +18 |
| `crates/waku-core/src/checkpoint.rs` | `snapshot_tree` seeds the temporary index from the repository's own index (stat cache) before `add -A`, falling back to the HEAD rebuild; `repository_index_path` helper | ~45 |
| `src/app/runtime.rs` | `prepare_submission` runs the turn-start snapshot and the provider start on two threads instead of one after the other; warm-start hooks for `runtime_prewarm` — `start_driver` takes a handed-over process (parked, or waited for while it boots), the submission path attaches that claim to its request, the idle sweep calls `reap_unused_prewarms`; `start_driver`, `driver_start_request_for_session`, `install_prepared_driver` are `pub(super)` | ~35 |
| `crates/waku-core/src/model_catalog.rs`, `crates/waku-protocol/src/model_catalog.rs` | `claude-fable-5-1` first in the curated Claude list (with `claude-opus-5-5`), and `gpt-6-astra` then `gpt-6-sol` first in the curated Codex list, which no longer carries `gpt-5.6-luna` (the Codex default stays `gpt-5.6-sol`); `grok-4.7` beside `grok-4.6` in `grok_model_reasoning_efforts`; waku-core additionally merges curated entries the CLI did not return (`merge_claude_catalog` / `with_curated_fallback`, Claude only) at the end of `discover_catalog`; the merge lives in a fork `discover_claude_catalog`, which `discover_catalog` calls for Claude, so upstream's `discover_claude_models` and its tests stay untouched | 1 + ~60 |
| `crates/waku-core/src/model.rs` | `apply_cached_models` runs the cached Claude catalog through `with_curated_fallback` | 1 |
| `src/lib.rs` | `init_confirm_dialog_keys(cx)` beside the other dialog key inits; `cx.bind_keys(surface_key_bindings())` right after the upstream bindings; the View menu's `items` chained with `surface_menu_items()` | 5 |
| `src/app/components.rs` | `resend_action` threaded through `MessageRender`, `render_message_footer`, `message_menu_items`; one footer child and one menu item from `message_resend` | ~10 |
| `src/app/transcript_view.rs` | `resend_action_for_message` computed beside `user_message_action`, passed into `MessageRender` (+ a `None` at the assistant footer call); `scroll_transcript_to_bottom` is `pub(super)` | 4 |
| `src/app/task_switcher.rs` | failed-task glyph is `circle-x` | 1 |
| `crates/waku-core/src/driver/claude.rs` | spawn uses `command_for_provider(.., "claude")` | 1 |
| `crates/waku-core/src/driver/codex.rs` | same, at both spawn sites (session + title turn) | 2 |
| `src/app.rs` | fork `mod` lines, `SettingsPage::{CloudAccount, ModelPlaza, CloudUsage}`, fork struct fields + initializers (cloud account, cli setup, custom API inputs, plaza, pay modal, confirm dialog, onboarding, runtime prewarms), `DriverStartRequest.prewarmed`, `init_confirm_dialog_keys` re-export, startup refresh loop (also kicks off CLI detection and loads onboarding state), `subscribe_custom_api_inputs` beside the other input subscriptions, `maybe_prewarm_selected_runtime` in the composer's `Edited` arm; `update_ui` field + `on_updater_event` call in `handle_updater_event`; `mod surface_bar` and its `surface_key_bindings` / `surface_menu_items` re-export; `SettingsPage::Workflow`, `mod workflow`, `workflow: WorkflowState` field + initializer | ~100 |
| `crates/waku-core/src/driver/mod.rs` | `mod turn_diagnosis;` | 2 |
| `crates/waku-core/src/driver/acp.rs` | the captured stderr is a `turn_diagnosis::ProviderStderr` ring instead of a bare `Vec` (its 128-line cap moves into that type), threaded into `run_sdk_connection`, `send_prompt` and the `_x.ai/session/prompt_complete` handler; `AcpStreamState` records this turn's `stderr_mark` and `wire_offset`; both prompt-settle paths call `turn_diagnosis::empty_turn_failure` (generalizing the Kimi-only lookup) and pass `produced_content` to `finish_prompt`, whose `EndTurn` arm now names an empty turn instead of leaving upstream's "Turn completed" fallback | ~117 |
| `src/app.rs` | fork `mod` lines, `SettingsPage::{CloudAccount, ModelPlaza, CloudUsage}`, fork struct fields + initializers (cloud account, cli setup, custom API inputs, plaza, pay modal, confirm dialog, onboarding, runtime prewarms), `DriverStartRequest.prewarmed`, `init_confirm_dialog_keys` re-export, startup refresh loop (also kicks off CLI detection and loads onboarding state), `subscribe_custom_api_inputs` beside the other input subscriptions, `maybe_prewarm_selected_runtime` in the composer's `Edited` arm; `update_ui` field + `on_updater_event` call in `handle_updater_event`; `mod surface_bar` and its `surface_key_bindings` / `surface_menu_items` re-export | ~96 |
| `src/app/render.rs` | pay-modal, announcements-modal and confirm-dialog composites in both render branches; onboarding strip above the composer; update banner above the header (main) and above the settings page (settings branch is now a flex column); `open_surface_action` registered beside `toggle_right_panel_action` | ~23 |
| `src/app/tests.rs` | `settings_search_filters_pages_for_arrow_cycling` expects the fork's nav pages (Workflow included) | 4 |
| `crates/waku-agent/core/src/lib.rs` | vendored engine, recorded departure: `#[serde(default)]` on `Config` so a partial `config` block in settings.json loads | 1 + comment |
| `crates/waku-agent/query/src/lib.rs` | vendored engine, recorded departure: an explicit `config.provider` outranks the model-name family table; a stream `error` event ends the turn | ~14 |
| `crates/waku-agent/api/src/lib.rs` | vendored engine, recorded departure: `StreamAccumulator` keeps the first stream `error` instead of discarding it | ~18 |
| `crates/waku-agent/core/src/system_prompt.rs` | vendored engine, recorded departure: the agent is named after the product, not after the engine or Anthropic | ~25 |
| `crates/waku-agent/query/src/runner/provider_options.rs` | vendored engine, recorded departure: Grok counts as a reasoning model, so its effort tier reaches the request; gpt-5 Codex's summary/include fields stay off it | ~14 |
| `crates/waku-agent/tools/src/{pty_bash,powershell,web_fetch}.rs` | vendored engine, recorded departure: truncate on character boundaries (`floor_char_boundary` / `ceil_char_boundary` in `pty_bash`) — the byte slices panicked on long non-ASCII output | ~30 |
| `crates/waku-agent/core/src/lib.rs` | vendored engine, recorded departure: the plan-mode arm allows the plan-safe tools and read-only invocations; the two plan switches stay read-level so the model never asks permission to restrict itself | ~30 |
| `crates/waku-agent/core/src/bash_classifier.rs` | vendored engine, recorded departure: `is_read_only_bash_command` — stricter than the `Safe` tier, splits on every separator and denies on doubt | ~100 |
| `crates/waku-agent/api/src/providers/codex.rs` | vendored engine, recorded departure: `decode_tool_arguments` accepts the object form a normalizing gateway returns, not only the specified JSON string | ~35 |
| `crates/waku-agent/api/src/{lib,provider_types}.rs` | vendored engine, recorded departure: the two stream accumulators warn instead of silently turning unparseable tool arguments into `{}` (the agent loop already errors — issue #215) | ~20 |
| `crates/waku-agent/query/src/runner/tools.rs` | vendored engine, recorded departure: `whole_floats_to_integers` at the one `.execute()` call site — repairs `120.0` for `usize` fields, which several non-Claude models emit | ~45 |
| `crates/waku-agent/tools/src/exit_plan_mode.rs` | vendored engine, recorded departure: `self_gates` and asks through `check_permission` with the plan summary as the description — its declared level is `None`, which the central backstop never gates, so the bridge's "finished planning" dialog was unreachable | ~20 |
| `crates/waku-agent/tools/src/lib.rs` | vendored engine, recorded departure: refusals name their reason, and the two the fork words itself are exported as markers for the driver to localize | ~40 |
| `crates/waku-agent-bridge/src/computer_use.rs` | fork-owned: writes the bundled skill where the engine's `Skill` tool reads it, and removes it when the toggle is off | ~110 |
| `crates/waku-agent-bridge/src/{config,permission,session}.rs` | fork-owned: the REPL MCP registration, the consented-tools short-circuit, the plan/computer-use prompt rules | ~200 |
| `crates/waku-agent-bridge/src/{mcp_tool,events}.rs` | fork-owned: MCP image content onto its own sideband so the model reads text and the transcript gets pixels | ~90 |
| `src/js_repl_image.rs`, `src/js_repl.rs` | fork-owned: `generate_image` as a third REPL tool, credentials read from the engine settings rather than the environment | ~380 |
| `src/app.rs`, `src/app/{runtime,settings}.rs` | fork: Computer Use reachable in release builds on macOS and Windows, plus the `cua-driver` install card | ~230 |
| `resources/computer-use/SKILL.windows.md` | fork: the Windows variant `scripts/bundle-windows.ts` already expected; pinned in step with the macOS one by a guard test | ~237 |
| `crates/waku-core/src/driver/{claude,acp}.rs` | fork: Computer Use for Claude Code (`--mcp-config` + `--plugin-dir`) and for Cursor/Fx (ACP `mcpServers`) | ~90 |
| `crates/waku-core/src/driver/claude.rs` (plan mode) | launches in the access mode and enters plan mode with a `set_permission_mode` control request, so an approved plan returns to the user's access mode (the CLI's `prePlanMode`) instead of `default`; `system/status` `permissionMode` tracked in a shared `plan_mode` flag and reported as `InteractionModeUpdated`; `apply_options` toggles plan mode in place and compares against the live mode; `ExitPlanMode` is never auto-approved and is shown as a plan dialog (`request_plan_approval`, `plan.*` keys, `keep_planning` answer); while planning, only plan-file writes follow the access mode's auto-approval (`writes_the_plan_file`) | ~200 + tests |
| `crates/sub2api/src/claude_compat.rs` | fork: probes `claude --help` once per binary for `--plugin-dir` | ~70 |
| `crates/waku-core/src/skills.rs`, `waku-protocol/src/skills.rs` | fork: the bundled-skill catalogue and its installer, with the target list as the write boundary | ~180 |
| `src/app/skills_page.rs` | fork: the "Built in" card and its per-CLI install action | ~110 |
| `crates/waku-core/src/settings.rs` | the daemon adopts the interface language the desktop pushes, so its own `tr!` strings are not always English | ~12 |
| `crates/waku-core/src/git_commit.rs` | its provider-argument test names `ProviderKind::Native` (skipped by the `is_builtin` guard above it); without the arm the crate's tests do not compile | 1 |
| `crates/waku-protocol/src/settings.rs` | `DaemonSettings::LOCALE_KEY` and its accessors — the language the daemon renders in | ~14 |
| `crates/waku-client/src/persistence.rs` | `daemon_settings()` stamps the interface language into the push | ~6 |
| `src/assets.rs` | `provider-waku` in the embedded icon list — the built-in provider's icon had never been registered | 1 |
| `crates/waku-protocol/src/model.rs` | `ProviderKind::Native` and its `is_builtin()`; Native excluded from `supports_model_discovery` (its catalog comes from the gateway, not a CLI) | ~8 |
| `src/app/runtime.rs` | `sync_native_models()` after `drain_provider_detection_events` / `drain_provider_probe_events`, so daemon probes never replace the built-in agent's catalog list | 2 |
| `src/app/settings.rs` | `sync_native_models()` after the language-change fallback reset | 1 |
| `src/app/sessions.rs`, `src/app/composer.rs` | `refresh_native_catalog` when the built-in agent's rail is opened or selected; `picker_rail_shows_provider` treats built-in providers as installed; the built-in agent's format bar and brand chip in the picker | ~10 + fork fns |
| `src/app.rs` | provider probes seeded `installed: provider.is_builtin()` | 1 |
| `src/assets.rs` | `bell`/`circle-x`/`store`/`wallet` icon entries; embedded `images/logo.png` brand mark | ~12 |
| `src/app/runtime.rs` | `cloud_balance_stale` set at the turn-settlement seam, drained in the event pump; `workflow.pending_settles` pushed at the same seam and `drain_workflow_settles` called beside that drain; `submit_submission_for_session` widened to `pub(super)` for stage starts | 11 |
| `src/app/sidebar.rs` | both empty states' icon (no project / project open) swapped for the brand mark; announcements bell in the window header; onboarding checklist + footer chip rows in the empty state; task rows carry a hover group, the failure badge and the remove button from `task_rows`; `localized_session_title` is `pub(super)`; the surface bar (`render_surface_bar`) in the window header where the panel toggle used to be (the toggle's `.child` line removed, fps counter kept) | 13 |
| `src/app/composer.rs` | balance chip in the status strip | 3 |
| `resources/AppIcon*.icns`, `resources/windows/AppIcon.ico`, `resources/linux/` | brand artwork and desktop entry name | assets |
| `scripts/bundle-linux.sh` | installs the brand icon | 5 |
| `src/app/settings.rs` | nav entries, title arms, dispatch arms, `SETTINGS_PAGES` length (7 upstream → 13); the Providers arm dispatches to the fork's `render_providers_page` (upstream's `render_providers_settings` kept under `#[allow(dead_code)]`); `render_provider_expanded_settings`, `toggle_provider_expanded`, `set_provider_enabled`, `detection_checked_label`, `abbreviate_home_path` widened to `pub(super)`; General page appends `render_update_check_card` after the automatic-updates toggle, outside the `updater_available` guard so the row shows in every build; `open_surface_action` registered beside `toggle_right_panel_action`; Workflow page: nav entry, title and dispatch arms, `fills_viewport` and wide `max_w` arms | ~38 |
| `src/app/right_panel.rs` | the panel header no longer renders `render_right_panel_toggle` beside the window controls (the fn stays, under `#[allow(dead_code)]`, so upstream edits to it merge cleanly) | 3 |
| `src/app/command_palette.rs` | `PaletteAction::OpenSurface(SurfaceKind)`; one `commands.extend(..)` statement after the right-panel toggle command building the four surface commands; its dispatch arm | 3 |
| `src/updater.rs` | Windows appcast URL built from the brand env var; `StagedUpdate.version` and `Updater::available_version()` on all three implementations, for the update banner and the settings row | ~25 |
| `src/app/sidebar.rs` (updater) | `start_available_update` is `pub(super)` so the banner and the settings row install through the same path as the footer pill | 1 |
| `src/analytics.rs` | early return unless `brand::ANALYTICS_ENABLED` | 4 |
| `build.rs` | `export_brand()`; Windows version block uses the brand | ~25 |
| `resources/Info.plist` | bundle identity + `SUFeedURL` | 6 |
| `scripts/release.ts` | `appName`/`executableName` from the brand | 6 |
| `scripts/appcast.ts` | default download prefix points at our release host | 3 |
| `scripts/delete-debug-app.ts` | branded debug data dirs added to the cleanup candidates | 4 |
| `locales/{app,ja,zh-CN}.yml` | our new `cloud.*`/`cli_setup.*`/`surface_bar.*`/`workflow.*` keys, plus a de-brand sweep: every user-visible "Waku" replaced (neutral wording, or `CheapRouter` where a name is load-bearing — consent prompts, hero copy, composer placeholder) | ~300 lines |
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
| `crates/waku-client/src/client.rs` | `DAEMON_DISCONNECTED` / `DAEMON_DROPPED` / `DAEMON_CONNECTION_CLOSED` wire-text constants (the five literals read them) + `is_daemon_transport_error`; `disconnected_for_test` | ~30 |
| `crates/waku-client/src/lib.rs` | re-exports of the above | 4 |
| `crates/waku-client/src/process.rs` | local socket reconnect: `DaemonProcess` keeps `client_address`/`token` (`endpoint`, `adopt_client`); `DaemonTarget::reconnect_endpoint`; `publish_client`, `reopen_socket`, `plan_local_recovery`, `RedialState` + `redial_backoff`; `monitor_daemon` rewritten around them (the remote branch redials through the same path, the dial never runs under the target lock, six refused redials fall back to `replace_local_daemon`); `DaemonSupervisor::restart`; lifecycle tests against an in-test fake daemon. Upstream PR egoist/waku#218 fixes the same bug differently (dials under the target lock, no backoff or fallback): keep ours | ~180 + tests |
| `crates/waku-core/src/server.rs` | the accept loop survives transient errors (`accept_error_is_transient`, `descriptor_exhausted`) instead of exiting the daemon | ~40 |
| `crates/waku-daemon/src/main.rs` | Windows `process_is_alive` treats only `ERROR_INVALID_PARAMETER` as a dead parent (`parent_is_gone`) | ~15 |
| `src/driver/mod.rs` | `RemoteDriverControl::notify` reports failures through `transport_failure_notice` (a dropped socket raises no `DriverEvent::Error`) | ~15 |
| `src/app.rs` (daemon banner) | `daemon_connection` / `daemon_banner_stage` / `daemon_banner_dismissed` fields + initializers, `mod daemon_banner`, `ToastState::refresh_if_same`, transport-aware `show_toast_with_tone` (same text only restarts the countdown; transport text is localized or left to the banner), `maintain_daemon_connection` on the maintenance clock | ~45 |
| `src/app/background_work.rs` | `should_refresh_background_work` gate: no background-work poll while the socket is down | ~20 |
| `src/app/runtime.rs` (daemon banner) | `save` swallows transport errors and keeps the dirty flag; the periodic save is gated on the connection; `drain_task_state_sync_events` calls `maintain_daemon_connection` | ~12 |
| `src/app/drafts.rs` | the draft-save toast is skipped for transport errors | 5 |
| `src/app/render.rs` (daemon banner) | `render_daemon_connection_banner` under the update banner in both branches | 2 |
| `src/app/settings.rs` (daemon banner) | the daemon status pill reads `daemon.phase_connecting` while the socket is down | 3 |
| `src/app/tests.rs` (daemon banner) | toast de-dup, refresh gate, connection phase and banner stage tests | ~115 |
| `locales/{app,ja,zh-CN}.yml` (daemon banner) | `daemon.restart`, `daemon.banner_reconnecting_detail`, `daemon.banner_unreachable_detail` | 3 keys |
| `src/app/settings.rs` (model providers) | `SettingsPage::ModelProviders` nav entry, `SETTINGS_PAGES` length (12 → 13), title and dispatch arms, and the mail-style split branch it shares with Skills | 5 |
| `src/input.rs` | `TextInput::masked` + the `.masked(bool)` builder, `set_masked`/`is_masked`, and the `masked_display` free function the element layout calls instead of using `content` directly — one ASCII `*` per byte so every byte offset (selection, IME marking, hit-testing) still lands in the same place; non-ASCII is left visible on purpose. Used for API keys on the Providers and Agent pages | ~47 + tests |
| `crates/waku-core/src/command_env.rs` (test) | `windows_environment_probe_captures_the_inherited_path_without_a_profile` waits 60 s instead of 10 s for the PowerShell probe (the ten-second ceiling flaked on the `windows-latest` runner under the parallel suite) | 1 |
| `src/js_repl.rs` (test) | `repl_supports_top_level_await_and_lazy_native_sky` expects `linux` off macOS and Windows | 6 |
| `.github/workflows/{test,release,sync-release}.yml` | no Linux: the test matrix drops Ubuntu and the generated-protocol checks move to the macOS runner; the two Linux release jobs, the `*.tar.gz` upload and `latest-linux.txt` are gone. The version, draft-release and R2-sync jobs still run on `ubuntu-latest` — they build nothing for Linux | ~140 removed |

Rebranding later: change `brand.rs`/`SUB2API_BRAND_NAME` **and** sweep
`CheapRouter` in `locales/` and the two i18n test expectations.

### Files that are ours entirely

`crates/sub2api/**`, `crates/workflow-engine/**`, `src/app/cloud_account.rs`, `src/app/cli_setup.rs`,
`src/app/providers_page.rs`, `src/app/confirm_dialog.rs`,
`src/app/onboarding.rs`, `src/app/message_resend.rs`, `src/app/task_rows.rs`,
`src/app/runtime_prewarm.rs`, `src/app/update_banner.rs`, `src/app/surface_bar.rs`,
`src/app/daemon_banner.rs`, `src/app/agent_page.rs`, `src/app/native_agent.rs`,
`src/app/model_providers_page.rs`,
`crates/waku-core/src/driver/turn_diagnosis.rs`,
`src/app/cloud_usage.rs`, `src/app/model_plaza.rs`, `src/app/cloud_pay.rs`,
`src/app/announcements.rs`, `src/app/workflow.rs`, `assets/icons/{bell,circle-x,store,wallet}.svg`,
`NOTICE.md`, `docs/FORK.md`.

### Conflict triage

- A conflict in one of the listed files: reapply the line, tick the row.
- A conflict anywhere else: our change leaked. Move it back into
  `crates/sub2api` or one of our own view files.
- `SETTINGS_PAGES` has a hard-coded length; upstream adding a page turns that
  into a type error rather than a silent break, which is the desired failure.
- `providers_page::card_button` is `pub(super)` because `cloud_account.rs`
  draws the same buttons; both files are ours, so this is not a hook point.

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

What feeds `desired_routes` on each side:

- **Custom endpoints** (`sub2api::custom_api` +`sub2api::providers`,
  `~/.cheaprouter/custom-api.json`) are a registry of described endpoints —
  address, key, wire format, models, alternate domains — that each CLI slot
  points at by `provider_ref`. `CustomApiConfig::resolved_endpoint` is the one
  place that resolution happens, and `desired_routes` goes through it; a ref
  that no longer resolves routes nothing rather than falling back to the copy
  the slot still carries. Two older on-disk shapes still load and are migrated
  on the first read: a single endpoint object per CLI (`deserialize_slot`
  reads it as one profile named "Custom") and per-CLI profiles with no
  registry (`adopt_into_registry` describes each configured slot once, merging
  those that agree on address, key and format). Both migrations leave the
  slot's own fields in place so an older build still finds an address —
  keep all of that.
- **The cloud gateway's own domain** (`sub2api::gateway_origin`,
  `~/.cheaprouter/gateway-origin.json`) chooses which of the service's
  origins goes into the CLI configs, via `gateway_config_with_origin`.
  `Credentials.endpoint` is deliberately *not* rewritten: sign-in, refresh
  and `/auth/me` stay on the origin the browser flow used.
- **Which group is bound** can move on its own when a platform has automatic
  failover on (`sub2api::failover`, `~/.cheaprouter/failover.json`), through
  the same `select_cloud_group` path a manual pick uses.

`sub2api::speedtest` measures a set of candidate origins for all three (one
warm-up request, one timed request, bounded concurrency, results in input
order). `sub2api::env_fix` removes the environment variables that would
otherwise outrank everything above, after saving them under
`~/.cheaprouter/env-backups/`; machine-wide Windows variables are never
touched, only reported with the elevated command.

## License

Fork stays GPL-3.0-only. Record every modification in [NOTICE.md](../NOTICE.md)
with its date (GPL §5(a)) and keep the "Built on Waku" attribution in the README.
