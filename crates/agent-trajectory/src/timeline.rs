//! The overview above the ledger: every cell on one of three lanes.
//!
//! Lane 0 holds the input (prompts, prompt changes), lane 1 the model
//! (requests, compactions), lane 2 the tools. In [`Mode::Sequence`] each cell
//! is one unit wide, in ledger order; in [`Mode::Duration`] cells are as wide
//! as they took and the time nothing ran is squeezed out; in [`Mode::Actual`]
//! the axis is the wall clock. A model span carries where its first token
//! arrived, so the overview can draw the wait apart from the decoding.
//!
//! Translated from DeepSeek Harness's `ui-trajectory` `timeline.ts` — MIT
//! License, Copyright (c) 2026 DeepSeek. See NOTICE.md.

use crate::snapshot::{CellDetail, CellKind, Snapshot};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Sequence,
    Duration,
    Actual,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// The cell this span draws.
    pub index: usize,
    pub lane: u8,
    pub kind: CellKind,
    pub start: f64,
    pub end: f64,
    /// Where the first token came, for a model span: the wait before it and
    /// the decoding after it are drawn apart.
    pub first_token: Option<f64>,
    pub is_error: bool,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnBoundary {
    pub turn: u32,
    pub at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub start: f64,
    pub end: f64,
    pub spans: Vec<Span>,
    pub turns: Vec<TurnBoundary>,
}

pub fn lane_for(kind: CellKind) -> u8 {
    match kind {
        CellKind::Tool | CellKind::Subtool => 2,
        CellKind::Message | CellKind::Compacted => 1,
        CellKind::System | CellKind::User => 0,
    }
}

fn first_token_of(detail: &CellDetail) -> Option<i64> {
    match detail {
        CellDetail::Request(request) => request.first_token_at,
        _ => None,
    }
}

/// The overview of `snapshot` in `mode`, or `None` with nothing to draw.
pub fn derive(snapshot: &Snapshot, mode: Mode) -> Option<Model> {
    match mode {
        Mode::Sequence => derive_sequence(snapshot),
        Mode::Duration => derive_timed(snapshot, true),
        Mode::Actual => derive_timed(snapshot, false),
    }
}

fn derive_sequence(snapshot: &Snapshot) -> Option<Model> {
    if snapshot.cells.is_empty() {
        return None;
    }
    let mut turns = Vec::new();
    let mut last_turn = None;
    let spans = snapshot
        .cells
        .iter()
        .enumerate()
        .map(|(position, cell)| {
            if cell.turn.is_some() && cell.turn != last_turn {
                last_turn = cell.turn;
                turns.push(TurnBoundary {
                    turn: cell.turn.unwrap_or_default(),
                    at: position as f64,
                });
            }
            Span {
                index: cell.index,
                lane: lane_for(cell.kind),
                kind: cell.kind,
                start: position as f64,
                end: position as f64 + 1.0,
                first_token: None,
                is_error: cell.is_error,
                running: cell.running,
            }
        })
        .collect();
    Some(Model {
        start: 0.0,
        end: snapshot.cells.len() as f64,
        spans,
        turns,
    })
}

