---
name: code-review
metadata:
  display-name.en: "Code review"
  display-name.zh-CN: "代码审查"
  display-name.zh-TW: "程式碼審查"
  display-name.ja: "コードレビュー"
  display-name.ko: "코드 검토"
  display-name.fr: "Revue de code"
  display-name.de: "Codeprüfung"
  display-name.es: "Revisión de código"
  display-name.pt-BR: "Revisão de código"
  display-name.ru: "Проверка кода"
  display-name: Code review
description: Review local changes, commits or GitHub pull requests for concrete regressions and scoped repository-rule violations. Use for requested code review, not implementation or a standalone security audit.
---

# Code review

Produce actionable findings grounded in the captured change. A review request does not authorize fixing files, committing, approving, merging or publishing feedback. These tools store private review records; they are unavailable in Plan mode. Answer questions or prepare a review plan there without claiming a completed structured review.

## Freeze the scope

If `delegate_review` supplied the task, reuse its `review_id`, unchanged snapshot and complete assigned groups. Do not create another scope. A receipt-producing child submits `finalize:false`; an evidence-only child reads the supplied captures and returns evidence without write-backed review tools. The following creation steps belong to the review creator.

Call `review_scope` before reading patches. Omit `target` for all staged, unstaged and untracked changes in the current session worktree. Use `target:{kind:"commit",data:{revision:"..."}}` for a commit or `target:{kind:"range",data:{base:"...",head:"..."}}` for an explicit tree comparison. Commit references resolve once into full immutable SHAs. For a pull request, use its exact base and head SHAs after reading its description and existing threads with available GitHub tools.

The scope exhausts metadata inventory, excludes credential, dependency, generated, lock, minified and build paths before patch reads, and records binary or unsupported files with explicit reasons. Tests remain eligible. It freezes strong file revisions, rules and groups. The optional `limit` bounds only the first returned page, not the complete frozen scope. Follow `review_scope({review_id,offset:next,limit:100})` until `next` is null; each page is partial evidence. Files, groups, skipped paths and retained findings share that paging order. Bounded-scope failures require a smaller authorized target, not silently sampling files or declaring success.

Read repository instructions applicable to the eligible paths. Read `rules/index.md` with `read_skill_resource` using `skill:"code-review:code-review"` and `path:"rules/index.md"`. Each scoped file provides the selected rule resource path and SHA-256, selected in index order with case-insensitive patterns and the documented Objective-C `.m` prefix rule. Load the supplied `rule.path` through the same skill resource tool before reviewing that file. Keep all original rule text and license notices intact. Rule resources are checklists, not findings; the exclusions below take precedence over them.

## Review every group

Read every captured diff page with `read_review_change({review_id:scope.review_id,path:file.path,part:"diff",offset:0})`. Follow the exact `next` until null. Use `part:"old"` or `part:"new"` for the complete captured changed-file context on that side; these pages preserve original CR/LF and a missing final newline. Never use live Git or `readFile` to verify captured changed-file content. The snapshot covers the selected diffs and their source context, not the entire repository.

Capture checks inventory and strong revisions before publication. If it fails or is interrupted, the review remains unpublished; invoke `review_scope` afresh in the same session. Published content remains authoritative after selected or unrelated files, HEAD or index change. To review the later state, create a separate scope. Older metadata-only scopes and receipts remain intact, but missing content is unavailable and must not be replaced with a fresh live read.

Do not fetch whole commit patches or inspect excluded credential bodies to establish coverage. Read saved old/new context for changed files. Relevant unchanged callers or tests outside the capture may be read within current permissions, but label these as live evidence rather than part of the immutable snapshot.

Groups always keep source/test or source/header counterparts and locale variants together, and add directory peers only while a group stays within ten files. When delegating, call `delegate_review({review_id,groups:[id],record_receipt:true,role})` for complete disjoint groups. Omit `role` for a new child inheriting this authorized Code or Flagship turn and model; use a named role only when its captured mode and tools fit the assignment. The package reads the authoritative frozen scope and supplies the unchanged snapshot, each file's metadata and strong revision, and selected rules. Do not compose these captures manually or reuse an old child. The creator alone delegates; a complete group that exceeds the task budget requires a smaller authorized review target, not partial coverage.

