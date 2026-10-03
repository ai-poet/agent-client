# DeepSeek Harness reference

English | [中文](deepseek-harness.zh.md)

This is what we know about how DeepSeek Harness (`dsh`) plugins plug into its UI
and its agent logic, and where each of those seams lives in our client. Use it
when porting a dsh feature into Waku. We **port** dsh features; we never load dsh
plugins (see §1).

Path conventions: dsh paths are relative to the git-ignored checkout
`deepseek-harness-master/` at the client root; Waku paths are relative to the
client root. Both are verified to exist as of 2026-10-04.

## 0. What it is and how to read it

- Upstream: `deepseek-ai/deepseek-harness`, every package `0.2.1-alpha.1`, MIT
  (`LICENSE`). TypeScript monorepo, ~319 packages under `packages/<group>/<name>`,
  apps under `apps/` (`cli` = the `dsh` binary, `web`, `desktop` = Electron,
  `desktop-host`). Node `^22.19.0 || >=24.0.0`.
- "Everything is a plugin" on Cordis, vendored under `vendor/cordis` (upstream
  `cordis` 4.0.0-rc.7, republished as `@deepseek-ai/cordis`, local changes listed
  in `vendor/README.md`).
- Read the English `.md` files; skip `.zh.md` and `.i18n.yaml`.
- Start with `docs/architecture.md`, `docs/cordis-primer.md`,
  `docs/user/develop/` (plugin authoring tutorials), `docs/cookbook/extension-cookbook.md`,
  then the relevant `docs/subsystems/<name>.md` and the package `README.md`.
- Generated documents are authoritative over prose: `docs/event-producer-consumer.md`
  (every event's producers and consumers), `docs/capability-seams.md` (every
  `ctx` key), `docs/tool-catalog.md`, `docs/persistence-catalog.md`. For UI slots
  the authoritative list is
  `packages/extensions/cordis-client-runner/src/client/slot-catalog.ts`;
  `docs/subsystems/slots.md` misses about 16 keys.
- The project calls itself a developer preview with breaking changes coming.
  Re-check a mechanism in source before porting it; this document can go stale.

## 1. Port, never load

- A dsh plugin is an ESM module running **inside** the dsh Node process. It is
  handed a live Cordis `Context` and trades live objects with it: `Agent`
  handles, `next()` closures in waterfalls, `AbortSignal`s, `Symbol` tokens,
  function-valued tool definitions. There is no out-of-process plugin mode and
  no plugin sandbox. A Rust client cannot host one; reimplementing Cordis and its
  service set would be rewriting dsh.
- UI halves are React 18 bundles for dsh's own web app. GPUI cannot render them.
- dsh plugins already run **unchanged** in one place: our existing
  `ProviderKind::DeepSeek` provider starts `dsh web` as a sidecar
  (`crates/waku-core/src/deepseek_session.rs`, `crates/waku-core/src/deepseek_pool.rs`,
  `crates/waku-core/src/driver/deepseek.rs`) and dsh loads the user's own
  plugins from its own home. That provider is not what this document is about.
  Do not confuse "port a dsh feature" with "change the DeepSeek provider".
- The vendored engine's `claurst-plugins` crate (`crates/waku-agent/plugins`) is
  dormant (nothing calls `load_plugins`). It is not a route for this work.

## 2. The plugin model (Cordis)

Five ideas (`docs/cordis-primer.md`):

1. A plugin is a function with optional `inject` / `Config`, an object with
   `apply(ctx, config)`, or a `Service` subclass.
2. A context is a repository of services claimed as `ctx.<key>` (`ctx.tools`,
   `ctx.llm`, `ctx.sessions`…). Plugins find services by key, never by import.
3. `inject = ['tools', …]` makes a plugin wait until those services exist. Load
   order is expressed by dependencies, not boot sequence. A missing service
   leaves the plugin pending, not crashed.
4. Typed events, declared by TypeScript declaration merging, each with a fixed
   dispatch mode.
5. Every registration is a reversible effect (`ctx.effect()` / `ctx.on()` return
   a disposer); unloading a plugin unwinds everything it registered (fiber states
   PENDING → LOADING → ACTIVE → FAILED/UNLOADING → DISPOSED,
   `docs/user/develop/framework/index.md`).

| Mode | Awaited | Order | Return value |
|---|---|---|---|
| `emit` | no | registration order | no |
| `waterfall` | no | registration order, around-middleware `(…args, next)` | yes |
| `parallel` | yes | all at once | no |
| `serial` | yes | registration order | yes |
| `bail` | no | until one bails | yes |

Waterfall listeners must call `next()` to delegate; returning without it
short-circuits. `prepend: true` runs a listener before ordinary ones.

Minimal host plugin (`docs/user/develop/basic/tool.md`):