fn derive_timed(snapshot: &Snapshot, compress_idle: bool) -> Option<Model> {
    let raw: Vec<(usize, i64, i64)> = snapshot
        .cells
        .iter()
        .filter_map(|cell| {
            let start = cell.started_at?;
            Some((cell.index, start, start + cell.duration_ms.unwrap_or(0).max(0)))
        })
        .collect();
    if raw.is_empty() {
        return None;
    }
    // Idle time removed before each span, by start order.
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by_key(|position| (raw[*position].1, raw[*position].2));
    let mut removed = vec![0i64; raw.len()];
    let mut removed_so_far = 0i64;
    let mut covered_until: Option<i64> = None;
    for position in order {
        let (_, start, end) = raw[position];
        if compress_idle
            && let Some(covered) = covered_until
            && start > covered
        {
            removed_so_far += start - covered;
        }
        removed[position] = removed_so_far;
        covered_until = Some(covered_until.map_or(end, |covered| covered.max(end)));
    }
    let origin = raw.iter().map(|(_, start, _)| *start).min().unwrap_or(0);
    let mut spans = Vec::with_capacity(raw.len());
    let mut turns: Vec<TurnBoundary> = Vec::new();
    for (position, (index, start, end)) in raw.iter().enumerate() {
        let cell = &snapshot.cells[*index];
        let offset = (removed[position] + origin) as f64;
        let span = Span {
            index: *index,
            lane: lane_for(cell.kind),
            kind: cell.kind,
            start: *start as f64 - offset,
            end: *end as f64 - offset,
            first_token: first_token_of(&cell.detail).map(|at| at as f64 - offset),
            is_error: cell.is_error,
            running: cell.running,
        };
        if let Some(turn) = cell.turn {
            match turns.iter_mut().find(|boundary| boundary.turn == turn) {
                Some(boundary) => boundary.at = boundary.at.min(span.start),
                None => turns.push(TurnBoundary { turn, at: span.start }),
            }
        }
        spans.push(span);
    }
    let start = spans.iter().map(|span| span.start).fold(f64::INFINITY, f64::min);
    let end = spans.iter().map(|span| span.end).fold(f64::NEG_INFINITY, f64::max);
    Some(Model {
        start,
        end: if end > start { end } else { start + 1.0 },
        spans,
        turns,
    })
}

/// The cells active anywhere in `[start, end]`, for filtering the ledger to
/// a dragged interval.
pub fn focus_indexes(model: &Model, start: f64, end: f64) -> Vec<usize> {
    let (start, end) = if start <= end { (start, end) } else { (end, start) };
    let mut indexes: Vec<usize> = model
        .spans
        .iter()
        .filter(|span| span.start <= end && span.end >= start)
        .map(|span| span.index)
        .collect();
    indexes.sort_unstable();
    indexes.dedup();
    indexes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Record;
    use crate::snapshot::build;
    use crate::snapshot::tests::{request, sample, turn_start};

    #[test]
    fn sequence_mode_gives_every_cell_one_unit_on_its_lane() {
        let snapshot = build(&sample());
        let model = derive(&snapshot, Mode::Sequence).unwrap();
        assert_eq!(model.end, 5.0);
        let lanes: Vec<u8> = model.spans.iter().map(|span| span.lane).collect();
        assert_eq!(lanes, [0, 0, 1, 2, 1]);
        assert_eq!(model.turns, [TurnBoundary { turn: 1, at: 0.0 }]);
    }

    #[test]
    fn duration_mode_squeezes_out_idle_time_and_actual_keeps_it() {
        let mut records = sample();
        // A second turn after a minute of nothing.
        records.push(turn_start(2, 63_000, "s1", "t1"));
        records.push(request(2, 1, 63_000, 63_200, 64_000, 10));
        let snapshot = build(&records);
        let compact = derive(&snapshot, Mode::Duration).unwrap();
        let actual = derive(&snapshot, Mode::Actual).unwrap();
        assert!(compact.end < 10_000.0, "{}", compact.end);
        assert!(actual.end >= 63_000.0, "{}", actual.end);
        // The model span keeps where its first token came.
        let first = compact.spans.iter().find(|span| span.kind == CellKind::Message).unwrap();
        assert_eq!(first.first_token.unwrap() - first.start, 400.0);
    }

    #[test]
    fn a_dragged_interval_selects_the_cells_it_touches() {
        let snapshot = build(&sample());
        let model = derive(&snapshot, Mode::Actual).unwrap();
        // Model span 0..1000, tool 1000..1300, model 1300..2000.
        assert_eq!(focus_indexes(&model, 1_100.0, 1_200.0), vec![3]);
        let all = focus_indexes(&model, 2_000.0, 0.0);
        assert!(all.contains(&2) && all.contains(&4));
        assert!(derive(&build(&[Record::Unknown]), Mode::Duration).is_none());
    }
}
