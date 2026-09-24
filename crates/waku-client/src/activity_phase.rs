//! Which phase of work a tool call belongs to, and how consecutive calls of
//! one phase gather into a group in the transcript.
//!
//! Reading around a codebase — opening files, searching, listing — is one
//! phase however it is done: through a dedicated tool or a shell command
//! like `rg`, `ls` or `git diff`. Everything else a shell runs is terminal
//! work. Consecutive calls of one phase collapse into a single line
//! ("查阅 · 2 搜索, 3 文件"), so a turn that read twenty files reads as one
//! step, not twenty.

use std::ops::Range;

use crate::model::ActivityKind;

/// What kind of reading an exploring call did.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExploreBucket {
    Search,
    List,
    File,
}

/// The phase of work one activity belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityPhase {
    /// Reading the codebase, by tool or by read-only command.
    Explore(ExploreBucket),
    /// A shell command that is not plain reading.
    Terminal,
    /// Thinking. Carried along inside a group rather than breaking it.
    Reasoning,
    /// Anything else: edits, plans, web search, other tools.
    Other,
}

/// Which group a run of calls forms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GroupKind {
    Explore,
    Terminal,
}

impl ActivityPhase {
    fn group(self) -> Option<GroupKind> {
        match self {
            Self::Explore(_) => Some(GroupKind::Explore),
            Self::Terminal => Some(GroupKind::Terminal),
            Self::Reasoning | Self::Other => None,
        }
    }
}

/// The phase of an activity of `kind`; `command` is a shell command's text.
pub fn phase(kind: ActivityKind, command: Option<&str>) -> ActivityPhase {
    match kind {
        ActivityKind::Reasoning => ActivityPhase::Reasoning,
        ActivityKind::FileRead => ActivityPhase::Explore(ExploreBucket::File),
        ActivityKind::FileSearch => ActivityPhase::Explore(ExploreBucket::Search),
        ActivityKind::FileList => ActivityPhase::Explore(ExploreBucket::List),
        ActivityKind::Command => match command.and_then(classify_shell) {
            Some(bucket) => ActivityPhase::Explore(bucket),
            None => ActivityPhase::Terminal,
        },
        ActivityKind::FileChange
        | ActivityKind::Search
        | ActivityKind::Plan
        | ActivityKind::Tool => ActivityPhase::Other,
    }
}

/// One entry of a block as the transcript lays it out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityRun {
    /// An activity shown on its own.
    Single(usize),
    /// Consecutive activities of one phase, shown as one collapsible line.
    /// Thinking between them is inside the range.
    Group {
        kind: GroupKind,
        members: Range<usize>,
    },
}

/// Lay out a block's activities: a group forms from the second consecutive
/// call of one phase — one call alone is just a row — and thinking between
/// those calls joins the group rather than splitting it.
pub fn group_activities(phases: &[ActivityPhase]) -> Vec<ActivityRun> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < phases.len() {
        let Some(kind) = phases[index].group() else {
            runs.push(ActivityRun::Single(index));
            index += 1;
            continue;
        };
        // Extend over same-phase calls and the thinking between them; the
        // run ends at its last call, so trailing thinking stays outside.
        let mut end = index + 1;
        let mut last_member = index;
        let mut members = 1;
        while end < phases.len() {
            match phases[end] {
                ActivityPhase::Reasoning => {}
                other if other.group() == Some(kind) => {
                    last_member = end;
                    members += 1;
                }
                _ => break,
            }
            end += 1;
        }
        if members >= 2 {
            runs.push(ActivityRun::Group {
                kind,
                members: index..last_member + 1,
            });
            index = last_member + 1;
        } else {
            runs.push(ActivityRun::Single(index));
            index += 1;
        }
    }
    runs
}

/// The locale keys for an activity's verb: `(while running, once done)`.
pub fn verb_keys(kind: ActivityKind) -> (&'static str, &'static str) {
    match kind {
        ActivityKind::Reasoning => ("activity.verb_thinking", "activity.verb_thought"),
        ActivityKind::FileRead => ("activity.verb_reading", "activity.verb_read"),
        ActivityKind::FileSearch | ActivityKind::Search => {
            ("activity.verb_searching", "activity.verb_searched")
        }
        ActivityKind::FileList => ("activity.verb_listing", "activity.verb_listed"),
        ActivityKind::FileChange => ("activity.verb_editing", "activity.verb_edited"),
        ActivityKind::Command => ("activity.verb_running", "activity.verb_ran"),
        ActivityKind::Plan => ("activity.verb_planning", "activity.verb_planned"),
        ActivityKind::Tool => ("activity.verb_calling", "activity.verb_called"),
    }
}

