# Changes

## 0.1.21

- Restore outline icons in navigation and tools

## 0.1.20

- Use a new plugin icon

## 0.1.19

- Remove duplicate workspace CLI entries and routine refresh from project context menus while preserving recovery actions

## 0.1.18

- Reject empty command wait text so output waits do not return immediately.

## 0.1.17

- Wait for command output, matching text or exit with read_command, up to 30 seconds
- Clarify background execution and require Sailry 0.1.5 for command waits
- Keep terminal splits associated with the requested terminal creation

## 0.1.16

- Ask agents to set follow_up when they wait for a background command

## 0.1.15

- Make conversation follow-up for background commands opt in
- Close terminals when their shell exits, collapsing split panes; report only non-zero exits

## 0.1.14

- Close terminals when their shell exits, collapsing split panes; report only non-zero exits

## 0.1.13

- Keep background command starts as activities without empty output expansion

## 0.1.12

- Refresh stable catalogs only when their settings or repository references change.

## 0.1.11

- Keep command access guidance in one parameter description
- Declare a 600000 ms timeout maximum without changing execution validation

## 0.1.10

- Coalesce terminal refreshes and preserve loaded content after background failures
- Page retained command streams and original tool results without starting a process

## 0.1.9

- Require Sailry 0.1.0-alpha.4 for terminal lifecycle support

## 0.1.8

- Release terminal input before closing and retain uncertain close requests

## 0.1.7

- Keep switching terminals free of transient stopped text and spinners

## 0.1.6

- Keep terminal panes loading until their terminal metadata arrives

## 0.1.5

- Add marketplace category and icon color
- Add localized tool names
- Add localized skill names
