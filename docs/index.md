# Waku engineering documentation

> This file is the Project Doc root index. With the `project-doc` skill loaded,
> read this file in full on the session's first task in this repository, then
> follow its routing. Reuse already-read content under the skill's rules;
> reread when content is missing or files have changed. If the session cannot
> invoke the skill, read `.agents/skills/project-doc/SKILL.md` directly and
> follow it — work is not blocked.

## Scope

This library records long-lived engineering knowledge for this fork: fork
maintenance and upstream sync, provider wire protocols, local features, port
references, platform packaging, and render performance. Collaboration rules
live in `../AGENTS.md` (loaded every session); legal/licensing notices in
`../NOTICE.md` and `../LICENSE`; the release process itself in
`../RELEASING.md` (upstream) and `RELEASING.zh.md` (this fork's procedure).

Documents keep their current filenames; a `snake_case` rename pass is deferred
because several filenames are referenced from `AGENTS.md` and cross-document
links. Membership comes from the listings below.

## Architecture — components, rendering, dependencies

- [Streaming render performance](performance.md): the streaming-CPU model (redraw rate × visible elements), frame budgets, and the counter-based measurement playbook. Read when: touching the event pump, pulse clock, veils, overlay scrollbars, pane caching, or anything a streaming frame reaches. `AGENTS.md` § Performance requires reading it first.
- [Local GPUI fork](gpui-fork.md): the vendored `vendor/agent-client-gpui/` workspace — base commit, Waku patches, and how it is updated. Read when: changing GPUI code or the `patch` entries in `Cargo.toml`.
- [Commit-message generation](commit-messages.md): headless one-shot provider-CLI invocation for the commit dialog, per-provider argument vectors, and model choice. Read when: touching `crates/waku-core/src/git_commit.rs` or provider CLI argument handling.
- [Session titles](titles.md): who writes each sidebar title (the provider names its own session wherever it can), the two `AgentSession` fields, and per-provider naming. Read when: touching session naming, `waku-protocol` model fields, or a provider's title path.

## Domains — local features and business rules

- [Fork maintenance](FORK.md): remotes and branches, the weekly upstream merge, hook-point register, files that are ours entirely, conflict triage, routing contract, and the register of local features merged from the server repository's CLAUDE.md. Read when: doing upstream merges, resolving conflicts, or touching any fork-owned path.
- [AgentTeams](agent-teams.md): the built-in-agent team feature — captain, member mailboxes, task DAG, quality gates, cold recovery; translation of dsh-agent-teams (MIT). Read when: touching `crates/waku-agent-bridge/src/team/**`, `session_team.rs`, or the Team panel.
- [Auto-memory](auto-memory.md): Claude Code-style persistent memory for the built-in agent — memory files, the six tools, project-memory mirroring; translation of dsh-auto-memory (MIT). Read when: touching `crates/waku-agent-bridge/src/memory/**` or memory settings.

## Interfaces — external integrations and port references

- [Provider integrations](providers.md): how Waku talks to each coding agent — the process it launches, the wire protocol, session lifetime, and what gets emulated. Read when: adding or changing a provider, its driver, or its CLI arguments.
- [DeepSeek Harness reference](deepseek-harness.md) ([中文](deepseek-harness.zh.md)): how dsh plugins hook the agent loop and UI, mapped to Waku files and functions. Read when: porting a dsh feature or designing an extensible seam. `AGENTS.md` requires it before dsh ports.
- [`@oai/sky` accessibility reconstruction](sky-macos-accessibility-reverse-engineering.md): semantic reconstruction of the macOS window-control path in `@oai/sky` 0.6.2. Read when: implementing or auditing accessibility/window-control behavior compatible with that client.

## Operations — platforms and releases

- [Fork release procedure](RELEASING.zh.md) (Chinese): the actual release steps for this fork — Sparkle keys, `releases.cheaprouter.cc`, `cheaprouter-releases` bucket, `CheapRouter-*` artifacts. Read when: cutting a release. The upstream process description is `../RELEASING.md`.
- [Waku on Linux](linux.md): install paths, desktop entry, requirements. Read when: changing Linux packaging or install.
- [Waku on Windows](windows.md): installer layout, portable mode, updates. Read when: changing Windows packaging or install.

## Outside the library

- `../AGENTS.md`, `../CLAUDE.md` (symlink), `../CONTRIBUTING.md`, `../README.md`, `../RELEASING.md`, `../SECURITY.md`: entry and collaboration material.
- `../CHANGELOG.md`, `../NOTICE.md`: append-only records.
- Reference checkouts at the repo root (`deepseek-harness-master/`, `cc-switch-main/`, `claurst-main/`, `codeg-main/`, `dsh-claude-style-master/`, `orca/`) are git-ignored and hold no documentation of their own.