/// What a shell command reads, when all it does is read: the bucket of its
/// first reading step. `None` for anything that writes, or runs something
/// this cannot vouch for — that is terminal work.
pub fn classify_shell(command: &str) -> Option<ExploreBucket> {
    classify_script(command, 0)
}

fn classify_script(script: &str, depth: usize) -> Option<ExploreBucket> {
    if depth > 3 {
        return None;
    }
    let mut first = None;
    for segment in split_segments(script) {
        match classify_segment(&segment, depth)? {
            Step::Neutral => {}
            Step::Read(bucket) => {
                first.get_or_insert(bucket);
            }
        }
    }
    first
}

enum Step {
    /// Changes nothing and reads nothing: `cd`, `echo`, an assignment.
    Neutral,
    Read(ExploreBucket),
}

/// `None` means the segment writes or is not known to be read-only.
fn classify_segment(segment: &str, depth: usize) -> Option<Step> {
    if writes_through_redirect(segment) {
        return None;
    }
    let tokens = tokenize(segment);
    let mut tokens = tokens.as_slice();
    // Leading `NAME=value` assignments configure the command, nothing more.
    while let Some(first) = tokens.first()
        && is_assignment(first)
    {
        tokens = &tokens[1..];
    }
    let Some(program) = tokens.first() else {
        return Some(Step::Neutral);
    };
    let verb = program_name(program);
    let args = &tokens[1..];

    // A shell running a script is the script.
    if let Some(script) = wrapped_script(&verb, args) {
        return classify_script(&script, depth + 1).map(Step::Read);
    }

    let has = |flag: &str| args.iter().any(|arg| arg.eq_ignore_ascii_case(flag));
    let step = match verb.as_str() {
        "cd" | "pushd" | "popd" | "set-location" | "sl" | "echo" | "printf" | "true" | "export" => {
            Step::Neutral
        }
        "rg" | "grep" | "egrep" | "fgrep" | "ag" | "ack" | "fd" | "select-string" | "sls"
        | "findstr" => Step::Read(ExploreBucket::Search),
        "find" => {
            if ["-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint"]
                .iter()
                .any(|flag| has(flag))
            {
                return None;
            }
            Step::Read(ExploreBucket::Search)
        }
        "ls" | "ll" | "la" | "tree" | "dir" | "gci" | "get-childitem" | "pwd" | "get-location"
        | "du" => Step::Read(ExploreBucket::List),
        "cat" | "bat" | "head" | "tail" | "wc" | "stat" | "file" | "nl" | "less" | "more"
        | "get-content" | "gc" | "type" | "test-path" | "get-item" | "readlink" | "realpath"
        | "jq" | "sort" | "uniq" | "cut" => Step::Read(ExploreBucket::File),
        "sed" => {
            let quiet = args.iter().any(|arg| arg == "-n" || arg == "--quiet");
            let in_place = args
                .iter()
                .any(|arg| arg.starts_with("-i") || arg.starts_with("--in-place"));
            if !quiet || in_place {
                return None;
            }
            Step::Read(ExploreBucket::File)
        }
        "git" => git_step(args)?,
        _ => return None,
    };
    Some(step)
}

fn git_step(args: &[String]) -> Option<Step> {
    // Skip global options such as `-C dir` or `--no-pager`.
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "-C" || arg == "-c" {
            index += 2;
        } else if arg.starts_with('-') {
            index += 1;
        } else {
            break;
        }
    }
    let subcommand = args.get(index)?.as_str();
    let rest = &args[index + 1..];
    Some(match subcommand {
        "grep" => Step::Read(ExploreBucket::Search),
        "status" | "ls-files" | "ls-tree" => Step::Read(ExploreBucket::List),
        // Listing branches reads; naming one, or deleting, does not.
        "branch" => {
            let listing = rest.iter().all(|arg| {
                matches!(
                    arg.as_str(),
                    "-a" | "-r"
                        | "-v"
                        | "-vv"
                        | "--list"
                        | "--all"
                        | "--remotes"
                        | "--show-current"
                )
            });
            if !listing {
                return None;
            }
            Step::Read(ExploreBucket::List)
        }
        "show" | "diff" | "log" | "blame" | "rev-parse" | "describe" | "shortlog" => {
            Step::Read(ExploreBucket::File)
        }
        _ => return None,
    })
}

