// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Ported from `scripts/quality-gates-tdd.mjs`, `quality-gates-amend.test.mjs`,
//! `quality-gates-repair-scope.test.mjs`, plus the pure cases of
//! `issue-159.test.mjs` and `stability-tdd.mjs`. Test names follow the
//! reference labels.

use regex::Regex;
use serde_json::{Map, Value, json};

use super::*;
use crate::types::{TeamMember, TeamPhase};
use crate::validate::{has_valid_quality_task_fields, normalize_blank_optional_task_fields};

const NOW: u64 = 1_700_000_000_000;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn some(items: &[&str]) -> Option<Vec<String>> {
    Some(strings(items))
}

/// `task(partial)`: in progress, attempt 1, attempt id `attempt-1`.
fn task(id: &str, edit: impl FnOnce(&mut TeamTask)) -> TeamTask {
    let mut task = TeamTask::new(id, "Task", NOW);
    task.status = TaskStatus::InProgress;
    task.attempt = Some(1);
    task.attempt_id = Some("attempt-1".to_owned());
    edit(&mut task);
    task
}

fn member(name: &str, role: &str) -> TeamMember {
    let mut member = TeamMember::new(name, NOW);
    member.id = format!("member-{name}");
    member.role = Some(role.to_owned());
    member
}

/// `team(partial)` of the TDD checklist.
fn team(tasks: Vec<TeamTask>) -> TeamState {
    let mut team = TeamState::new("Quality", "quality", "captain-session", NOW);
    team.description = Some("quality-gates".to_owned());
    team.members = vec![
        member("implementer", "implementer"),
        member("reviewer", "correctness-reviewer"),
    ];
    team.task_seq = tasks.len() as u64;
    team.tasks = tasks;
    team.review_policy = Some(ReviewPolicy {
        requirements_min_rounds: Some(1),
        requirements_max_rounds: Some(4),
        code_max_rounds: Some(3),
        max_repair_attempts: Some(2),
        required_reviewers: some(&["correctness", "security", "scope"]),
    });
    team
}

fn impl_contract(task: &mut TeamTask) {
    task.kind = Some(TaskKind::Implementation);
    task.objective = Some("Ship the parser".to_owned());
    task.in_scope = some(&["src/parser.ts"]);
    task.out_of_scope = some(&["docs/"]);
    task.acceptance = some(&["parser accepts empty input"]);
    task.verify = some(&["pnpm test"]);
}

fn review_contract(task: &mut TeamTask) {
    task.kind = Some(TaskKind::Review);
    task.objective = Some("Review the implementation".to_owned());
    task.acceptance = some(&["no blocker or high findings"]);
    task.reviewed_task_id = Some("t1".to_owned());
}

fn impl_input(subject: &str) -> CreateTaskInput {
    CreateTaskInput {
        subject: subject.to_owned(),
        kind: Some(TaskKind::Implementation),
        objective: Some("Ship the parser".to_owned()),
        in_scope: some(&["src/parser.ts"]),
        out_of_scope: some(&["docs/"]),
        acceptance: some(&["parser accepts empty input"]),
        verify: some(&["pnpm test"]),
        ..Default::default()
    }
}

fn finding(id: &str, severity: FindingSeverity, problem: &str, fix: &str) -> ReviewFinding {
    ReviewFinding {
        id: id.to_owned(),
        severity,
        file: None,
        line: None,
        problem: problem.to_owned(),
        required_fix: fix.to_owned(),
        resolved: None,
    }
}

fn with_file(mut finding: ReviewFinding, file: &str) -> ReviewFinding {
    finding.file = Some(file.to_owned());
    finding
}

fn accepted(criterion: &str) -> AcceptanceResult {
    AcceptanceResult {
        criterion: criterion.to_owned(),
        status: CheckStatus::Passed,
        evidence: None,
    }
}

fn ran(command: &str, status: CheckStatus) -> CommandResult {
    CommandResult {
        command: command.to_owned(),
        status,
        exit_code: None,
        evidence: None,
    }
}

fn error_of(result: Result<(), CompletionRejection>) -> String {
    result.expect_err("the gate must reject").error
}

// ---------------------------------------------------------------------------
// A. create contract
// ---------------------------------------------------------------------------

#[test]
fn tdd_create_work_kind_remains_compatible() {
    let created = validate_create_task(
        &team(Vec::new()),
        &CreateTaskInput {
            subject: "legacy work".to_owned(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(created.kind, TaskKind::Work);
    assert_eq!(created.task.kind, Some(TaskKind::Work));
    assert_eq!(created.task.subject, "legacy work");
    assert!(!created.clears_halt);
}

#[test]
fn tdd_normalize_blank_optional_fields_to_omitted() {
    let mut record = json!({
        "kind": "repair",
        "subject": "repair with blanks",
        "objective": "",
        "reviewedTaskId": "",
        "sourceTaskId": "t1",
        "sourceFindingIds": [""],
        "inScope": ["", "src/repair.ts"],
        "acceptance": ["fixed"],
    });
    normalize_blank_optional_task_fields(record.as_object_mut().unwrap());
    assert!(record.get("reviewedTaskId").is_none());
    assert!(record.get("objective").is_none());
    assert!(record.get("sourceFindingIds").is_none());
    assert_eq!(record["inScope"], json!(["src/repair.ts"]));
    assert_eq!(record["sourceTaskId"], json!("t1"));
    assert_eq!(record["acceptance"], json!(["fixed"]));
}

#[test]
fn tdd_normalize_non_blank_values_untouched() {
    let mut record = json!({"reviewedTaskId": "t1", "inScope": ["src/b.ts"], "subject": "s"});
    normalize_blank_optional_task_fields(record.as_object_mut().unwrap());
    assert_eq!(record["reviewedTaskId"], json!("t1"));
    assert_eq!(record["inScope"], json!(["src/b.ts"]));
}

#[test]
fn tdd_normalize_retains_invalid_list_item() {
    for invalid in [json!(123), Value::Null, json!({"path": "src/private/"})] {
        let mut record = serde_json::to_value(task("t1", |_| {})).unwrap();
        record["acceptance"] = json!(["", invalid.clone(), "real criterion"]);
        let original = record.clone();
        let mut normalized = record.clone();
        normalize_blank_optional_task_fields(normalized.as_object_mut().unwrap());
        let acceptance = normalized["acceptance"].as_array().unwrap();
        assert_eq!(acceptance.len(), 2, "{invalid}");
        assert_eq!(acceptance[0], invalid);
        assert!(serde_json::from_value::<TeamTask>(normalized).is_err());
        assert_eq!(original["acceptance"].as_array().unwrap().len(), 3);
    }
}

#[test]
fn tdd_create_implementation_requires_objective() {
    let input = CreateTaskInput {
        subject: "impl".to_owned(),
        kind: Some(TaskKind::Implementation),
        acceptance: some(&["done"]),
        in_scope: some(&["src/a.ts"]),
        verify: some(&["pnpm test"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "implementation tasks require a non-empty objective"
    );
}

#[test]
fn tdd_create_implementation_requires_acceptance() {
    let input = CreateTaskInput {
        subject: "impl".to_owned(),
        kind: Some(TaskKind::Implementation),
        objective: Some("Ship it".to_owned()),
        in_scope: some(&["src/a.ts"]),
        verify: some(&["pnpm test"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "implementation tasks require at least one acceptance criterion"
    );
}

#[test]
fn tdd_create_implementation_requires_inscope_and_verify() {
    let mut input = CreateTaskInput {
        subject: "impl".to_owned(),
        kind: Some(TaskKind::Implementation),
        objective: Some("Ship it".to_owned()),
        acceptance: some(&["done"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "implementation tasks require a non-empty inScope"
    );
    input.in_scope = some(&["src/a.ts"]);
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "implementation tasks require a non-empty verify list"
    );
}

#[test]
fn tdd_create_review_requires_reviewed_task() {
    let mut input = CreateTaskInput {
        subject: "review".to_owned(),
        kind: Some(TaskKind::Review),
        objective: Some("Review it".to_owned()),
        acceptance: some(&["pass"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "review tasks require reviewedTaskId"
    );
    input.reviewed_task_id = Some("t9".to_owned());
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "reviewed task \"t9\" does not exist"
    );
}

/// The pure half of `tdd.create.review-blank-reviewedTaskId-rejected.tool`.
#[test]
fn tdd_create_review_blank_reviewed_task_id_rejected() {
    let input = CreateTaskInput {
        subject: "review blank id".to_owned(),
        kind: Some(TaskKind::Review),
        objective: Some("Review it".to_owned()),
        acceptance: some(&["pass"]),
        reviewed_task_id: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "review tasks require reviewedTaskId"
    );
}

#[test]
fn tdd_create_repair_requires_source_and_findings() {
    let input = CreateTaskInput {
        subject: "repair".to_owned(),
        kind: Some(TaskKind::Repair),
        objective: Some("Fix findings".to_owned()),
        acceptance: some(&["fixed"]),
        in_scope: some(&["src/a.ts"]),
        verify: some(&["pnpm test"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "repair tasks require sourceTaskId and at least one sourceFindingId"
    );
}

#[test]
fn tdd_create_repair_must_not_depend_on_failed_review() {
    let current = team(vec![
        task("t1", |t| {
            impl_contract(t);
            t.status = TaskStatus::Completed;
        }),
        task("t2", |t| {
            t.kind = Some(TaskKind::Review);
            t.status = TaskStatus::Failed;
            t.verdict = Some(ReviewVerdict::NeedsRevision);
            t.reviewed_task_id = Some("t1".to_owned());
        }),
    ]);
    let input = CreateTaskInput {
        subject: "repair".to_owned(),
        kind: Some(TaskKind::Repair),
        objective: Some("Fix findings".to_owned()),
        acceptance: some(&["fixed SEC-001"]),
        in_scope: some(&["src/parser.ts"]),
        verify: some(&["pnpm test"]),
        source_task_id: Some("t1".to_owned()),
        source_finding_ids: some(&["SEC-001"]),
        dependencies: some(&["t2"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&current, &input).unwrap_err(),
        "repair must not depend on failed task \"t2\""
    );
}

#[test]
fn tdd_create_overlapping_inscope_rejects_parallel_ready_tasks() {
    let current = team(vec![task("t1", |t| {
        impl_contract(t);
        t.status = TaskStatus::Pending;
        t.in_scope = some(&["src/parser.ts", "src/util.ts"]);
    })]);
    let mut input = impl_input("other impl");
    input.in_scope = some(&["src/util.ts"]);
    let error = validate_create_task(&current, &input).unwrap_err();
    assert!(error.contains("t1"));
    assert_eq!(
        error,
        "inScope overlaps t1 at src/util.ts; serialize these tasks or split the paths"
    );
}

#[test]
fn tdd_create_overlapping_inscope_allowed_when_serialized() {
    let current = team(vec![task("t1", |t| {
        impl_contract(t);
        t.status = TaskStatus::Pending;
    })]);
    let mut input = impl_input("serialized impl");
    input.dependencies = some(&["t1"]);
    assert!(validate_create_task(&current, &input).is_ok());
}

#[test]
fn tdd_create_implementation_blocked_until_requirements_pass() {
    let current = team(vec![task("t1", |t| {
        t.kind = Some(TaskKind::Requirements);
        t.objective = Some("Converge requirements".to_owned());
        t.acceptance = some(&["no open questions"]);
    })]);
    assert_eq!(
        validate_create_task(&current, &impl_input("impl")).unwrap_err(),
        "implementation must depend on a requirements task until requirements completes with verdict=pass"
    );
}

fn pending_requirements() -> TeamTask {
    task("t1", |t| {
        t.kind = Some(TaskKind::Requirements);
        t.status = TaskStatus::Pending;
        t.objective = Some("Converge requirements".to_owned());
        t.acceptance = some(&["no open questions"]);
    })
}

#[test]
fn tdd_create_staged_implementation_can_follow_pending_requirements() {
    let mut staged = team(vec![pending_requirements()]);
    staged.phase = Some(TeamPhase::Staged);
    let mut input = impl_input("planned implementation");
    input.dependencies = some(&["t1"]);
    assert!(validate_create_task(&staged, &input).is_ok());
}

#[test]
fn tdd_create_running_implementation_can_follow_pending_requirements() {
    let mut running = team(vec![pending_requirements()]);
    running.phase = Some(TeamPhase::Running);
    let mut input = impl_input("planned automatic implementation");
    input.dependencies = some(&["t1"]);
    assert!(validate_create_task(&running, &input).is_ok());
}

#[test]
fn tdd_create_staged_implementation_must_depend_on_requirements() {
    let mut staged = team(vec![pending_requirements()]);
    staged.phase = Some(TeamPhase::Staged);
    assert!(validate_create_task(&staged, &impl_input("unsequenced implementation")).is_err());
}

#[test]
fn create_reports_unknown_dependency_and_kind() {
    let mut input = CreateTaskInput {
        subject: "w".to_owned(),
        dependencies: some(&["t7"]),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&team(Vec::new()), &input).unwrap_err(),
        "dependency \"t7\" does not exist"
    );
    input.dependencies = None;
    assert!(validate_create_task(&team(Vec::new()), &input).is_ok());
    assert_eq!(
        parse_task_kind("ship-it").unwrap_err(),
        "unknown task kind \"ship-it\""
    );
    assert_eq!(parse_task_kind("repair"), Ok(TaskKind::Repair));
}

#[test]
fn create_input_reads_camel_case_tool_arguments() {
    let input: CreateTaskInput = serde_json::from_value(json!({
        "subject": "s",
        "kind": "review",
        "reviewedTaskId": "t1",
        "inScope": ["src/"],
        "resumeReason": "go",
        "resume": true,
        "round": 2
    }))
    .unwrap();
    assert_eq!(input.kind, Some(TaskKind::Review));
    assert_eq!(input.reviewed_task_id.as_deref(), Some("t1"));
    assert_eq!(input.in_scope, some(&["src/"]));
    assert_eq!(input.resume_reason.as_deref(), Some("go"));
    assert_eq!(input.round, Some(2));
}

#[test]
fn create_input_from_task_revalidates_an_edited_task() {
    let mut current = team(vec![task("t1", |t| {
        impl_contract(t);
        t.status = TaskStatus::Pending;
    })]);
    let edited = task("t2", |t| {
        impl_contract(t);
        t.status = TaskStatus::Pending;
    });
    assert!(validate_create_task(&current, &CreateTaskInput::from(&edited)).is_err());
    current.tasks.clear();
    assert!(validate_create_task(&current, &CreateTaskInput::from(&edited)).is_ok());
}

// ---------------------------------------------------------------------------
// B. completion gates
// ---------------------------------------------------------------------------

fn review_task() -> TeamTask {
    task("t1", review_contract)
}

fn impl_task() -> TeamTask {
    task("t1", impl_contract)
}

#[test]
fn tdd_complete_review_without_verdict_rejected() {
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        output: Some("looks good".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&review_task(), &update)),
        "review cannot complete without verdict=pass"
    );
}

#[test]
fn tdd_complete_review_needs_revision_cannot_complete() {
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        verdict: Some(ReviewVerdict::NeedsRevision),
        findings: Some(vec![finding(
            "C-001",
            FindingSeverity::Medium,
            "bug",
            "fix it",
        )]),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&review_task(), &update)),
        "review with verdict=needs_revision cannot complete"
    );
}

#[test]
fn tdd_complete_review_pass_requires_no_open_high_findings() {
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        verdict: Some(ReviewVerdict::Pass),
        findings: Some(vec![finding(
            "C-001",
            FindingSeverity::High,
            "bug",
            "fix it",
        )]),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&review_task(), &update)),
        "review pass cannot leave unresolved high/blocker findings"
    );
    let mut resolved = finding("C-001", FindingSeverity::High, "bug", "fix it");
    resolved.resolved = Some(true);
    let update = QualityCompletionUpdate {
        findings: Some(vec![resolved]),
        ..update
    };
    assert!(evaluate_quality_completion(&review_task(), &update).is_ok());
}

#[test]
fn review_needs_revision_failure_requires_a_finding() {
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Failed),
        verdict: Some(ReviewVerdict::NeedsRevision),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&review_task(), &update)),
        "review needs_revision requires at least one finding"
    );
}

