use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::composer::{FileEntry, SlashCommand};
use crate::git::{AgentInvocation, BranchSnapshot, CommitSnapshot, CreatedWorktree};
use crate::model::{Checkpoint, ProviderKind};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDiffSource {
    LastTurn {
        session_id: Uuid,
        turn_id: Uuid,
        turn_count: usize,
    },
    Uncommitted,
    Unstaged,
    Staged,
    Committed,
    Branch,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffData {
    pub source: ReviewDiffSource,
    pub numstat: String,
    pub patch: String,
    pub complete_context: bool,
}

/// The error text an undo fails with when the files moved after the person
/// looked at the plan. The app matches it to ask them to look again.
pub const TURN_UNDO_STALE: &str = "waku:turn-undo-stale";

/// Why a file a turn changed is not put back by undoing it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UndoReason {
    /// Changed again after the turn ended; putting it back would lose that.
    ChangedSinceTurn,
    /// Written by a command rather than edited through a tool.
    ShellWritten,
    /// A submodule: its content belongs to another repository.
    Submodule,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UndoFile {
    pub path: String,
    pub reason: UndoReason,
}

/// What undoing one turn's file changes would do, file by file. Paths are
/// relative to the repository root, `/`-separated.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnUndoPlan {
    /// Files that go back to how they were before the turn.
    pub safe: Vec<String>,
    /// Files the turn changed that cannot go back without losing later work.
    pub blocked: Vec<UndoFile>,
    /// Files the turn changed that undo leaves alone by design.
    pub ignored: Vec<UndoFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeEntry {
    pub relative_path: String,
    #[ts(type = "string")]
    pub absolute_path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub depth: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkspaceOperation {
    ListTree {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string[]")]
        expanded_paths: Vec<PathBuf>,
    },
    BrowseDirectory {
        #[ts(type = "string | null")]
        path: Option<PathBuf>,
    },
    ReadTextFile {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
    },
    WriteTextFile {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
        content: String,
    },
    ListProjectFiles {
        #[ts(type = "string")]
        root: PathBuf,
        cap: usize,
    },
    DiscoverSlashCommands {
        provider: ProviderKind,
        #[ts(type = "string")]
        project_root: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binary_override: Option<String>,
    },
    CreateProjectlessWorkspace {
        prompt: Option<String>,
    },
    MigrateProjectlessWorkspace {
        #[ts(type = "string")]
        path: PathBuf,
    },
    InspectBranches {
        #[ts(type = "string")]
        cwd: PathBuf,
    },
    CheckoutBranch {
        #[ts(type = "string")]
        cwd: PathBuf,
        branch: String,
        create: bool,
    },
    CreateWorktree {
        #[ts(type = "string")]
        project_path: PathBuf,
        project_id: Uuid,
        session_id: Uuid,
        prompt: String,
        base_branch: Option<String>,
    },
    InspectCommit {
        #[ts(type = "string")]
        cwd: PathBuf,
    },
    GenerateCommitMessage {
        #[ts(type = "string")]
        cwd: PathBuf,
        include_unstaged: bool,
        invocation: AgentInvocation,
    },
    Commit {
        #[ts(type = "string")]
        cwd: PathBuf,
        message: String,
        include_unstaged: bool,
        push: bool,
    },
    Push {
        #[ts(type = "string")]
        cwd: PathBuf,
    },
    CaptureTurnStart {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
        turn_count: usize,
    },
    CaptureTurn {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
        turn_count: usize,
    },
    CaptureRef {
        #[ts(type = "string")]
        cwd: PathBuf,
        git_ref: String,
    },
    RestoreRef {
        #[ts(type = "string")]
        cwd: PathBuf,
        git_ref: String,
    },
    HasRef {
        #[ts(type = "string")]
        cwd: PathBuf,
        git_ref: String,
    },
    SessionTurnRefs {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
    },
    DeleteRef {
        #[ts(type = "string")]
        cwd: PathBuf,
        git_ref: String,
    },
    DeleteTurnRefsAfter {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
        retained_turn_count: usize,
        previous_turn_count: usize,
    },
    DeleteSessionRefs {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
    },
    /// Remove the transcript the built-in agent stored for a session.
    /// `transcript_id` is the `ProviderResumeCursor::Native` session id, which
    /// names the file; it is not Waku's session id.
    DeleteAgentTranscript {
        transcript_id: String,
    },
    CopySessionRefs {
        #[ts(type = "string")]
        cwd: PathBuf,
        source_session_id: Uuid,
        target_session_id: Uuid,
        through_turn_count: usize,
    },
    CollectReviewDiff {
        #[ts(type = "string")]
        cwd: PathBuf,
        source: ReviewDiffSource,
    },
    /// Work out which of a turn's changed files can go back. `edited_paths`
    /// are the files the agent edited through tools (repository-relative);
    /// empty when the provider does not report them.
    PlanTurnUndo {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
        turn_count: usize,
        edited_paths: Vec<String>,
    },
    /// Put the files of a plan back, all or none. `expected_safe` is the
    /// plan the person confirmed; if it no longer holds nothing is written
    /// and the request fails with [`TURN_UNDO_STALE`].
    ApplyTurnUndo {
        #[ts(type = "string")]
        cwd: PathBuf,
        session_id: Uuid,
        turn_count: usize,
        edited_paths: Vec<String>,
        expected_safe: Vec<String>,
    },
    /// Fork addition: the Files panel's edits. Paths are relative to `root`
    /// on the daemon host; each answers `Ack` and fails rather than replace
    /// anything that is already there.
    CreateFile {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
    },
    CreateDirectory {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
    },
    RenamePath {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        from: PathBuf,
        #[ts(type = "string")]
        to: PathBuf,
    },
    /// Moves a file or directory to the daemon host's recycle bin.
    TrashPath {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
    },
    /// Fork addition: a file the Files panel shows as something other than
    /// text — a picture, a spreadsheet's cells, a document's words. See
    /// [`preview_kind`]. Answers [`WorkspaceResult::FilePreview`].
    PreviewFile {
        #[ts(type = "string")]
        root: PathBuf,
        #[ts(type = "string")]
        relative_path: PathBuf,
    },
}

/// Fork addition: how the Files panel shows a file, by its extension.
/// `None` is text, which the editor shows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PreviewKind {
    Image,
    Table,
    Document,
    /// Recognised, but nothing in the app can draw it — a PDF, an archive,
    /// audio or video. Shown as facts and an "open" button.
    Binary,
}

pub fn preview_kind(path: &str) -> Option<PreviewKind> {
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tif" | "tiff" => {
            PreviewKind::Image
        }
        "xlsx" | "xlsm" | "xlsb" | "xls" | "xla" | "xlam" | "ods" | "csv" | "tsv" => {
            PreviewKind::Table
        }
        "docx" | "pptx" | "odt" | "odp" => PreviewKind::Document,
        "pdf" | "doc" | "ppt" | "zip" | "7z" | "rar" | "gz" | "tgz" | "xz" | "bz2" | "tar"
        | "exe" | "dll" | "so" | "dylib" | "bin" | "msi" | "dmg" | "pkg" | "deb" | "rpm"
        | "apk" | "jar" | "class" | "wasm" | "o" | "a" | "lib" | "pdb" | "mp3" | "wav"
        | "flac" | "ogg" | "m4a" | "aac" | "mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv"
        | "ttf" | "otf" | "woff" | "woff2" | "psd" | "ai" | "sketch" | "fig" | "heic"
        | "avif" | "sqlite" | "db" | "parquet" | "pyc" => PreviewKind::Binary,
        _ => return None,
    })
}

