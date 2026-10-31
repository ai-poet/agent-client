# Documentation library protocol

## Navigation

- [Location and membership](#location-and-membership)
- [Index hierarchy](#index-hierarchy)
- [Document content and granularity](#document-content-and-granularity)
- [Reading rules](#reading-rules)
- [Update rules](#update-rules)
- [Deleting, moving, and merging](#deleting-moving-and-merging)
- [ADRs](#adrs)
- [Evidence and conflicts](#evidence-and-conflicts)
- [Structural checks](#structural-checks)

## Location and membership

Choose the documentation root in this order:

1. A path explicitly specified by the user.
2. The root `index.md` path declared in the project-level skill gate.
3. The repository's single existing documentation root with a clear purpose.
4. `docs/` if none of the above applies.

Place category directories directly under the documentation root. A generic intermediate directory such as `project/` adds a level to every path without conveying useful information. If several roots are plausible and project instructions do not resolve the choice, ask the user during initial setup.

A document belongs to Project Doc when the root or a category `index.md` lists it as a canonical entry. Membership comes from the index. READMEs, legal materials, marketing materials, generated documentation, and temporary plans remain outside the library even when stored in the same directory.

Name library members as follows:

- Use lowercase `snake_case` for Markdown filenames: `[a-z][a-z0-9_]*.md`.
- Use lowercase `snake_case` for directory names: `[a-z][a-z0-9_]*`.
- Name each index `index.md`.
- Name the bootstrap memo `.project_doc_bootstrap.md`. It is the library's only working file whose name starts with a dot.
- Rename adopted legacy files or directories that do not follow these rules, and repair every reference within the repository.
- Keep names of documents outside the library unchanged.

Determine the primary language from existing documentation and project instructions. Preserve the established division by language. Use one primary language per document by default, and create bilingual copies only when requested.

## Index hierarchy

The root `index.md` routes repository tasks to documentation. Reuse it after the first read under the reading rules. Keep it compact and limit it to:

- The library's purpose and current coverage.
- A one-sentence summary and reading conditions for each root-level document.
- A one-sentence summary, applicable scenarios, and `index.md` link for each existing category.
- Incomplete-status notices during setup.

Create categories from this fixed set as needed, with at least one document per category:

- `architecture/`: system components, their relationships, cross-module data flows, and important invariants.
- `domains/`: domain concepts, business rules, state machines, and core terminology.
- `interfaces/`: public APIs, events, data formats, configuration formats, and third-party integrations.
- `operations/`: deployment, migration, recovery, observability, operational security, and maintenance.
- `decisions/`: architectural decision records that preserve reasons, tradeoffs, and consequences.

Each category's `index.md` lists every document in that category. Give each document one canonical entry in a category index. Use ordinary related-document links elsewhere. Maintaining the same entry in multiple indexes causes their descriptions to drift.

Each index entry contains three pieces of information:

```markdown
- [Document title](relative_path.md): One-sentence summary. Read when: tasks, code areas, or concepts that require this document.
```

During setup, entries may use `[planned]`, `[in_progress]`, `[verified]`, or `[blocked]` prefixes. If the target file does not exist yet, show the planned path as code and add a link after creating the file. Remove temporary statuses when setup is complete.

## Document content and granularity

Each current-state document contains:

- One H1 title.
- An opening scope paragraph that names what the document covers and which documents own adjacent topics.
- Facts verified against current code, tests, configuration, or operational constraints.
- Sections organized by topic. Sections referenced by code anchors need stable IDs; add IDs elsewhere as needed.
- Necessary related-document links. Link to facts already recorded in other documents.

Add brief section navigation near the start when a document has more than five major sections or readers often need only part of it. List section names and reading conditions there; keep details in the body.

Extend an existing section when new knowledge shares its topic, audience, and update cycle. Create a new document when all of these conditions hold:

1. The knowledge remains useful after the current task.
2. Understanding it requires multiple files, modules, interfaces, or operational procedures.
3. It has a scope and reading conditions that can be described independently.
4. Adding it to an existing document would mix topics, audiences, or update cycles.

Keep facts evident from a single function, mechanical refactoring, test fixtures, generated code, temporary experiments, implementation plans, and work progress in code, commit history, or plan files.

## Reading rules

Follow [Reading and context reuse in SKILL.md](../SKILL.md#reading-and-context-reuse). Use task keywords, code paths, and "Read when" entries in known indexes to select a category index and then a document. Additional reads cover content that has not been obtained, has been lost, or needs refreshing.

Read enough of a document to complete the task:

- Start with the section targeted by the anchor.
- Read the scope, terminology, and directly dependent sections needed to understand it.
- Expand the reading scope for cross-domain tasks or when indexes cannot route the task.
- If a bootstrap memo exists, read it immediately after the root index and resume from its recorded next step.

## Update rules

Reassess and synchronize relevant documentation when:

- External or cross-module behavior, interfaces, events, or error handling change.
- Domain rules, state transitions, invariants, permissions, or security restrictions change.
- Configuration, data models, migrations, deployment, recovery, or observability procedures change.
- Code or documentation moves and changes paths, symbol locations, ordinary links, or anchors.
- Work reveals a discrepancy between documentation and verified current behavior.

For internal refactoring that preserves documented behavior, keep the prose unchanged and repair references and anchors affected by moves.

## Deleting, moving, and merging

Delete a current-state document or section only when all three conditions hold: its topic has disappeared from the current system, no supported consumers remain, and no other document needs its valid knowledge.

Before deletion:

1. Search the root index, category indexes, other documents, and all `@project-doc` anchors.
2. Decide whether to delete valid content, move it, or merge it into another document.
3. Update indexes, ordinary links, anchors, and related explanations in the same change.
4. Search again for old paths and section IDs to confirm that references have been removed.

For deprecated features that remain supported, retain their status, alternatives, and migration paths. Keep documentation for temporarily disabled features and features whose flags are off by default.

When moving a document, preserve its content and section IDs where possible, and update all paths in the same change. When merging documents, choose a specific destination section for each referenced topic and map old anchors individually. Pointing every anchor at the whole new document loses the original location.

## ADRs

Record consequential design tradeoffs that may be questioned again in `decisions/`. An ADR includes at least its status, context, decision, tradeoffs, consequences, and related documents.

Preserve the historical context of accepted ADRs. Update current-state documents with the implementation. When a new decision replaces an old one:

- Create an ADR that identifies the decision it supersedes.
- Mark the old ADR `superseded` and link to the new one.
- Point current code anchors and current-state documents to the applicable decision or explanation.

## Evidence and conflicts

When code, tests, and documentation disagree, record an unresolved inconsistency. Either side may be wrong. Verify intent using:

- The user's current request and explicit public conventions.
- Executable tests, schemas, configuration validation, and interface definitions.
- Callers, data flows, and actual error handling.
- Current-state documents, ADRs, and available change history.

When evidence establishes intent, fix the incorrect side within the user's authorized scope and synchronize remaining references. Pause the affected changes and ask the user if a consequential ambiguity cannot be resolved.

## Structural checks

Before finishing a write task, use the diff to check affected documents, their index entries, and relevant incoming and outgoing references. Reuse existing check results while the relevant content remains unchanged. Choose checks below according to impact. Reserve root-index rereads, reading all category indexes, and whole-library scans for initial acceptance or an explicitly requested library audit:

- Link targets in the root and category indexes exist.
- Every Project Doc document has exactly one canonical index entry.
- Files, directories, and stable section IDs use lowercase `snake_case`.
- Section IDs are unique within each document.
- Every `@project-doc` path and section ID exists.
- Old paths, old IDs, and broken links have been removed after moves or deletions.
- Temporary statuses and the bootstrap memo have been removed after setup.
- Git diff has no whitespace errors, and tests required by code changes have run.

Use existing repository tools, `rg`, focused reads, and Git diff. After moving or deleting content, search the repository for old paths and IDs to find all references. Add project scripts and CI only when requested.