#[test]
fn tdd_complete_implementation_requires_acceptance_results() {
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        changed_paths: some(&["src/parser.ts"]),
        commands_run: Some(vec![ran("pnpm test", CheckStatus::Passed)]),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&impl_task(), &update)),
        "implementation completion requires passed acceptanceResults for every acceptance item"
    );
}

#[test]
fn tdd_complete_implementation_requires_all_verify_commands() {
    let current = task("t1", |t| {
        impl_contract(t);
        t.verify = some(&["pnpm test", "pnpm lint"]);
    });
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        changed_paths: some(&["src/parser.ts"]),
        acceptance_results: Some(vec![accepted("parser accepts empty input")]),
        commands_run: Some(vec![ran("pnpm test", CheckStatus::Passed)]),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&current, &update)),
        "implementation completion requires a passed commandsRun entry for every verify command"
    );
}

fn complete_with_paths(paths: &[&str]) -> QualityCompletionUpdate {
    QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        changed_paths: some(paths),
        acceptance_results: Some(vec![accepted("parser accepts empty input")]),
        commands_run: Some(vec![ran("pnpm test", CheckStatus::Passed)]),
        ..Default::default()
    }
}

#[test]
fn tdd_complete_out_of_scope_path_cannot_complete() {
    assert_eq!(
        error_of(evaluate_quality_completion(
            &impl_task(),
            &complete_with_paths(&["docs/secret.md"])
        )),
        "implementation cannot complete: docs/secret.md is out_of_scope"
    );
}

#[test]
fn tdd_complete_undeclared_path_cannot_complete() {
    assert_eq!(
        error_of(evaluate_quality_completion(
            &impl_task(),
            &complete_with_paths(&["src/unlisted.ts"])
        )),
        "implementation cannot complete: src/unlisted.ts is undeclared"
    );
}

#[test]
fn implementation_completion_requires_changed_paths() {
    let update = QualityCompletionUpdate {
        changed_paths: None,
        ..complete_with_paths(&[])
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&impl_task(), &update)),
        "implementation completion requires changedPaths"
    );
    assert!(
        evaluate_quality_completion(&impl_task(), &complete_with_paths(&["src/parser.ts"])).is_ok()
    );
}

#[test]
fn tdd_complete_verify_failure_must_fail_task() {
    let mut failed = ran("pnpm test", CheckStatus::Failed);
    failed.exit_code = Some(1);
    let update = QualityCompletionUpdate {
        commands_run: Some(vec![failed]),
        ..complete_with_paths(&["src/parser.ts"])
    };
    let rejection = evaluate_quality_completion(&impl_task(), &update).unwrap_err();
    assert_eq!(rejection.error, "verify failure must fail the task");
    assert_eq!(rejection.required_status, Some(TaskStatus::Failed));
    let failing = QualityCompletionUpdate {
        status: Some(TaskStatus::Failed),
        ..update
    };
    assert!(evaluate_quality_completion(&impl_task(), &failing).is_ok());
}

#[test]
fn tdd_complete_claimed_still_cannot_jump_to_completed() {
    let current = task("t1", |t| {
        t.kind = Some(TaskKind::Work);
        t.status = TaskStatus::Claimed;
    });
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        output: Some("done".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        error_of(evaluate_quality_completion(&current, &update)),
        "task status cannot move from \"claimed\" to \"completed\""
    );
}

#[test]
fn tdd_complete_work_kind_keeps_legacy_output_only_complete() {
    let current = task("t1", |t| t.kind = Some(TaskKind::Work));
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        output: Some("free-text result is enough".to_owned()),
        ..Default::default()
    };
    assert!(evaluate_quality_completion(&current, &update).is_ok());
}

#[test]
fn tdd_complete_ordered_evidence_tolerates_model_paraphrase() {
    let current = task("t1", |t| {
        impl_contract(t);
        t.acceptance = some(&["文档确实不含“回滚/rollback”相关章节或说明（故意遗漏）"]);
        t.verify = some(&["! grep -qiE \"回滚|rollback\" file.md"]);
    });
    let update = QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        changed_paths: some(&["src/parser.ts"]),
        acceptance_results: Some(vec![accepted("文档确实不含回滚或 rollback 说明")]),
        commands_run: Some(vec![ran("grep reverse check", CheckStatus::Passed)]),
        ..Default::default()
    };
    assert!(evaluate_quality_completion(&current, &update).is_ok());
}

#[test]
fn acceptance_by_name_uses_the_last_result_for_a_criterion() {
    let current = task("t1", |t| {
        impl_contract(t);
        t.acceptance = some(&["a", "b"]);
    });
    let mut failed_a = accepted("a");
    failed_a.status = CheckStatus::Failed;
    let update = QualityCompletionUpdate {
        acceptance_results: Some(vec![accepted("a"), failed_a, accepted("b")]),
        ..complete_with_paths(&["src/parser.ts"])
    };
    assert!(evaluate_quality_completion(&current, &update).is_err());
}

// ---------------------------------------------------------------------------
// C. path rules
// ---------------------------------------------------------------------------

#[test]
fn tdd_scope_file_match() {
    assert!(path_matches_scope("src/foo.ts", "src/foo.ts"));
    assert_eq!(
        classify_changed_path("src/foo.ts", &["src/foo.ts"], &[]),
        PathClassification::InScope
    );
}

#[test]
fn tdd_scope_directory_prefix_match() {
    assert!(path_matches_scope("src/foo/bar.ts", "src/foo/"));
    assert_eq!(
        classify_changed_path("src/foo/bar.ts", &["src/foo/"], &[]),
        PathClassification::InScope
    );
    assert!(!path_matches_scope("src/foobar.ts", "src/foo/"));
    assert!(!path_matches_scope("src/foo/bar.ts", "src/foo"));
}

#[test]
fn tdd_scope_out_of_scope_wins() {
    assert_eq!(
        classify_changed_path("src/foo/secret.ts", &["src/foo/"], &["src/foo/secret.ts"]),
        PathClassification::OutOfScope
    );
}

