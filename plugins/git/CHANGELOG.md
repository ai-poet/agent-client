# Changes

## 0.1.36

- Restore outline icons in navigation and tools

## 0.1.35

- Use a new plugin icon

## 0.1.34

- Refresh composer branch choices after reference changes without scanning file status
- Keep pending branch reads from replacing newer choices
- Distinguish local and remote branches with the same name when forking a conversation

## 0.1.33

- Form dialogs submit with a plain Submit button

## 0.1.32

- The branch chip shows only the branch name
- Starting from a new branch opens its name form instead of failing

## 0.1.31

- Choose where a new conversation starts from the branch chip; another branch runs in a new worktree on send and the checkout is never switched
- Copy the branch name or fork from another branch in a running conversation
- Show branch choices in the shared Kit menu with a remote branches submenu; the current branch keeps its check
- Branch management moves from the project menu to the Git panel
- Ask for confirmation before switching the branch of a shared worktree

## 0.1.29

- Render branch choices with native Kit menus and a single hover surface

## 0.1.28

- Select local and remote branches in the composer popup with current-branch checks
- Keep branch navigation inside the popup and use the existing creation forms

## 0.1.27

- Preserve row offsets in history cursors to avoid repeating path comparisons on later pages

## 0.1.26

- Keep the captured path filter in history cursors and reject cross-path continuation

## 0.1.25

- Page history with exact commit cursors and preserve the original head when returning to earlier pages
- Filter history by starting revision and literal path
- Require Sailry 0.1.5 for the updated history contract

## 0.1.24

- List Git repositories inside a plain directory and confirm before initializing around them

## 0.1.23

- Read status counts on the execution Node and share localized change details across controllers
- Keep clean repository counts visible without duplicate Desktop status scans

## 0.1.22

- Refresh stable catalogs only when their settings or repository references change.

## 0.1.21

- Keep captured Git targets and snapshots opaque in model schemas
- Preserve host validation while removing redundant numeric schema bounds

## 0.1.20

- Refresh repository views only for relevant changes and pause hidden controllers
- Reuse collected line statistics for badges and report bounded counts as incomplete

## 0.1.19

- Add an initialization button to the Git sidebar for plain directories

## 0.1.18

- Keep the normal page visible while its initial data loads

## 0.1.17

- Show ordinary repository queries as concise activity while retaining actual differences

## 0.1.16

- Read branches only after confirming the directory contains a repository

## 0.1.15

- Load independent repository, document, history and diff reads concurrently

## 0.1.14

Hide the branch control until repository information is available.

## 0.1.13

- Keep branch controls visible while repository inspection recovers

## 0.1.12

- Keep the session Git sidebar compact until a diff is opened
- Preserve open diffs when reopening the sidebar

## 0.1.11

- Open branch choices immediately and share active reads without blocking unrelated requests

## 0.1.10

- Align package contract tests with the current release metadata

## 0.1.9

- Add marketplace category and icon color
- Add localized tool names