/// Fork addition: what [`WorkspaceOperation::PreviewFile`] read.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilePreview {
    Image {
        /// `png`, `jpeg`, `svg`… — the format to decode it as.
        format: String,
        /// The file's bytes, base64.
        data: String,
        size: u64,
    },
    Table {
        sheets: Vec<PreviewSheet>,
        size: u64,
    },
    Document {
        text: String,
        /// Only the first part of a long document is sent.
        truncated: bool,
        size: u64,
    },
    /// Nothing to draw: too large, a format the app cannot show, or one it
    /// could not read. `reason` is for the person.
    Unavailable {
        size: u64,
        reason: Option<String>,
    },
}

/// One sheet of a spreadsheet, as much of it as a preview shows.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSheet {
    pub name: String,
    /// Where `rows` starts in the sheet, zero-based: a spreadsheet's used
    /// range need not begin at A1.
    #[serde(default)]
    pub first_row: usize,
    #[serde(default)]
    pub first_column: usize,
    pub rows: Vec<Vec<String>>,
    /// How big the sheet really is, when `rows` stops short of it.
    pub total_rows: usize,
    pub total_columns: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkspaceResult {
    Ack,
    WorkingTree {
        entries: Vec<WorkingTreeEntry>,
    },
    Directory {
        #[ts(type = "string")]
        path: PathBuf,
        #[ts(type = "string | null")]
        parent: Option<PathBuf>,
        #[ts(type = "string")]
        home: PathBuf,
        #[ts(type = "string")]
        filesystem_root: PathBuf,
        entries: Vec<WorkingTreeEntry>,
    },
    TextFile {
        content: String,
    },
    ProjectFiles {
        entries: Vec<FileEntry>,
    },
    SlashCommands {
        commands: Vec<SlashCommand>,
    },
    ProjectlessWorkspace {
        #[ts(type = "string")]
        cwd: PathBuf,
    },
    Branches {
        snapshot: Option<BranchSnapshot>,
    },
    BranchChanged {
        snapshot: BranchSnapshot,
    },
    WorktreeCreated {
        worktree: CreatedWorktree,
    },
    CommitSnapshot {
        snapshot: CommitSnapshot,
    },
    CommitMessage {
        message: String,
    },
    Checkpoint {
        checkpoint: Checkpoint,
    },
    Bool {
        value: bool,
    },
    TurnRefs {
        turn_counts: Vec<usize>,
    },
    ReviewDiff {
        data: ReviewDiffData,
    },
    TurnUndoPlan {
        plan: TurnUndoPlan,
    },
    TurnUndone {
        restored: Vec<String>,
    },
    /// Fork addition: the answer to [`WorkspaceOperation::PreviewFile`].
    FilePreview {
        preview: FilePreview,
    },
}
