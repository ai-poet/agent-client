//! Line-ending detection and preservation for the file-editing tools.
//!
//! Issue #225: the edit tools normalized `\r\n` -> `\n` for matching and then
//! wrote the LF-normalized content back, silently rewriting *every* line ending
//! of a CRLF file to LF. That produces a massive spurious diff and corrupts
//! files that must stay CRLF (this repo's own `crates/tui/src/lib.rs` is
//! intentionally CRLF).
//!
//! These helpers let a tool match/replace on an LF-normalized view while
//! writing back with the file's original line endings, so only the lines an
//! edit actually changes ever differ.
//!
//! The matching/offset-mapping approach follows Sailry `node-runtime`
//! `files/edit/text.rs`: matching runs against an LF-normalized view of the
//! file, and every normalized match span is mapped back onto original byte
//! offsets before any bytes are spliced, so untouched regions keep their exact
//! bytes. Multi-edit batches match every `old_string` against the *original*
//! content and are applied in one ascending pass, with overlap detection
//! (`file edit matches overlap` in Sailry).
//!
//! The bounded edit excerpts follow Sailry `node-runtime`
//! `files/edit/excerpt.rs`: numbered regions with a few lines of context
//! around each replacement, under hunk/line/total byte budgets.

use std::ops::Range;

use serde_json::{json, Value};

/// The dominant line ending of a text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Unix `\n`.
    Lf,
    /// Windows `\r\n`.
    Crlf,
}

impl LineEnding {
    /// Detect the dominant line ending of `content`.
    ///
    /// CRLF is reported only when it is the majority of the file's line
    /// endings, so a predominantly-CRLF file stays CRLF and a predominantly-LF
    /// file (or one with only a stray `\r\n`) stays LF. A file with no newline
    /// at all defaults to [`LineEnding::Lf`].
    pub fn detect(content: &str) -> LineEnding {
        let crlf = content.matches("\r\n").count();
        if crlf == 0 {
            return LineEnding::Lf;
        }
        // Every `\r\n` contributes one `\n`, so bare LF endings are the total
        // `\n` count minus the CRLF count.
        let bare_lf = content.matches('\n').count() - crlf;
        if crlf >= bare_lf {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }

    /// The byte sequence for this line ending.
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Render LF-normalized `content` with this line ending.
    ///
    /// `content` must already use `\n` for every line ending (as produced by
    /// replacing `\r\n` with `\n`). For [`LineEnding::Lf`] this is a no-op; for
    /// [`LineEnding::Crlf`] every `\n` becomes `\r\n`.
    pub fn apply(self, content: &str) -> String {
        match self {
            LineEnding::Lf => content.to_string(),
            LineEnding::Crlf => content.replace('\n', "\r\n"),
        }
    }
}

/// An LF-normalized byte view of a file's text, with a map from every
/// normalized byte index back to its offset in the original bytes.
///
/// A `\r\n` collapses to one `\n` whose mapped offset is the `\r`'s offset,
/// so a match found in normalized space can be spliced into the original
/// bytes without touching anything outside the matched span.
struct View<'a> {
    orig: &'a [u8],
    norm: Vec<u8>,
    /// `map[i]` is the original byte offset of `norm[i]`; the final entry is a
    /// sentinel for a match that ends at EOF.
    map: Vec<usize>,
}

impl<'a> View<'a> {
    fn new(content: &'a str) -> Self {
        let orig = content.as_bytes();
        let mut norm: Vec<u8> = Vec::with_capacity(orig.len());
        let mut map: Vec<usize> = Vec::with_capacity(orig.len() + 1);
        let mut i = 0;
        while i < orig.len() {
            map.push(i);
            if orig[i] == b'\r' && i + 1 < orig.len() && orig[i + 1] == b'\n' {
                norm.push(b'\n');
                i += 2;
            } else {
                norm.push(orig[i]);
                i += 1;
            }
        }
        map.push(orig.len()); // sentinel: index for a match that ends at EOF
        Self { orig, norm, map }
    }

    /// First index of `needle` in normalized space at or after `from`.
    fn find(&self, needle: &[u8], from: usize) -> Option<usize> {
        find_subslice(&self.norm[from..], needle).map(|rel| from + rel)
    }

