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
| `Cargo.toml` (root) | `crates/sub2api` and `crates/agent-teams` in `members`/`default-members`; `sub2api` and `agent-teams` dependencies | 6 |
| `crates/waku-core/Cargo.toml` | `sub2api` dependency | 1 |
| `crates/waku-core/src/command_env.rs` | added `command_for_provider()` beside `command()` (calls `sub2api::cli_install::apply_provider_launch_env`: managed Node runtime on `PATH`, Claude's nonessential-traffic switch off; routing itself is written into each CLI's own config by the desktop — `sub2api::global_config`); `sub2api::cli_detect::detection_dirs()` (managed runtime, version-manager dirs, remembered npm prefixes) appended to `executable_search_paths()` so a just-installed CLI is detected without a restart | +18 |
| `crates/waku-core/src/checkpoint.rs` | `snapshot_tree` seeds the temporary index from the repository's own index (stat cache) before `add -A`, falling back to the HEAD rebuild; `repository_index_path` helper | ~45 |
| `crates/waku-core/src/usage.rs` | `--suppress-connect-headers` in the usage probe's curl arguments — behind an HTTP proxy `-D -` printed the proxy's CONNECT reply ahead of the real headers and `split_status_and_body` read that block as the response (same fix as `sub2api::http`) | 1 |
| `src/app/runtime.rs` | `prepare_submission` runs the turn-start snapshot and the provider start on two threads instead of one after the other; warm-start hooks for `runtime_prewarm` — `start_driver` takes a handed-over process (parked, or waited for while it boots), the submission path attaches that claim to its request, the idle sweep calls `reap_unused_prewarms`; `start_driver`, `driver_start_request_for_session`, `install_prepared_driver` are `pub(super)` | ~35 |
| `crates/waku-core/src/model_catalog.rs`, `crates/waku-protocol/src/model_catalog.rs` | `claude-fable-5-1` first in the curated Claude list (with `claude-opus-5-5`), `claude-sonnet-5-5` ahead of `claude-sonnet-5` (the default), and `gpt-6.1-sol`, `gpt-6-astra` then `gpt-6-sol` first in the curated Codex list, which no longer carries `gpt-5.6-luna` (the Codex default stays `gpt-5.6-sol`); `grok-4.7` beside `grok-4.6` in `grok_model_reasoning_efforts`; waku-core additionally merges curated entries the CLI did not return (`merge_claude_catalog` / `with_curated_fallback`, Claude only) at the end of `discover_catalog`; the merge lives in a fork `discover_claude_catalog`, which `discover_catalog` calls for Claude, so upstream's `discover_claude_models` and its tests stay untouched | 1 + ~60 |
| `crates/waku-core/src/model.rs` | `apply_cached_models` runs the cached Claude catalog through `with_curated_fallback` | 1 |
| `src/lib.rs` | `init_confirm_dialog_keys(cx)` beside the other dialog key inits; `cx.bind_keys(surface_key_bindings())` right after the upstream bindings; the View menu's `items` chained with `surface_menu_items()` | 5 |
| `src/app/components.rs` | `resend_action` threaded through `MessageRender`, `render_message_footer`, `message_menu_items`; one footer child and one menu item from `message_resend` | ~10 |
| `src/app/transcript_view.rs` | `resend_action_for_message` computed beside `user_message_action`, passed into `MessageRender` (+ a `None` at the assistant footer call); `scroll_transcript_to_bottom` is `pub(super)` | 4 |
| `src/app/task_switcher.rs` | failed-task glyph is `circle-x` | 1 |
| `crates/waku-core/src/driver/claude.rs` | spawn uses `command_for_provider(.., "claude")` | 1 |
| `crates/waku-core/src/driver/codex.rs` | same, at both spawn sites (session + title turn) | 2 |
| `src/app.rs` | fork `mod` lines, `SettingsPage::{CloudAccount, ModelPlaza, CloudUsage}`, fork struct fields + initializers (cloud account, cli setup, custom API inputs, plaza, pay modal, confirm dialog, onboarding, runtime prewarms), `DriverStartRequest.prewarmed`, `init_confirm_dialog_keys` re-export, startup refresh loop (also kicks off CLI detection and loads onboarding state), `subscribe_custom_api_inputs` beside the other input subscriptions, `maybe_prewarm_selected_runtime` in the composer's `Edited` arm; `update_ui` field + `on_updater_event` call in `handle_updater_event`; `mod surface_bar` and its `surface_key_bindings` / `surface_menu_items` re-export; AgentTeams: `mod agent_teams_settings`, `mod team_panel`, `team_panel: TeamPanelState` field + initializer, `RightPanelSurface::Team` | ~100 |
| `crates/waku-core/src/driver/mod.rs` | `mod turn_diagnosis;` | 2 |
| `crates/waku-core/src/driver/acp.rs` | the captured stderr is a `turn_diagnosis::ProviderStderr` ring instead of a bare `Vec` (its 128-line cap moves into that type), threaded into `run_sdk_connection`, `send_prompt` and the `_x.ai/session/prompt_complete` handler; `AcpStreamState` records this turn's `stderr_mark` and `wire_offset`; both prompt-settle paths call `turn_diagnosis::empty_turn_failure` (generalizing the Kimi-only lookup) and pass `produced_content` to `finish_prompt`, whose `EndTurn` arm now names an empty turn instead of leaving upstream's "Turn completed" fallback | ~117 |
| `src/app/render.rs` | pay-modal, announcements-modal and confirm-dialog composites in both render branches; onboarding strip above the composer; update banner above the header (main) and above the settings page (settings branch is now a flex column); `open_surface_action` registered beside `toggle_right_panel_action` | ~23 |
| `src/app/tests.rs` | `settings_search_filters_pages_for_arrow_cycling` expects the fork's nav pages | 4 |
| `crates/waku-agent/core/src/lib.rs` | vendored engine, recorded departure: `#[serde(default)]` on `Config` so a partial `config` block in settings.json loads | 1 + comment |
| `crates/waku-agent/query/src/lib.rs` | vendored engine, recorded departure: an explicit `config.provider` outranks the model-name family table; a stream `error` event ends the turn | ~14 |
| `crates/waku-agent/api/src/lib.rs` | vendored engine, recorded departure: `StreamAccumulator` keeps the first stream `error` instead of discarding it | ~18 |
| `crates/waku-agent/core/src/system_prompt.rs` | vendored engine, recorded departure: the agent is named after the product, not after the engine or Anthropic | ~25 |
| `crates/waku-agent/query/src/runner/provider_options.rs` | vendored engine, recorded departure: Grok counts as a reasoning model, so its effort tier reaches the request; gpt-5 Codex's summary/include fields stay off it | ~14 |
| `crates/waku-agent/tools/src/{pty_bash,powershell,web_fetch}.rs` | vendored engine, recorded departure: truncate on character boundaries (`floor_char_boundary` / `ceil_char_boundary` in `pty_bash`) — the byte slices panicked on long non-ASCII output | ~30 |
| `crates/waku-agent/core/src/lib.rs` | vendored engine, recorded departure: the plan-mode arm allows the plan-safe tools and read-only invocations; the two plan switches stay read-level so the model never asks permission to restrict itself; in plan mode an allow rule no longer opens the shell or an editing tool (`plan_mode_overrides_allow_rule`) | ~45 |
| `crates/waku-agent/core/src/bash_classifier.rs` | vendored engine, recorded departure: `is_read_only_bash_command` — stricter than the `Safe` tier and denies on doubt; parses quoting and redirections (a descriptor or `/dev/null` is fine, a file is not) and judges each simple command by its own rules (`cd`, `sed -n`, `awk`, `xargs`, git's listing forms, `gh` reads) | ~700 |
| `crates/waku-agent/api/src/providers/codex.rs` | vendored engine, recorded departure: `decode_tool_arguments` accepts the object form a normalizing gateway returns, not only the specified JSON string | ~35 |
| `crates/waku-agent/api/src/{prompt_cache,claude_effort}.rs` (new), `crates/waku-agent/api/src/providers/anthropic.rs`, `crates/waku-agent/api/src/lib.rs`, `crates/waku-agent/api/src/codex_adapter.rs` (test), `crates/waku-agent/query/src/lib.rs` | vendored engine, recorded departure: Claude Code's prompt-cache breakpoints (last tool, system prompt, last message, the user message before it) on every Messages request — in `build_request` and on the request the Anthropic route sends from the query loop; current Claude families send adaptive thinking plus `output_config.effort` (new optional `CreateMessageRequest` field) instead of a thinking budget; `claude-sonnet-5-5` has its own row and dotted minor versions (`claude-sonnet-5.5`) read as dashed ones | ~40 + new files |
| `crates/waku-agent/api/src/endpoint.rs` (new), `crates/waku-agent/api/src/{lib,registry}.rs`, `crates/waku-agent/api/src/providers/openai.rs` | vendored engine, recorded departure: request URLs go through `versioned_url`, which appends `/v1` only when the base does not already end in a version segment — `…/api/paas/v4` and `…/api/v3` no longer become `…/v4/v1/…`; mirrored by `crates/sub2api/src/gateway.rs::versioned_url` for the probes | ~10 + new file |
| `crates/waku-agent-bridge/src/{config,session,events,oneshot}.rs` | fork-owned: a session on `custom:<provider id>::<model>` is routed by `options.endpoints` alone — declared format, the endpoint's address and key on all three engine entries, gateway key table and endpoint table removed from the session's config (`select_route`, `UnknownEndpoint`, `route_fingerprint`); a 404 is reported with the model, route and URL (`AgentEvent::RouteNotFound`) | ~250 |
| `crates/sub2api/src/{providers,global_config/mod,global_config/native,model_test}.rs` | fork-owned: `agent_model_id` / `parse_agent_model_id`, `ProviderEntry::offers_models_to_agent`, the `endpoints` table the routing writer files for every such provider, and the one-token per-model test | ~400 |
| `src/app/{native_agent,composer,sessions,runtime,cloud_subscriptions,model_providers_page,agent_page}.rs` | fork-owned: every endpoint of the user's own is a vendor-column entry of its own in the built-in agent's picker, its declared format never overruled by a name rule; a bound line hides the gateway rows it shadows; "Manage models…" at the end of those sections; per-model Test, request URL and picker status on Model providers; picker sources on Settings → Agent; `migrate_bare_endpoint_ids` at launch | ~600 |
| `crates/waku-agent/api/src/{lib,provider_types}.rs` | vendored engine, recorded departure: the two stream accumulators warn instead of silently turning unparseable tool arguments into `{}` (the agent loop already errors — issue #215) | ~20 |
| `crates/waku-agent/query/src/runner/tools.rs` | vendored engine, recorded departure: `whole_floats_to_integers` at the one `.execute()` call site — repairs `120.0` for `usize` fields, which several non-Claude models emit | ~45 |
| `crates/waku-agent/tools/src/exit_plan_mode.rs` | vendored engine, recorded departure: `self_gates` and asks through `check_permission` with the plan summary as the description — its declared level is `None`, which the central backstop never gates, so the bridge's "finished planning" dialog was unreachable | ~20 |
| `crates/waku-agent/tools/src/lib.rs` | vendored engine, recorded departure: refusals name their reason, and the two the fork words itself are exported as markers for the driver to localize; the plan-mode refusal tells the model not to retry and to make the step part of the plan | ~40 |
| `crates/waku-agent-bridge/src/computer_use.rs` | fork-owned: writes the bundled skill where the engine's `Skill` tool reads it, and removes it when the toggle is off | ~110 |
| `crates/waku-agent-bridge/src/{config,permission,session}.rs` | fork-owned: the REPL MCP registration, the consented-tools short-circuit, the plan/computer-use prompt rules | ~200 |
| `crates/waku-agent-bridge/src/{mcp_tool,events}.rs` | fork-owned: MCP image content onto its own sideband so the model reads text and the transcript gets pixels | ~90 |
| `crates/waku-agent-bridge/src/images.rs` (new), `crates/waku-agent-bridge/src/{lib,session}.rs`, `crates/waku-agent-bridge/Cargo.toml` | fork-owned: the pictures a prompt mentions (`@path`, one per composer attachment) sent as image blocks before its text, for every model | ~200 |
| `crates/waku-agent/query/src/lib.rs` | vendored engine, recorded departure: image blocks are no longer swapped for "[Image not supported by this model]" on non-Anthropic routes — every model gets them, and one without vision answers with its API's error | ~10 |
| `src/js_repl_image.rs`, `src/js_repl.rs` | fork-owned: `generate_image` as a third REPL tool, credentials read from the engine settings rather than the environment; the request is `sub2api::images` (a gateway task where there are tasks, else streamed) and an image model goes out with its own routed key (`gateway_keys.models`) before the OpenAI one | ~380 |
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
| `src/app/sessions.rs`, `src/app/composer.rs` | `refresh_native_catalog` when the built-in agent's rail is opened or selected; `picker_rail_shows_provider` treats built-in providers as installed; the built-in agent's vendor column in the picker (`native_vendor_column`, vendor marks on its rows, `picker_stops` for `tab`) | ~10 + fork fns |
| `src/app.rs` | provider probes seeded `installed: provider.is_builtin()` | 1 |
| `src/assets.rs` | `bell`/`circle-x`/`store`/`wallet` icon entries; embedded `images/logo.png` brand mark | ~12 |
| `src/app/runtime.rs` | `cloud_balance_stale` set at the turn-settlement seam, drained in the event pump; AgentTeams: `mark_team_stale` at the same seam and `drain_team_panel` beside that drain; the two `pending_permissions` clears (rewind, submission install) keep a team member's dialogs; `submit_submission_for_session` widened to `pub(super)` (used by `cloud_subscriptions.rs`) | 11 |
| `src/app/sidebar.rs` | both empty states' icon (no project / project open) swapped for the brand mark; announcements bell in the window header; onboarding checklist + footer chip rows in the empty state; task rows carry a hover group, the failure badge and the remove button from `task_rows`; `localized_session_title` is `pub(super)`; the surface bar (`render_surface_bar`) in the window header where the panel toggle used to be (the toggle's `.child` line removed, fps counter kept) | 13 |
| `src/app/composer.rs` | balance chip in the status strip | 3 |
| `resources/AppIcon*.icns`, `resources/windows/AppIcon.ico`, `resources/linux/` | brand artwork and desktop entry name | assets |
| `scripts/bundle-linux.sh` | installs the brand icon | 5 |
| `src/app/settings.rs` | nav entries, title arms, dispatch arms, `SETTINGS_PAGES` length (7 upstream → 13); the Providers arm dispatches to the fork's `render_providers_page` (upstream's `render_providers_settings` kept under `#[allow(dead_code)]`); `render_provider_expanded_settings`, `toggle_provider_expanded`, `set_provider_enabled`, `detection_checked_label`, `abbreviate_home_path` widened to `pub(super)`; General page appends `render_update_check_card` after the automatic-updates toggle, outside the `updater_available` guard so the row shows in every build; `open_surface_action` registered beside `toggle_right_panel_action` | ~32 |
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
| `locales/{app,ja,zh-CN}.yml` | our new `cloud.*`/`cli_setup.*`/`surface_bar.*`/`team.*` keys, plus a de-brand sweep: every user-visible "Waku" replaced (neutral wording, or `CheapRouter` where a name is load-bearing — consent prompts, hero copy, composer placeholder) | ~300 lines |
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
| `crates/waku-core/src/driver/{claude,codex,opencode,acp,pi}.rs` (Computer Use optional) | the Computer Use setup goes through `support::optional_computer_use` instead of `?`: a helper that is not installed leaves the session without desktop control instead of failing to start it; Pi folds its extension path into the same setup | 1 each, Pi ~10 |
| `crates/waku-core/src/driver/support.rs` | `optional_computer_use` (+ test) | ~30 |
| `src/app.rs` (subscriptions) | `mod cloud_subscriptions` | 1 |
| `src/app.rs` (cloud account split) | `mod cloud_failover` / `cloud_groups` / `cloud_menu` / `cloud_origins` | 4 |
| `src/app.rs`, `src/app/render.rs` (sign-in window) | `mod cloud_sign_in`; the `cloud_sign_in_input` field, initializer and its `Submit` / `Edited` subscription beside `skills_search`'s; `render_cloud_sign_in_modal` beside the announcements modal in both render branches | ~25 |
| `src/app/usage_meter.rs` | `meter_bar` is `pub(super)`, reused by the subscription cards | 1 |
| `crates/waku-core/src/git_commit.rs`, `crates/waku-core/src/driver/codex.rs` (Codex pins) | Codex commit messages and titles pinned to `gpt-5.6-terra` instead of `gpt-5.6-luna`, commit effort `low` instead of `none` (+ the title test's name and assertion) | 5 |
| `src/js_repl.rs` (test) | `repl_supports_top_level_await_and_lazy_native_sky` expects `linux` off macOS and Windows | 6 |
| `src/app/transcript.rs`, `src/app/transcript_view.rs` (agent flow) | `TurnFold(Uuid, usize)` per steer segment with `turn_fold_overrides`; `PendingSteer` rows; the working row hidden while waiting, a divider while compacting; `render_activities_row` rewritten over `activity_phase::group_activities` (flat rows, phase groups, reasoning collapsed with a ticker; `activity_groups_expanded` replaces `activities_expanded`); the changed-files card collapsed with clickable files and undo; `markdown_ctx` is `pub(super)` | large — resolve toward ours |
| `src/app/components.rs` (agent flow) | `activity_summary` / `activity_header_title` / `activity_group_is_live` / `activity_action_label` replaced by `activity_verb`, `activity_row_target`, `activity_group_title`, `activity_failure_tail`; one-line system messages drawn as dividers | ~200 |
| `src/app/streaming.rs`, `src/app/sessions.rs`, `src/app/runtime.rs` (agent flow) | turn pauses around permissions, questions and compactions; `complete_turn_blocks(stopped)`; failures recorded on the turn instead of as assistant messages, a connecting turn finished on error; `pending_permissions` queue; `RewindOrigin` and `restore_workspace` in the rewind path, `retry_failed_turn`; waiting notifications | ~250 |
| `src/app.rs`, `src/app/{sidebar,render,sessions,command_palette,composer}.rs`, `src/assets.rs` (image studio) | `mod image_studio` / `image_studio_view` and the `image_studio` field; an "Images" row under Search in the sidebar (the Search row's height is three action rows: Images, Model status), session rows not marked selected and the header titled by the open page (`main_page_open` / `main_page_title`, in `model_status.rs`); the main column renders the studio in place of transcript + composer; `request_session_activation` and `new_session_action` close it (`close_main_pages`), the model-picker shortcut ignores it; `PaletteAction::OpenImageStudio`; `stage_attachment_paths` is `pub(super)` for "Send to task"; the `image` icon | ~60 |
| `src/app.rs`, `src/app/{sidebar,render,sessions,command_palette}.rs` (model status) | `mod model_status` / `model_status_view` and the `model_status` field; a "Model status" row under "Images" (`render_sidebar_model_status`; `render_sidebar_action_row` is `pub(super)`); the main column renders the page when `model_status.open`, and the transcript branch waits on `!main_page_open()`; the details dialog mounted beside the announcements modal; `close_main_pages()` where the studio used to be closed alone, `main_page_open()` for the model-picker shortcut; `PaletteAction::OpenModelStatus` | ~30 |
| `src/app/render.rs`, `src/app/composer.rs`, `src/app/sidebar.rs`, `src/app/right_panel.rs`, `src/app/background_work.rs` (agent flow) | error banner and status capsule mounted; the permission branch of `render_permission` delegates to `permission_card`; waiting count in the task row; `open_turn_diff` takes a path; `live_background_work` | ~60 |
| `src/ui/motion.rs`, `src/ui/mod.rs` | `shimmer` text; `activity_noun` removed with the block header | ~110 |
| `crates/waku-protocol/src/{model,workspace,lib}.rs` (agent flow) | additive: `AgentTurn::{pauses, error, undone_at}`, `ActivityItem::stopped` (all `serde(default)`), `PlanTurnUndo` / `ApplyTurnUndo` and their results, `TurnUndoPlan` / `UndoFile` / `UndoReason`, `TURN_UNDO_STALE`; TS bindings regenerated | ~150 |
| `crates/waku-core/src/{checkpoint,workspace}.rs` (agent flow) | `plan_turn_undo` / `apply_turn_undo` and the undo backup ref, cleared with the turn's other refs | ~300 + tests |
| `.github/workflows/{test,release,sync-release}.yml` | no Linux: the test matrix drops Ubuntu and the generated-protocol checks move to the macOS runner; the two Linux release jobs, the `*.tar.gz` upload and `latest-linux.txt` are gone. The version, draft-release and R2-sync jobs still run on `ubuntu-latest` — they build nothing for Linux | ~140 removed |
| `crates/waku-protocol/src/model.rs` (sub-agents) | additive: `ActivityItem::subagent` (`serde(default)`) with `SubagentCall` and `with_subagent`; `BackgroundWorkEvent::Transcript` with `SubagentTranscriptEntry` / `SubagentTranscriptBody`; TS bindings regenerated (`SubagentCall.ts`) | ~90 |
| `crates/waku-agent-bridge/src/{session,events,background,lib}.rs` (sub-agents) | `builtin_tools` offers the bridge's `SubagentTool` instead of `claurst_query::AgentTool` (`engine_tools` split out for the child's set); `Inner.subagents` with `begin_turn`/`end_turn` around the loop, `announce`/`forget` in `forward_events`, `stop` ahead of `background::stop`, `cancel_all` on drop; `AgentEvent::Subagent` + `SubagentEvent`/`SubagentStatus`; `background::snapshot_owned` | ~120 |
| `crates/waku-agent-bridge/src/config.rs`, `session.rs` (context window) | `CONTEXT_WINDOWS_OPTION`, `declared_windows`, `context_window_for`, `session_model_registry` / `window_overrides`; `build_query_config` uses the overlaid registry; the meter's window at turn start and after `/compact` comes from `context_window_for` | ~140 |
| `crates/waku-core/src/driver/{mod,native}.rs` (sub-agents) | `mod subagent;`; native rows carry `SubagentCall`, `Agent` titled by its description, `AgentEvent::Subagent` → one `BackgroundWorkItem` + `Transcript` entries (`handle_subagent`), registry sub-agents linked to their row | ~150 |
| `crates/waku-core/src/driver/claude.rs` (sub-agents) | `#[path] mod claude_subagent`; state fields `task_keys` / `subagent_feeds` / `subagent_calls` (replacing `streamed_task_output`); `forward_subagent_transcript` replaced by `claude_subagent::{forward_assistant, forward_tool_results}`; `link_task` + `rekey` in `handle_claude_system`; `note_parent_call`, `with_subagent` on the task row, `settle_parent` on its result | ~40 |
| `crates/sub2api/src/{client,auth,lib,gateway}.rs`, `global_config/{mod,native}.rs`, `src/app/{cloud_subscriptions,model_providers_page}.rs` (context window) | `ModelCatalogItem::context_window`; `Credentials::model_windows` filled by `refresh_model_routes`; `GatewayConfig::model_windows`; `NativeRoutes::context_windows` (+ custom declarations); the writer files `options.context_windows`; a routing refresh compares windows too, and a Model Providers save re-applies live built-in sessions | ~120 |
| `src/app/{background_work,transcript_view,streaming,runtime,right_panel}.rs`, `src/app.rs` (sub-agents) | the registry keeps each sub-agent's record (`transcripts`, `transcript_entry`, refreshed on the output tick) and the surface branches to `render_subagent_surface` (takes `window` now); `render_activity_item` is `pub(super)` and hands sub-agent calls to `render_subagent_row`; `toggle_activity_item` also toggles rows outside the transcript; `update_activity` merges `subagent`; record text batched like log output in the event pump; three `mod` lines | ~50 |
| `locales/{app,zh-CN,ja}.yml` | `subagent.*` keys | 21 |
| `locales/{app,zh-CN,ja}.yml` (model status, top-up promotions) | `model_status.*`, `sidebar.model_status`, `command_palette.open_model_status`; `pay.promo_*`, `pay.max_daily`, `pay.help_title`, `pay.amount_meta_bonus`, `pay.success_toast_bonus` | ~110 keys |
| `crates/waku-agent/query/src/{lib.rs,runner/tools.rs}` | vendored engine, recorded departure: the provider branch (Responses, Chat Completions) runs each stretch of calls that may overlap — sub-agents and read-only tools (`runs_concurrently`, split by `concurrency_runs`) — through `run_tool_batch` instead of one `for` loop awaiting every call; the rest still run alone and in order, results keep the calls' order, a cancel returns `Cancelled` with every call answered | ~80 |
| `crates/waku-client/src/persistence.rs`, `crates/waku-core/src/persistence.rs` | new state starts on the built-in agent: `default_provider()`, `PersistedState::empty()`'s `last_provider` and `fresh()`'s first session are `ProviderKind::Native` instead of `Codex` | 3 each |
| `src/app.rs` (default provider) | `native_agent::adopt_built_in_default` beside `migrate_legacy_pay_as_you_go` at launch, and its flag in the startup save condition — moves a state an earlier build wrote on the untouched Codex default (no model picked, no CLI task started, built-in agent not switched off) onto the built-in agent | 3 |
| `src/app.rs`, `src/lib.rs`, `src/app/render.rs` (CLI takeover) | `mod cli_takeover` and its `init_cli_takeover_keys` re-export; the key init beside the confirm dialog's; `render_cli_takeover_prompt` beside the sign-in window in both render branches | 7 |
| `locales/{app,zh-CN,ja}.yml` (CLI takeover) | `cloud.cli_takeover.*`, `providers.route_own_login`; CLI words added to `settings.cloud_account_keywords` | 22 keys |
| `src/app.rs`, `src/app/{usage_meter,composer,sessions}.rs` (context ring windows) | `mod model_windows`, the `model_windows` field + initializer, `load_model_windows` in the startup task; the ring and `/context` read `ring_context_usage(session)` instead of `session.context_usage`; `note_model_changed` in `choose_model` and `set_context_window` (drops the previous model's reported window, looks the new one up). Windows come from OpenRouter's public `/api/v1/models` via `sub2api::model_windows`, cached in `~/.cheaprouter/model-windows.json` for a day; a window the agent reports still wins | ~15 |
| `src/app/composer.rs` (model descriptions) | a description line under each model picker row's name from `sub2api::model_copy` (row 72px tall when it has one), and the panel 440px tall instead of 390 | ~14 |
| `src/app.rs`, `src/app/{streaming,transcript_view}.rs` (turn status line) | `mod turn_status`, the `turn_output_baselines` field + initializer; `note_turn_output_baseline` before a `TokenUsageUpdated` is applied; the working row appends `live_turn_status_suffix()` ("· 2.3k tokens · Thinking…") after "Working for N" | ~10 |
| `src/app.rs`, `src/app/{render,sidebar}.rs` (activity overview) | `mod home_overview`, the `home_overview` field + initializer; `maybe_refresh_home_overview` in the window render while the welcome screen shows; `.children(self.render_home_overview(cx))` under the welcome headline | ~10 |
| `crates/waku-protocol/src/{lib,protocol}.rs`, `crates/waku-core/src/{lib,daemon}.rs` (activity overview) | `pub mod activity_overview` in both crates; additive `Command::LoadActivityOverview` and `ResponsePayload::ActivityOverview { records }`; the daemon's arm (reads the task store) and its entry in the wrong-path list | ~16 |
| `locales/{app,zh-CN,ja}.yml` (dsh-claude-style ports) | `model_copy.*`, `turn_status.*`, `home_overview.*` | ~70 keys |
| `AGENTS.md` (DeepSeek Harness reference) | a `## DeepSeek Harness reference` section after "Product reference": when to consult `docs/deepseek-harness.md`, port-never-load, not the DeepSeek provider, porting rules | ~28 |
| `src/app.rs` (content translations) | `mod content_translations`, the `content_translations` field + initializer, `reset_content_translations` in the startup task; `WakuPane::render` calls `flush_content_translations` after the pane's content (a pane renders after the root, so text it queues would otherwise wait for the next root frame); `announcement_markdown` is keyed by the rendered text as well as the id, so a translation landing re-parses | ~14 |
| `src/app/render.rs` (content translations) | `flush_content_translations` before `render_window_frame` in both branches | 4 |
| `src/app/settings.rs` (content translations) | `reset_content_translations` in `set_language`, ahead of `sync_native_models` (the picker's route notes name groups) | 3 |
| `src/app/{cloud_groups,cloud_menu,cloud_failover,cloud_usage,cloud_subscriptions,model_plaza,image_studio_view,model_status_view,announcements,plans_page,cloud_pay}.rs` (content translations, fork-owned) | display sites wrapped in `self.tx(..)`: group names and descriptions, announcement titles and bodies, plan names / descriptions / features, promotion names, the pay help text (translated whole, then split into lines). Free helpers take the shown string from the caller (`subscription_menu_line`, `subscription_card`, `usage_log_card`, `plan_summary`). **Display only** — `cloud_lane`, `model_routing::{group_lane, plan_lane, named_domestic}`, failover and every stored struct keep the originals; never write a `tx` result back | ~45 |
| `src/app.rs` (Effort slider card) | `mod effort_{fire,panel,relaunch,scale}` under a fork comment; the `effort_card` and `effort_launches` fields + initializers | ~14 |
| `src/app/composer.rs` (Effort slider card) | the control row calls `render_effort_card_control` instead of `render_model_traits_control`, which stays with `#[allow(dead_code)]` so upstream edits to it keep merging | 5 |
| `src/app/runtime.rs` (Claude effort relaunch) | `self.relaunch_for_effort(session_id)` in `submit_submission_for_session`, after the native route hold — Claude Code takes its effort only as a launch flag, so an idle runtime with a stale effort is relaunched with `--resume` there | 2 |
| `crates/waku-core/src/driver/acp.rs` (effort-only change) | `#[path = "acp_effort.rs"] mod acp_effort;` and the `Options` arm dispatching on `acp_effort::reapply` — Kimi gets an effort-only change as its `thinking` option, Grok and Cursor re-select the model as before | ~25 |
| `crates/waku-core/src/driver/claude.rs` (tests only) | `an_effort_change_never_asks_for_a_restart`: an effort change must never make `apply_options` refuse, which would cancel a running turn — the app-side relaunch relies on it | 12 |
| `crates/waku-agent-bridge/src/config.rs` | fork-owned: `effort_level` reads `off` / `disabled` as the engine's `None` rung; unparsed it meant "unset", which the DeepSeek and GLM routes treat as thinking on at high | ~15 |
| `locales/{app,zh-CN,ja}.yml` (Effort slider card) | `effort_panel.*` | 3 keys |
| `scripts/bundle-windows.ts`, `scripts/bundle-linux.sh`, `scripts/bundle.sh` | `NOTICE.md` copied beside `LICENSE` (macOS: into `Resources/`), carrying the BSD-3 notice binary redistributions must include | 3 each |
| `crates/waku-protocol/src/workspace.rs`, `crates/waku-core/src/workspace.rs` (Files panel edits) | additive `WorkspaceOperation::{CreateFile, CreateDirectory, RenamePath, TrashPath}`; `#[path] mod workspace_edit` and the four `execute` arms calling it | ~45 |
| `src/app/right_panel.rs` (Files panel menu, agent browser) | `#[path]` mods `file_tree_menu` and `browser_agent`; in `render_right_panel_working_tree`: `FileTarget::of` per entry, `decorate_file_tree_row`, `file_tree_create_row` before the first row and after each, `file_tree_header_actions` in the header, the scroll area wrapped by `file_tree_area`; `RightPanelSurface::File` labels split on `\` too (Windows tabs showed the whole path) | ~15 |
| `src/ui/menu.rs` | `ContextMenuHandle::open_context_menu_at`, for a keyboard-opened menu anchored at one row of a larger trigger | 5 |
| `src/app.rs` (Files panel menu, agent browser) | the `file_tree` and `browser_agent` fields + initializers | 6 |
| `locales/{app,zh-CN,ja}.yml` (Files panel menu) | `file_tree.*` | 14 keys |
| `src/js_repl.rs`, `src/js_repl_image.rs` (pictures) | `WAKU_REPL_TOOLS` narrows the offered tools (`tool_offered`); a tool that ran and failed answers an `isError` result instead of a JSON-RPC error; a relative `output_dir` resolves against `WAKU_SESSION_CWD` | ~40 |
| `crates/waku-agent-bridge/src/{config,session}.rs` (pictures, agent browser) | fork-owned: `ComputerUseWiring::image_only` (`WAKU_REPL_TOOLS=generate_image`, only that tool consented, the picture rule asking for the result as a Markdown image); `AgentStartOptions::browser_tools`, `browser_rule`, the browser tools added in `builtin_tools_with`, `browser_result`, released on cancel | ~120 |
| `crates/waku-core/src/driver/native.rs` (pictures, agent browser) | fork-owned: with Computer Use off the REPL still comes image-only; `prompt` names a tool row; `AgentEvent::BrowserRequest` → `DriverEvent::BrowserRequest`; `browser_result`; `in_app_browser_offered` reads `WAKU_IN_APP_BROWSER` | ~45 |
| `crates/waku-protocol/src/model.rs` | command rows read `code` (the built-in agent's REPL / JavaScript calls showed no source) and a `title` / `code` summary; additive `DriverEvent::BrowserRequest` | ~15 |
| `src/md/render.rs` | `local_image_path`: an absolute path or `file:` URL in a Markdown image loads from disk (it was read as a bundled asset) | ~20 |
| `crates/waku-protocol/src/{protocol,driver_wire}.rs`, `crates/waku-core/src/{daemon,server}.rs`, `crates/waku-core/src/driver/mod.rs`, `crates/waku-client/src/driver.rs`, `src/driver/mod.rs` (agent browser) | additive `Command::BrowserResult`; the `browserRequest` event encoded and decoded in both copies; `DriverControl::browser_result` (default no-op) and its `DriverHandle` passthrough on both sides; the command routed to the runtime | ~70 |
| `crates/waku-client/src/process.rs` (agent browser) | a Windows desktop starts its daemon with `WAKU_IN_APP_BROWSER=1`, which is what offers the built-in agent the browser tools | 6 |
| `src/app/streaming.rs` (agent browser) | `DriverEvent::BrowserRequest` deferred to `handle_browser_request` | 12 |
| `src/browser.rs` (agent browser) | `#[path] mod automation`; `Webview::call_devtools` on the WebView2 host (`CallDevToolsProtocolMethod`) | ~40 |
| `packages/waku-client/src/generated/{Command,WorkspaceOperation}.ts` | regenerated (`protocol:generate`) | — |
| `crates/waku-protocol/src/workspace.rs`, `crates/waku-core/src/workspace.rs` (Files panel previews) | additive `WorkspaceOperation::PreviewFile`, `WorkspaceResult::FilePreview`, the `FilePreview` / `PreviewSheet` / `PreviewKind` types and `preview_kind`; `#[path] mod workspace_preview` and its `execute` arm | ~110 |
| `crates/waku-core/Cargo.toml` (Files panel previews) | `calamine` (Excel / OpenDocument spreadsheets), plus `quick-xml` and `zip` at the versions calamine already brings, for Word / PowerPoint / OpenDocument text | 6 |
| `src/app/right_panel.rs`, `src/app.rs` (Files panel previews) | `#[path] mod file_preview`; `render_right_panel_file` returns `render_file_preview_surface` first for a file `preview_kind` recognises; the `file_previews` field + initializer | 10 |
| `locales/{app,zh-CN,ja}.yml` (Files panel previews) | `file_preview.*` | 12 keys |
| `packages/waku-client/src/generated/{WorkspaceOperation,WorkspaceResult,FilePreview,PreviewSheet,index}.ts` | regenerated (`protocol:generate`) | — |
| `crates/waku-agent-bridge/src/session.rs` (AgentTeams) | fork-owned: `#[path] mod team_seam` (`session_team.rs`); `Inner.team`; `Turn.compacting`; `ToolSets::build` / `builtin_tools_with` take the team (captain tools once active); `prompt` hands a Team panel control to `team.intercept`; `run_turn` takes a `PromptOrigin`, runs `augment_prompt` first (the `/agent-teams` directive, parked contexts) and `extend_rules` after the sub-agent query is cloned; both background snapshots add the team's running members; `stop_background_work` tries `stop_member` first; `cancel` releases only the captain's own dialogs; `after_turn` at the end of `run_turn` and `compact_now`; `team.shutdown()` on drop | ~70 |
| `crates/waku-agent-bridge/src/{permission,subagent,lib}.rs`, `Cargo.toml` (AgentTeams) | fork-owned: `MemberScope` + `GuiPermissionHandler::for_member` (own manager, `team:` request ids via `prompt_as`), `release_where` / `release_member`; `child_event` and `join_prompts` widened to `pub(crate)`; `mod team` and `agent_teams_slash_commands()`; the `agent-teams` dependency | ~90 |
| `crates/waku-core/src/driver/native.rs`, `composer_complete.rs`, `Cargo.toml` (AgentTeams) | a member's permission title names the member (`team.permission_title`); a member's record keeps one feed across its turns (`member_feeds`); `/agent-teams` and its profile aliases among the built-in commands; the `agent-teams` dependency | ~45 |
| `src/app/streaming.rs`, `src/app/sessions.rs` (AgentTeams) | a `team:` permission is accepted with no captain turn running and survives `TurnFinished` / Stop; answering one leaves an idle session idle | ~20 |
| `src/app/right_panel.rs`, `src/app/surface_bar.rs`, `src/app/background_work.rs`, `src/app/agent_page.rs`, `src/app/usage_page.rs`, `src/assets.rs` (AgentTeams) | `RightPanelSurface::Team` arms (label, icon, single instance, render, "+" menu for built-in sessions); `active_right_panel_surface` widened to `pub(super)`; the header's Team button; `ReconcileLive` marks the team stale and `has_background_work`; the Teams section on Settings → Agent and its load hook; `users` icon | ~50 |
| `src/app.rs`, `src/app/right_panel.rs`, `src/app/surface_bar.rs`, `src/app/streaming.rs`, `src/app/sessions.rs`, `src/app/transcript_view.rs`, `src/assets.rs`, `Cargo.toml` (Plan surface) | the header's Plan button (`render_plan_surface_button`, shown once the session has a plan, a dot while one waits); `RightPanelSurface::Plan` + `mod plan_review` + `plan_review` field (replacing `plan_card_expanded` / `plan_markdown`); its label, icon, single-instance and render arms and the "+" menu entry once the session has a plan; `plan_requested` after a permission is queued; `note_plan_answer` at the top of `respond_permission`; `plan_selected_text` in the copy chain; `chevron-left` icon; the `similar` dependency | ~20 |
| `crates/waku-core/src/driver/native.rs` (Plan surface) | `PlanDraft`: the turn's latest assistant text becomes the `ExitPlanMode` dialog's body (`localize_exit_plan_detail` takes it) | ~45 |

Rebranding later: change `brand.rs`/`SUB2API_BRAND_NAME` **and** sweep
`CheapRouter` in `locales/` and the two i18n test expectations.

### Files that are ours entirely

`crates/sub2api/**`, `crates/agent-teams/**`, `src/app/cloud_account.rs`,
`src/app/{cloud_failover,cloud_groups,cloud_menu,cloud_origins,cloud_sign_in}.rs`, `src/app/cli_setup.rs`,
`src/app/cli_takeover.rs`, `src/app/model_windows.rs`,
`src/app/providers_page.rs`, `src/app/confirm_dialog.rs`,
`src/app/onboarding.rs`, `src/app/message_resend.rs`, `src/app/task_rows.rs`,
`src/app/runtime_prewarm.rs`, `src/app/update_banner.rs`, `src/app/surface_bar.rs`,
`src/app/daemon_banner.rs`, `src/app/agent_page.rs`, `src/app/native_agent.rs`,
`src/app/model_providers_page.rs`, `src/app/cloud_subscriptions.rs`,
`crates/waku-core/src/driver/turn_diagnosis.rs`,
`crates/waku-core/src/driver/{subagent,claude_subagent}.rs`,
`crates/waku-agent-bridge/src/subagent.rs`,
`src/app/{subagent_row,subagent_panel,subagent_transcript}.rs`,
`src/app/cloud_usage.rs`, `src/app/model_plaza.rs`, `src/app/cloud_pay.rs`,
`src/app/announcements.rs`, `assets/icons/{bell,circle-x,store,users,wallet}.svg`,
`src/app/{team_panel,agent_teams_settings,plan_review}.rs`, `assets/icons/chevron-left.svg`,
`crates/waku-agent-bridge/src/{team/**,session_team.rs}`, `docs/agent-teams.md`,
`src/app/error_banner.rs`, `src/app/turn_undo.rs`, `src/app/status_capsule.rs`,
`src/app/permission_card.rs`, `src/app/shortcuts.rs`,
`src/app/image_studio.rs`, `src/app/image_studio_view.rs`, `assets/icons/image.svg`,
`src/app/model_status.rs`, `src/app/model_status_view.rs`,
`crates/waku-client/src/{activity_phase,turn_segments,status_capsule}.rs`,
`src/app/home_overview.rs`, `src/app/turn_status.rs`,
`src/app/content_translations.rs`,
`src/app/{effort_panel,effort_scale,effort_fire,effort_relaunch}.rs`,
`crates/waku-core/src/driver/acp_effort.rs`,
`src/app/{file_tree_menu,browser_agent,file_preview}.rs`, `src/browser_automation.rs`,
`crates/waku-core/src/{workspace_edit,workspace_preview}.rs`, `crates/waku-agent-bridge/src/browser.rs`,
`crates/waku-protocol/src/activity_overview.rs`, `crates/waku-core/src/activity_overview.rs`,
`docs/deepseek-harness.md`, `docs/deepseek-harness.zh.md`,
`NOTICE.md`, `docs/FORK.md`.

### Conflict triage

- A conflict in one of the listed files: reapply the line, tick the row.
- A conflict anywhere else: our change leaked. Move it back into
  `crates/sub2api` or one of our own view files.
- `SETTINGS_PAGES` has a hard-coded length; upstream adding a page turns that
  into a type error rather than a silent break, which is the desired failure.
- `providers_page::card_button` is `pub(super)` because `cloud_origins.rs`
  draws the same buttons; both files are ours, so this is not a hook point.

## What we deliberately do not touch

- `crates/waku-protocol` — the wire contract. Routing is desktop-local (the
  desktop writes each CLI's own global configuration; the daemon carries no
  routing state), so the protocol stays byte-identical to upstream and the
  browser client keeps working unchanged. (`identity.rs` constants are branded,
  but no message shape changes.) The exceptions are additive and listed in the
  register above: new `serde(default)` fields on stored records and new
  workspace operations, none of which an upstream client or daemon has to
  understand.
- Provider drivers' protocol handling.

User data now lives under `~/.cheaprouter` (platform folders `CheapRouter`
/ `CheapRouter Debug`); `sub2api::migrate` renames the legacy `~/.waku` /
`Waku` directories in place at startup, from both the desktop and daemon
entry points.

## Routing contract (cc-switch model)

`sub2api::global_config` edits the live CLI configs — `~/.claude/settings.json`
(env block, deep-merged), `~/.codex/config.toml` (toml_edit, comments
preserved; the documented shape: `[model_providers.OpenAI]` with the key as
`experimental_bearer_token`, `requires_openai_auth = false`, the image
extension header and two `[features]` switches, the gateway's bare origin as
`base_url`), `~/.grok/config.toml`, `opencode.json` and Pi's `models.json`
(one additive `cheaprouter` provider entry each). Before the first write per
CLI the originals are backed up into `~/.cheaprouter/takeover.json` and
restored on sign-out / clearing the endpoint. Codex's `auth.json` and Pi's
`auth.json` and `settings.json` are never written — Codex's is only handed
back once to users an earlier build had replaced it for.

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
- **Which CLIs the account may configure** (`sub2api::cli_takeover`,
  `~/.cheaprouter/cli-takeover.json`, one `managed` / `own` per CLI for
  Claude Code, Codex and Grok; absent means managed). A CLI kept on the
  user's own sign-in is listed in `GatewayConfig::own_login_clis`, which
  `desired_routes` uses to drop only the *cloud* target — an endpoint the
  user bound to that CLI still routes, and the built-in agent is never
  affected. The desktop fills the set in `providers_page::cloud_config`, the
  one builder `apply_cloud_routing` and the pages share. At sign-in,
  `detect_own_setup` looks for an own setup on a CLI that was never asked
  about: Claude's `oauthAccount` in `~/.claude.json` or
  `.credentials.json`, an own key / `apiKeyHelper` / relay in
  `settings.json`; Codex's ChatGPT `tokens` or `OPENAI_API_KEY` in
  `auth.json`, or a declared custom `model_provider` in `config.toml`. The
  files the takeover rewrites are read from the takeover backup while one is
  held. Each one found is recorded as `own` *before* the first reconcile —
  nothing is written into it even if the app quits with the question open
  — and the question (`src/app/cli_takeover.rs`) lets the user hand any of
  them to the account. Settings → CheapRouter Account carries the per-CLI
  switches. Since the daemon still reads only the global files, a CLI kept
  on its own sign-in runs on that sign-in inside the app too.

`sub2api::speedtest` measures a set of candidate origins for all three (one
warm-up request, one timed request, bounded concurrency, results in input
order). `sub2api::env_fix` removes the environment variables that would
otherwise outrank everything above, after saving them under
`~/.cheaprouter/env-backups/`; machine-wide Windows variables are never
touched, only reported with the elevated command.

## License

Fork stays GPL-3.0-only. Record every modification in [NOTICE.md](../NOTICE.md)
with its date (GPL §5(a)) and keep the "Built on Waku" attribution in the README.
