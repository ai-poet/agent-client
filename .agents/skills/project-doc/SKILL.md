---
name: project-doc
description: Build, read, maintain, and audit a layered engineering documentation library kept in sync with code for people and coding agents. Use when creating or restoring the library, organizing or migrating existing documentation, updating docs with code changes, checking documentation against code, following a repository's Project Doc workflow, reading docs/index.md, or encountering an @project-doc code anchor.
---

# Project Doc

Maintain the documentation library alongside the code. Record durable engineering knowledge that requires reading multiple parts of the codebase to understand. Keep indexes, prose, code anchors, and the current implementation consistent.

## Starting a task

1. Determine whether the request is read-only or authorizes file changes. For read-only tasks, report discrepancies between documentation and code; leave repairs for an authorized task.
2. Identify the repository root and follow its existing agent instructions.
3. Find the documentation root and root `index.md` declared by project instructions. If none is declared, prefer an existing documentation root with a clear purpose, then `docs/`.
4. If the library exists, read or reuse the root `index.md` under "Reading and context reuse" below before analyzing business code. Follow its "Read when" guidance to the relevant category's `index.md`.
5. When encountering `@project-doc`, read or reuse the target section under the same rules before interpreting, reviewing, or changing the code containing the anchor.
6. If `<doc_root>/.project_doc_bootstrap.md` exists, read it and resume from its recorded next step. Preserve completed work.

If project instructions declare a Project Doc gate but both the root index and bootstrap memo are missing, stop repository work and report the incomplete setup. Creating the whole library for the first time requires an explicit user request.

## Reading and context reuse

- Read the entire root `index.md` on the session's first task in a repository. Reuse that content for later tasks, subtasks, file edits, and delivery. Reread only under the conditions in the next two rules. Apply the same reuse rule to skill instructions, reference protocols, category indexes, and document bodies.
- Reuse requires enough content in the current context or compacted summary: routing categories, paths, reading conditions, and constraints for the current operation. Remembering only that a file was read, or retaining only its name, counts as missing content. Reread missing indexes or sections; reread the entire root index if its routing has been lost. Apply these conditions after context compaction.
- Refresh affected content when a relevant file is known to have changed externally, the repository or documentation root changes, routing conflicts with actual files, or the user requests a refresh. Use changes made in this turn directly while they remain in context. Determine whether files changed from these known events; checking timestamps or hashes before every operation wastes reads.
- When the task expands into an unread area, follow the known routing to that category and the necessary sections. Continue reusing the root and other categories already read.
- Reuse reduces repeated reading. Continue assessing documentation impact, maintaining consistency, and performing necessary verification. Keep reuse state in conversation context. Repository caches, read-file lists, and "no impact" statements create extra maintenance work.

## Choosing a workflow

- For initial creation or recovery, read [references/bootstrap_workflow.md](references/bootstrap_workflow.md), [references/library_protocol.md](references/library_protocol.md), and [references/anchor_protocol.md](references/anchor_protocol.md), then use the templates in `assets/`.
- For maintenance alongside code, read [references/library_protocol.md](references/library_protocol.md). When encountering, adding, moving, or deleting anchors, also read [references/anchor_protocol.md](references/anchor_protocol.md).
- For a consistency audit, read the library and anchor protocols, then check indexes, prose, code, and references. Fix issues when authorized; otherwise report them.
- For answers based on documentation, read or reuse the root index, matching category indexes, and necessary sections.

## Maintaining consistency

1. Identify potentially affected documents from the task's code paths, symbols, configuration, tests, and anchors.
2. Read or reuse the target sections and the adjacent material needed to understand them under "Reading and context reuse."
3. Determine the conventions the code should follow from the implementation, callers, tests, configuration, history, and user goals.
4. Use [references/library_protocol.md](references/library_protocol.md) to decide whether to add a section, create a document, update, move, merge, delete, or leave content unchanged.
5. When changes are authorized, maintain code, document bodies, indexes at every level, anchors, and ordinary links in the same change.
6. Use existing repository tools, `rg`, focused reads, and Git diff to check the affected documentation structure and references. Limit final checks to the task's scope. Reserve index rereads and whole-library audits for dedicated audit tasks. Run the tests required by code changes as usual.
7. Report which documents changed. If no documents are affected, state in the response that documentation impact was assessed.

## Invariants

- Follow "Reading and context reuse" for the initial full root-index read and subsequent reuse. Read category indexes and prose as needed.
- The root index routes readers to categories. Each category index lists every document in that category and when to read it.
- Code anchors target sections with stable IDs written as `<a id="..."></a>`. Heading-generated slugs change with wording and can break anchors.
- Ordinary documents describe the current state. Record important historical decisions as ADRs, and preserve superseded decisions with status `superseded`.
- Document knowledge that requires looking beyond local code. Keep local implementation details, mechanical refactoring, generated code, temporary plans, and facts apparent from a single symbol in code or plan files.
- When documentation and code disagree, verify intent before deciding which to change. Ask the user if a consequential ambiguity remains unresolved.
- Report issues in read-only tasks and wait for authorization to repair them. During documentation maintenance, limit behavior changes to the user's authorized scope.
- Create project-specific validation scripts, CI, and documentation-impact declarations only when explicitly requested.

## Integration requirements

During initial creation, install the project-level gate described in [references/bootstrap_workflow.md](references/bootstrap_workflow.md). It requires agents to use `project-doc` before repository work. When the current session cannot invoke the skill, agents read this `SKILL.md` directly and follow it; they do not stop work. (Local adaptation for this repository, which is worked on with several agent tools that do not all load skills.)

When building or auditing the library, check for a missing or outdated gate. Repair it in write tasks and report it in read-only tasks.