#[test]
fn tdd_scope_rejects_parent_escape() {
    assert_eq!(
        classify_changed_path("../outside.ts", &["src/"], &[]),
        PathClassification::Illegal
    );
    assert!(!path_matches_scope("../outside.ts", "src/"));
}

#[test]
fn tdd_scope_rejects_absolute_path() {
    assert_eq!(
        classify_changed_path("/etc/passwd", &["src/"], &[]),
        PathClassification::Illegal
    );
    assert_eq!(
        classify_changed_path("C:\\work\\a.ts", &["./"], &[]),
        PathClassification::Illegal
    );
    assert_eq!(
        classify_changed_path("~/a.ts", &["./"], &[]),
        PathClassification::Illegal
    );
}

#[test]
fn tdd_scope_default_excludes_env_and_git() {
    assert_eq!(
        classify_changed_path(".env", &["./"], &[]),
        PathClassification::OutOfScope
    );
    assert_eq!(
        classify_changed_path(".git/config", &["./"], &[]),
        PathClassification::OutOfScope
    );
    assert_eq!(
        classify_changed_path("pkg/.env.local", &["pkg/"], &[]),
        PathClassification::OutOfScope
    );
    assert_eq!(
        classify_changed_path("a/secrets/key.txt", &["./"], &[]),
        PathClassification::OutOfScope
    );
    assert_eq!(
        classify_changed_path("home/id_rsa.pub", &["./"], &[]),
        PathClassification::OutOfScope
    );
}

#[test]
fn normalize_workspace_path_folds_separators_and_dots() {
    assert_eq!(
        normalize_workspace_path(" .\\src\\.\\a.ts "),
        Some("src/a.ts".to_owned())
    );
    assert_eq!(normalize_workspace_path("./"), Some(String::new()));
    assert_eq!(
        normalize_workspace_path("src//b/"),
        Some("src/b".to_owned())
    );
    assert_eq!(normalize_workspace_path("  "), None);
    assert_eq!(normalize_workspace_path("a/../b"), None);
    assert_eq!(normalize_workspace_path("F:\\team"), None);
    assert!(path_matches_scope("anything/at/all.ts", "."));
    assert!(path_matches_scope("anything.ts", "./"));
    assert!(!path_matches_scope("src/a.ts", ""));
    assert_eq!(PathClassification::OutOfScope.as_str(), "out_of_scope");
    assert_eq!(
        serde_json::to_value(PathClassification::Undeclared).unwrap(),
        json!("undeclared")
    );
}

#[test]
fn collect_changed_paths_reads_git_porcelain() {
    let status = " M src/a.ts\r\n?? \"docs/new file.md\"\nR  old.ts -> src/renamed.ts\n\nA  src/a.ts\nM  ../escape.ts\n!! /abs/path\nplain/list.txt\n";
    assert_eq!(
        collect_changed_paths(status),
        strings(&[
            "src/a.ts",
            "docs/new file.md",
            "src/renamed.ts",
            "plain/list.txt"
        ])
    );
    assert!(collect_changed_paths("").is_empty());
}

#[test]
fn in_scope_overlap_reports_left_entries() {
    let left = strings(&["src/", "docs/a.md", "x.ts"]);
    let right = strings(&["src/util.ts", "docs/"]);
    assert_eq!(
        in_scope_overlap(Some(&left), Some(&right)),
        strings(&["src/", "docs/a.md"])
    );
    assert!(in_scope_overlap(None, Some(&right)).is_empty());
    assert!(in_scope_overlap(Some(&left), None).is_empty());
}

// ---------------------------------------------------------------------------
// D. auto loop
// ---------------------------------------------------------------------------

fn needs_revision_review(edit: impl FnOnce(&mut TeamTask)) -> TeamTask {
    task("t2", |t| {
        t.assignee = Some("reviewer".to_owned());
        t.status = TaskStatus::Failed;
        t.verdict = Some(ReviewVerdict::NeedsRevision);
        t.round = Some(1);
        t.reviewed_attempt = Some(1);
        edit(t);
        review_contract(t);
    })
}

fn completed_source(edit: impl FnOnce(&mut TeamTask)) -> TeamTask {
    task("t1", |t| {
        t.assignee = Some("implementer".to_owned());
        t.status = TaskStatus::Completed;
        edit(t);
        impl_contract(t);
    })
}

fn first_loop() -> PlanQualityFollowUpResult {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.findings = Some(vec![with_file(
            finding(
                "C-001",
                FindingSeverity::High,
                "missing null check",
                "guard empty input",
            ),
            "src/parser.ts",
        )]);
    });
    plan_quality_follow_up(&team(vec![source, review.clone()]), &review)
}

fn created_of(planned: &PlanQualityFollowUpResult, kind: TaskKind) -> Option<&PlannedFollowUpTask> {
    planned.created.iter().find(|item| item.kind == kind)
}

#[test]
fn tdd_loop_needs_revision_creates_repair_and_next_review() {
    let planned = first_loop();
    assert!(created_of(&planned, TaskKind::Repair).is_some());
    assert!(created_of(&planned, TaskKind::Review).is_some());
    assert!(!planned.escalated);
}

#[test]
fn tdd_loop_repair_depends_on_source_not_failed_review() {
    let planned = first_loop();
    let repair = created_of(&planned, TaskKind::Repair).unwrap();
    let dependencies = repair.dependencies.as_ref().unwrap();
    assert!(dependencies.contains(&"t1".to_owned()));
    assert!(!dependencies.contains(&"t2".to_owned()));
}

#[test]
fn tdd_loop_next_review_assigned_to_original_reviewer() {
    let planned = first_loop();
    let review = created_of(&planned, TaskKind::Review).unwrap();
    assert_eq!(review.assignee.as_deref(), Some("reviewer"));
}

#[test]
fn tdd_loop_next_review_cannot_be_implementer() {
    let planned = first_loop();
    let review = created_of(&planned, TaskKind::Review).unwrap();
    assert_ne!(review.assignee.as_deref(), Some("implementer"));
    assert_eq!(
        created_of(&planned, TaskKind::Repair)
            .unwrap()
            .assignee
            .as_deref(),
        Some("implementer")
    );
}

#[test]
fn tdd_loop_round_increments() {
    let planned = first_loop();
    assert_eq!(
        created_of(&planned, TaskKind::Repair).unwrap().round,
        Some(2)
    );
    assert_eq!(
        created_of(&planned, TaskKind::Review).unwrap().round,
        Some(2)
    );
    let repair = created_of(&planned, TaskKind::Repair).unwrap();
    assert_eq!(repair.id.as_deref(), Some("repair-round-2"));
    assert_eq!(repair.subject.as_deref(), Some("repair-round-2"));
    let review = created_of(&planned, TaskKind::Review).unwrap();
    assert_eq!(review.subject.as_deref(), Some("review-round-2"));
    assert_eq!(review.reviewed_task_id.as_deref(), Some("repair-round-2"));
    assert_eq!(review.dependencies, some(&["repair-round-2"]));
}

#[test]
fn tdd_loop_next_review_contract_is_not_a_gate_test() {
    let planned = first_loop();
    let review = created_of(&planned, TaskKind::Review).unwrap();
    let gate = Regex::new("(?i)needs_revision").unwrap();
    assert!(!gate.is_match(review.objective.as_deref().unwrap_or("")));
    assert!(
        review
            .acceptance
            .as_ref()
            .unwrap()
            .iter()
            .all(|item| !gate.is_match(item))
    );
}

#[test]
fn tdd_loop_stops_at_max_review_rounds() {
    let source = completed_source(|t| t.round = Some(3));
    let review = needs_revision_review(|t| {
        t.round = Some(3);
        t.findings = Some(vec![finding(
            "C-001",
            FindingSeverity::High,
            "still broken",
            "fix",
        )]);
    });
    let mut current = team(vec![source, review.clone()]);
    current.review_policy = Some(ReviewPolicy {
        code_max_rounds: Some(3),
        ..Default::default()
    });
    let planned = plan_quality_follow_up(&current, &review);
    assert!(planned.created.is_empty());
    assert!(planned.escalated);
}

#[test]
fn tdd_loop_reject_does_not_autoresume() {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.verdict = Some(ReviewVerdict::Reject);
        t.round = None;
        t.findings = Some(vec![finding(
            "C-001",
            FindingSeverity::Blocker,
            "wrong approach",
            "redesign",
        )]);
    });
    let planned = plan_quality_follow_up(&team(vec![source, review.clone()]), &review);
    assert!(planned.created.is_empty());
    assert!(planned.escalated);
}

#[test]
fn tdd_loop_requirements_needs_revision_opens_next_requirements_round() {
    let requirements = task("t1", |t| {
        t.kind = Some(TaskKind::Requirements);
        t.assignee = Some("reviewer".to_owned());
        t.status = TaskStatus::Failed;
        t.verdict = Some(ReviewVerdict::NeedsRevision);
        t.round = Some(1);
        t.objective = Some("Converge requirements".to_owned());
        t.acceptance = some(&["no open questions"]);
        t.findings = Some(vec![finding(
            "R-001",
            FindingSeverity::High,
            "scope unclear",
            "close open questions",
        )]);
    });
    let planned = plan_quality_follow_up(&team(vec![requirements.clone()]), &requirements);
    assert!(
        planned
            .created
            .iter()
            .any(|item| item.kind == TaskKind::Requirements && item.round == Some(2))
    );
    assert!(created_of(&planned, TaskKind::Repair).is_none());
    let next = &planned.created[0];
    assert_eq!(next.subject.as_deref(), Some("requirements-round-2"));
    assert_eq!(next.assignee.as_deref(), Some("reviewer"));
    assert_eq!(next.objective.as_deref(), Some("Converge requirements"));
    assert_eq!(next.acceptance, some(&["close open questions"]));
}

#[test]
fn tdd_loop_repair_skips_captain_assignee() {
    let source = completed_source(|t| t.assignee = Some("captain".to_owned()));
    let review = needs_revision_review(|t| {
        t.findings = Some(vec![with_file(
            finding("C-001", FindingSeverity::High, "bug", "fix"),
            "src/parser.ts",
        )]);
    });
    let planned = plan_quality_follow_up(&team(vec![source, review.clone()]), &review);
    let repair = created_of(&planned, TaskKind::Repair).unwrap();
    assert_eq!(repair.assignee.as_deref(), Some("implementer"));
}

#[test]
fn tdd_loop_same_failed_review_does_not_duplicate_follow_up() {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.findings = Some(vec![finding("C-001", FindingSeverity::High, "bug", "fix")]);
    });
    let existing_repair = task("t3", |t| {
        t.status = TaskStatus::Pending;
        t.assignee = Some("implementer".to_owned());
        t.source_task_id = Some("t1".to_owned());
        t.source_finding_ids = some(&["C-001"]);
        t.dependencies = strings(&["t1"]);
        t.round = Some(2);
        impl_contract(t);
        t.kind = Some(TaskKind::Repair);
        t.acceptance = some(&["fix"]);
    });
    let existing_review = task("t4", |t| {
        t.status = TaskStatus::Pending;
        t.assignee = Some("reviewer".to_owned());
        t.dependencies = strings(&["t3"]);
        t.round = Some(2);
        review_contract(t);
        t.reviewed_task_id = Some("t3".to_owned());
    });
    let current = team(vec![
        source,
        review.clone(),
        existing_repair,
        existing_review,
    ]);
    let planned = plan_quality_follow_up(&current, &review);
    assert!(planned.created.is_empty());
    assert!(!planned.escalated);
}

