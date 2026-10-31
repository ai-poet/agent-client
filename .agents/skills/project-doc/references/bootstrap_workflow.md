# Initial setup and recovery

## Navigation

- [Prerequisites](#prerequisites)
- [Inventory the repository](#inventory-the-repository)
- [Design the complete index](#design-the-complete-index)
- [Create the bootstrap memo](#create-the-bootstrap-memo)
- [Install the skill gate](#install-the-skill-gate)
- [Write documents in batches](#write-documents-in-batches)
- [Resume incomplete setup](#resume-incomplete-setup)
- [Completion criteria](#completion-criteria)

## Prerequisites

Use this workflow when the user explicitly asks to create, integrate, restore, or rebuild a Project Doc library. If an ordinary coding task reveals that no library exists, report that fact and let the user decide whether to create one.

Before starting:

1. Identify the repository root and read existing project instructions.
2. Confirm that the user authorizes creating documentation, renaming adopted legacy documents, repairing references, and updating agent instruction entry points.
3. Find the documentation root specified by the user. Otherwise, choose an existing root or `docs/` under the library protocol.
4. Read [library_protocol.md](library_protocol.md) and [anchor_protocol.md](anchor_protocol.md).
5. Use [index.md](../assets/index.md) and [project_doc_bootstrap.md](../assets/project_doc_bootstrap.md) as structural templates. Rewrite them in the repository's primary language and replace every placeholder.

## Inventory the repository

Build an evidence inventory before writing prose. Inspect at least:

- Root and nested agent instructions, including their scope and precedence.
- Package management, build, deployment, and workspace manifests.
- Application entry points, modules, public interfaces, configuration, schemas, migrations, and tests.
- Generated directories, third-party code, build outputs, and temporary files that do not need documentation.
- READMEs, existing documentation directories, source-adjacent documents, and repository links to them.
- Existing ADRs, design notes, operational manuals, and potentially outdated documents.

Choose a disposition for each existing document:

- `adopt`: the content and scope are mostly correct; index it and add any required structure.
- `update`: keep the document and correct it against current code.
- `merge`: move valid knowledge into a more suitable existing or planned document.
- `split`: separate topics, audiences, or update cycles mixed in one file.
- `exclude`: keep material outside the library when it is not durable engineering knowledge.

Rename adopted files or directories that do not use lowercase `snake_case`. List every repository reference first, then move files in batches and repair links so each caller changes with its target path.

## Design the complete index

Decide the whole library structure before writing document bodies:

1. List root-level documents and categories actually needed.
2. For each category, list planned documents, scopes, supporting code sources, and reading conditions.
3. Decide which topics extend existing documents, need new documents, or belong in ADRs.
4. Plan code entry points and stable IDs for sections that need code anchors.
5. Create skeletons for the root `index.md` and every planned category's `index.md`.

The root index routes to categories and root-level documents. Category indexes list every document in their category. Show paths as code until planned files exist, then add Markdown links.

Show the user a summary of the directory structure. If domain divisions, the documentation root, legacy-document disposition, or extensive renaming present a consequential ambiguity, pause and ask the user. Otherwise, record assumptions and continue.

## Create the bootstrap memo

Create `<doc_root>/.project_doc_bootstrap.md` from the template. Before writing the first document bodies, fill in:

- Documentation root, repository primary language, and last update time.
- Agreed structural decisions and naming constraints.
- Each planned document's scope, evidence sources, legacy disposition, and anchor targets.
- Integration status of agent instruction entry points.
- Current blockers, next action, and handoff notes. Make the next action concrete.
- Checks already run and their results.

Use one of four document statuses:

- `planned`: scope and sources are defined; writing has not started.
- `in_progress`: verification or writing is underway. At most one item may have this status at a time.
- `verified`: prose, indexes, anchors, and links have been checked.
- `blocked`: a specific blocker exists, with the information needed to resume recorded in the memo.

Update the memo immediately after each completed document, legacy rename, or gate update. Also update it before an intentional handoff or context compaction. The memo may remain in the repository and be committed while setup is incomplete. Delete it after all completion criteria are met.

## Install the skill gate

After creating the root-index skeleton and memo, install the project-level gate. Check these root entry points:

- `AGENTS.md`
- `CLAUDE.md`
- `GEMINI.md`
- `.github/copilot-instructions.md`

Update every existing entry point. If none exists, create only the root `AGENTS.md`. Modify a nested entry point only when the user explicitly limits the library to that subtree.

The gate block starts with a unique H2 heading, `## Project Doc gate`, and ends at the next H2 heading or end of file. Use these headings to delimit the block; extra HTML comment markers duplicate them.

```markdown
## Project Doc gate

- Before any repository task, use the `project-doc` skill. If the current session cannot invoke it, do not stop: read `<skill_dir>/SKILL.md` directly and follow it.
- On the session's first task in this repository, read all of `<doc_root>/index.md`, then follow its routing for the task.
- Reuse skill instructions, indexes, and sections already read in the same session while enough of their content remains in context. This also applies when editing files, continuing subtasks, and finishing work. If content is missing, relevant files change, or routing conflicts arise, reread the affected material under the skill's "Reading and context reuse" rules.
- When a change affects cross-module behavior, domain rules, public interfaces, configuration, migrations, or upstream-merge rules, update the affected `<doc_root>/` documents and indexes in the same change.
```

When translating the gate into the repository's primary language, preserve the meaning of its requirements. Replace `<doc_root>` and `<skill_dir>` with the actual paths.

Make gate updates idempotent:

- If no gate heading exists, append a complete gate section and preserve existing content.
- If one gate heading exists, update that section in place.
- If several identical gate headings exist, merge them and report the prior duplication.
- A session that cannot invoke the skill follows it by reading the file; the gate never blocks work.
- Agents that can load skills discover it from the repository's skill directory; this setup workflow writes the gate.

If existing project instructions directly conflict with the gate, record the evidence and ask the user. Change the conflicting rules after the user resolves them.

## Write documents in batches

Follow dependencies from broad topics to narrow ones. The usual order is:

1. Project scope, core terminology, and overall structure.
2. Architecture, component relationships, and cross-module flows.
3. Domain rules, state machines, and invariants.
4. Interfaces, configuration, events, and data formats.
5. Deployment, migration, recovery, observability, and operational security.
6. ADRs that preserve reasons and tradeoffs.

Handle one defined document per batch:

1. Mark the item `in_progress` and record its next step.
2. Read its listed evidence sources and necessary callers and tests.
3. Write the current-state content. Verify behavior from code; treat filenames and old documentation as leads.
4. Add stable IDs to sections that need code anchors.
5. Add `@project-doc` anchors selectively at handwritten entry points.
6. Update the category index and relevant ordinary links.
7. Check prose, links, anchors, and naming, then mark the item `verified`.
8. Update the memo immediately before starting the next item.

Verify each document when it is written and mark it complete after verification passes. Placeholder documents remain `planned` or `in_progress`.

## Resume incomplete setup

When `.project_doc_bootstrap.md` exists:

1. Read the memo immediately after the root `index.md`.
2. Check it against the current file tree and Git diff to confirm that it still applies.
3. Inspect remaining `in_progress` items and decide whether they are complete, need continued work, or should return to `planned`.
4. Resume from the recorded next step and preserve verified areas.
5. If new findings change the overall structure, update structural decisions and affected index entries.

Resume from the memo and current repository state. Use conversation memory as supporting context.

## Completion criteria

Initial setup is complete when all of these hold:

- Every planned item is `verified` or `blocked` with an explanation.
- The root and category indexes route accurately to all documents, with planned statuses and broken links removed.
- All adopted files, directories, and section IDs follow naming rules.
- Every `@project-doc` anchor targets an existing, specific section.
- Old paths and links have been removed after legacy renames.
- Each agent instruction entry point has one valid gate block.
- Document content has been checked against current code, tests, and configuration.

After the final audit, delete `.project_doc_bootstrap.md` and confirm that the root index has no links to it. If work remains, keep the memo and list the remaining work in the response.