/// A shell invocation's script: `bash -lc '…'`, `pwsh -Command …`,
/// `cmd /c …`.
fn wrapped_script(verb: &str, args: &[String]) -> Option<String> {
    match verb {
        "bash" | "sh" | "zsh" | "dash" => {
            let flag = args.iter().position(|arg| {
                arg.starts_with('-') && !arg.starts_with("--") && arg.contains('c')
            })?;
            args.get(flag + 1).cloned()
        }
        "powershell" | "pwsh" => {
            let flag = args.iter().position(|arg| {
                arg.eq_ignore_ascii_case("-command") || arg.eq_ignore_ascii_case("-c")
            })?;
            let rest = &args[flag + 1..];
            (!rest.is_empty()).then(|| rest.join(" "))
        }
        "cmd" => {
            let flag = args
                .iter()
                .position(|arg| arg.eq_ignore_ascii_case("/c") || arg.eq_ignore_ascii_case("/k"))?;
            let rest = &args[flag + 1..];
            (!rest.is_empty()).then(|| rest.join(" "))
        }
        _ => None,
    }
}

/// The program a token names, lowercased, without its directory or `.exe`.
fn program_name(token: &str) -> String {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let base = base.to_ascii_lowercase();
    base.strip_suffix(".exe").map(str::to_owned).unwrap_or(base)
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !name.starts_with(|character: char| character.is_ascii_digit())
}

/// Whether the segment sends output into a file. Discarding it or merging
/// streams is not writing.
fn writes_through_redirect(segment: &str) -> bool {
    const HARMLESS: [&str; 8] = [
        ">&1",
        ">&2",
        ">/dev/null",
        "> /dev/null",
        ">$null",
        "> $null",
        ">nul",
        "> nul",
    ];
    let bytes = segment.as_bytes();
    let mut quote: Option<u8> = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None if byte == b'\'' || byte == b'"' => quote = Some(byte),
            None if byte == b'>' => {
                let rest = &segment[index..];
                // `>>` appends: the same verdict as `>`.
                let rest_single = rest.strip_prefix('>').map_or(rest, |tail| {
                    if tail.starts_with('>') {
                        &rest[1..]
                    } else {
                        rest
                    }
                });
                if !HARMLESS
                    .iter()
                    .any(|harmless| rest_single.to_ascii_lowercase().starts_with(harmless))
                {
                    return true;
                }
            }
            None => {}
        }
        index += 1;
    }
    false
}