#[test]
fn repair_attempt_ceiling_escalates() {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.findings = Some(vec![finding("C-001", FindingSeverity::High, "bug", "fix")]);
    });
    let spent = |id: &str| {
        task(id, |t| {
            impl_contract(t);
            t.kind = Some(TaskKind::Repair);
            t.status = TaskStatus::Completed;
            t.source_task_id = Some("t1".to_owned());
            t.source_finding_ids = some(&["C-001"]);
        })
    };
    let current = team(vec![source, review.clone(), spent("t3"), spent("t4")]);
    let planned = plan_quality_follow_up(&current, &review);
    assert!(planned.created.is_empty());
    assert!(planned.escalated);
}

#[test]
fn tdd_plan_default_quality_graph_order() {
    let graph = default_quality_delivery_graph(&QualityGraphInput {
        goal: "ship the parser".to_owned(),
        implementer: Some("implementer".to_owned()),
        reviewer: Some("reviewer".to_owned()),
        analyst: Some("analyst".to_owned()),
        ..Default::default()
    });
    let order: Vec<&str> = graph.iter().map(|item| item.kind.as_str()).collect();
    assert_eq!(
        order.join(">"),
        "requirements>implementation>verification>review>integration"
    );
    let gate = Regex::new("(?i)needs_revision").unwrap();
    assert!(graph[3].acceptance.iter().all(|item| !gate.is_match(item)));
    assert_eq!(graph[2].assignee.as_deref(), Some("implementer"));
    assert_eq!(graph[4].assignee.as_deref(), Some("reviewer"));
    assert_eq!(
        graph[0].objective,
        "Converge requirements for: ship the parser"
    );
    let blank = default_quality_delivery_graph(&QualityGraphInput::default());
    assert_eq!(blank[0].coverage_of, some(&["the stated user goal"]));
}

#[test]
fn tdd_plan_prompt_explains_staged_full_dag_and_review_rewire() {
    let prompt = quality_planning_prompt();
    assert!(
        Regex::new("(?i)entire DAG while.*staged")
            .unwrap()
            .is_match(&prompt)
    );
    assert!(
        Regex::new("(?i)automatically rewires.*downstream")
            .unwrap()
            .is_match(&prompt)
    );
}

#[test]
fn tdd_loop_pending_downstream_rewired_to_new_review_gate() {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.dependencies = strings(&["t1"]);
        t.findings = Some(vec![with_file(
            finding(
                "DOC-001",
                FindingSeverity::Medium,
                "missing rollback",
                "add rollback",
            ),
            "src/parser.ts",
        )]);
    });
    let integration = task("t3", |t| {
        t.kind = Some(TaskKind::Integration);
        t.status = TaskStatus::Pending;
        t.dependencies = strings(&["t2"]);
    });
    let mut current = team(vec![source, review.clone(), integration]);
    let follow_up = apply_quality_follow_up(&mut current, &review);
    let kinds: Vec<&str> = follow_up
        .created
        .iter()
        .map(|item| item.kind_or_work().as_str())
        .collect();
    assert_eq!(kinds.join(">"), "repair>review");
    let replacement = follow_up.created.last().unwrap();
    assert_eq!(replacement.kind, Some(TaskKind::Review));
    assert_eq!(
        current.task("t3").unwrap().dependencies,
        vec![replacement.id.clone()]
    );
    // ids, draft-reference resolution and attempt 0, as applyQualityFollowUp does
    assert_eq!(current.task_seq, 5);
    let repair = &follow_up.created[0];
    assert_eq!(repair.id, "t4");
    assert_eq!(repair.attempt, Some(0));
    assert_eq!(repair.dependencies, strings(&["t1"]));
    assert_eq!(replacement.id, "t5");
    assert_eq!(replacement.dependencies, strings(&["t4"]));
    assert_eq!(replacement.reviewed_task_id.as_deref(), Some("t4"));
    assert_eq!(replacement.subject, "review-round-2");
    assert_eq!(current.tasks.len(), 5);
    assert!(!follow_up.escalated);
    assert_eq!(current.escalated, None);
}

#[test]
fn apply_follow_up_records_escalation() {
    let source = completed_source(|_| {});
    let review = needs_revision_review(|t| {
        t.verdict = Some(ReviewVerdict::Reject);
        t.findings = Some(vec![finding("C-001", FindingSeverity::Blocker, "p", "f")]);
    });
    let mut current = team(vec![source, review.clone()]);
    let applied = apply_quality_follow_up(&mut current, &review);
    assert!(applied.escalated && applied.created.is_empty());
    assert_eq!(current.escalated, Some(true));
    assert_eq!(current.task_seq, 2);
}

#[test]
fn tdd_plan_review_contract_strips_gate_tests() {
    let cleaned = sanitize_review_acceptance(Some(&strings(&[
        "reviewer 可领取并提交 needs_revision 触发拒绝验证",
        "The latest implementation meets the user goal",
    ])));
    let objective = sanitize_review_objective(
        Some("验证 review needs_revision 不能 completed"),
        DEFAULT_REVIEW_OBJECTIVE,
    );
    let gate = Regex::new("(?i)needs_revision").unwrap();
    assert!(cleaned.iter().all(|item| !gate.is_match(item)));
    assert_eq!(
        cleaned,
        strings(&["The latest implementation meets the user goal"])
    );
    assert!(!gate.is_match(&objective));
    assert_eq!(objective, DEFAULT_REVIEW_OBJECTIVE);
    assert_eq!(
        sanitize_review_acceptance(None),
        strings(&DEFAULT_REVIEW_ACCEPTANCE)
    );
    assert_eq!(
        sanitize_review_objective(Some("  Judge the parser  "), DEFAULT_REVIEW_OBJECTIVE),
        "Judge the parser"
    );
    assert!(looks_like_gate_test_contract(Some(
        "Verdict = NEEDS_REVISION"
    )));
    assert!(looks_like_gate_test_contract(Some("it CANNOT COMPLETE")));
    assert!(!looks_like_gate_test_contract(None));
}

#[test]
fn tdd_status_escalated_is_not_halted() {
    let mut escalated = team(vec![
        task("t1", |t| {
            t.kind = Some(TaskKind::Implementation);
            t.status = TaskStatus::Completed;
        }),
        task("t2", |t| {
            t.kind = Some(TaskKind::Review);
            t.status = TaskStatus::Failed;
            t.verdict = Some(ReviewVerdict::NeedsRevision);
        }),
    ]);
    escalated.escalated = Some(true);
    let snapshot = describe_quality_loop(&escalated);
    assert_eq!(snapshot.state, QualityLoopState::Escalated);
    assert!(!snapshot.halted);
    assert!(
        Regex::new("(?i)not halt|不是 halt|ceiling")
            .unwrap()
            .is_match(&snapshot.summary)
    );

    let mut halted = team(Vec::new());
    halted.halted = Some(true);
    halted.halted_at = Some(NOW);
    let snapshot = describe_quality_loop(&halted);
    assert_eq!(snapshot.state, QualityLoopState::Halted);
    assert!(snapshot.halted);
}

#[test]
fn describe_quality_loop_running_blocked_and_deliverable() {
    let running = team(vec![task("t1", |t| t.status = TaskStatus::Pending)]);
    let snapshot = describe_quality_loop(&running);
    assert_eq!(snapshot.state, QualityLoopState::Running);
    assert_eq!(
        snapshot.summary,
        "Work remains on the shared task list; wait for the scheduler or complete owned tasks."
    );

    let blocked = team(vec![task("t1", |t| t.status = TaskStatus::Failed)]);
    let snapshot = describe_quality_loop(&blocked);
    assert_eq!(snapshot.state, QualityLoopState::Blocked);
    assert_eq!(
        snapshot.summary,
        "Delivery is blocked: t1 (work) is not completed."
    );

    let done = team(vec![task("t1", |t| t.status = TaskStatus::Completed)]);
    let snapshot = describe_quality_loop(&done);
    assert_eq!(snapshot.state, QualityLoopState::Deliverable);
    assert!(snapshot.deliverable);
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(value["state"], json!("deliverable"));
    assert_eq!(value["deliverable"], json!(true));
}

// ---------------------------------------------------------------------------
// E. halt / resume
// ---------------------------------------------------------------------------

fn halted_team(tasks: Vec<TeamTask>) -> TeamState {
    let mut halted = team(tasks);
    halted.halted = Some(true);
    halted.halted_at = Some(NOW);
    halted
}

#[test]
fn tdd_resume_create_task_does_not_unhalt() {
    let halted = halted_team(Vec::new());
    let input = CreateTaskInput {
        subject: "more work".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        validate_create_task(&halted, &input).unwrap_err(),
        "team is halted; resume with a non-empty reason before create_task"
    );
    assert!(halted.is_halted());
    let blank_reason = CreateTaskInput {
        resume: Some(true),
        resume_reason: Some("  ".to_owned()),
        ..input
    };
    assert!(validate_create_task(&halted, &blank_reason).is_err());
}

#[test]
fn tdd_resume_explicit_resume_clears_halt() {
    let mut halted = halted_team(Vec::new());
    let resumed = resume_team_state(&mut halted, "user asked to continue");
    assert_eq!(resumed.status, ResumeStatus::Resumed);
    assert!(resumed.ok);
    assert!(!halted.is_halted());
    assert_eq!(halted.halted_at, None);
    let again = resume_team_state(&mut halted, "again");
    assert_eq!(again.status, ResumeStatus::AlreadyRunning);
}