```ts
export const name = 'greet-tool'
export const inject = ['tools']
export function apply(ctx: Context) {
  ctx.tools.register(defineTool({
    name: 'greet', description: 'Greet someone by name.',
    parameters: { name: { type: 'string', required: true } },
    output: { schema: { type: 'string' }, render: (_a, v) => [{ type: 'text', text: v }] },
    async execute(args) { return `Hello, ${args.name}!` },
  }))
}
```

`Config` is a Standard Schema (Schemastery in practice); fields marked
`.volatile()` are edited live without remounting
(`docs/user/develop/basic/config.md`).

Packaging (`docs/user/develop/basic/publish.md`, `packages/boot/app-boot/README.md`):

- **Bundle**: an npm package whose `package.json` has
  `"dsh": { "bundle": { "patch": "./cordis.patch.yml" } }`. The patch is YAML
  rows: `- insert: [{ id, name, config, disabled, inject, group, isolate }]`;
  `name` is the module specifier; `!!js` expressions are evaluated against `ctx`,
  so config files are code. A later layer targets a row by `id` and replaces its
  **whole** `config` (no deep merge).
- **Profile**: `$DSH_HOME/profiles/<name>/` (`$DSH_HOME` defaults to `~/.dsh`)
  with a `package.json` (`dsh.profile.bundles`, pnpm dependencies), its own
  `cordis.patch.yml`, `compatibility.json`, `pnpm-workspace.yaml`
  (`allowBuilds`). Shipped templates: `web`, `headless`, `sdk`, `sdk-minimal`,
  `acp`.
- Layer order: bundles in list order (`@deepseek-ai/dsh-base` first) → profile
  patch → `$DSH_HOME/cordis.patch.yml` → each `--patch` file.
- Install: `dsh plugin --profile <p> add <spec>` (pnpm underneath) or
  `ctx.pluginManager.installBundle`. Plugins whose `peerDependencies` on
  `@deepseek-ai/dsh*` do not match the runtime are refused.
- Plugin Manager reports `applied`, `restart-required`, `overridden` or `failed`
  (`packages/boot/plugin-manager/README.md`).
- Display metadata is read without activating code: `locale/<lang>.json`
  `meta.title` / `meta.description` and an exported `./icon`
  (`packages/preset/agent-preset/skills/cordis-plugin-development/references/host-plugin.md`).
- Templates: `packages/preset/agent-preset/skills/cordis-plugin-development/templates/`
  (`decoration/` = UI plugin, `mcp/` = config-only bundle).

## 3. Host-side logic surface

Main services (`docs/capability-seams.md` classifies all ~90 keys as `core`,
`seam`, `bundle` or `service`):

| `ctx` key | Owner package | What plugins do with it |
|---|---|---|
| `tools` | `packages/core/tools` | `register(defineTool(...))`, `guard(fn)` (final deny) |
| `commands` | `packages/interaction/commands` | `register({name, description, input?, handler(inv)})` — runs without a model turn |
| `skills` | `packages/skill/skill` | `registerProvider(...)` / `register(...)` |
| `llm` | `packages/llm/llm` | `registerAdapter(providers, adapter)`; adapter is `async *stream(options)` |
| `systemPrompt` | `packages/core/system-prompt` | `section({name, order, text})`, `context()`, `variable()` |
| `agents` | `packages/core/agent` | live `Agent` handles: `followup`, `steer`, `inject`, `cancel` |
| `sessions` / `sessionProjections` | `packages/core/session`, `packages/session/session-projection` | append-only log; typed projections folded from it |
| `approval` | `packages/interaction/user-approval` | `request(...)` → `approval/request` waterfall |
| `userQuestions` | `packages/interaction/user-questions` | structured questions → `user-questions/request` waterfall |
| `jobs` | `packages/jobs/jobs` | background jobs; `job_*` tools read and stop them |
| `subagents` | `packages/subagent/subagent` | named providers, continuable children |
| `goals` / `planMode` / `compaction` | see §6 | feature services |

Tool definition (`docs/cookbook/adding-a-tool.md`, `packages/core/tools/src/schema.ts`):

- `parameters` is dsh's own schema DSL compiled to a JSON Schema subset (not zod).
  Raw JSON-Schema `ToolDefinition`s are also accepted (that is how MCP tools
  arrive).
- `execute(args, exec)` returns a canonical JSON value validated against
  `output.schema`; `output.render(args, value)` turns it into model-facing
  `ContentBlock[]`; `output.presentationMeta(args, value)` is stored on the
  result for UI cards; a throw becomes `isError`. `exec` carries `signal`,
  `agent`, `deferContext()`, `concludeTurn()`.
- Tools carry no permission field. Policy is the `tools/pre-execute` waterfall
  returning `{kind:'allow'} | {kind:'deny', reason} | {kind:'cancel'} |
  {kind:'ask', reason?, displayReason?}`; `ask` goes to `ctx.approval`.

Where new behavior goes (condensed from `docs/architecture.md`):

