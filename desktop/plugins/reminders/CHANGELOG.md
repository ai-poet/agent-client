# Changes

## 0.1.20

- Compare original reminder content across hosts before using the execution Node's local revision
- Preserve edits and reject concurrent content changes without recreating schedules
- Keep pending notifications valid across acknowledgement-only revision changes

## 0.1.19

- Retain the original request when a nested operation has an uncertain outcome.

## 0.1.18

- Read remote records and edit revisions from their execution Node.
- Keep unavailable copies visible without allowing stale edits.

## 0.1.17

- Require the first Sailry version with execution-Node callback routing

## 0.1.16

- Keep edits, deletion and delivery history on each reminder's original execution Node
- Select an execution host for new reminders from the controller resource directory
- Preserve unknown legacy ownership without creating replacement schedules

## 0.1.15

- Keep the card border on rows that are not open in the details panel

## 0.1.14

- Preserve the originating conversation scope when scheduling reminders
- Register notification handlers before their schedules in the same transaction

## 0.1.13

- Allow conversations without a project to manage unassigned reminders
- Preserve project isolation when listing or changing reminders

## 0.1.12

- Restore outline icons in navigation and tools

## 0.1.11

- Use a new plugin icon

## 0.1.10

- Keep scheduling concurrency checks aligned with team execution

## 0.1.9

- Remove redundant numeric schema bounds while preserving host validation

## 0.1.8

- Increase spacing between form labels and controls

## 0.1.7

- Read reminders, delivery records and project choices concurrently

## 0.1.6

- Reduce status tags in lists and details by one size

## 0.1.5

- Add marketplace category and icon color
- Add localized tool names
