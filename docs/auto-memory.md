# Auto-memory

Auto-memory gives the built-in agent Claude Code-style persistent memory: facts worth keeping across sessions — who the user is, how they want the work done, what a project is in the middle of, where things live — saved as one Markdown file each. An index of them goes into every request, and the agent reads, writes and forgets them with six tools. Project memories are also copied into Claude Code's own project memory, so the Claude Code CLI in the same folder sees them.

It is a translation of dsh-auto-memory (0.3.0, MIT, Copyright (c) 2026 AskTheWay; licence in `NOTICE.md`). The memory-writing policy (`auto_memory::prompt::MEMORY_POLICY`) and the consolidation prompt are the plugin's, verbatim; everything else is a Rust port with the differences listed below.

## Scope

- **Built-in agent only** (`ProviderKind::Native`). Claude Code keeps its own memory; the mirror below is how the two meet.
- **Root session only.** Sub-agents and team members build their tool sets from `engine_tools` and never see the memory tools, and the index section is appended after the turn's sub-agent query is cloned.
- **No wire protocol change.** The desktop reaches the files directly through the engine's config directory (`sub2api::global_config::native::config_dir()`), the same directory the bridge finds through `Settings::global_settings_path()`.

## Where things live

```
<engine config dir>/
  auto-memory.json            settings (never settings.json: the engine's save drops unknown keys)
  auto-memory/
    user/                     global memories, shared by every workspace
      <name>.md
      MEMORY.md               derived index
    projects/<key>/           one directory per workspace
      <name>.md
      MEMORY.md
      project.json            {"cwd": "<workspace>"}, for the settings page's labels
```

Not `<config dir>/memory/`: the vendored engine's AutoDream claims that directory.

A memory file is YAML frontmatter plus a Markdown body:

```
---
name: use-pnpm
title: "Uses pnpm"
description: "The repo uses pnpm, not npm"
type: project
pinned: true
created: 1791331200000
updated: 1791331200000
lastRead: 1791417600000
reads: 3
---

Why and how to apply it…
```