    /// Non-overlapping occurrence count of `needle` in normalized space.
    fn count(&self, needle: &[u8]) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let mut count = 0;
        let mut from = 0;
        while let Some(at) = self.find(needle, from) {
            count += 1;
            from = at + needle.len();
        }
        count
    }

    /// Map a normalized-space range onto original byte offsets. UTF-8 is
    /// self-synchronizing, so a valid needle can only match on a char
    /// boundary — these offsets always land on boundaries.
    fn original_range(&self, start: usize, end: usize) -> Range<usize> {
        self.map[start]..self.map[end]
    }
}

/// Replace `old_norm` with `new_norm` inside `original`, preserving the exact
/// bytes — and therefore the line endings — of every region that is NOT
/// replaced.
///
/// Matching is line-ending agnostic: it runs against an LF-normalized view of
/// `original`, so an `old_norm` that uses `\n` still matches a region stored
/// with `\r\n`. `old_norm` and `new_norm` must already be LF-normalized; the
/// replacement text is re-rendered with `eol` so inserted lines adopt the
/// file's dominant line ending. Untouched regions keep their original bytes
/// verbatim, which is what makes a mixed-EOL file safe to edit (nothing outside
/// the replaced span is mass-converted).
///
/// Returns `(new_content, replacements_made, replaced_ranges)` where
/// `replaced_ranges` holds the ascending byte ranges within `new_content` that
/// came from replacements (the edit excerpts are built from them). When
/// `replace_all` is false only the first occurrence is replaced.
pub(crate) fn replace_preserving_eol(
    original: &str,
    old_norm: &str,
    new_norm: &str,
    eol: LineEnding,
    replace_all: bool,
) -> (String, usize, Vec<Range<usize>>) {
    let view = View::new(original);
    let old_b = old_norm.as_bytes();
    let new_rendered = eol.apply(new_norm);
    let new_b = new_rendered.as_bytes();

    let mut result: Vec<u8> = Vec::with_capacity(original.len());
    let mut copied = 0usize; // bytes of `original` already emitted
    let mut search_from = 0usize; // index into `norm`
    let mut count = 0usize;
    let mut ranges: Vec<Range<usize>> = Vec::new();

    while let Some(start) = view.find(old_b, search_from) {
        let end = start + old_b.len(); // norm index just past the match
        let orig_start = view.map[start];
        let orig_end = view.map[end];

        result.extend_from_slice(&view.orig[copied..orig_start]);
        let inserted_at = result.len();
        result.extend_from_slice(new_b);
        ranges.push(inserted_at..result.len());
        copied = orig_end;
        count += 1;
        search_from = end;

        if !replace_all {
            break;
        }
    }

    result.extend_from_slice(&view.orig[copied..]);

    // Safe: we only drop a `\r` that precedes a `\n` and splice in valid UTF-8.
    // The lossy fallback is purely defensive and should be unreachable.
    let new_content = match String::from_utf8(result) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    (new_content, count, ranges)
}

/// One replacement of a multi-edit batch (`replace_many_preserving_eol`).
/// `old`/`new` may carry `\r\n`; they are LF-normalized before matching, and
/// `new` is re-rendered with the file's dominant line ending.
pub(crate) struct EditReplacement<'a> {
    pub(crate) old: &'a str,
    pub(crate) new: &'a str,
}

/// Why one edit of a multi-edit batch could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplaceManyError {
    /// The edit at this index matched nothing in the original content.
    NotFound(usize),
    /// The edit at this index matched this many times (it must be unique).
    NotUnique(usize, usize),
    /// The edits at these two indexes match overlapping regions.
    Overlap(usize, usize),
}

