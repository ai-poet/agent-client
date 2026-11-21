# Goals

`/goal <text>` sets one goal for the conversation and keeps the session working on it, turn after turn, until the goal is complete or the user closes it. A message, a Stop or a pause in between stays inside the goal. The contract, with every state, reason and limit, is in the application repository's `docs/goal.md`.

## How it runs

- The package is a loop over the Node's `after_turn` callback. Each ended main turn either admits one continuation (`continue_turn`, fenced on the goal's stored revision), waits for a turn or background work that will end by itself, or pauses the goal with a named reason.
- The objective is a pinned context part from `turn.initialize`; the per-state instruction is fixed text. Counters that change every turn appear only in continuation messages.
- Completion takes two steps by the working model: `update_goal(complete, evidence)` records a claim, and the "Verify goal" continuation that follows must call it again. An audit that does not confirm drops the claim and work goes on.
- The Node bounds automatic turns. Every continuation declares the goal's turn limit, so notice turns and continuations share one count since the user's last message.

## Who may do what

| Action | User | Model |
|---|---|---|
| Start | `/goal <text>` | `create_goal`, only on request, asks for approval |
| Replace | `/goal <new text>` | No |
| End | Panel, `/goal end` | No |
| Pause, Resume | Panel, `/goal pause`, `/goal resume`, Stop and reply | `update_goal(paused or active)`, only in a turn the user started |
| Complete | Reopen, on the completed goal's panel, undoes it | `update_goal(complete)`: claim, then audit |
| Blocked | A reply or Resume continues | `update_goal(blocked, what is needed)` |

## Storage

Conversation values `goal` (the current goal: objective, status, reason, counters, pending claim) and `history` (the last 20 goals the session finished, replaced or ended; an ended goal keeps the stored outcome `cleared`). Both follow fork and rewind; a restored goal is paused until Resume.

## Settings

`turn_limit` (automatic turns without user input, default 25, at most 100), `token_budget` (thousands of tokens, 0 for none) and `time_limit` (minutes of active time, 0 for none). Resuming a goal paused at a budget grants that budget again.

## Tests

`node --experimental-vm-modules --test goals/tests/*.test.mjs` covers the state machine (`host.test.mjs`), the panel (`view.test.mjs`) and its controller (`controller.test.mjs`). The loop on a real Node is `TestGoalLoop` in the application repository.