#[test]
fn tdd_resume_create_with_resume_reason_unhalts() {
    let halted = halted_team(Vec::new());
    let created = validate_create_task(
        &halted,
        &CreateTaskInput {
            subject: "next stage".to_owned(),
            resume: Some(true),
            resume_reason: Some("continue after user answer".to_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(created.clears_halt);
}

#[test]
fn tdd_resume_cancelled_tasks_stay_cancelled() {
    let mut halted = halted_team(vec![task("t1", |t| {
        t.status = TaskStatus::Cancelled;
        t.output = Some("Stopped from the captain chat.".to_owned());
    })]);
    resume_team_state(&mut halted, "continue");
    assert_eq!(halted.tasks[0].status, TaskStatus::Cancelled);
}

#[test]
fn tdd_resume_missing_reason_rejected() {
    let mut halted = halted_team(Vec::new());
    let result = resume_team_state(&mut halted, "");
    assert!(!result.ok);
    assert_eq!(result.status, ResumeStatus::Rejected);
    assert_eq!(
        result.error.as_deref(),
        Some("resume requires a non-empty reason")
    );
    assert!(halted.is_halted());
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        json!({"ok": false, "status": "rejected", "error": "resume requires a non-empty reason"})
    );
}

// ---------------------------------------------------------------------------
// F. coverage / delivery
// ---------------------------------------------------------------------------

#[test]
fn tdd_coverage_missing_item_blocks_delivery() {
    let covered = task("t1", |t| {
        t.kind = Some(TaskKind::Implementation);
        t.status = TaskStatus::Completed;
        t.coverage_of = some(&["TDD required"]);
    });
    let matrix = build_coverage_matrix(
        &strings(&["TDD required", "explicit resume"]),
        std::slice::from_ref(&covered),
    );
    let missing = matrix
        .iter()
        .find(|row| row.goal_item == "explicit resume")
        .unwrap();
    assert_eq!(missing.status, CoverageStatus::Missing);
    assert_eq!(matrix[0].status, CoverageStatus::Passed);
    assert_eq!(matrix[0].task_ids, strings(&["t1"]));
    let delivery = can_declare_delivery(&team(vec![covered]));
    assert!(!delivery.ok);
    assert_eq!(
        delivery.blockers,
        strings(&["completed implementation has no passing review"])
    );
}

#[test]
fn tdd_coverage_passed_item_requires_completed_task() {
    let matrix = build_coverage_matrix(
        &strings(&["TDD required"]),
        &[task("t1", |t| {
            t.kind = Some(TaskKind::Implementation);
            t.coverage_of = some(&["TDD required"]);
        })],
    );
    assert_eq!(matrix[0].goal_item, "TDD required");
    assert_eq!(matrix[0].status, CoverageStatus::InProgress);
    let blocked = build_coverage_matrix(
        &strings(&["g"]),
        &[task("t1", |t| {
            t.status = TaskStatus::Failed;
            t.coverage_of = some(&["g"]);
        })],
    );
    assert_eq!(blocked[0].status, CoverageStatus::Blocked);
    assert_eq!(
        serde_json::to_value(&blocked[0]).unwrap(),
        json!({"goal_item": "g", "task_ids": ["t1"], "status": "blocked"})
    );
}

#[test]
fn tdd_delivery_ok_only_when_all_gates_pass() {
    let ready = team(vec![
        task("t1", |t| {
            t.kind = Some(TaskKind::Requirements);
            t.status = TaskStatus::Completed;
            t.verdict = Some(ReviewVerdict::Pass);
            t.coverage_of = some(&["goal"]);
        }),
        task("t2", |t| {
            t.status = TaskStatus::Completed;
            t.coverage_of = some(&["goal"]);
            impl_contract(t);
        }),
        task("t3", |t| {
            t.kind = Some(TaskKind::Verification);
            t.status = TaskStatus::Completed;
        }),
        task("t4", |t| {
            t.kind = Some(TaskKind::Review);
            t.status = TaskStatus::Completed;
            t.verdict = Some(ReviewVerdict::Pass);
            t.reviewed_task_id = Some("t2".to_owned());
        }),
        task("t5", |t| {
            t.kind = Some(TaskKind::Integration);
            t.status = TaskStatus::Completed;
        }),
    ]);
    let delivery = can_declare_delivery(&ready);
    assert!(delivery.ok, "{:?}", delivery.blockers);
}

#[test]
fn tdd_delivery_failed_review_without_repair_blocks() {
    let blocked = team(vec![
        task("t1", |t| {
            t.kind = Some(TaskKind::Implementation);
            t.status = TaskStatus::Completed;
            t.coverage_of = some(&["goal"]);
        }),
        task("t2", |t| {
            t.kind = Some(TaskKind::Review);
            t.status = TaskStatus::Failed;
            t.verdict = Some(ReviewVerdict::NeedsRevision);
            t.reviewed_task_id = Some("t1".to_owned());
            t.findings = Some(vec![finding("C-001", FindingSeverity::High, "bug", "fix")]);
        }),
    ]);
    let result = can_declare_delivery(&blocked);
    assert!(!result.ok);
    // Neither blocker text contains "review", so both are listed.
    assert_eq!(
        result.blockers,
        strings(&[
            "t2 failed without a follow-up repair",
            "completed implementation has no passing review"
        ])
    );
}

#[test]
fn delivery_flags_unaudited_paths_and_unpassed_reviews() {
    let current = team(vec![
        task("t1", |t| {
            impl_contract(t);
            t.status = TaskStatus::Completed;
            t.changed_paths = some(&["src/parser.ts", "docs/x.md"]);
        }),
        task("t2", |t| {
            t.kind = Some(TaskKind::Review);
            t.status = TaskStatus::Completed;
            t.verdict = Some(ReviewVerdict::NeedsRevision);
        }),
    ]);
    assert_eq!(
        can_declare_delivery(&current).blockers,
        strings(&[
            "t2 completed without verdict=pass",
            "completed implementation has no passing review",
            "t1 has unaudited path docs/x.md"
        ])
    );
}

/// `stability-tdd.mjs`: work, empty, staged, halted and escalated teams cannot
/// falsely declare delivery.
#[test]
fn work_empty_staged_halted_and_escalated_teams_cannot_falsely_declare_delivery() {
    let mut state = team(vec![task("t1", |t| {
        t.subject = "Work".to_owned();
        t.assignee = Some("worker".to_owned());
        t.attempt_id = Some("a1".to_owned());
    })]);
    for status in [
        TaskStatus::Pending,
        TaskStatus::Claimed,
        TaskStatus::InProgress,
        TaskStatus::Failed,
    ] {
        state.tasks[0].status = status;
        assert!(!can_declare_delivery(&state).ok, "{status}");
    }
    state.tasks[0].status = TaskStatus::Completed;
    assert!(can_declare_delivery(&state).ok);
    let variants: [fn(&mut TeamState); 4] = [
        |team| team.phase = Some(TeamPhase::Staged),
        |team| team.halted = Some(true),
        |team| team.escalated = Some(true),
        |team| team.tasks.clear(),
    ];
    for variant in variants {
        let mut copy = state.clone();
        variant(&mut copy);
        assert!(!can_declare_delivery(&copy).ok);
    }
    state.tasks[0].status = TaskStatus::Cancelled;
    assert_eq!(
        can_declare_delivery(&state).blockers,
        strings(&["all work was cancelled"])
    );
}

// ---------------------------------------------------------------------------
// H. generated repair scope (#173) and quality-gates-repair-scope.test.mjs
// ---------------------------------------------------------------------------

fn h_section_repair() -> PlannedFollowUpTask {
    let mut source = completed_source(|_| {});
    source.in_scope = some(&[
        "deploy/compose/postfix/master.cf.inc",
        "internal/pkg/releasedeliver/deliver.go",
    ]);
    // A directory pattern covering the first inScope entry; inheriting it
    // verbatim would make the repair's own inScope impossible to register.
    source.out_of_scope = some(&["deploy/compose/postfix/", "go.mod"]);
    let review = needs_revision_review(|t| {
        t.findings = Some(vec![
            with_file(
                finding(
                    "C-001",
                    FindingSeverity::Medium,
                    "the reinjection mount key is missing",
                    "add no_address_mappings",
                ),
                "deploy/compose/postfix/master.cf.inc",
            ),
            with_file(
                finding(
                    "C-002",
                    FindingSeverity::Low,
                    "second finding on the same file",
                    "and land it once",
                ),
                "deploy/compose/postfix/master.cf.inc",
            ),
        ]);
    });
    let planned = plan_quality_follow_up(&team(vec![source, review.clone()]), &review);
    created_of(&planned, TaskKind::Repair).unwrap().clone()
}

#[test]
fn tdd_scope_generated_repair_inscope_has_no_duplicates() {
    let repair = h_section_repair();
    let in_scope = repair.in_scope.unwrap();
    assert_eq!(
        in_scope
            .iter()
            .filter(|path| *path == "deploy/compose/postfix/master.cf.inc")
            .count(),
        1,
        "{in_scope:?}"
    );
}

#[test]
fn tdd_scope_generated_repair_drops_out_of_scope_covering_its_inscope() {
    let repair = h_section_repair();
    let out_of_scope = repair.out_of_scope.unwrap();
    assert!(!out_of_scope.contains(&"deploy/compose/postfix/".to_owned()));
    assert!(out_of_scope.contains(&"go.mod".to_owned()));
}

#[test]
fn tdd_scope_every_generated_repair_inscope_path_is_registrable() {
    let repair = h_section_repair();
    let in_scope = repair.in_scope.clone().unwrap_or_default();
    let out_of_scope = repair.out_of_scope.clone().unwrap_or_default();
    assert!(!in_scope.is_empty());
    for path in &in_scope {
        assert_eq!(
            classify_changed_path(path, &in_scope, &out_of_scope),
            PathClassification::InScope
        );
    }
}

fn scope_finding(edit: impl FnOnce(&mut ReviewFinding)) -> ReviewFinding {
    let mut item = finding("F1", FindingSeverity::Medium, "problem", "fix it");
    edit(&mut item);
    item
}

#[test]
fn repair_scope_includes_the_fix_target_named_in_required_fix() {
    let scope = repair_scope_from_findings(
        &[scope_finding(|f| {
            f.file = Some("data/sample.txt".to_owned());
            f.required_fix = "change README.md:5 to \"2 lines / 5 words\"".to_owned();
        })],
        Some(&strings(&["data/sample.txt"])),
    )
    .unwrap();
    assert!(scope.contains(&"README.md".to_owned()), "{scope:?}");
    assert!(scope.contains(&"data/sample.txt".to_owned()), "{scope:?}");
}

#[test]
fn repair_scope_drops_absolute_paths() {
    let scope = repair_scope_from_findings(
        &[scope_finding(|f| {
            f.file = Some("F:\\team\\data\\sample.txt".to_owned());
            f.required_fix = "update README.md sample reference to 5 words".to_owned();
        })],
        Some(&strings(&["src/"])),
    );
    assert_eq!(scope, some(&["README.md"]));
}

#[test]
fn repair_scope_falls_back_to_the_source_inscope() {
    let scope = repair_scope_from_findings(
        &[scope_finding(|f| {
            f.required_fix = "reinstall the tool globally and retry".to_owned();
        })],
        Some(&strings(&["src/"])),
    );
    assert_eq!(scope, some(&["src/"]));
    assert_eq!(
        repair_scope_from_findings(&[scope_finding(|_| {})], None),
        None
    );
}

#[test]
fn repair_scope_strips_line_number_suffixes() {
    let scope = repair_scope_from_findings(
        &[scope_finding(|f| {
            f.required_fix = "rewrite docs/guide.md:42 to match the new flow".to_owned();
        })],
        None,
    );
    assert_eq!(scope, some(&["docs/guide.md"]));
    let observed = repair_scope_from_findings(
        &[scope_finding(|f| f.file = Some("src/a.ts:12".to_owned()))],
        None,
    );
    assert_eq!(observed, some(&["src/a.ts"]));
}

#[test]
fn repair_scope_ignores_versions_hashes_and_prose() {
    let scope = repair_scope_from_findings(
        &[scope_finding(|f| {
            f.required_fix =
                "bump v0.1.17 to match deadbeef1234 and edit wc.js, also Ünïcode.md".to_owned();
        })],
        None,
    );
    // `\w` is ASCII in the reference: the non-ASCII stem is cut off.
    assert_eq!(scope, some(&["wc.js", "code.md"]));
}

fn scope_team(edit: impl FnOnce(&mut TeamTask)) -> TeamState {
    let mut source = TeamTask::new("t1", "impl", 0);
    source.status = TaskStatus::Completed;
    source.attempt = Some(1);
    source.kind = Some(TaskKind::Implementation);
    source.assignee = Some("implementer".to_owned());
    source.in_scope = some(&["src/"]);
    source.acceptance = some(&["done"]);
    source.verify = some(&["pnpm test"]);
    edit(&mut source);
    let mut state = TeamState::new("t", "t", "captain", 0);
    state.description = Some(String::new());
    state.task_seq = 1;
    state.members = vec![
        member("implementer", "implementer"),
        member("reviewer", "reviewer"),
    ];
    state.tasks = vec![source];
    state
}

fn failed_review(findings: Vec<ReviewFinding>) -> TeamTask {
    let mut review = TeamTask::new("t2", "review", 0);
    review.status = TaskStatus::Failed;
    review.dependencies = strings(&["t1"]);
    review.attempt = Some(1);
    review.kind = Some(TaskKind::Review);
    review.round = Some(1);
    review.assignee = Some("reviewer".to_owned());
    review.verdict = Some(ReviewVerdict::NeedsRevision);
    review.reviewed_task_id = Some("t1".to_owned());
    review.objective = Some("Review".to_owned());
    review.acceptance = some(&["no blockers"]);
    review.findings = Some(findings);
    review
}

fn planned_as_task(planned: &PlannedFollowUpTask) -> TeamTask {
    let mut task = TeamTask::new(
        planned.id.clone().unwrap_or_default(),
        planned.subject.clone().unwrap_or_default(),
        0,
    );
    task.status = TaskStatus::InProgress;
    task.kind = Some(planned.kind);
    task.assignee = planned.assignee.clone();
    task.dependencies = planned.dependencies.clone().unwrap_or_default();
    task.round = planned.round;
    task.objective = planned.objective.clone();
    task.in_scope = planned.in_scope.clone();
    task.out_of_scope = planned.out_of_scope.clone();
    task.acceptance = planned.acceptance.clone();
    task.verify = planned.verify.clone();
    task.source_task_id = planned.source_task_id.clone();
    task.source_finding_ids = planned.source_finding_ids.clone();
    task.reviewed_task_id = planned.reviewed_task_id.clone();
    task
}

#[test]
fn end_to_end_generated_repair_round_is_satisfiable_for_the_incident_shape() {
    let closed = failed_review(vec![scope_finding(|f| {
        f.file = Some("data/sample.txt".to_owned());
        f.required_fix =
            "A (recommended) change README.md to \"2 lines / 5 words\"; do not modify wc.js"
                .to_owned();
    })]);
    let result = plan_quality_follow_up(&scope_team(|_| {}), &closed);
    let repair = created_of(&result, TaskKind::Repair).expect("repair round must be generated");
    let in_scope = repair.in_scope.as_ref().unwrap();
    assert!(in_scope.contains(&"README.md".to_owned()), "{in_scope:?}");
    assert!(
        repair
            .acceptance
            .as_ref()
            .unwrap()
            .iter()
            .all(|criterion| !criterion.is_empty())
    );
}

fn assert_generated_repair_is_completable(
    label: &str,
    source_scope: &[&str],
    source_exclusions: &[&str],
    findings: Vec<ReviewFinding>,
    expected_scope: &[&str],
    expected_exclusions: &[&str],
) {
    let state = scope_team(|t| {
        t.in_scope = some(source_scope);
        t.out_of_scope = some(source_exclusions);
    });
    let original = state.clone();
    let closed = failed_review(findings.clone());
    let result = plan_quality_follow_up(&state, &closed);
    let repair = created_of(&result, TaskKind::Repair).unwrap();
    assert_eq!(repair.in_scope, some(expected_scope), "{label}");
    assert_eq!(repair.out_of_scope, some(expected_exclusions), "{label}");
    assert_eq!(
        state, original,
        "{label}: source contract must remain untouched"
    );
    assert_eq!(
        repair.acceptance,
        Some(findings.iter().map(|f| f.required_fix.clone()).collect()),
        "{label}"
    );
    let in_scope = repair.in_scope.clone().unwrap();
    let out_of_scope = repair.out_of_scope.clone().unwrap();
    for path in expected_scope {
        let concrete = if path.ends_with('/') {
            format!("{path}fixed.ts")
        } else {
            (*path).to_owned()
        };
        assert_eq!(
            classify_changed_path(&concrete, &in_scope, &out_of_scope),
            PathClassification::InScope,
            "{label}: {concrete}"
        );
        let mut exit_zero = ran("pnpm test", CheckStatus::Passed);
        exit_zero.exit_code = Some(0);
        let update = QualityCompletionUpdate {
            status: Some(TaskStatus::Completed),
            changed_paths: Some(vec![concrete.clone()]),
            acceptance_results: Some(
                repair
                    .acceptance
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|criterion| accepted(criterion))
                    .collect(),
            ),
            commands_run: Some(vec![exit_zero]),
            ..Default::default()
        };
        let completion = evaluate_quality_completion(&planned_as_task(repair), &update);
        assert!(completion.is_ok(), "{label}: {completion:?}");
    }
    assert_eq!(
        created_of(&result, TaskKind::Review)
            .unwrap()
            .reviewed_task_id,
        repair.id,
        "{label}"
    );
    for path in expected_exclusions {
        assert_eq!(
            classify_changed_path(path, &in_scope, &out_of_scope),
            PathClassification::OutOfScope,
            "{label}: {path}"
        );
    }
}

#[test]
fn generated_repair_is_completable_real_f_doc_incident() {
    assert_generated_repair_is_completable(
        "real F_DOC incident",
        &["data/sample.txt"],
        &["README.md", "check-data.mjs", "reports/"],
        vec![scope_finding(|f| {
            f.file = Some("data/sample.txt".to_owned());
            f.required_fix =
                "Edit README.md to contain exactly 2 words. Keep data/sample.txt unchanged."
                    .to_owned();
        })],
        &["data/sample.txt", "README.md"],
        &["check-data.mjs", "reports/"],
    );
}

#[test]
fn generated_repair_is_completable_directory_exclusion_covers_observed_and_required_paths() {
    assert_generated_repair_is_completable(
        "directory exclusion covers observed and required paths",
        &["src/"],
        &["deploy/", "go.mod"],
        vec![
            scope_finding(|f| {
                f.file = Some("deploy/old.txt".to_owned());
                f.required_fix = "edit deploy/new.txt and deploy/old.txt".to_owned();
            }),
            scope_finding(|f| f.file = Some("deploy/old.txt".to_owned())),
        ],
        &["deploy/old.txt", "deploy/new.txt"],
        &["go.mod"],
    );
}

#[test]
fn generated_repair_is_completable_fallback_scope_has_duplicates_and_overlapping_exclusions() {
    assert_generated_repair_is_completable(
        "fallback scope has duplicates and overlapping exclusions",
        &["src/", "src/"],
        &["src/private.ts", "docs/"],
        vec![scope_finding(|_| {})],
        &["src/"],
        &["docs/"],
    );
}

// ---------------------------------------------------------------------------
// quality-gates-amend.test.mjs
// ---------------------------------------------------------------------------

fn amend_team(tasks: Vec<TeamTask>) -> TeamState {
    let mut state = TeamState::new("t", "t", "captain", 0);
    state.description = Some(String::new());
    state.task_seq = tasks.len() as u64;
    state.members = vec![
        member("implementer", "implementer"),
        member("reviewer", "reviewer"),
    ];
    state.tasks = tasks;
    state
}

fn amend_impl_task(edit: impl FnOnce(&mut TeamTask)) -> TeamTask {
    let mut task = TeamTask::new("t1", "impl", 0);
    task.status = TaskStatus::InProgress;
    task.attempt = Some(1);
    task.kind = Some(TaskKind::Implementation);
    task.assignee = Some("implementer".to_owned());
    task.objective = Some("Write the sample".to_owned());
    task.in_scope = some(&["docs/"]);
    task.acceptance = some(&["src/amend-e2e.txt says ok"]);
    task.verify = some(&["node -e \"process.exit(0)\""]);
    edit(&mut task);
    task
}

fn judgment(kind: TaskKind, status: TaskStatus, verdict: ReviewVerdict) -> TeamTask {
    let mut task = TeamTask::new("t2", "review", 0);
    task.status = status;
    task.dependencies = strings(&["t1"]);
    task.kind = Some(kind);
    task.verdict = Some(verdict);
    task.reviewed_task_id = Some("t1".to_owned());
    task
}

fn completed_results(task: &TeamTask) -> QualityCompletionUpdate {
    QualityCompletionUpdate {
        status: Some(TaskStatus::Completed),
        changed_paths: some(&["src/amend-e2e.txt"]),
        acceptance_results: Some(
            task.acceptance
                .iter()
                .flatten()
                .map(|criterion| accepted(criterion))
                .collect(),
        ),
        commands_run: Some(
            task.verify
                .iter()
                .flatten()
                .map(|command| ran(command, CheckStatus::Passed))
                .collect(),
        ),
        ..Default::default()
    }
}

fn amend(
    state: &mut TeamState,
    input: ContractAmendmentInput,
    by: &str,
    reason: &str,
) -> Result<TaskRevision, String> {
    amend_task_contract(state, "t1", &input, by, reason)
}

#[test]
fn amend_fixes_an_unsatisfiable_in_scope_and_records_the_revision() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    let before = state.tasks[0].clone();
    let revision = amend(
        &mut state,
        ContractAmendmentInput {
            in_scope: some(&["docs/", "src/"]),
            ..Default::default()
        },
        "captain",
        "objective names src/ but inScope forbade it",
    )
    .unwrap();
    let task = &state.tasks[0];
    assert_eq!(task.in_scope, some(&["docs/", "src/"]));
    let revisions = task.revisions.as_ref().unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0], revision);
    assert!(crate::validate::is_valid_revision(&revision));
    assert_eq!(revision.by, "captain");
    assert_eq!(revision.fields, strings(&["inScope"]));
    assert_eq!(revision.previous.get("inScope"), Some(&json!(["docs/"])));
    assert!(revision.reason.contains("src/"));
    assert!(task.updated_at >= before.updated_at);
}