Select the receipt mode explicitly. `record_receipt:true` rejects Plan turns, read-only roles, tool-disabled role models and unavailable receipt tools before creating a child. It never changes a role or grants tools. A writable child uses `finalize:false`; only the creator finalizes. For a read-only role choose `record_receipt:false` deliberately: the child gathers evidence using the supplied frozen fields without calling write-backed review tools or claiming a receipt. The authorized parent must independently verify every assigned file and submit findings and complete group coverage. Evidence-only output never completes coverage by itself.

Prioritize correctness, security, data loss and scoped instruction violations. Inspect added and modified code. Report a removal only when remaining callers, contracts, data or tests are left broken; intentionally removing a feature or control in line with the change's intent is not a finding. Exclude speculation, unrelated old defects, style preferences, maintainability or naming opinions and generic requests for tests. Combine reports of the same root cause. Agreement and confidence scores do not establish evidence. Run a focused existing check only when it resolves a concrete uncertainty and is authorized; distinguish actual checks from reasoning, and do not modify code or weaken checks to obtain a passing result.

## Record evidence and coverage

Severities:

- `P0`: data loss, a security flaw, or a certain crash or corruption
- `P1`: likely incorrect behavior in normal use
- `P2`: a defect limited to edge cases
- `P3`: a minor but real defect

Before calling `report_finding`, search the diff and surrounding code for anything that directly contradicts the claim, such as an existing guard, caller or test. Drop a contradicted finding. Being unable to prove the issue yourself does not count as contradiction.

Call `report_finding` for each established issue with `review_id`, exact scoped `path` and `revision`, severity `P0`–`P3`, concise title, trigger/consequence explanation and concrete supporting evidence. Include `side:"new"` for added lines or `side:"old"` for deleted lines, a one-based suggested `line`, and a snippet of complete source lines joined by LF (one final LF is allowed). Preserve source whitespace. The snippet must uniquely match the mapped diff on that side and contain a changed line. Context-only, absent or ambiguous snippets fail; a unique match corrects a mistaken suggested line. Use the returned canonical line when citing the finding.

Submit `review_complete` with explicit `reviewed_files:[{path,revision}]`, `reviewed_groups:[id]` and `finalize:false` after fully reviewing complete groups, even if they yielded zero findings. If context compaction, turn or budget limits, or read failures kept you from reading every diff page of every file in a group, do not submit that group; report it as uncovered. Unknown, duplicate, stale or partial group receipts fail. Already accepted groups must not be resubmitted. Failures include bounded remaining coverage; use scope pages for the full inventory. A failed transaction cannot continue writing; after `revision_conflict`, invoke the tool afresh and reread scope before deciding whether to retry. Later repository changes do not invalidate stored evidence or accepted receipts.

The creator uses `finalize:true` only after stored and incoming receipts cover every eligible file and group. If children already submitted all coverage, pass empty receipt arrays. Finalization validates the complete stored diff and context. It does not reread the worktree. Skipped credential paths contain metadata only. Success proves explicit recorded coverage and captured-content validation, not that tests ran or that the project is correct or safe.

## Deliver findings

List retained findings first by impact with narrow canonical file/side/line locations, the triggering condition, consequence and evidence. State the captured scope or full commit SHAs, file and group counts, skips with reasons and checks actually run. Say when no actionable issue was established without claiming exhaustive project correctness. If any limit or read failure was hit, say the review is incomplete, including any zero-findings result, and name the uncovered groups. Follow the user's requested format.

For explicitly requested external publication, recheck the head revision and existing threads before posting through the available GitHub tools. If the outcome is uncertain, inspect the remote result before retrying. A local review receipt alone does not authorize publication.
