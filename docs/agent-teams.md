# AgentTeams

AgentTeams turns a built-in agent session into the **captain** of a small team. The captain drafts the members and a task DAG and waits for the user's approval. A scheduler then hands each ready task to a member: a persistent sub-agent with its own model route, conversation and permission scope. Members talk to the captain and to each other through mailboxes, and every task passes the quality gates before it counts as done.

It is a translation of dsh-agent-teams (`@nanmicoder/dsh-agent-teams` 0.1.22, MIT, Copyright (c) 2026 程序员阿江(Relakkes); licence in `NOTICE.md`). It replaces the former Settings → Workflow page. Do not bring that page back.

## Scope

- **Built-in agent only** (`ProviderKind::Native`). Claude Code, Codex and the other CLI providers do not get it.
- **No wire protocol change.** Members reach the desktop as ordinary `Subagent` background-work records of the captain's session. The Team panel reads the state files through the daemon's existing `WorkspaceOperation::{BrowseDirectory, ReadTextFile}`, so a remote daemon works too.
- **Fully ported:**
  - every tool and its rules;
  - the quality gates: seven task kinds, contracts, completion checks, automatic repair and re-review follow-ups, limits and escalation, delivery audit;
  - profiles;
  - cold recovery.
- **Not ported yet:** the automatic switch to a member's `fallback` route on 429 / 401 / 402. A member record whose `fallbackActive` is already set runs on its fallback route (`member.rs::route_of`), but nothing sets that flag yet.

## Using it

1. In a built-in agent task, send `/agent-teams <goal>`. Two other forms pick a profile:
   - `/agent-teams --profile <name> <goal>`;
   - `/agent-teams-<name> <goal>`.

   The composer offers all three forms when the feature is enabled and `slashCommand` is on.
2. The captain drafts the team and ends its turn. The Team surface opens in the right panel with the draft and three buttons:
   - **Approve and start.** Saying "approve" in the chat works too.
   - **Back to chat to revise.** The captain asks what to change, rewrites the draft as a whole, and waits again.
   - **Discard plan.** The draft is archived and the captain does not rebuild it.
3. Once approved:
   - members claim ready tasks;
   - a member reporting back wakes the captain;
   - a member's permission dialog appears in the captain's session, titled with the member's name;
   - clicking a member in the panel opens its live record.
4. When every task is done, the captain summarizes and archives the team. **Stop team** in the panel asks for a second press. The composer's stop button stops only the captain's current turn; the members keep working.

Settings → Agent → Teams edits the configuration:

- the enable switch;
- the slash command;
- the member cap;
- whether members may start one level of sub-agents;
- the default member model and effort;
- the execution prompt;
- the profiles, edited as JSON with the same validation team creation uses.

## Code map

| Where | What |
|---|---|
| `crates/agent-teams` | Pure logic, no tokio and no GPUI. It holds:<ul><li>the types (camelCase, the same on-disk format as the reference, unknown keys kept in `extra`);</li><li>the store and mailbox;</li><li>the state machine (`transitions.rs`);</li><li>the quality gates (`quality.rs`);</li><li>profiles, prompts, `/agent-teams` parsing and the control sentinel (`command.rs`);</li><li>the config;</li><li>the panel snapshot and DAG layout (`snapshot.rs`);</li><li>the synchronous runtime (`runtime/`: tool operations, scheduler, status rendering, process-wide keyed locks) behind a `Host` trait.</li></ul> |
| `crates/waku-agent-bridge/src/team/` | Implements `Host` for one session:<ul><li>`TeamHost` (`mod.rs`): activation, prompt augmentation, controls, the idle edge;</li><li>`BridgeHost` (`host.rs`);</li><li>`MemberRuntime` (`member.rs`): one member's engine, history and turns;</li><li>`TeamTool` (`tools.rs`): the runtime's operations as engine tools, run on `spawn_blocking`.</li></ul> |
| `crates/waku-agent-bridge/src/session_team.rs` | Mounted inside `session.rs` (`#[path] mod team_seam`) so it can reach the private turn state. It contains:<ul><li>`Port`, which wakes the captain;</li><li>the snapshots that carry the team's running members;</li><li>`PromptOrigin`.</li></ul> |
| `crates/waku-core/src/driver/native.rs` | Keeps one feed per member across its turns (`member_feeds`) and titles a member's permission dialog with the member's name. |
| `src/app/team_panel.rs` | The right-panel Team surface and its surface-bar button. |
| `src/app/agent_teams_settings.rs` | The Teams section of Settings → Agent. |