/// Apply a whole batch of replacements to `original` in one pass, following
/// Sailry `node-runtime files/edit.rs`:
///
/// - Every `old` is matched against the *original* content, before any edit of
///   the batch is applied. An edit whose text only exists inside another
///   edit's replacement therefore does not match (sequential re-matching is
///   not a batch semantic).
/// - Each `old` must occur exactly once (`NotUnique`/`NotFound` otherwise).
/// - Matched spans must not overlap (`Overlap` otherwise).
/// - The spans are spliced in ascending order, so every untouched region keeps
///   its original bytes and line endings, and inserted lines adopt the file's
///   dominant `eol`.
///
/// On success returns `(new_content, replacements_made, replaced_ranges)` with
/// `replaced_ranges` ascending in `new_content`, ready for `edit_excerpt`.
pub(crate) fn replace_many_preserving_eol(
    original: &str,
    edits: &[EditReplacement],
    eol: LineEnding,
) -> Result<(String, usize, Vec<Range<usize>>), ReplaceManyError> {
    let view = View::new(original);

    // Match every edit against the original content and collect original-byte
    // ranges, tagged with the edit's index for error reporting.
    let mut matches: Vec<(usize, Range<usize>)> = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        let old_normalized = edit.old.replace("\r\n", "\n");
        let old_b = old_normalized.as_bytes();
        let Some(start) = view.find(old_b, 0) else {
            return Err(ReplaceManyError::NotFound(index));
        };
        let count = view.count(old_b);
        if count > 1 {
            return Err(ReplaceManyError::NotUnique(index, count));
        }
        matches.push((index, view.original_range(start, start + old_b.len())));
    }

    // Ascending order makes the overlap scan a single pass over neighbours;
    // two spans that start together overlap unless both are empty (impossible
    // here: every `old` is non-empty by contract of the callers).
    matches.sort_by(|a, b| (a.1.start, a.1.end).cmp(&(b.1.start, b.1.end)));
    for pair in matches.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        if next.1.start < prev.1.end {
            return Err(ReplaceManyError::Overlap(prev.0, next.0));
        }
    }

    let mut result: Vec<u8> = Vec::with_capacity(original.len());
    let mut copied = 0usize;
    let mut ranges: Vec<Range<usize>> = Vec::with_capacity(matches.len());
    for (index, range) in matches {
        result.extend_from_slice(&view.orig[copied..range.start]);
        let new_rendered = eol.apply(&edits[index].new.replace("\r\n", "\n"));
        let inserted_at = result.len();
        result.extend_from_slice(new_rendered.as_bytes());
        ranges.push(inserted_at..result.len());
        copied = range.end;
    }
    result.extend_from_slice(&view.orig[copied..]);

    // Safe for the same reason as in `replace_preserving_eol`.
    let new_content = match String::from_utf8(result) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    let count = ranges.len();
    Ok((new_content, count, ranges))
}

// ---------------------------------------------------------------------------
// Edit excerpts
// ---------------------------------------------------------------------------
//
// Bounded numbered regions around the replacements in an edited file, so the
// caller can verify an edit without reading the file again. Ported from Sailry
// `node-runtime files/edit/excerpt.rs`.

/// Unchanged lines shown before and after each edited region.
const EXCERPT_CONTEXT: usize = 3;
const EXCERPT_MAX_HUNKS: usize = 16;
const EXCERPT_HUNK_LINES: usize = 60;
const EXCERPT_LINE_BYTES: usize = 1024;
const EXCERPT_TOTAL_BYTES: usize = 12 * 1024;

/// One numbered line of an excerpt, in the same `FileLine` shape Sailry
/// protocol uses (`number` is one-based, `text` keeps its line ending unless
/// shortened).
pub(crate) struct ExcerptLine {
    pub(crate) number: usize,
    pub(crate) text: String,
}

/// Consecutive lines of the edited file, with a few lines of context.
pub(crate) struct ExcerptHunk {
    pub(crate) lines: Vec<ExcerptLine>,
    /// Lines past the hunk budget or long line text were left out.
    pub(crate) shortened: bool,
}

/// A bounded set of hunks around the edited regions of one file.
pub(crate) struct EditExcerpt {
    pub(crate) hunks: Vec<ExcerptHunk>,
    /// Edited regions left out by the excerpt budget.
    pub(crate) omitted: usize,
}

impl EditExcerpt {
    /// The `FileExcerpt` shape Sailry protocol defines, as plain JSON (this
    /// crate must not depend on sailry-protocol).
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "hunks": self
                .hunks
                .iter()
                .map(|hunk| json!({
                    "lines": hunk
                        .lines
                        .iter()
                        .map(|line| json!({"number": line.number, "text": line.text}))
                        .collect::<Vec<_>>(),
                    "shortened": hunk.shortened,
                }))
                .collect::<Vec<_>>(),
            "omitted": self.omitted,
        })
    }

    /// Plain-text rendering of the excerpt, for tool results that have no
    /// structured projection (numbered lines, mirroring the engine's
    /// `file_written` preview).
    pub(crate) fn render_text(&self) -> String {
        let mut text = String::new();
        for hunk in &self.hunks {
            let (Some(first), Some(last)) = (hunk.lines.first(), hunk.lines.last()) else {
                continue;
            };
            text.push_str(&format!("Lines {}–{}\n", first.number, last.number));
            for line in &hunk.lines {
                let content = line
                    .text
                    .strip_suffix('\n')
                    .map(|stripped| stripped.strip_suffix('\r').unwrap_or(stripped))
                    .unwrap_or(&line.text);
                text.push_str(&format!("{}\t{}\n", line.number, content));
            }
            if hunk.shortened {
                text.push_str("[Some lines or text in this region are omitted]\n");
            }
        }
        if self.omitted > 0 {
            text.push_str(&format!(
                "{} more edited region(s) omitted; read the file to see them\n",
                self.omitted
            ));
        }
        text
    }
}

