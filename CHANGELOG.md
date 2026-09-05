# Changelog

All notable changes to Waku. This file is the **source of truth for the release
notes shown in the in-app updater**: [`scripts/release.ts`](scripts/release.ts)
extracts the section whose heading matches the version being released
(`MARKETING_VERSION`) and publishes it next to the update, so Sparkle shows it in
the update prompt.

Format follows [Keep a Changelog](https://keepachangelog.com). Add a new
`## [<version>]` section at the top for each release, matching the version you
set in the Xcode project.

Write release notes for the final product users receive, not the development
history. When a feature is still unreleased, fold its fixes and refinements into
the original feature bullet instead of adding separate entries for them.

## [unreleased]

## [0.1.19]

- Terminal, Files, Browser and Review buttons now sit in the window header: one click opens the surface, switches to its tab, or hides the panel when it is already in front. Shortcuts ⇧⌘T / E / O / D (Ctrl+Shift on Windows and Linux), plus View-menu and command-palette entries; the old right-panel toggle button is gone (⇧⌘B still toggles the panel)
- Check for updates from Settings → General; when an update is ready a banner appears across the top of the window and installs on click
- Providers settings rebuilt as one card per CLI: install status, "installed but not runnable" diagnostics, Node/npm runtime status, environment-variable conflict warnings, and a cleaner custom endpoint form (labelled fields, hidden keys, URL validation, an endpoint test with its result, confirmation before clearing)
- CLI detection now looks where your shell does (login-shell PATH, nvm/fnm/volta/pnpm/scoop directories) and tells "not installed" from "installed but broken"; installs are verified after npm finishes, with specific hints for permission and network failures
- Onboarding checklist for new installs: sign in, install a CLI, open a project — remembers completion and dismissal
- Edit and resend a sent message when rewind is not available (pencil button and context menu)
- Failed tasks show a failure badge in the sidebar with the error on hover, and a remove button that asks before removing
- Fix being signed out of the cloud account after a while; an expired session now says so and restores the local CLI configs
- Faster first message: the turn snapshot reuses the repository's index, the provider starts in parallel with it, and the CLI is prewarmed while you type
- The Codex model list follows the selected cloud group: switching a group drops Codex's cached manifest and re-probes the CLI right away
- Model catalogs: Claude Fable 5.1 and curated Claude entries the CLI does not report; GPT-6-Astra added to the Codex catalog
- /resume for Codex, Claude Code and the other providers; the sidebar scrolls the selected task into view; live work indicator in the environment summary
- Fix Claude model discovery (#185) and navigation-rail scrolls reaching the transcript
- Fix a CLI probe hanging for the full timeout on Linux and macOS when the CLI's shell script left a child running

## [0.1.18]

- Fix a console window flashing on Windows for every cloud-account request (balance refresh, announcements, group status, payment polling)

## [0.1.17]

- Fix Codex sessions reporting a startup error for the geo-blocked OpenAI docs MCP server
- Sign-in now routes each platform through its first available group automatically; the "account default" option is removed
- Fix turns failing with "turn starting checkpoint … is unavailable" when the starting snapshot ref disappears mid-turn (agent-run git gc, rewind cleanup); the checkpoint now falls back to the previous turn's diff base

## [0.1.16]

- Codex thread goals: type /goal to set a persistent objective the task keeps pursuing — before or after the first message — with its autonomous pursuit streaming into the transcript, a status chip showing live budget or elapsed time, and a dialog to edit, pause, resume, or clear the goal (also in Waku Web)
- Fix Codex sessions failing to start on newer Codex CLI versions ("invalid transport" config error)
- Fix Grok Build and other ACP agents failing to launch on Windows (path not found)
- Discover provider-native slash commands and skills from installed agent CLIs, including multiline YAML descriptions
- Add reasoning effort selection for Grok
- Reconnect remote daemon sessions automatically after connection interruptions
- Fix Command/Ctrl+Enter steering after a provider response starts streaming
- Fix transcript file links on Windows
- Fix OpenCode dropping the first streamed event and hanging during cancellation on Windows

## [0.1.14]

- Group sidebar tasks by project or update date, order them newest or oldest first, and collapse sections
- Find in page: Search the full transcript by keywords using cmd-f or ctrl-f
- Switch between recent tasks with Ctrl+Tab and Ctrl+Shift+Tab
- Carry the current access mode into new tasks and remember it between launches
- Fix OpenCode access-mode permissions and restore pending permission prompts when resuming sessions
- Show Codex file reads, listings, and searches as file activity instead of raw commands
- Keep long panel and background-work titles on one truncated line
- Increase the minimum UI text size for better legibility

## [0.1.13]

- Add Vercel Fx support
- Support DeepSeek Harness 0.1.1 without opening its web UI
- Collapse earlier activity groups when a running turn moves on to newer transcript output

## [0.1.12]

- Invoke Codex, Pi, and Oh My Pi skills with their native syntax
- Stream live output from Claude background tasks
- Steer the oldest queued follow-up with Command/Ctrl+Enter in an empty composer
- Fix model and reasoning option selection for Cursor
- Fix npm-installed provider detection on Windows
- Fix daemon terminal sessions hanging during shutdown
- Exclude copied history from forked Codex sessions from usage totals
- Keep separate Codex reasoning sections on separate lines

## [0.1.11]

- Highlight Markdown in the file editor, and toggle between source and a rendered preview
- Add UI and code font size settings
- macOS: Add "Open in.." button to open project folder in selected application

## [0.1.10]
- Add Kimi Code support
- Add Oh My Pi support
- Fix markdown table rendering

## [0.1.8]

- Fix `PATH` resolution on Windows

## [0.1.4]

- Fix text selection in diff view

## [0.1.3]

- Pin Codex and Claude commit message generation to cheap models: gpt-5.6-luna and claude-4.5-haiku
- Animate sidebars
- Render provider file edits as inline diffs in the transcript
- Fix claude task title generation

## [0.1.2]

- Fix regression: user bubble should fit its content width

## [0.1.1]

- Give nested Markdown the full message width
- Cap composer height and scroll overflow with an overlay scrollbar
- Keep drag-selecting text past the input bounds
- Fix char boundary panic when sliding the live reasoning window

## [0.1.0]

- Add standalone Waku daemon and browser client
- Add Linux support (X11 and Wayland, you need to build from source for now)
- Answer agent questions directly in the composer
- Redesign queued follow-ups as composer cards with per-message steering
- Add DeepSeek agent preset selection (Standard, Code, Minimal, and Creator)
- Add Claude context window and ultracode effort options
- Add /fast command to toggle fast mode for Codex
- Show the latest activity in live transcript headers
- Add soft wrapping and keyboard copy feedback
- Add terminal overlay scrollbar and measure cell width from the font
- Restore window position, size, and display across launches
- Contain wheel scrolling in activity and command output viewports
- Smooth streaming markdown and reduce CPU usage while streaming

## [0.0.13]

- Add DeepSeek Harness provider
- Render user message as Markdown and linkify bare URLs
- Share one resident OpenCode serve per workspace across sessions

## [0.0.12]

- Inherit the login-shell environment for provider commands
- Fix model traits across provider switches
- Keep branch change counts current and include untracked files
- Normalize SIGCHLD for provider children
- Fix Grok model discovery

## [0.0.11]

- Fix provider detection for CLIs installed through shell PATH managers such as
  nvm and fnm
- Show models registered by Pi extensions
- Fix the model picker closing when entering a space in search
- Fix duplicate transcript history and lost interaction mode when resuming ACP
  sessions

## [0.0.10]

- Fix crash in due to IME composition
- Fix typo

## [0.0.9]

- Add OpenCode Go support in usage popover
- Fix app icon
- Fix Cursor model detection

## [0.0.8]

- Initial release