| Goal | dsh mechanism |
|---|---|
| Model provider | adapter on `ctx.llm` |
| Model-facing capability | register on `ctx.tools` |
| Human command | register on `ctx.commands` |
| Background job | register on `ctx.jobs` |
| Intercept request / tool / turn | `agent/*` or `tools/*` events; `agent/turn-stopping` |
| Model-facing context | `agent.inject()` (lands in the next admitted request) |
| Durable state | extend `SessionEventMap`; render and replay from the log |
| Same-session objective | `ctx.goals` |
| Per-agent registration | that agent's `agent.ctx` |

Rule worth copying: **model-visible means logged** — every model request must be
reconstructable from the session log.

Three-role capability pattern (`docs/user/develop/practice/index.md`): a
swappable capability is a **Service Definition** (abstract service owning the
key and request/result types), **Service Providers** (implementations) and a
**Consumer** (usually the tool). Provider and Consumer depend only on the
Definition. In Rust: a trait, its impls, and a tool holding `Arc<dyn Trait>`.

## 4. One turn, seam by seam

Sources: `docs/architecture.md` (Turn flow), `docs/agent-lifecycle.md`,
`docs/tool-execution-pipeline.md`. Durable session events are in **bold**.

1. Inbox: `agent.followup/steer/inject/send` → `agent/inbox/inserted`. Every
   message carries a typed `source` (`packages/llm/llm/src/message.ts`). `inject`
   does not wake the agent.
2. Wake: `agent/status` → running, **`turn/start`**, `agent/inbox/claimed` per message.
3. `system-prompt/assemble` (W): rewrite sections, contexts, tools, variables.
4. `agent/pre-step` (W): `{kind:'reject'}` or `{kind:'enter', messages,
   startsRequestSeries?}`. The main injection point — compaction pressure, plan
   mode, goal rounds, AGENTS.md, time context, skill catalog, hooks all sit here.
5. **`step/start`**.
6. `agent/request` (W): replace provider / model / effort / max tokens (not messages).
7. Reconcile: **`system/message`**, **`user/message`**, **`request/header`**,
   **`request/context`**; the request is derived from the log and frozen.
8. `llm/stream` (W): wrap the adapter (retry, replay, routing).
9. `agent/assistant-stream` (E): live frames only, nothing durable.
10. **`assistant/message`**, or **`assistant/attempt`** + `agent/request-error`
    (W) which may return `{kind:'retry'}` (compaction, image offload, llm-retry).
11. Per tool call: **`tool/call`** → `tools/pre-execute` (W, allow/deny/cancel/ask)
    → guards → `tools/execute` (W, around dispatch) → tool body
    (`fs/write-intent` / `fs/edit-intent` W, `fs/observed` E) →
    `tools/post-execute` (W, may replace content, block, add `additionalContexts`)
    → `tools/result` (E) → **`tool/result`**. Pre runs in order, execute
    concurrently (`maxParallelToolCalls`), post in model order.
12. `additionalContexts` appended as `user/message`.
13. **`step/end`**; loop to 4 if tools owe a request or input arrived.
14. `agent/turn-stopping` (S): a listener can `agent.steer()` to force another step.
15. **`turn/end`**, `agent/status` → idle. Every durable append also emits
    `session/event`.

Events by mode (from `docs/event-producer-consumer.md`):

- Waterfall: `agent/pre-step`, `agent/request`, `agent/request-error`,
  `llm/stream`, `tools/pre-execute`, `tools/execute`, `tools/post-execute`,
  `tools/ptc-dispatch-log`, `system-prompt/assemble`, `approval/request`,
  `user-questions/request`, `fs/write-intent`, `fs/edit-intent`,
  `compaction/summary-error`, `session-telemetry/record`, `connection/request`,
  `workspace/session-activity`.
- Serial: `agent/created`, `agent/turn-stopping`.
- Parallel: `session/flush`, `feedback/committed`, `workspace/session-stop`.
- Emit: `agent/status|error|disposed|assistant-stream|inbox/*`,
  `session/event|created|disposed`, `tools/result|change`, `commands/change`,
  `skills/change`, `subagent/start|end`, `goal/*`, `workflow/*`,
  `plugin-manager/*`, among others.

## 5. UI side

### 5.1 Client modules

- A package opts in with `"dsh": { "client": { "platform": "web", "inject": [...],
  "immediately"?, "external"? } }` and an `exports["./client"]` browser bundle.
  `web` is the only platform.
- Host `ctx.clientModules` (`packages/client/modules/src/index.ts`) scans enabled
  rows, builds a boot graph, injects `window.__DSH_BOOT__` into `index.html`, and
  serves bundles at `/plugins/??a/client.js,b/client.js&rev=`.
- Each bundle registers `window.__ModuleLoader__.load({ id, factory(require) {
  … return { inject, apply(ctx) } } })`. Shared modules a bundle may `require`:
  `react`, `react-dom`, `@deepseek-ai/cordis`, `dsh-client-store`,
  `dsh-client-ui-slots`, `dsh-client-ui-primitives`, `dsh-client-ui-dockkit`
  (`packages/client/web/src/platform.ts`). The browser runs its own Cordis tree
  (`packages/client/web/src/boot-client.ts`); `ctx.uiRenderer` mounts slot `root`.