/// Build the bounded excerpt of `text` around `changes`, the ascending byte
/// ranges of the replacements within `text` (as returned by the
/// `replace_*_preserving_eol` helpers).
pub(crate) fn edit_excerpt(text: &str, changes: &[Range<usize>]) -> EditExcerpt {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .filter(|start| *start < text.len())
        .collect();
    let mut excerpt = EditExcerpt {
        hunks: Vec::new(),
        omitted: 0,
    };
    if starts.is_empty() {
        return excerpt;
    }
    let line = |position: usize| starts.partition_point(|start| *start <= position) - 1;
    // Merge overlapping/adjacent context windows into one span per region.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for change in changes {
        let first = line(change.start.min(text.len() - 1));
        let last = if change.end > change.start {
            line(change.end - 1)
        } else {
            first
        };
        let span = (
            first.saturating_sub(EXCERPT_CONTEXT),
            (last + EXCERPT_CONTEXT).min(starts.len() - 1),
        );
        match spans.last_mut() {
            Some(previous) if span.0 <= previous.1 + 1 => previous.1 = previous.1.max(span.1),
            _ => spans.push(span),
        }
    }
    let mut budget = EXCERPT_TOTAL_BYTES;
    for (index, (first, last)) in spans.iter().copied().enumerate() {
        if index == EXCERPT_MAX_HUNKS || budget == 0 {
            excerpt.omitted = spans.len() - index;
            break;
        }
        let mut hunk = ExcerptHunk {
            lines: Vec::new(),
            shortened: last - first + 1 > EXCERPT_HUNK_LINES,
        };
        for number in first..=last.min(first + EXCERPT_HUNK_LINES - 1) {
            let end = starts.get(number + 1).copied().unwrap_or(text.len());
            let full = &text[starts[number]..end];
            let mut limit = full.len().min(EXCERPT_LINE_BYTES).min(budget);
            while !full.is_char_boundary(limit) {
                limit -= 1;
            }
            if limit == 0 && !full.is_empty() {
                hunk.shortened = true;
                break;
            }
            hunk.shortened |= limit < full.len();
            budget -= limit;
            hunk.lines.push(ExcerptLine {
                number: number + 1,
                text: full[..limit].into(),
            });
        }
        if hunk.lines.is_empty() {
            excerpt.omitted = spans.len() - index;
            break;
        }
        excerpt.hunks.push(hunk);
    }
    excerpt
}