/// Split a script into the commands it chains: `&&`, `||`, `;`, `|` and
/// newlines, outside quotes.
fn split_segments(script: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = script.chars().peekable();
    while let Some(character) = chars.next() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                } else if character == '\\' && open == '"' {
                    current.push(character);
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                    continue;
                }
                current.push(character);
            }
            None => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    current.push(character);
                }
                // `2>&1` is a redirect, not a chain.
                '&' if current.ends_with('>') => current.push(character),
                ';' | '\n' | '|' | '&' => {
                    if matches!(character, '|' | '&') && chars.peek() == Some(&character) {
                        chars.next();
                    }
                    segments.push(std::mem::take(&mut current));
                }
                _ => current.push(character),
            },
        }
    }
    segments.push(current);
    segments
        .into_iter()
        .map(|segment| segment.trim().to_owned())
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Whitespace-separated words, with quotes removed from quoted ones.
fn tokenize(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;
    for character in segment.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                in_token = true;
            }
            None if character.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None => {
                current.push(character);
                in_token = true;
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use ExploreBucket::{File, List, Search};

    #[test]
    fn reading_commands_are_exploring() {
        for (command, bucket) in [
            ("rg -n 'fn main' src", Search),
            ("grep -rn TODO .", Search),
            ("find . -name '*.rs'", Search),
            ("git grep needle", Search),
            ("Select-String -Path *.rs -Pattern foo", Search),
            ("findstr /s foo *.txt", Search),
            ("ls -la", List),
            ("tree -L 2", List),
            ("Get-ChildItem -Recurse", List),
            ("pwd", List),
            ("git status --short", List),
            ("git branch -a", List),
            ("cat Cargo.toml", File),
            ("head -n 40 src/main.rs", File),
            ("sed -n '1,80p' src/lib.rs", File),
            ("wc -l src/*.rs", File),
            ("git diff HEAD~1", File),
            ("git --no-pager log --oneline -5", File),
            ("git -C crates/core show HEAD:src/lib.rs", File),
            ("Get-Content README.md", File),
            ("type notes.txt", File),
        ] {
            assert_eq!(classify_shell(command), Some(bucket), "{command}");
        }
    }

    #[test]
    fn anything_that_writes_or_runs_is_terminal() {
        for command in [
            "cargo test",
            "npm install",
            "rm -rf target",
            "mkdir -p out",
            "sed -i 's/a/b/' file.rs",
            "sed 's/a/b/' file.rs",
            "echo hi > notes.txt",
            "cat a >> b",
            "ls | tee listing.txt",
            "find . -name '*.tmp' -delete",
            "find . -exec rm {} \\;",
            "git commit -m wip",
            "git checkout main",
            "git branch -D old",
            "git branch feature",
            "Remove-Item -Recurse out",
            "Set-Content a.txt hi",
            "cat script.sh | sh",
            "ls | xargs rm",
            "python -c 'print(1)'",
            "cd src",
            "",
        ] {
            assert_eq!(classify_shell(command), None, "{command:?}");
        }
    }

    #[test]
    fn chains_read_when_every_step_reads() {
        assert_eq!(classify_shell("cd src && ls -la"), Some(List));
        assert_eq!(classify_shell("rg foo | head -20"), Some(Search));
        assert_eq!(classify_shell("git status; git diff --stat"), Some(List));
        assert_eq!(classify_shell("cat a && cargo build"), None);
        assert_eq!(classify_shell("RUST_LOG=debug rg foo"), Some(Search));
        // A `|` or `;` inside quotes does not chain anything.
        assert_eq!(classify_shell("rg 'a|b;c' src"), Some(Search));
    }

    #[test]
    fn discarding_output_or_merging_streams_is_not_writing() {
        assert_eq!(classify_shell("rg foo 2>&1"), Some(Search));
        assert_eq!(classify_shell("ls missing 2>/dev/null"), Some(List));
        assert_eq!(classify_shell("Get-ChildItem x 2>$null"), Some(List));
        assert_eq!(classify_shell("dir 2>nul"), Some(List));
        // A `>` inside quotes is text.
        assert_eq!(classify_shell("rg '>' src"), Some(Search));
    }

    #[test]
    fn a_shell_running_a_script_is_the_script() {
        assert_eq!(classify_shell("bash -lc 'rg foo src'"), Some(Search));
        assert_eq!(classify_shell("/bin/sh -c \"ls -la\""), Some(List));
        assert_eq!(
            classify_shell("pwsh -NoProfile -Command Get-Content a.txt"),
            Some(File)
        );
        assert_eq!(
            classify_shell("powershell.exe -Command \"Get-ChildItem\""),
            Some(List)
        );
        assert_eq!(classify_shell("cmd /c dir"), Some(List));
        assert_eq!(classify_shell("bash -lc 'cargo test'"), None);
        assert_eq!(classify_shell("C:\\Tools\\rg.exe foo"), Some(Search));
    }

    #[test]
    fn tools_and_commands_map_to_phases() {
        assert_eq!(
            phase(ActivityKind::FileRead, None),
            ActivityPhase::Explore(File)
        );
        assert_eq!(
            phase(ActivityKind::FileSearch, None),
            ActivityPhase::Explore(Search)
        );
        assert_eq!(
            phase(ActivityKind::Command, Some("ls")),
            ActivityPhase::Explore(List)
        );
        assert_eq!(
            phase(ActivityKind::Command, Some("cargo build")),
            ActivityPhase::Terminal
        );
        assert_eq!(phase(ActivityKind::Command, None), ActivityPhase::Terminal);
        assert_eq!(phase(ActivityKind::FileChange, None), ActivityPhase::Other);
        assert_eq!(phase(ActivityKind::Search, None), ActivityPhase::Other);
        assert_eq!(
            phase(ActivityKind::Reasoning, None),
            ActivityPhase::Reasoning
        );
    }

    const READ: ActivityPhase = ActivityPhase::Explore(File);
    const SEARCH: ActivityPhase = ActivityPhase::Explore(Search);
    const RUN: ActivityPhase = ActivityPhase::Terminal;
    const THINK: ActivityPhase = ActivityPhase::Reasoning;
    const EDIT: ActivityPhase = ActivityPhase::Other;

    #[test]
    fn a_group_forms_from_the_second_call_of_a_phase() {
        assert_eq!(group_activities(&[READ]), [ActivityRun::Single(0)]);
        assert_eq!(
            group_activities(&[READ, SEARCH, READ, EDIT, RUN]),
            [
                ActivityRun::Group {
                    kind: GroupKind::Explore,
                    members: 0..3
                },
                ActivityRun::Single(3),
                ActivityRun::Single(4),
            ]
        );
        assert_eq!(
            group_activities(&[RUN, RUN, READ]),
            [
                ActivityRun::Group {
                    kind: GroupKind::Terminal,
                    members: 0..2
                },
                ActivityRun::Single(2),
            ]
        );
    }

    #[test]
    fn thinking_between_calls_joins_the_group_but_not_at_its_edges() {
        assert_eq!(
            group_activities(&[THINK, READ, THINK, READ, THINK]),
            [
                ActivityRun::Single(0),
                ActivityRun::Group {
                    kind: GroupKind::Explore,
                    members: 1..4
                },
                ActivityRun::Single(4),
            ]
        );
        // One call with thinking after it is still one call.
        assert_eq!(
            group_activities(&[READ, THINK, RUN]),
            [
                ActivityRun::Single(0),
                ActivityRun::Single(1),
                ActivityRun::Single(2),
            ]
        );
    }
}
