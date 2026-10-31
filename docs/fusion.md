# Fusion (Lead + Sidekick)

Fusion is a per-session, built-in-agent-only pairing. Its interaction follows
Devin Fusion's model: one lead faces the user, a cost-efficient sidekick executes
well-defined briefs, and the lead reviews the result. No Devin code is used.

## User interaction

- The native model picker has a **Fusion** family row. Its pair editor keeps the
  existing model search, vendor column and keyboard navigation: the Lead and
  Sidekick buttons decide which role the rows configure. The lead effort can be
  changed beside them.
- The composer summarizes the pair as `Fusion · <lead> ◆ <sidekick>`.
- `/fusion` opens the pair editor. `/model` leaves Fusion and opens the ordinary
  picker. Picking a model outside the pair editor also leaves Fusion.
- There is no second chat window and no separate Fusion surface. `SidekickBrief`
  appears as a Sidekick activity and uses the existing sub-agent row, record and
  stop controls.

## State machine and ownership

The lead is the ordinary `AgentSession`: it owns planning, ambiguity,
correctness-critical investigation, user questions and final review. The bridge
adds a stable lead protocol on every Fusion turn.

`SidekickBrief` delegates mechanical implementation, exploration needed to carry
out an already-settled plan, builds and tests. The brief contains the goal,
relevant files, constraints and success criteria. The sidekick reports changed
files, verification and unresolved issues; the lead must review the actual work
before reporting completion.

- `SIDEKICK_REPORT`: a result is ready for lead review.
- `SIDEKICK_FAILED`: execution or its retry budget ended unsuccessfully. The
  lead diagnoses it, then fixes it or sends a corrected brief.
- `SIDEKICK_ESCALATION_REQUIRED`: an otherwise-undecided tool permission needs
  the lead. The lead may perform the action through its ordinary permission
  flow, or refuse it. The sidekick never approves it and never opens a second
  user-facing dialog.

This is not model failover: retries stay on the selected model and route.

## Independent route and context

`crates/waku-agent-bridge/src/fusion.rs` builds a second `Config`,
`QueryConfig`, client, provider registry and permission manager from the same
routing document, using the sidekick's model. The lead's resolved client and key
are never reused. Gateway per-model keys and `custom:<provider>::<model>`
endpoints therefore work for either role.

The sidekick keeps its own `Vec<Message>` across briefs. Completed history is
stored separately at `agent-sessions/sidekick/<session id>.json`; it never enters
the lead transcript. A rewind clears it because its view of the discarded future
would be stale. Route refreshes replace snapshots for later briefs without
interrupting a running one. Access-mode changes rebuild the sidekick's rule
manager as well as the lead's.

## Permission rule

`GuiPermissionHandler::for_sidekick` preserves settled decisions: allow rules,
written deny rules, plan-mode denials and explicit consents behave normally.
Only a request that would otherwise open a dialog is recorded and denied as an
escalation. Re-performing it as the lead runs the unchanged permission path, so
Allow Always / Reject Always and step-up rules cannot be bypassed.

## Protocol and merge boundary

`AgentSession.fusion` and `WireDriverStartOptions.fusion` are optional additive
fields with `serde(default)`. The lead continues using the existing model,
reasoning-effort and service-tier fields; `FusionPair` carries only the
sidekick's equivalents. Other providers ignore the field. Regenerate TypeScript
with `bun run protocol:generate` after changing it.

Implementation lives in `crates/waku-agent-bridge/src/fusion.rs` and
`src/app/fusion.rs`, with hooks in the existing session, permission, sub-agent,
native-driver and picker files. **Do not replace this with the vendored
engine's `ManagedAgentConfig` or `claurst_query::AgentTool`:** this fork
intentionally runs its own bridge sub-agents. Keep the feature built-in-agent
only and preserve it on upstream merges.