## State on disk

Everything lives under `<workspace>/<stateDir>/`, where `stateDir` defaults to `.agent-teams`. That directory gets a `.gitignore` containing `*`.

```
.agent-teams/
  <teamId>/
    team.json              members, tasks, phase, plan review, attempts
    inbox/<key>.jsonl      one mailbox per member, plus "captain"
    sessions/<key>.json    a member's conversation (engine Message list)
    retired-members.json   removed members, so their late writes are refused
  archive/<teamId>/        finished, discarded or deleted teams
```

- **Writes are atomic.** A write goes to a temp file and is renamed into place. On Windows, `EPERM` is retried three times 50 ms apart before the file is written directly. A BOM is stripped on read.
- **Keys** come from `sanitize_key`:
  - letters and digits of any script survive, everything else folds to `-`;
  - a name with no letters or digits becomes `k-<digest>`;
  - a name longer than 48 code points is cut and gets a digest appended.
- **`captainSessionId`** is the session's `provider_native_id()` (the native resume cursor), not Waku's session UUID. The panel and cold recovery both find a team by it.
- **A member's id** is `{captain}::team:{team}:{key}:{uuid8}`. It is minted at the member's first assignment and stored in `team.json`. It serves three purposes:
  - the member's `ToolContext.session_id`, which is how a team tool knows who is calling;
  - the key of its background-work record;
  - the key of its feed in `native.rs`.

## Lazy activation and the prompt cache

A session starts with neither the team tools nor the captain protocol. It activates, for the rest of the session, when:

- a user message starts with `/agent-teams` or `/agent-teams-<profile>` while the feature is enabled; or
- `attach` finds an unarchived team on disk whose `captainSessionId` is this session (cold recovery).

Activation happens at the top of `run_turn`, before the tool list is read, so the turn that carried the command already has the 14 captain tools. Two consequences follow:

- The captain protocol section is rendered once, when the session starts, and appended only while the session is active. It is therefore byte-identical on every turn, and activation costs exactly one prompt-cache miss.
- Sub-agents spawned with `Agent` never see the protocol. `extend_rules` runs after the sub-agent query template is cloned.

A member gets the four member tools and nothing else from the team:

- `claim_task`;
- `update_task`;
- `send_message`;
- `status`.

Its persona and `TEAM_MEMBER_PROMPT` are frozen at spawn. It never gets plan mode. It does not get `Agent` unless `memberMaxDepth` is 1.

## Scheduling

The scheduler is event-driven. It runs `kick_team` / `kick_member` in four situations:

- after approval;
- after every task update;
- on a member's idle edge;
- on the captain's idle edge (`after_turn`).

Each step works as follows:

- **Attempts.** `plan_dispatch` picks the next ready task, one whose dependencies are all completed, for each idle, valid member. It opens an attempt and sends the assignment prompt. The dependencies' outputs are cut to 2,000 characters each and 12,000 in total.
- **Attempt ids.** Each attempt has an `attemptId`. A member's `update_task` must carry the current one, so a stale member cannot complete a task that was reassigned.
- **Delivery modes.** An assignment is delivered as `Queue`: it becomes the member's next turn. A message is delivered as `Steer`: it joins the running turn at its next step, or starts one. Before every turn or injection, an admission check runs under the team lock. It refuses delivery when:
  - the team is staged or halted;
  - the member was removed;
  - the member is stopping or being reassigned.
- **When a member's turn ends**, its history is saved. Then:
  - an error or budget stop runs `fail_member_open_attempt`: the task is marked failed and the captain is mailed and woken;
  - otherwise the member's queue is drained first, and only then does the idle edge park the open attempt or dispatch the next task.
- **Reassignment** has two phases. The old member must go quiet before the new one gets the task.

## Waking the captain

`Port::wake` handles mail for the captain:

| Captain state | What happens |
|---|---|
| idle | A turn starts with `PromptOrigin::Team`. |
| running a turn | The mail is injected into the turn's command queue **silently**, never through `push_steer`. The app would echo a steer into the transcript as the user's own message. |
| compacting | The wake is deferred until the compaction ends. |

At the end of the captain's turn, `after_turn` finds any injected mail the model never consumed and wakes the captain again with it. When the captain goes idle, it hands back any task it still holds and flushes its mailbox.

Mail uses a 60-second delivery lease. Mail that was delivered but never acknowledged is offered again once the lease runs out.

## Member permissions

A member's dialogs go through its own `PermissionManager`, scoped by `MemberScope`. The request id is `team:<uuid>:<member>` (`agent_teams::requests`). The app treats these ids specially:

- it accepts them while no captain turn is running;
- it keeps them across `TurnFinished` and Stop (`streaming.rs`, `runtime.rs`, `sessions.rs`);
- answering one leaves an idle session idle.

A captain cancel releases only the captain's own dialogs (`release_where(!is_member_request)`). Stopping or removing a member releases that member's dialogs (`release_member`).

## Background work

While a member's turn runs, it appears in **every** background-work snapshot of the captain's session. If it is missing from a snapshot, `reconcile_live` marks the record lost. When the member goes idle, the record settles as `Finished`.

These records also have two other effects:

- A running member keeps the captain's runtime from being reclaimed.
- `stop_background_work` on a member's id interrupts that member alone, and its open attempt is parked.

## Panel controls

The panel never sends a chat message. It sends a control through `runtime.driver.prompt`: the prefix `<waku:agent-teams>` followed by JSON `{ "action", "teamId" }`. The bridge intercepts it in `prompt()` before a turn is opened. If the session has no runtime, the app queues the control and starts one.

| Action | Effect |
|---|---|
| `approve` | Validates the graph and every member route, sets the phase to running, kicks the team, and wakes the captain with the approved context. |
| `revise` | Sets the plan to `awaiting_feedback`, cancels the captain's turn if one is running, and wakes it with the revise context. |
| `discard` | Archives the team, cancels the captain's turn, and parks the discard context for the user's next message. |
| `stop` | Halts every task and member, cancels the captain, waits for the members to drain (up to 10 s), and cancels the captain again. |
| `kick` | Runs `kick_team` only. This is the panel's "continue" after the runtime was reclaimed. |

The panel polls every 2 s while the team is active and every 15 s otherwise. It also refreshes when a turn settles and when a member's background work changes.

## Recovery

After a restart, `attach` finds the session's unarchived team, activates the session, and calls `resume_after_start`:

- attempts that were open become parked;
- members' conversations are read back from `sessions/<key>.json`;
- the scheduler re-dispatches.

A member is rebuilt on its recorded route. Its client is rebuilt only when the route's fingerprint changes.

## Configuration

The configuration is `agent-teams.json`, beside the engine's `settings.json` (`native::config_dir()`). It is a separate file because the engine's `Settings::save_sync` writes back only the keys it knows, so an "always allow" answer would wipe anything else. Keys this build does not know are kept and written back unchanged.

```jsonc
{
  "enabled": true,                 // master switch; off = no tools and /agent-teams is not intercepted
  "stateDir": ".agent-teams",
  "memberModel": "openai::gpt-6-sol", // optional; "platform::model"; unset = the captain's model
  "memberReasoningEffort": "high",    // optional; "default" = none; inherited only when the model is the captain's
  "executionPrompt": "…",            // optional; appended to every member persona
  "fallback": { "provider": "…", "model": "…" }, // stored, see Scope
  "memberMaxDepth": 0,             // 0..=1
  "maxMembers": 8,                 // 1..=16
  "slashCommand": true,
  "profiles": { "<name>": { /* description, protocol, taskPlanning, members, tasks, … */ } }
}
```

- **`profiles` absent** means the three built-in profiles:
  - `prd-implement-review` (seed);
  - `implement-test-fix` (captain);
  - `dual-review` (seed).
- **`profiles: {}`** means none.
- **Limits:** at most 16 profiles and 32 seed tasks each. An unknown key gets a "did you mean" hint.

Changes take effect for sessions started afterwards.