- UI is optional. A UI-only package still needs an empty host
  `export function apply() {}` to get a Loader row. About 23% of packages have a
  client half; tools, LLM adapters, MCP, skills and hooks never do.

Template (`packages/preset/agent-preset/skills/cordis-plugin-development/templates/decoration/client.js`):

```js
window.__ModuleLoader__.load({ id: '@local/my-decoration', factory(require) {
  const React = require('react')
  return { inject: ['slots'], apply(ctx) {
    ctx.slots.inject('conversation.composer.dock', () => ctx.slots.register(
      { name: 'conversation.composer.dock', id: 'my-decoration', order: 5 }, Decoration))
  } }
} })
```

### 5.2 Slots and client services

A slot has a cardinality — **S**ingle, **L**ist, **K**eyed (by an entry key),
**C**hain — and a scope: **R**oot, **S**ession, or session-**M**aybe. Components
receive props, never `ctx`. Standard props (`docs/subsystems/slots.md`):
`useResource`, `useSessions`, `useSessionStatus`, `useWorkspaces`,
`usePanelInfo` everywhere; `sessionId`, `useSession`, `useProjection`,
`useConversation`, `useInput`, `inputActions`, `useChat`, `useTrajectory` in
session scope; `t` from the registration's `locale`.

| Client service | Purpose | Source |
|---|---|---|
| `ctx.slots` | `register`, `inject`, `renderSlot` | `packages/client/ui-renderer/src/client/registry.ts` |
| `ctx.resources` / `useResource` | `dsh-resource://` providers (`file`, `plan`, `subagentchat`) | `packages/client/resources`, `docs/subsystems/client-resources.md` |
| `ctx.remote.<ns>` | generated RPC to host services (Typert `@Remote`) | `packages/api/remotes/src/client/index.ts`, `docs/api-gateway.md` |
| `ctx.sidebarRightTabs` / `ctx.sidebarRight` | right-pane tab kinds; open / split / float | `packages/client/ui-sidebar-right/src/client/index.ts` |
| `ctx.configForms` | plugin config forms (`state`, `mutate(ops, rev)`) | `packages/client/ui-settings/src/client/config-form.ts` |
| `ctx.locale`, `ctx.theme`, `ctx.shortcuts` | i18n, tokens, keybindings | `packages/client/{locale,ui-theme,shortcuts}` |
| `ctx.commandUi`, `ctx.inputTriggers` | slash popup; `/` and `@` sources | `packages/client/{ui-commands,ui-input-trigger}` |
| `ctx.layout`, `ctx.uiSession` | panels; pending interactions taking over the composer | `packages/client/{ui-layout,ui-session}` |

Not present: a toast or dialog service (packages put `Toast` / `Modal`
primitives into `shell.overlay`), a command palette, OS notifications.

Slot catalog, condensed (all 92 keys in `slot-catalog.ts`):

| Area | Keys (card/scope) | Main occupants |
|---|---|---|
| Shell | `root` S/R, `sidebar` S/R, `main` K/R (panel id), `rightbar` S/R, `shell.bottom` S/R, `shell.leading` S/R, `shell.overlay` L/R | ui-layout; plugin-manager and schedule add `main` panels; many toasts and dialogs in `shell.overlay` |
| Left sidebar | `sidebar.panellist` L/R, `sidebar.footer.action` L/R, `sidebar.workspaces` S/R, `sidebar.workspaces.session.menu.item` L/R, `sidebar.workspaces.session.row.action` L/R, `sidebar.session.row.hover/.leading` L/R | ui-sidebar, ui-workspace, plugin-manager, schedule |
| Settings | `settings.section` L/R, `settings.general.item` L/R, `settings.models.provider-card` K/R, `settings.plugins.tab` L/R, `settings.onboarding` L/R | settings-* packages, theme, locale, shortcuts |
| Plugin manager | `plugins.item` L/R, `plugins.bundle.config` K/R, `plugins.row.config` K/R (`pkg#rowId`), `plugins.detail.*` L/R | settings-agent-loop / shell / subagent / web-search, voice-input |
| Conversation header | `conversation.session.header.actions` L/S, `.utilities` L/S, `.corner` S/S, `.lineage` S/S | agent-preset, jobs, subagent, agent-team; open-in-app, schedule, session-log-export |
| Composer | `conversation.composer` C/S, `conversation.composer.bar` S/M, `conversation.composer.dock` L/S, `conversation.input.dock` L/S, `conversation.input.overlay` L/S, `conversation.input.{model,permission,plan}` S/S, `conversation.input.activity` S/S, `conversation.input.attachments` S/M | approval and user-questions take over the composer (chain); todo / queue / goal docks; slash and `@` menus; model / permission / plan pickers; voice input |
| Transcript | `conversation.chat.node` K/S (by chat-node kind), `conversation.chat.assistant-actions` L/S, `conversation.chat.turnTail` L/S, `conversation.approval.detail` S/S, `conversation.plan-review.actions` L/S | ui-chat (15 node kinds), goal, tool, user-questions, workflow-run; deliverables / plan / schedule at turn tail |
| Tools | `tool.call.toolview` K/S (wire tool name), `tool.call.images` S/S, `tool.view.cordis` K/S | ui-tool, ui-deliverables, ui-skill, ui-cordis |
| Right sidebar | `sidebar.right.pane.tab` K/S (+ `.title`), `sidebar.right.tab.document` K/S (renderer id), `sidebar.right.tab.guide.entry` K/S | deliverables, plan, schedule, browser, documentpreview (8 renderers), files, terminal, subagent |