#[test]
fn amended_contract_is_the_one_the_completion_gate_evaluates() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    let blocked = evaluate_quality_completion(&state.tasks[0], &completed_results(&state.tasks[0]));
    let error = blocked.unwrap_err().error;
    assert!(error.contains("src/amend-e2e.txt is undeclared"), "{error}");
    amend(
        &mut state,
        ContractAmendmentInput {
            in_scope: some(&["docs/", "src/"]),
            ..Default::default()
        },
        "captain",
        "scope was wrong",
    )
    .unwrap();
    let amended = &state.tasks[0];
    let gate = evaluate_quality_completion(amended, &completed_results(amended));
    assert!(
        gate.is_ok(),
        "gate must evaluate the amended contract, got: {gate:?}"
    );
}

#[test]
fn mid_flight_amendments_are_allowed_until_a_review_passes_judgment() {
    let mut open = amend_team(vec![
        amend_impl_task(|_| {}),
        judgment(
            TaskKind::Review,
            TaskStatus::Failed,
            ReviewVerdict::NeedsRevision,
        ),
    ]);
    assert!(
        amend(
            &mut open,
            ContractAmendmentInput {
                verify: some(&["node -e \"process.exit(0)\""]),
                ..Default::default()
            },
            "captain",
            "r1"
        )
        .is_ok()
    );
    let mut passed = amend_team(vec![
        amend_impl_task(|_| {}),
        judgment(TaskKind::Review, TaskStatus::Completed, ReviewVerdict::Pass),
    ]);
    let before = passed.clone();
    let frozen = amend(
        &mut passed,
        ContractAmendmentInput {
            verify: some(&["node -e \"process.exit(1)\""]),
            ..Default::default()
        },
        "captain",
        "late idea",
    )
    .unwrap_err();
    assert!(frozen.contains("frozen"), "{frozen}");
    assert_eq!(
        frozen,
        "task t1 already passed review; its contract is frozen"
    );
    assert_eq!(passed, before, "a refused amendment changes nothing");
}

