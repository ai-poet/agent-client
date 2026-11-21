# Scheduled tasks

This ordinary external package owns task definitions and scheduling rules. Its
headless handlers use private KV and bounded host transactions to register
schedules, event handlers, and concurrency groups. The callback invokes the
public `sessions.start` capability, which atomically creates a conversation and
admits its first turn through the existing Node command ledger and Agent queue.

`completion: "turn"` holds the dispatch slot through model execution, tools and
approvals. Closing the plugin page does not stop execution. Removing a task
cancels its unstarted deliveries, not running conversations. History reads the
original admission receipt; unknown effects are never automatically replayed.

New schedules choose their execution host from the controller resource catalog.
Projects, worktrees, model choices and teams are read for that host. Every new
record stores `execution_node`; editing, pausing, manual runs, deletion, history
and queue cancellation or stopping return to that same Node. Schedule and queue
state stays in its local dispatcher, even when personal task records are visible
through another controller or execution host. Opening a run uses its recorded
Node as well as its session ID.

Source and executor KV revisions are independent. Edits, removal and manual runs
can carry `expected_value`, the original task content without its projected
revision, status or next-run metadata. The execution host first checks that
content, then uses its own local revision for mutations. Editors retain their
original snapshot through list refreshes and reject concurrent content changes.
Calls without this content condition keep their existing local revision checks.

Changing a host is available only while creating a task. An unavailable host
does not create a local replacement. Legacy records without an owner require
an existing dispatch handler or schedule on the local Node before their next
write can claim ownership.

Handlers: `list`, `save`, `remove`, `run`, `history`, `queue`, `cancel`, `stop`, `projects`.
Task and queue revisions are decimal strings. New task IDs may be omitted; the
host invocation ID provides a stable identity. Every task stores an explicit
model configuration. Project and team catalog access is explicit and does not
expose conversations or private scratch worktrees.

New desktop tasks default to model execution. Tasks may instead select the
default team or a named team; existing tasks without an execution selection
retain default-team behavior. Named selections store their identity and resolve
the current team revision at each admission. Admitted turns retain their frozen
team and member configuration. Agent-created tasks inherit the admitted turn's
execution selection unless the tool explicitly selects another mode or team;
updates preserve omitted fields. Switching execution mode or team clears member
model overrides. A fixed team lead previews its current model and effort in the
editor and disables manual lead overrides. Save and Run resolve that current
lead through Node model selection while preserving task scope and options;
each execution still captures the latest team at admission.

Newly saved handlers create each session through the shared Node path and
associate it with `{namespace:"scheduled-tasks", kind:"task", id:task.id}`.
Running an existing task manually checks its local KV revision with a same-value
write, refreshes its current handler and enqueues the run in one transaction.
This advances the personal record's revision without changing its content and
preserves the schedule cursor and previously captured deliveries.
Older recurring handlers retain their original
command until the task is saved or run manually. Execution selection and task
association use the same session API as other plugins and external clients;
there is no package-owned Agent runner.

The desktop entry owns its Kit page, forms and locale resources. Read-only views
share package queries with the headless entry; writes use durable host callbacks.
History opens the existing host conversation route. Stopping a run is confined to
the original turn admitted by this package's dispatch job.

New profiles install this as an ordinary default package. Existing profiles are
not migrated or rewritten, and restarting never reinstalls a removed package.