### 5.3 Tool cards

- `ToolCallTree` (`packages/client/ui-tool/src/client/tool/ToolCallTree.tsx`)
  renders every root call and nested PTC sub-call through
  `renderSlot('tool.call.toolview', owner, { entryKey: toolName, fallback: <GenericToolCard/> })`.
- Props: `callId`, `toolName`, `useDisclosure`, `cwd`, `openFile`, `loadImage`,
  plus a phase union — `preparing` (streaming partial args, the only streaming),
  `start` (`argsRaw`), `result` (`content`, `isError`, `error`, `meta`,
  `subCalls`). Contract: `packages/client/ui-tool/src/client/contract/slots.ts`.
- The card reads the host's `presentationMeta` as `meta` through pure card models
  (`diff`, `read`, `search`, `web`, `image`). Host `presentCall` / `presentResult`
  never reach the web client.
- Dedicated views live in `packages/client/ui-tool/src/client/apply.ts`: bash,
  read, write/edit, grep/glob, web_search/web_fetch, todo_write,
  ask_user_question, plus a generic DetailsRow for goal, schedule, job,
  terminal, subagent, team and session-query tools. `present` (deliverables),
  `skill` and `cordis_*` have their own packages. Everything else, MCP tools
  included, falls back to `GenericToolCard.tsx`.
- Representative business-owned card: `packages/client/ui-skill/src/client/index.ts`.

### 5.4 Fixtures worth reading

`apps/web/tests/fixtures/plugins/`: `fixture-input-extension` (hand-written
`client.js` for `conversation.input.activity` and plugin-manager config pages),
`fixture-layout-bottom` (`shell.bottom`), `fixture-live-client` (live add/remove,
HMR, plugin-manager detail slots, session menu items), `fixture-bundle`
(multi-row bundle metadata, no client half).

## 6. Feature plugin catalog

Host package → client package → subsystem doc. Tools in `code`.

| Area | Host packages | Tools / commands | Client | Doc |
|---|---|---|---|---|
| Agent loop | `core/agent`, `core/agent-loop`, `core/tools`, `core/system-prompt`, `core/session` | `run_code` (PTC mode) | ui-tool, ui-settings-agent-loop | `subsystems/core.md`, `tools.md` |
| Presets | `preset/agent-preset-registry`, `preset/agent-preset`, `preset/persona` | — | ui-agent-preset | `core.md` |
| Context | `context/agent-instructions` (AGENTS.md / CLAUDE.md chain), `context/time-context`, `context/session-reference`, `context/file-reference` | — | ui-reference | `workspace.md`, `session-reference.md` |
| Compaction | `compaction/compaction`, `compaction-basic`, `compaction-tool-result-pruner`, `compaction-image-offload`, `command-compact`, `llm/token-meter` | `/compact` | ui-chat | `compaction.md`, `token-meter.md` |
| Goal | `goal/goal`, `goal/goal-round-driver`, `goal/tool-goal`, `goal/command-goal` | `create_goal`, `get_goal`, `update_goal`, `/goal` | ui-goal | `goal.md` |
| Plan | `plan/plan-mode` | `exit_plan_mode`, `/plan` | ui-plan | `plan.md` |
| Todo | `todo/tool-todo` | `todo_write` | ui-conversation (TodoPanel), ui-tool | `todo.md` |
| Workflow | `workflow/workflow`, `workflow/workflow-ptc`, `workflow/tool-workflow`, `workflow/tool-ralph` | `workflow`, `ralph` | ui-workflow-run | `workflow.md` |
| Schedule | `schedule/schedule`, `schedule/tool-schedule` | `schedule_create/list/update/delete` | ui-schedule | `schedule.md` |
| Jobs | `jobs/jobs`, `jobs/jobs-local`, `jobs/tool-jobs` | `job_output`, `job_list`, `job_kill` | ui-jobs | `jobs.md` |
| Subagents | `subagent/subagent`, `subagent/tool-subagent`, `subagent/tool-subagent-control`, providers `subagent-{spawn,fork}-in-process`, `subagent-{acp,codex,claude-code,dsh-sdk}` | `subagent`, `subagent_fork`, `list_subagent_models`, `send_message`, `interrupt_agent`, `list_agents` | ui-subagent, ui-settings-subagent | `subagent.md` |
| Agent team | `experimental/agent-team`, `experimental/tool-agent-team` | `spawn_teammate`, `wait_agent`, `team_task_*` | `experimental/client-ui-agent-team` | `agent-team.md` |
| Approval | `interaction/user-approval`, `interaction/permission-presets`, `sandbox/sandbox-policy`, `experimental/auto-review` | `/permission` | ui-approval, ui-permission-presets | `approval.md`, `permission-presets.md` |
| Questions | `interaction/user-questions`, `interaction/tool-ask-user` | `ask_user_question` | ui-user-questions | `user-questions.md` |
| Guards | `guard/repeat-tool-reminder`, `guard/timeout-policy`, `fs/fs-observation-policy` | — | — | `tools.md`, `filesystem.md` |
| Deliverables | `deliverables/tool-present`, `deliverables/workspace-changes` | `present` | ui-deliverables | `deliverables.md` |
| Session tools | `session-query/session-query`, `session-query/tool-session-query`, `session-query/session-log-export`, `session/session-title` | `session_search`, `session_trace`, `session_event_*`, `/export` | session-log-export | `session-query.md`, `session-title.md` |
| Skills / MCP / hooks | `skill/skill`, `skill/tool-skill`, `mcp/mcp-client`, `mcp/mcp-resources`, `hooks/hooks-claude-code`, `hooks/hooks-codex` | `skill`, `mcp__<server>__<tool>`, `read_mcp_resource` | ui-skill | `skills.md`, `mcp.md` |
| Other | `lsp/tool-lsp`, `webhook/webhook`, `browser-use/browser-use`, `computer-use/computer-use`, `spill/spill`, `feedback/message-feedback`, `experimental/claude-code-mods`, `experimental/voice-input-bundle` | `lsp`, `/feedback` | ui-message-feedback, `experimental/client-ui-voice-input` | `lsp.md`, `webhook.md`, `spill.md`, `feedback.md`, `claude-code-mods.md`, `voice-input.md` |