#[test]
fn requirements_passing_judgment_also_freezes_the_contract() {
    let mut requirements = judgment(
        TaskKind::Requirements,
        TaskStatus::Completed,
        ReviewVerdict::Pass,
    );
    requirements.subject = "requirements-round-2".to_owned();
    requirements.dependencies.clear();
    requirements.round = Some(2);
    let mut state = amend_team(vec![amend_impl_task(|_| {}), requirements]);
    assert!(
        amend(
            &mut state,
            ContractAmendmentInput {
                objective: Some("different goal".to_owned()),
                ..Default::default()
            },
            "captain",
            "nope"
        )
        .is_err()
    );
}

#[test]
fn terminal_tasks_and_work_tasks_have_nothing_amendable() {
    let objective = || ContractAmendmentInput {
        objective: Some("x".to_owned()),
        ..Default::default()
    };
    let mut done = amend_team(vec![amend_impl_task(|t| t.status = TaskStatus::Completed)]);
    let error = amend(&mut done, objective(), "captain", "r").unwrap_err();
    assert!(error.contains("terminal"), "{error}");
    assert_eq!(
        error,
        "task t1 is completed; terminal contracts are immutable"
    );
    let mut work = amend_team(vec![amend_impl_task(|t| t.kind = Some(TaskKind::Work))]);
    let error = amend(&mut work, objective(), "captain", "r").unwrap_err();
    assert!(error.contains("kind=work"), "{error}");
}

#[test]
fn amendment_requires_a_reason_an_author_and_at_least_one_contract_field() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    let objective = || ContractAmendmentInput {
        objective: Some("x".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        amend(&mut state, objective(), "captain", "  ").unwrap_err(),
        "contract amendment requires a non-empty reason"
    );
    assert_eq!(
        amend(&mut state, objective(), "  ", "r").unwrap_err(),
        "contract amendment requires a non-empty author identity"
    );
    assert_eq!(
        amend(
            &mut state,
            ContractAmendmentInput::default(),
            "captain",
            "r"
        )
        .unwrap_err(),
        "amendment requires at least one of: objective, acceptance, verify, inScope, outOfScope"
    );
    assert!(
        amend_task_contract(&mut state, "t9", &objective(), "captain", "r")
            .unwrap_err()
            .starts_with("no task \"t9\" in team \"t\"")
    );
}

#[test]
fn replacement_values_must_be_well_formed() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    let before = state.clone();
    let error = |state: &mut TeamState, input: ContractAmendmentInput| {
        amend(state, input, "captain", "r").unwrap_err()
    };
    assert_eq!(
        error(
            &mut state,
            ContractAmendmentInput {
                objective: Some("  ".to_owned()),
                ..Default::default()
            }
        ),
        "amended objective must be a non-empty string"
    );
    assert_eq!(
        error(
            &mut state,
            ContractAmendmentInput {
                acceptance: Some(Vec::new()),
                ..Default::default()
            }
        ),
        "amended acceptance must be a non-empty list of non-empty strings"
    );
    assert_eq!(
        error(
            &mut state,
            ContractAmendmentInput {
                verify: some(&["run", "  "]),
                ..Default::default()
            }
        ),
        "amended verify must be a non-empty list of non-empty strings"
    );
    let absolute = error(
        &mut state,
        ContractAmendmentInput {
            in_scope: some(&["F:\\team\\src\\"]),
            ..Default::default()
        },
    );
    assert!(absolute.contains("workspace-relative"), "{absolute}");
    assert_eq!(
        absolute,
        "amended inScope entry \"F:\\team\\src\\\" is not a workspace-relative path (absolute paths and \"..\" can never match scope patterns)"
    );
    let traversal = error(
        &mut state,
        ContractAmendmentInput {
            out_of_scope: some(&["../secrets"]),
            ..Default::default()
        },
    );
    assert!(traversal.contains("workspace-relative"), "{traversal}");
    assert_eq!(state, before);
}

#[test]
fn revisions_accumulate_and_previous_values_snapshot_the_prior_contract() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    amend(
        &mut state,
        ContractAmendmentInput {
            verify: some(&["node -e \"process.exit(1)\""]),
            ..Default::default()
        },
        "captain",
        "first fix",
    )
    .unwrap();
    amend(
        &mut state,
        ContractAmendmentInput {
            verify: some(&["node -e \"process.exit(0)\""]),
            objective: Some("Write the sample (revised)".to_owned()),
            ..Default::default()
        },
        "captain",
        "second fix",
    )
    .unwrap();
    let revisions = state.tasks[0].revisions.as_ref().unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(
        revisions[0].previous.get("verify"),
        Some(&json!(["node -e \"process.exit(0)\""]))
    );
    assert_eq!(
        revisions[1].previous.get("verify"),
        Some(&json!(["node -e \"process.exit(1)\""]))
    );
    let mut fields = revisions[1].fields.clone();
    fields.sort();
    assert_eq!(fields, strings(&["objective", "verify"]));
    assert_eq!(
        revisions[1].previous.get("objective"),
        Some(&json!("Write the sample"))
    );
}

#[test]
fn amendment_previous_omits_fields_absent_before() {
    let mut state = amend_team(vec![amend_impl_task(|_| {})]);
    let revision = amend(
        &mut state,
        ContractAmendmentInput {
            out_of_scope: some(&["vendor/"]),
            ..Default::default()
        },
        "captain",
        "fence vendor",
    )
    .unwrap();
    assert_eq!(revision.fields, strings(&["outOfScope"]));
    assert!(revision.previous.is_empty());
    assert_eq!(state.tasks[0].out_of_scope, some(&["vendor/"]));
}

#[test]
fn durable_state_validation_accepts_well_formed_revisions_and_rejects_malformed_ones() {
    let with_revisions = |revisions: Vec<TaskRevision>| {
        let mut task = TeamTask::new("t1", "s", 0);
        task.revisions = Some(revisions);
        has_valid_quality_task_fields(&task)
    };
    let revision = |by: &str, fields: &[&str], previous: Map<String, Value>| TaskRevision {
        at: 1,
        by: by.to_owned(),
        reason: "r".to_owned(),
        fields: strings(fields),
        previous,
    };
    let mut previous = Map::new();
    previous.insert("inScope".to_owned(), json!(["docs/"]));
    assert!(with_revisions(vec![revision(
        "captain",
        &["inScope"],
        previous
    )]));
    assert!(with_revisions(Vec::new()));
    assert!(!with_revisions(vec![revision(
        "",
        &["inScope"],
        Map::new()
    )]));
    assert!(!with_revisions(vec![revision("captain", &[], Map::new())]));
    // `revisions: 'nope'` cannot be typed at all.
    let mut record = serde_json::to_value(TeamTask::new("t1", "s", 0)).unwrap();
    record["revisions"] = json!("nope");
    assert!(serde_json::from_value::<TeamTask>(record).is_err());
}

// ---------------------------------------------------------------------------
// issue-159.test.mjs (the pure cases)
// ---------------------------------------------------------------------------

fn issue_task() -> TeamTask {
    let mut task = TeamTask::new("t1", "repair", 1);
    task.assignee = Some("worker".to_owned());
    task.status = TaskStatus::Failed;
    task.attempt = Some(1);
    task.attempt_id = Some("old".to_owned());
    task.updated_at = 2;
    task.kind = Some(TaskKind::Repair);
    task.source_task_id = Some("source".to_owned());
    task.source_finding_ids = some(&["F1"]);
    task.objective = Some("Fix parser".to_owned());
    task.in_scope = some(&["src/"]);
    task.acceptance = some(&["works"]);
    task.verify = some(&["test"]);
    task.output = Some("old result".to_owned());
    task.verdict = Some(ReviewVerdict::Pass);
    task.findings = Some(Vec::new());
    task.changed_paths = some(&["src/a"]);
    task.acceptance_results = Some(vec![accepted("works")]);
    task.commands_run = Some(vec![ran("test", CheckStatus::Passed)]);
    task
}

#[test]
fn captain_cannot_create_a_duplicate_repair_for_the_same_unresolved_findings() {
    let mut existing = issue_task();
    existing.status = TaskStatus::Pending;
    let mut source = TeamTask::new("source", "source", 1);
    source.status = TaskStatus::Completed;
    let current = amend_team(vec![source, existing.clone()]);
    let mut input = CreateTaskInput::from(&existing);
    input.subject = "duplicate".to_owned();
    input.dependencies = some(&["t1"]);
    let error = validate_create_task(&current, &input).unwrap_err();
    assert!(error.contains("t1"), "{error}");
    assert_eq!(
        error,
        "repair task t1 already covers these findings; use that task instead of creating duplicate work"
    );
}