- `name` is kebab-case `[a-z0-9-]`, at most 64 characters, and must equal the file stem; a file whose stem differs is ignored. `memory` and the Windows device names (`con`, `nul`, `com1`…) are reserved.
- `type` is `user`, `feedback`, `project` or `reference`. A missing `type` falls back to `metadata.type` (Claude Code's layout), then to `reference`.
- `title` is optional; the index shows it, or the name without one.
- The lifecycle fields are milliseconds since the epoch. Files without them never go stale and are never pruned.

The frontmatter parser is a hand-written YAML subset (BOM, CRLF, plain and quoted scalars, integers, booleans, comments, continuation lines, one level of nested map). Malformed files, symlinks and the index are skipped when listing.

**Project key.** The workspace path with `/`, `\` and `:` runs folded to `-`, every other character outside `[A-Za-z0-9._-]` escaped as `~XXXX` (UTF-16 units), wrapped as `--slug--`. On Windows the drive letter is lowercased, trailing separators are dropped, and a slug over 96 bytes is cut and suffixed with `-<first 8 hex of its sha256>` so the path stays under `MAX_PATH`.

## The index and the prompt

Every write, delete and clear rebuilds the scope's `MEMORY.md` under the scope lock: one line per memory, `- [title](name.md)[ 📌] — description`, pinned first, then by name. Stale memories are left out.

On its first turn the root session reads both indexes and appends one section to its system prompt; every later turn of the session sends the same section:

```
# Persistent memory index

## User memories
…
## Project memories
…

<MEMORY_POLICY>
```

- No memories in either scope: no section at all.
- The budget is `maxBytes` (2 / 4 / 8 KB) for the whole section, policy included. The indexes get `maxBytes − policy − 96` bytes (`prompt::index_budget`) and are cut at a whole line, with a truncation marker. Pinned memories come first, so they survive a cut.
- Read once per session (`MemoryHost::extend_rules`). The section sits in the system prompt, ahead of the whole conversation, so it must not change mid-session: re-reading it each turn let every memory the agent or a consolidation pass wrote change the prompt, which missed the prompt cache from the first token — and on the gateway, whose sticky routing for GPT hashes the system prompt, could move the session to another upstream account. A memory written mid-session is already in the conversation as the tool call; the next session reads the new index.

## Tools

| Tool | Permission | What it does |
|---|---|---|
| `memory_read` | read-only | One memory by name, with up to 3 `[[name]]` links expanded one level (name + description). Counts the read (`reads`, `lastRead`), which revives a stale memory. |
| `memory_list` | read-only | Every memory in the reachable scopes. |
| `memory_write` | none | Create or update by name. Without `scope`, updates the scope that already has the name, else writes to the project. Keeps `created`, the read counters and the pin unless `pinned` is given. |
| `memory_delete` | none | One memory by name. |
| `memory_prune` | asks a person | Memories not updated for `olderThanDays` (≥ 1); pinned and unstamped memories are never candidates. `dryRun` only lists them. |
| `memory_delete_all` | asks a person | Every memory in a scope; needs `confirm: true`. |

Writes and single deletes do not ask and work in plan mode too: memory files are not workspace changes. The two bulk deletes call `check_permission_with_details` with the list of what would go, and `GuiPermissionHandler::decide` turns that into a two-button dialog (allow once / reject) **whatever the access mode** — full access and standing "always allow" answers do not apply, so the model can never approve a bulk delete itself. A team member asking is refused.

The transcript names each call in the user's language (`crates/waku-core/src/driver/memory_titles.rs`); without that, `memory_write`'s `title` argument would become the row title.

## Consolidation

With `autoSummarize` on (the default), the session keeps a short record of the conversation: what the person wrote (up to 2,000 characters a message) and the agent's visible answers (up to 500), at most 40 lines / 24,000 characters. A pass runs:

- after every `autoSummarizeEveryTurns` (default 12) messages the person sent, when no pass is already running;
- when the session is torn down (closed, or reclaimed after 30 idle minutes), if anything was said since the last pass.

A pass asks the session's own model and route once (`oneshot::ask_once`): one turn, no tools, no extra rules, no thinking or effort, `max_tokens` from `autoSummarizeMaxTokens` (default 4,096), 120-second timeout. The answer is parsed as candidates; each is sanitized, names that already exist are skipped, and at most `autoSummarizeMaxMemories` (default 5) are written through the store — so they are indexed and mirrored like any other write. Any failure is one `tracing::warn` and nothing else. The pass's tokens are not counted in the session's usage ring.

## Claude Code mirror

With `mirrorToClaudeCode` on (the default), every project memory is also written, one way, into Claude Code's project memory:

- **Where:** `$CLAUDE_CONFIG_DIR` (else `~/.claude`) `/projects/<slug>/memory/`, where the slug is the workspace path with every character outside `[A-Za-z0-9]` replaced by `-` (`C:\Projects\sub2api\client` → `C--Projects-sub2api-client`). Slugs over 200 characters are not mirrored.
- **Format:** Claude Code's — `name`, `description`, and a `metadata` map with `node_type: memory`, `type`, `origin: cheaprouter` and an ISO `modified`; the body as is. The mirror's `MEMORY.md` is updated line by line: the line pointing at `<name>.md` is replaced or appended (`- [title](name.md) — description`), and every other line, Claude Code's own included, is left alone.
- **Never touches Claude Code's own memories.** A file of the same name without `origin: cheaprouter` is skipped with a warning, never overwritten. Deletes, clears and prunes remove only marked files and their index lines.
- **Best effort.** A mirror failure is a warning; the primary store and the tool result are unaffected. Claude Code does not know our lock, so mirror writes are atomic but unlocked.
- **Not mirrored:** user-scope memories (Claude Code has no global auto-memory directory). Nothing is imported back from Claude Code.
- Switching the mirror back on in the settings copies every project memory that has none yet (`MemoryStore::sync_mirror`).

## Locking

Each scope directory has an in-process mutex plus a lock file, `MEMORY.md.lock`, created with `create_new` and holding `pid timestamp token`. A writer retries every 20 ms for up to 12 s. A lock older than 10 s, or one stamped with this process's pid, is treated as orphaned and taken over. Only the holder whose token is still in the file removes it. A timeout reaches the model as "memory store is busy … try again in a moment".

## Settings → Agent → Memory

`src/app/agent_memory_settings.rs`. Every read and write runs on the background executor; a generation counter drops stale scans.

- **Switches:** memory on/off, global memories, automatic consolidation, sync to Claude Code.
- **Presets:** index budget 2 / 4 / 8 KB; hide stale memories off / 30 / 90 / 180 days. Changing the stale threshold rebuilds every index at once.
- **Memories:** global first, then each workspace by its real path. Each group shows its index size against the budget, how many are hidden as stale, and a Clear button (asks first). A row shows title, type, pin and stale marks, description, name and read count. Opening it shows the body as plain text with Edit (title, summary, type, body), Pin / Unpin and Delete (asks first).
- Edits, pins and deletes go through the same `MemoryStore` as the tools, so the index is rebuilt and the mirror follows.

Running sessions keep the settings they started with.

## Differences from dsh-auto-memory

- A file's stem must equal its `name`. Windows device names are reserved.
- The project key gets a digest suffix when long, and Windows paths are normalized.
- No 1,024-byte floor on the index budget: the whole section, policy included, stays within the setting, so the 2 KB preset holds.
- Consolidation output is capped at 4,096 tokens by default.
- Consolidation is **on** by default.
- Prune never offers pinned memories.
- The bulk-delete approval bypasses every access mode.
- The section is given to the root session only.
- The Claude Code mirror is new.
- The `{{` neutralization is not ported: our prompt path has no template interpolation.
- The settings page shows a memory's body as plain text, not rendered Markdown.

## Known gaps

- The vendored engine still carries `SessionMemoryExtractor` (writes `<cwd>/.claurst/AGENTS.md`, and only runs with `ANTHROPIC_API_KEY` in the environment) and AutoDream (`<config dir>/memory/`). Neither is wired into Waku; they are unrelated to this feature.
- With a remote daemon, the settings page lists this machine's memories, not the remote's.
- Consolidation tokens are not counted in the usage ring.
- The mirror is one way and skips the user scope.

## Files

- `crates/auto-memory/` — the pure part: types, names and project keys, frontmatter, the store and its lock, the Claude Code mirror, scan, links, prompt section, consolidation prompt and parsing, settings. Tests: `cargo test -p auto-memory`.
- `crates/waku-agent-bridge/src/memory/{mod,tools,consolidate}.rs` — `MemoryHost`, the six tools and the consolidation pass; hooks in `session.rs`, `session_team.rs`, `permission.rs` and `oneshot.rs`.
- `crates/waku-core/src/driver/memory_titles.rs` — transcript and dialog titles, mounted from `native.rs`.
- `src/app/agent_memory_settings.rs` — the settings section; hooks in `agent_page.rs`, `usage_page.rs` and `app.rs`.
- `locales/{app,zh-CN,ja}.yml` — `memory.tool.*`, `memory.approval.*`, `memory.settings.*`.