Mechanism notes for the features most worth porting:

- **Goal** (`packages/goal/`): a revisioned goal folded from durable
  `goal/change` snapshots (`active | paused | blocked | complete`). The round
  driver waits for `agent/status` idle, flushes, then `followup()`s a
  `<goal_round>` prompt with source `{kind:'goal', goalId, revision, round}` and
  re-checks it in `agent/pre-step`. Only an admitted message counts a round; the
  cap records a `round-limit` blocker. `create`/`edit`/`pause`/`resume` require a
  human-sourced message in the current turn; `blocked` needs ≥ 3 rounds. After
  resume or fork a goal stays disarmed until a human resumes it; cancelling a
  round pauses the goal.
- **Plan** (`packages/plan/plan-mode/src/index.ts`): log-only `plan/mode`; the
  guidance text is a prompt section at order 50, configured in
  `packages/bundle/base/cordis.patch.yml`. A prepended `agent/pre-step` listener
  applies a pending switch only after the step is accepted. `exit_plan_mode` is
  always registered (stable tool list → stable cache), fails outside plan mode,
  and presents the plan through `ctx.userQuestions`. Plan mode is guidance only;
  sandbox and approval enforce.
- **Todo** (`packages/todo/tool-todo`): `todo_write` replaces the whole list
  (`{content, status}`, no ids); log-only `todo/write`, last write wins; the
  projection clears at the next turn start; nothing is re-injected into the
  prompt.
- **Compaction** (`packages/compaction/compaction-basic/src`): a serial pre-step
  check prices the last request via `ctx.tokenMeter`; trigger
  `floor(min(W×0.8, W−O−65536))`; prune first, then summarize the oldest balanced
  span and keep the newest 16% of `W−O` verbatim. Overflow recovery hangs off
  `agent/request-error`. The transaction is bracketed in the log
  (`compaction/start` → `compaction/summary` + replacement `user/message` with
  `surfaceOp` → `compaction/end`); an unmatched start is a lock. It never splits
  a tool call from its result, never shadows system node 0, and rejects a
  summary that does not shrink. The summary request replays the cached prefix
  byte for byte to stay warm.
- **Subagents / team** (`packages/subagent/subagent`, `packages/experimental/agent-team`):
  named providers with capability flags checked before start; the tool
  registers and unregisters as its provider appears; background one-shots become
  jobs; continuable children report back via `parent.inject()` / `steer()` /
  `followup()`. Team: the lead's log is the only source of truth; roster
  `team/member`, mailbox `team/message/queued|delivered` with resend-on-recovery,
  tasks as a compare-and-set DAG `team/task`.
- **Auto-review** (`packages/experimental/auto-review/src/index.ts`): an `auto`
  permission preset (full-access sandbox + `ask` policy) plus a prepended
  `tools/pre-execute` listener that asks the current model for strict JSON
  `{risk, decision, reason?}`. A deny falls through to `ask` so a human decides;
  in-process children pin `never`. Arguments are never rewritten.