#[test]
fn supplements_retain_author_attempt_and_old_result_without_reopening_a_failed_gate() {
    let mut task = issue_task();
    let before = task.clone();
    let mut late_check = ran("late-check", CheckStatus::Failed);
    late_check.exit_code = Some(1);
    late_check.evidence = Some("new observation".to_owned());
    let mut edge_case = accepted("edge case");
    edge_case.status = CheckStatus::Failed;
    let input = QualityCompletionUpdate {
        commands_run: Some(vec![late_check]),
        acceptance_results: Some(vec![edge_case]),
        evidence_note: Some(" Follow-up ".to_owned()),
        ..Default::default()
    };
    assert_eq!(append_task_evidence(&mut task, &input, "worker"), Ok(true));
    let entry = &task.supplemental_evidence.as_ref().unwrap()[0];
    assert_eq!(entry.by, "worker");
    assert_eq!(entry.attempt_id.as_deref(), Some("old"));
    assert_eq!(entry.attempt, 1);
    assert_eq!(entry.note.as_deref(), Some("Follow-up"));
    let mut without = task.clone();
    without.supplemental_evidence = None;
    assert_eq!(without, before);

    assert_eq!(append_task_evidence(&mut task, &input, "worker"), Ok(false));
    // The same observation with its keys in another order is still a repeat.
    let reordered: CommandResult = serde_json::from_value(json!({
        "evidence": "new observation", "exitCode": 1, "status": "failed", "command": "late-check"
    }))
    .unwrap();
    let repeat = QualityCompletionUpdate {
        commands_run: Some(vec![reordered]),
        ..Default::default()
    };
    assert_eq!(
        append_task_evidence(&mut task, &repeat, "worker"),
        Ok(false)
    );
    assert_eq!(task.supplemental_evidence.as_ref().unwrap().len(), 1);

    let captain_note = QualityCompletionUpdate {
        evidence_note: Some("Captain independently verified".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        append_task_evidence(&mut task, &captain_note, "captain"),
        Ok(true)
    );
    assert!(has_valid_quality_task_fields(&task));
    let reloaded: TeamTask = serde_json::from_value(serde_json::to_value(&task).unwrap()).unwrap();
    assert!(has_valid_quality_task_fields(&reloaded));

    let count = task.supplemental_evidence.as_ref().unwrap().len();
    // The reference's `status: 'maybe'` cannot be typed; a blank command is
    // the representable invalid observation.
    let invalid = QualityCompletionUpdate {
        commands_run: Some(vec![ran("  ", CheckStatus::Passed)]),
        ..Default::default()
    };
    let error = append_task_evidence(&mut task, &invalid, "worker").unwrap_err();
    assert!(error.contains("invalid"), "{error}");
    assert_eq!(task.supplemental_evidence.as_ref().unwrap().len(), count);
}

#[test]
fn terminal_immutable_fields_cannot_be_silently_changed() {
    let changes = [
        QualityCompletionUpdate {
            status: Some(TaskStatus::Completed),
            ..Default::default()
        },
        QualityCompletionUpdate {
            output: Some("overwrite".to_owned()),
            ..Default::default()
        },
        QualityCompletionUpdate {
            verdict: Some(ReviewVerdict::Reject),
            ..Default::default()
        },
        QualityCompletionUpdate {
            findings: Some(vec![finding("new", FindingSeverity::Low, "p", "f")]),
            ..Default::default()
        },
        QualityCompletionUpdate {
            changed_paths: some(&["elsewhere"]),
            ..Default::default()
        },
    ];
    for (change, key) in
        changes
            .into_iter()
            .zip(["status", "output", "verdict", "findings", "changedPaths"])
    {
        let mut task = issue_task();
        let before = task.clone();
        let error = append_task_evidence(&mut task, &change, "captain").unwrap_err();
        assert!(error.contains("immutable"), "{error}");
        assert_eq!(
            error,
            format!(
                "terminal task t1 is immutable: cannot change {key}; append evidence with evidence_note, acceptanceResults or commandsRun instead. Do not reclaim or redo completed work."
            )
        );
        assert_eq!(task, before);
    }
    // Restating the unchanged result is allowed.
    let mut task = issue_task();
    let same = QualityCompletionUpdate {
        status: Some(TaskStatus::Failed),
        output: Some("old result".to_owned()),
        evidence_note: Some("note".to_owned()),
        ..Default::default()
    };
    assert_eq!(append_task_evidence(&mut task, &same, "worker"), Ok(true));
}

#[test]
fn supplemental_evidence_requires_a_terminal_task() {
    let mut task = issue_task();
    task.status = TaskStatus::InProgress;
    let note = QualityCompletionUpdate {
        evidence_note: Some("x".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        append_task_evidence(&mut task, &note, "worker").unwrap_err(),
        "supplemental evidence requires a terminal task"
    );
}

#[test]
fn malformed_persisted_supplements_are_rejected() {
    let evidence =
        |by: &str, note: Option<&str>, commands: Option<Vec<CommandResult>>| TaskEvidence {
            at: 1,
            by: by.to_owned(),
            attempt: 1,
            attempt_id: None,
            note: note.map(str::to_owned),
            acceptance_results: None,
            commands_run: commands,
        };
    for item in [
        evidence("worker", None, None),
        evidence("", Some("x"), None),
        evidence("worker", None, Some(vec![ran("", CheckStatus::Passed)])),
    ] {
        let mut task = issue_task();
        task.supplemental_evidence = Some(vec![item]);
        assert!(!has_valid_quality_task_fields(&task));
    }
    // `null`, `{}`, `attempt: -1` and `commandsRun: [{}]` cannot be typed.
    for raw in [
        Value::Null,
        json!({}),
        json!({"at": 1, "by": "worker", "attempt": -1, "note": "x"}),
        json!({"at": 1, "by": "worker", "attempt": 1, "commandsRun": [{}]}),
    ] {
        let mut record = serde_json::to_value(issue_task()).unwrap();
        record["supplementalEvidence"] = json!([raw]);
        assert!(serde_json::from_value::<TeamTask>(record).is_err());
    }
}

// ---------------------------------------------------------------------------
// Tool-argument parsers, policy
// ---------------------------------------------------------------------------

#[test]
fn parse_findings_normalizes_and_reports_the_reference_errors() {
    assert_eq!(parse_findings(None), Ok(None));
    assert_eq!(
        parse_findings(Some(&json!("x"))).unwrap_err(),
        "findings must be an array"
    );
    assert_eq!(
        parse_findings(Some(&json!([1]))).unwrap_err(),
        "findings[0] must be an object"
    );
    let base = json!({"id": " C-1 ", "severity": "high", "problem": "p", "requiredFix": "f"});
    let broken = |key: &str, value: Value| {
        let mut item = base.clone();
        item[key] = value;
        parse_findings(Some(&json!([base.clone(), item]))).unwrap_err()
    };
    assert_eq!(broken("id", json!(" ")), "findings[1].id is required");
    assert_eq!(
        broken("severity", json!("critical")),
        "findings[1].severity is invalid"
    );
    assert_eq!(
        broken("problem", json!(7)),
        "findings[1].problem is required"
    );
    assert_eq!(
        broken("requiredFix", json!("")),
        "findings[1].requiredFix is required"
    );

    let mut full = base.clone();
    full["file"] = json!("src/a.ts");
    full["line"] = json!(12);
    full["resolved"] = json!(true);
    let mut blank_file = base.clone();
    blank_file["file"] = json!("  ");
    blank_file["line"] = json!("12");
    let parsed = parse_findings(Some(&json!([full, blank_file])))
        .unwrap()
        .unwrap();
    assert_eq!(parsed[0].id, "C-1");
    assert_eq!(parsed[0].severity, FindingSeverity::High);
    assert_eq!(parsed[0].file.as_deref(), Some("src/a.ts"));
    assert_eq!(parsed[0].line, Some(12));
    assert_eq!(parsed[0].resolved, Some(true));
    assert_eq!(parsed[1].file, None);
    assert_eq!(parsed[1].line, None);
    assert_eq!(parsed[1].resolved, None);
}

#[test]
fn parse_acceptance_and_command_results_report_the_reference_errors() {
    assert_eq!(parse_acceptance_results(None), Ok(None));
    assert_eq!(
        parse_acceptance_results(Some(&json!({}))).unwrap_err(),
        "acceptanceResults must be an array"
    );
    assert_eq!(
        parse_acceptance_results(Some(&json!([null]))).unwrap_err(),
        "acceptanceResults[0] must be an object"
    );
    assert_eq!(
        parse_acceptance_results(Some(&json!([{"criterion": " ", "status": "passed"}])))
            .unwrap_err(),
        "acceptanceResults[0].criterion is required"
    );
    assert_eq!(
        parse_acceptance_results(Some(&json!([{"criterion": "c", "status": "ok"}]))).unwrap_err(),
        "acceptanceResults[0].status must be passed or failed"
    );
    let parsed = parse_acceptance_results(Some(&json!([
        {"criterion": "c", "status": "failed", "evidence": ""}
    ])))
    .unwrap()
    .unwrap();
    assert_eq!(parsed[0].status, CheckStatus::Failed);
    assert_eq!(parsed[0].evidence.as_deref(), Some(""));

    assert_eq!(parse_command_results(None), Ok(None));
    assert_eq!(
        parse_command_results(Some(&json!(3))).unwrap_err(),
        "commandsRun must be an array"
    );
    assert_eq!(
        parse_command_results(Some(&json!([[]]))).unwrap_err(),
        "commandsRun[0] must be an object"
    );
    assert_eq!(
        parse_command_results(Some(&json!([{"status": "passed"}]))).unwrap_err(),
        "commandsRun[0].command is required"
    );
    assert_eq!(
        parse_command_results(Some(&json!([{"command": "x", "status": "maybe"}]))).unwrap_err(),
        "commandsRun[0].status must be passed or failed"
    );
    let parsed = parse_command_results(Some(&json!([
        {"command": "pnpm test", "status": "failed", "exitCode": 1, "evidence": "boom"},
        {"command": "pnpm lint", "status": "passed", "exitCode": "0"}
    ])))
    .unwrap()
    .unwrap();
    assert_eq!(parsed[0].exit_code, Some(1));
    assert_eq!(parsed[0].evidence.as_deref(), Some("boom"));
    assert_eq!(parsed[1].exit_code, None);
}

#[test]
fn review_policy_resolution_and_json_check() {
    let resolved = resolve_review_policy(None);
    assert_eq!(
        resolved.requirements_min_rounds,
        DEFAULT_REQUIREMENTS_MIN_ROUNDS
    );
    assert_eq!(
        resolved.requirements_max_rounds,
        DEFAULT_REQUIREMENTS_MAX_ROUNDS
    );
    assert_eq!(resolved.code_max_rounds, DEFAULT_CODE_MAX_ROUNDS);
    assert_eq!(resolved.max_repair_attempts, DEFAULT_MAX_REPAIR_ATTEMPTS);
    assert_eq!(resolved.required_reviewers, None);
    let partial = resolve_review_policy(Some(&ReviewPolicy {
        code_max_rounds: Some(5),
        required_reviewers: some(&["security"]),
        ..Default::default()
    }));
    assert_eq!(partial.code_max_rounds, 5);
    assert_eq!(partial.max_repair_attempts, 2);
    assert_eq!(partial.required_reviewers, some(&["security"]));

    assert!(is_review_policy(None));
    assert!(is_review_policy(Some(&json!({}))));
    assert!(is_review_policy(Some(
        &json!({"codeMaxRounds": 2.0, "requiredReviewers": ["a"]})
    )));
    assert!(!is_review_policy(Some(&Value::Null)));
    assert!(!is_review_policy(Some(&json!([]))));
    assert!(!is_review_policy(Some(&json!({"codeMaxRounds": 0}))));
    assert!(!is_review_policy(Some(&json!({"codeMaxRounds": 1.5}))));
    assert!(!is_review_policy(Some(&json!({"codeMaxRounds": null}))));
    assert!(!is_review_policy(Some(
        &json!({"requirementsMinRounds": 5})
    )));
    assert!(!is_review_policy(Some(
        &json!({"requiredReviewers": ["a", " "]})
    )));
    assert!(!is_review_policy(Some(&json!({"requiredReviewers": "a"}))));
    assert!(!is_review_policy(Some(&json!({"bogus": 1}))));
}

#[test]
fn kind_predicates() {
    assert!(QUALITY_KINDS.iter().all(|kind| is_quality_kind(*kind)));
    assert!(!is_quality_kind(TaskKind::Work));
    assert_eq!(WRITE_KINDS, [TaskKind::Implementation, TaskKind::Repair]);
    assert_eq!(task_kind_of(&TeamTask::new("t1", "s", 0)), TaskKind::Work);
    assert!(is_task_kind("integration") && !is_task_kind("Work"));
    assert!(is_review_verdict("needs_revision") && !is_review_verdict("ship-it"));
    assert!(is_finding_severity("blocker") && !is_finding_severity("critical"));
}
