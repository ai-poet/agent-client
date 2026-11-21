# Changes

## 0.2.4

- Keep replies completed during a manual pause out of goal totals when their callbacks arrive after Resume
- Preserve continuation admission when a paused reply's completion callback arrives late

## 0.2.3

- Shorten test labels without changing assertions
- Correct the production SDK history fixture and verify configured goal limits

## 0.2.2

- The Status badge shows the state and the time, or the reason once paused, after its icon; it no longer repeats the word Goal
- The panel has Pause or Resume and End. Edit is removed: `/goal <new text>` replaces the goal. Clear is now End, `/goal end`, and an ended goal is listed as Ended
- A goal turn that fails is reported once, by the app's own notification; the goal still shows Paused · Error

## 0.2.1

- A goal no longer stays Waiting or Running when nothing can wake it: it shows paused, reason Stopped, after the background work it waited on was stopped, Turn limit reached when the result was held at the limit, and App restarted when its last turn ended before a restart

## 0.2.0

- `/goal <text>` starts at once and keeps going through continuation turns until the goal is complete; a message or a pause stays inside the goal
- Completion is a claim with evidence that a separate Verify goal turn must confirm; Reopen undoes it
- Stop pauses and a reply resumes; Pause holds until Resume; a restart, fork, rewind or Plan mode pauses with its reason
- Limits: 25 automatic turns by default, optional token and time budgets, and a no-progress rule; a reached limit wraps up and pauses
- The goal waits for background work, and notifies when it completes, is blocked or is paused by anything but you
- Three tools replace six: the model reads, starts on request and reports status; only you edit, replace or clear
- The badge shows state and time; the panel shows turns, time, tokens, Edit and earlier goals

## 0.1.10

- Restore outline icons in navigation and tools

## 0.1.9

- Use a new plugin icon

## 0.1.8

- Read localized goal status on the execution Node for every controller
- Show the goal chip only after a submitted goal exists

## 0.1.7

- Read bounded run metadata for goal checks and coalesce repeated status refreshes.

## 0.1.6

- Keep inactive goals idle and observe only run status changes

## 0.1.5

- Check paged goal history and advance verified progress atomically

## 0.1.4

- Add marketplace category and icon color
- Add localized tool names