- **Workflow** (`packages/workflow/`): the `workflow` tool takes
  `{script, meta, args}`; `workflow-ptc` runs the JS in a fresh sandboxed Node
  process with `agent()`, `parallel()`, `pipeline()`, `phase()`, `log()`; every
  `agent()` goes to `ctx.subagents`; a child failure resolves `null`. Durable
  `tool-workflow/*` records feed the run node in the transcript.
- **Schedule / jobs** (`packages/schedule/schedule/src`, `packages/jobs/tool-jobs`):
  tasks live in a host storage domain, not the session log; rules `after`, `at`,
  `every` (≥ 60 s), `daily`/`weekly` with an IANA zone, five-field `cron`. One
  timer for the earliest target; delivery `followup()`s a `[SCHEDULE REMINDER]`
  into the (possibly cold) session; recurring tasks deliver only their latest
  missed occurrence; delivery is not atomic. Job completion is `inject()`ed when
  busy or wakes the agent when idle, with a wake cap reset by human input.

## 7. dsh → Waku mapping

Waku has no slot or service registry. A dsh slot becomes either a closed enum
variant plus `match` arms, or one `.child()` / `.children()` line in an upstream
render function; the feature itself goes in a new fork-owned file
(`docs/FORK.md`, "Design rule").

### 7.1 UI

| dsh seam | Waku seam | How to hook in |
|---|---|---|
| `sidebar.panellist`, `main` panel | `src/app/sidebar.rs` `render_sidebar_search`, `render_sidebar_action_row`; `src/app/model_status.rs` `main_page_open` / `close_main_pages` / `main_page_title` | a row in a new file; one `.child()`; bump the row-height multiplier in both places; state field on `Waku`; `.when(open, …)` in `src/app/render.rs` (pattern: `src/app/image_studio.rs`) |
| `sidebar.workspaces.session.*` | `src/app/sidebar.rs` `SidebarRow`, `sidebar_row` | new variant or row action |
| `sidebar.right.pane.tab` | `RightPanelSurface` in `src/app.rs`; `src/app/right_panel.rs`; `src/app/surface_bar.rs` `SurfaceKind` | new variant; compiler lists the arms (`label`, `icon_path`, `reusable_surface_index`, `render_right_panel`); optional header button and shortcut |
| `shell.overlay` (dialogs, toasts) | `Option<XState>` + `render_x` in both branches of `src/app/render.rs`; `src/app/confirm_dialog.rs` `request_confirm`; `show_toast*` in `src/app.rs` | new modal file; toast helpers |
| banners, `shell.quota-notice` | `src/app/update_banner.rs`, `src/app/daemon_banner.rs`, `src/app/error_banner.rs` | `render_x_banner` + `.children()` in `render.rs` |
| `shell.bottom` | not found (only `render_workspace_footer` in `src/app/composer.rs`) | would be new |
| `conversation.session.header.actions` | `render_header` in `src/app/sidebar.rs` | one `.child(self.render_x_action(cx))`; `icon_button` / `popover` from `src/ui` |
| `conversation.chat.node`, `turnTail` | `TranscriptRowKind` in `src/app/transcript.rs`; `transcript_row` in `src/app/transcript_view.rs` | new row kind or a branch in the row builder |
| `tool.call.toolview` | `render_activity_item` in `src/app/transcript_view.rs` (sub-agent precedent: `src/app/subagent_row.rs`) | there is no per-tool registry and `ActivityItem` (`crates/waku-protocol/src/model.rs`) has no tool-name field: add a `#[serde(default)]` field, set it in `crates/waku-core/src/driver/native.rs` / `driver/activity.rs`, early-return to a renderer in a new file, run `bun run protocol:generate` |
| `conversation.composer.dock`, `conversation.input.dock` | `src/app/render.rs` (above / below the composer); `src/app/composer.rs` `render_composer` control row | one `.children()` line; a control beside the model / effort / mode / goal chips |
| `conversation.input.overlay` (slash, `@`) | `src/app/autocomplete.rs`; `crates/waku-client/src/composer_complete.rs` | new trigger source or rows |
| `conversation.composer` chain (approval, questions), `conversation.approval.detail`, `plan-review.actions` | `src/app/permission_card.rs` `render_permission_card` (plan branch: `is_plan_approval`); `render_user_input` in `src/app/composer.rs` | branch inside the card like the plan approval |
| `settings.section`, `settings.general.item`, `plugins.*` | `SettingsPage` in `src/app.rs`; `SETTINGS_PAGES` in `src/app/settings.rs` | variant + row (bump the array length) + title and dispatch arms + `settings.x` / `settings.x_keywords` keys + `src/app/tests.rs` |
| `ctx.shortcuts`, palette | `src/app/command_palette.rs` `PaletteAction`; `src/app/shortcuts.rs` | new action variant |
| `ctx.locale` | `locales/app.yml` (English, `en:` lines), `locales/zh-CN.yml`, `locales/ja.yml`; `tr!` | one key namespace per feature |
| `ctx.theme`, primitives | `src/theme.rs`; `src/ui/mod.rs`, `src/ui/menu.rs`, `src/ui/motion.rs` | reuse; icons in `assets/icons/` + `src/assets.rs` |