/// Return the index of the first occurrence of `needle` within `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_pure_lf() {
        assert_eq!(LineEnding::detect("a\nb\nc\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("no newline at all"), LineEnding::Lf);
        assert_eq!(LineEnding::detect(""), LineEnding::Lf);
    }

    #[test]
    fn detect_pure_crlf() {
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\r\n"), LineEnding::Crlf);
    }

    #[test]
    fn detect_mixed_uses_majority() {
        // Majority CRLF (2 CRLF vs 1 bare LF) -> Crlf.
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\nd"), LineEnding::Crlf);
        // Majority LF (1 CRLF vs 2 bare LF) -> Lf.
        assert_eq!(LineEnding::detect("a\r\nb\nc\nd"), LineEnding::Lf);
    }

    #[test]
    fn apply_roundtrips() {
        assert_eq!(LineEnding::Lf.apply("a\nb\n"), "a\nb\n");
        assert_eq!(LineEnding::Crlf.apply("a\nb\n"), "a\r\nb\r\n");
    }

    #[test]
    fn replace_lf_stays_lf() {
        let (out, n, ranges) = replace_preserving_eol("a\nb\nc\n", "b", "B", LineEnding::Lf, false);
        assert_eq!(out, "a\nB\nc\n");
        assert_eq!(n, 1);
        assert_eq!(ranges, vec![2..3]);
    }

    #[test]
    fn replace_crlf_keeps_crlf_and_only_target_changes() {
        // LF-normalized old_string matches a CRLF region; every other EOL stays.
        let (out, n, ranges) =
            replace_preserving_eol("a\r\nb\r\nc\r\n", "b", "B", LineEnding::Crlf, false);
        assert_eq!(out, "a\r\nB\r\nc\r\n");
        assert_eq!(n, 1);
        assert_eq!(ranges, vec![3..4]);
    }

    #[test]
    fn replace_crlf_multiline_new_text_gets_crlf() {
        let (out, _, _) = replace_preserving_eol(
            "one\r\ntwo\r\nthree\r\n",
            "two",
            "TWO\nEXTRA",
            LineEnding::Crlf,
            false,
        );
        assert_eq!(out, "one\r\nTWO\r\nEXTRA\r\nthree\r\n");
    }

    #[test]
    fn replace_all_crlf() {
        let (out, n, ranges) =
            replace_preserving_eol("x\r\nx\r\nx\r\n", "x", "y", LineEnding::Crlf, true);
        assert_eq!(out, "y\r\ny\r\ny\r\n");
        assert_eq!(n, 3);
        // Each replacement is one byte; ranges are ascending and disjoint.
        assert_eq!(ranges, vec![0..1, 3..4, 6..7]);
    }

    #[test]
    fn mixed_file_leaves_untouched_eols_byte_identical() {
        // A deliberately mixed file: CRLF, then LF, then CRLF. Editing the LF
        // line must not flip the surrounding CRLF endings (and vice versa).
        let original = "a\r\nb\nc\r\n";
        let eol = LineEnding::detect(original); // Crlf is the majority (2 vs 1)
        let (out, n, _) = replace_preserving_eol(original, "b", "B", eol, false);
        // Only "b" -> "B"; the trailing `\n` after b and the CRLFs are intact.
        assert_eq!(out, "a\r\nB\nc\r\n");
        assert_eq!(n, 1);
    }

    // -----------------------------------------------------------------
    // replace_many_preserving_eol
    // -----------------------------------------------------------------

    fn edits<'a>(pairs: &'a [(&'a str, &'a str)]) -> Vec<EditReplacement<'a>> {
        pairs
            .iter()
            .map(|(old, new)| EditReplacement { old, new })
            .collect()
    }

    #[test]
    fn many_applies_disjoint_edits_in_one_pass() {
        let original = "alpha\nbeta\ngamma\ndelta\n";
        let (out, n, ranges) = replace_many_preserving_eol(
            original,
            &edits(&[("beta", "BETA"), ("delta", "DELTA")]),
            LineEnding::Lf,
        )
        .unwrap();
        assert_eq!(out, "alpha\nBETA\ngamma\nDELTA\n");
        assert_eq!(n, 2);
        assert_eq!(&out[ranges[0].clone()], "BETA");
        assert_eq!(&out[ranges[1].clone()], "DELTA");
    }

    #[test]
    fn many_matches_against_the_original_not_previous_replacements() {
        // "b" exists only inside the first edit's replacement text; a batch
        // matches the original content (Sailry semantics), so this is NotFound
        // rather than a sequential cascade to "c".
        let error =
            replace_many_preserving_eol("a\n", &edits(&[("a", "b"), ("b", "c")]), LineEnding::Lf)
                .unwrap_err();
        assert_eq!(error, ReplaceManyError::NotFound(1));
    }

    #[test]
    fn many_reports_not_unique_with_count() {
        let error = replace_many_preserving_eol("x\nx\n", &edits(&[("x", "y")]), LineEnding::Lf)
            .unwrap_err();
        assert_eq!(error, ReplaceManyError::NotUnique(0, 2));
    }

    #[test]
    fn many_detects_overlapping_matches() {
        // "beta ga" overlaps "beta"; the batch must refuse, not silently pick
        // an order.
        let error = replace_many_preserving_eol(
            "alpha beta gamma\n",
            &edits(&[("beta", "B"), ("beta ga", "X")]),
            LineEnding::Lf,
        )
        .unwrap_err();
        assert_eq!(error, ReplaceManyError::Overlap(0, 1));
    }

    #[test]
    fn many_detects_contained_matches() {
        // One edit's match wholly inside another's is an overlap too.
        let error = replace_many_preserving_eol(
            "one two three\n",
            &edits(&[("two", "2"), ("one two three", "123")]),
            LineEnding::Lf,
        )
        .unwrap_err();
        assert_eq!(error, ReplaceManyError::Overlap(1, 0));
    }

    #[test]
    fn many_crlf_file_keeps_crlf() {
        let (out, n, _) = replace_many_preserving_eol(
            "alpha\r\nbeta\r\ngamma\r\n",
            &edits(&[("alpha", "ALPHA"), ("gamma", "GAMMA")]),
            LineEnding::Crlf,
        )
        .unwrap();
        assert_eq!(out, "ALPHA\r\nbeta\r\nGAMMA\r\n");
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
        assert_eq!(n, 2);
    }

    #[test]
    fn many_adjacent_matches_are_not_an_overlap() {
        // Touching spans (end == next start) splice cleanly.
        let (out, n, _) = replace_many_preserving_eol(
            "abcdef\n",
            &edits(&[("abc", "ABC"), ("def", "DEF")]),
            LineEnding::Lf,
        )
        .unwrap();
        assert_eq!(out, "ABCDEF\n");
        assert_eq!(n, 2);
    }

    // -----------------------------------------------------------------
    // edit_excerpt
    // -----------------------------------------------------------------

    fn changed_line_numbers(excerpt: &EditExcerpt) -> Vec<usize> {
        excerpt
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter().map(|l| l.number))
            .collect()
    }

    #[test]
    fn excerpt_covers_the_replaced_region_with_context() {
        // 9 lines; replace line 5. The hunk must show lines 2..=8 (3 lines of
        // context either side), numbered as in the edited file.
        let mut text = String::new();
        for i in 1..=9 {
            text.push_str(&format!("line {i}\n"));
        }
        let change_start = text.find("line 5").unwrap();
        let change_end = change_start + "line 5".len();
        let excerpt = edit_excerpt(&text, &[change_start..change_end]);
        assert_eq!(excerpt.omitted, 0);
        assert_eq!(excerpt.hunks.len(), 1);
        assert!(!excerpt.hunks[0].shortened);
        assert_eq!(changed_line_numbers(&excerpt), vec![2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn excerpt_merges_neighbouring_changes_into_one_hunk() {
        let mut text = String::new();
        for i in 1..=12 {
            text.push_str(&format!("line {i}\n"));
        }
        // Replace lines 4 and 7: contexts (1..=7) and (4..=10) overlap, so
        // one hunk covers lines 1..=10.
        let a = text.find("line 4").unwrap();
        let b = text.find("line 7").unwrap();
        let excerpt = edit_excerpt(&text, &[a..a + 6, b..b + 6]);
        assert_eq!(excerpt.hunks.len(), 1);
        assert_eq!(changed_line_numbers(&excerpt), (1..=10).collect::<Vec<_>>());
    }

    #[test]
    fn excerpt_bounds_long_lines_and_marks_shortened() {
        let long = "x".repeat(5000);
        let text = format!("{long}\n");
        let excerpt = edit_excerpt(&text, &[0..long.len()]);
        assert_eq!(excerpt.hunks.len(), 1);
        assert!(excerpt.hunks[0].shortened);
        assert!(excerpt.hunks[0].lines[0].text.len() <= EXCERPT_LINE_BYTES);
    }

    #[test]
    fn excerpt_caps_hunks_and_counts_omitted() {
        // 40 well-separated single-line changes (10 lines apart, so their
        // context windows never merge): only the first EXCERPT_MAX_HUNKS
        // survive and the rest are counted as omitted.
        let mut text = String::new();
        let mut changes = Vec::new();
        for i in 0..400 {
            let line = format!("l{i}\n");
            if i % 10 == 0 {
                let start = text.len();
                changes.push(start..start + 2);
            }
            text.push_str(&line);
        }
        let excerpt = edit_excerpt(&text, &changes);
        assert_eq!(excerpt.hunks.len(), EXCERPT_MAX_HUNKS);
        assert_eq!(excerpt.omitted, 40 - EXCERPT_MAX_HUNKS);
    }

    #[test]
    fn excerpt_of_empty_text_is_empty() {
        let excerpt = edit_excerpt("", &[0..0]);
        assert!(excerpt.hunks.is_empty());
        assert_eq!(excerpt.omitted, 0);
    }

    #[test]
    fn excerpt_serializes_to_the_sailry_shape() {
        let text = "a\nb\nc\n";
        let start = text.find('b').unwrap();
        let excerpt = edit_excerpt(text, &[start..start + 1]);
        let value = excerpt.to_value();
        assert_eq!(value["hunks"][0]["lines"][0]["number"], 1);
        assert_eq!(value["hunks"][0]["lines"][1]["text"], "b\n");
        assert_eq!(value["hunks"][0]["shortened"], false);
        assert_eq!(value["omitted"], 0);
    }
}