### 7.2 Logic

| dsh seam | Waku seam | Scope |
|---|---|---|
| `ctx.tools.register` | `Tool` trait (`crates/waku-agent/tools/src/lib.rs`, vendored — do not edit); `builtin_tools` / `engine_tools` in `crates/waku-agent-bridge/src/session.rs`; pattern `crates/waku-agent-bridge/src/subagent.rs`; switchable list `BUILTIN_TOOLS` in `crates/sub2api/src/agent_settings.rs`; titles in `native.rs` | built-in agent only; for every provider ship an MCP server instead |
| `ctx.systemPrompt.section`, `agent.inject` | `session_rules` / `refresh_session_rules` in `crates/waku-agent-bridge/src/config.rs` | built-in agent |
| `tools/pre-execute` policy, `ctx.approval` | `crates/waku-agent-bridge/src/permission.rs` (`PermissionBridge`, `GuiPermissionHandler`) | built-in agent |
| `ctx.userQuestions` | `forward_questions` in `session.rs` → `DriverEvent::UserInputRequested` | built-in agent; other providers map their own |
| `ctx.jobs`, subagent records | `BackgroundWorkEvent` (`crates/waku-protocol/src/model.rs`), `crates/waku-agent-bridge/src/background.rs`, `src/app/background_work.rs` | all providers that report it |
| goal / plan / todo / compaction | goal: `crates/waku-agent-bridge/src/goal.rs`, `src/app/goal_dialog.rs`; plan: `InteractionMode`, `/plan`; todo: `crates/waku-protocol/src/todo.rs`, `src/app/todo_list.rs`; compaction: `DriverEvent::ContextCompaction` | exists today; port refinements, not new systems |
| `ctx.commands` | `assemble_slash_commands` in `crates/waku-core/src/composer_complete.rs`; `execute_local_composer_command` in `src/app/composer.rs`; no code: `.cheaprouter/commands/*.md` | app-level = every provider |
| host service + `ctx.remote` | `Command` (`crates/waku-protocol/src/protocol.rs`) + `handle_driver_command` (`crates/waku-core/src/daemon.rs`); stateless: `WorkspaceOperation` (`crates/waku-protocol/src/workspace.rs`) | daemon |
| session events | `DriverEvent` (`model.rs`) + `crates/waku-protocol/src/driver_wire.rs` | an unknown kind is an error on older clients — prefer `BackgroundWorkEvent` or a `serde(default)` field |
| durable state | `AgentSession` / `AgentTurn` `#[serde(default)]` fields (`model.rs`, stored in `app.db`); tables via `db/schema.ts` + `bun run db:generate`; fork-local JSON in `~/.cheaprouter/` via `crates/sub2api` | — |

Always state which scope a port lands in: **built-in agent only** (bridge) or
**every provider** (daemon / app layer).

## 8. Porting recipe

1. Find the pair: host package (logic) and client package (UI) — §6.
2. Read its `README.md`, its `docs/subsystems/*.md` page and its `src/`. List the
   seams it uses (§4), the durable events it writes, and any prompt text.
3. Decide the Waku layer and scope (§7): bridge, driver, daemon or app; built-in
   agent only or every provider.
4. Follow `docs/FORK.md`: new functionality in new files, upstream files get
   one-to-three-line hook points recorded in the register, new files listed under
   "Files that are ours entirely", a dated row in `NOTICE.md`, protocol changes
   additive only (`serde(default)`, new workspace operations). Never edit the
   vendored engine except as a recorded departure.
5. Follow `AGENTS.md`: no I/O or spawning reachable from `render`; work on
   `cx.background_executor()`; keyboard operable.
6. Add i18n keys to all three locale files under one namespace.
7. Keep the attribution when translating closely — see §9.

## 9. Licenses

- The repository is MIT, "Copyright (c) 2026 DeepSeek" (`LICENSE`). Vendored
  Cordis and siblings are MIT, "Copyright (c) 2021-present Shigma"
  (`vendor/cordis/LICENSE`). When code or prompt text (goal round prompt,
  compaction preamble, plan guidance in `packages/bundle/base/cordis.patch.yml`)
  is translated closely, keep the MIT notice in the new file's header and in
  `NOTICE.md`. Our fork is GPL-3.0-only; MIT code may be included with its notice.
- Do not copy: `@deepseek-ai/libreoffice-kit*` (MPL-2.0) and
  `@anthropic-ai/claude-agent-sdk` (not permissive). See `THIRD_PARTY_NOTICES.md`.
