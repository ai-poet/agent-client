//! Sidebar rows glide to their new places instead of jumping.
//!
//! Fork addition, after Zeron's sidebar resort (crates/ui/src/shell.rs; MIT,
//! Copyright (c) 2026 Wing). The sidebar keeps upstream's ordering — running
//! tasks and tasks that need the person rise to the top — and that ordering
//! moves rows under the pointer. Each time the row list changes, every row
//! that is in both lists starts drawn where it was and eases into its new
//! slot over [`RESORT`]; rows that arrive (a group unfolding, a new task) fade
//! in. Rows have fixed heights, so both positions come from arithmetic, not
//! from measuring.
//!
//! The offset is a relative `top` applied after layout, so the list's
//! measured heights and scroll position never see it. While anything moves,
//! the sidebar pane leases the shared pulse clock rather than requesting
//! display-rate frames: a resort usually happens as a turn starts streaming,
//! when nothing may run at display rate (docs/performance.md). Under reduce
//! motion rows jump, and a change that replaces most of the list — switching
//! views, opening the archive — jumps too.

use std::time::Instant;

use super::sidebar::SidebarRow;
use super::*;
use crate::ui::motion::pulse_lease;
use crate::ui::motion_kit::{FADE_QUICK, RESORT, Tween};

/// What identifies a row across two snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowKey {
    Session(Uuid),
    Header(SidebarGroup),
    ShowMore(SidebarGroup),
}

fn row_key(row: SidebarRow) -> Option<RowKey> {
    match row {
        SidebarRow::Session(id) => Some(RowKey::Session(id)),
        SidebarRow::Header(group, _) => Some(RowKey::Header(group)),
        SidebarRow::ShowMore(group) => Some(RowKey::ShowMore(group)),
        SidebarRow::Search
        | SidebarRow::Toolbar
        | SidebarRow::Empty(_)
        | SidebarRow::GroupSpacer => None,
    }
}

#[derive(Default)]
pub(super) struct SidebarResort {
    slides: Vec<(RowKey, Tween)>,
    arrivals: Vec<(RowKey, Instant)>,
}

impl SidebarResort {
    /// From `sync_sidebar_rows`, before the cached snapshot is replaced.
    /// `heights` gives each row's fixed height; `limit` is the farthest a row
    /// may glide (the viewport) before it simply jumps.
    pub(super) fn rows_changed(
        &mut self,
        old: &[SidebarRow],
        new: &[SidebarRow],
        heights: impl Fn(SidebarRow) -> f32,
        limit: f32,
    ) {
        let old_positions = keyed_positions(old, &heights);
        let new_positions = keyed_positions(new, &heights);
        if old_positions.is_empty() || new_positions.is_empty() {
            self.clear();
            return;
        }
        let kept = new_positions
            .iter()
            .filter(|(key, _)| position_of(&old_positions, *key).is_some())
            .count();
        if kept * 2 < new_positions.len() {
            self.clear();
            return;
        }

        let now = Instant::now();
        let mut slides = Vec::new();
        let mut arrivals = Vec::new();
        for &(key, new_y) in &new_positions {
            let Some(old_y) = position_of(&old_positions, key) else {
                arrivals.push((key, now));
                continue;
            };
            let drawn = old_y + self.offset(key).unwrap_or(0.0);
            let start = drawn - new_y;
            if start.abs() >= 0.5 && start.abs() <= limit.max(1.0) {
                slides.push((key, Tween::new(start, RESORT)));
            }
        }
        // A row still fading in from the last change keeps its fade.
        for &(key, started) in &self.arrivals {
            if arrivals.iter().all(|(arrived, _)| *arrived != key)
                && position_of(&new_positions, key).is_some()
            {
                arrivals.push((key, started));
            }
        }
        self.slides = slides;
        self.arrivals = arrivals;
    }

    fn clear(&mut self) {
        self.slides.clear();
        self.arrivals.clear();
    }

    /// The row's drawn offset from its slot, while it glides.
    fn offset(&self, key: RowKey) -> Option<f32> {
        self.slides
            .iter()
            .find(|(slid, _)| *slid == key)
            .and_then(|(_, tween)| tween.value_toward(0.0))
    }

    /// The row's opacity, while it fades in.
    fn opacity(&self, key: RowKey) -> Option<f32> {
        self.arrivals
            .iter()
            .find(|(arrived, _)| *arrived == key)
            .and_then(|(_, started)| FADE_QUICK.progress_after(started.elapsed()))
    }

    fn retire_finished(&mut self) {
        self.slides.retain(|(_, tween)| tween.is_running());
        self.arrivals
            .retain(|(_, started)| started.elapsed() < FADE_QUICK.total());
    }
}

fn keyed_positions(rows: &[SidebarRow], heights: &impl Fn(SidebarRow) -> f32) -> Vec<(RowKey, f32)> {
    let mut y = 0.0;
    let mut positions = Vec::new();
    for &row in rows {
        if let Some(key) = row_key(row) {
            positions.push((key, y));
        }
        y += heights(row);
    }
    positions
}

fn position_of(positions: &[(RowKey, f32)], key: RowKey) -> Option<f32> {
    positions
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, y)| *y)
}

impl Waku {
    /// Wrap one rendered sidebar row in its glide or fade, if it has one.
    pub(super) fn sidebar_resort_row(
        &self,
        row: Option<SidebarRow>,
        element: AnyElement,
        window: &Window,
        cx: &mut App,
    ) -> AnyElement {
        let Some(key) = row.and_then(row_key) else {
            return element;
        };
        let mut resort = self.fork_motion.sidebar_resort.borrow_mut();
        if cx.reduce_motion() {
            resort.clear();
            return element;
        }
        resort.retire_finished();
        let offset = resort.offset(key);
        let opacity = resort.opacity(key);
        if offset.is_none() && opacity.is_none() {
            return element;
        }
        drop(resort);
        pulse_lease(window.current_view(), cx);
        div()
            .relative()
            .when_some(offset, |wrapper, offset| wrapper.top(px(offset)))
            .when_some(opacity, |wrapper, opacity| wrapper.opacity(opacity))
            .child(element)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(n: u128) -> SidebarRow {
        SidebarRow::Session(Uuid::from_u128(n))
    }

    fn key(n: u128) -> RowKey {
        RowKey::Session(Uuid::from_u128(n))
    }

    fn height(_: SidebarRow) -> f32 {
        30.0
    }

    #[test]
    fn a_row_that_rises_starts_where_it_was_drawn() {
        let mut resort = SidebarResort::default();
        resort.rows_changed(
            &[session(1), session(2), session(3)],
            &[session(3), session(1), session(2)],
            height,
            1000.0,
        );
        let risen = resort.offset(key(3)).expect("the risen row glides");
        assert!((risen - 60.0).abs() < 0.5, "starts 60px below its slot, got {risen}");
        let pushed = resort.offset(key(1)).expect("the rows it passed glide too");
        assert!((pushed + 30.0).abs() < 0.5, "starts 30px above, got {pushed}");
    }

    #[test]
    fn the_first_fill_and_a_wholesale_change_do_not_animate() {
        let mut resort = SidebarResort::default();
        resort.rows_changed(&[], &[session(1), session(2)], height, 1000.0);
        assert!(resort.slides.is_empty() && resort.arrivals.is_empty());

        resort.rows_changed(
            &[session(1), session(2)],
            &[session(3), session(4), session(5), session(1)],
            height,
            1000.0,
        );
        assert!(resort.slides.is_empty() && resort.arrivals.is_empty());
    }

    #[test]
    fn a_move_farther_than_the_viewport_jumps() {
        let mut resort = SidebarResort::default();
        let old: Vec<_> = (1..=40).map(session).collect();
        let mut new = old.clone();
        let last = new.pop().unwrap();
        new.insert(0, last);
        resort.rows_changed(&old, &new, height, 300.0);
        assert_eq!(resort.offset(key(40)), None, "1170px is past a 300px viewport");
        assert!(resort.offset(key(1)).is_some(), "its neighbours shift 30px and glide");
    }

    #[test]
    fn rows_that_arrive_fade_in_and_rows_that_stay_put_do_not_move() {
        let mut resort = SidebarResort::default();
        resort.rows_changed(
            &[session(1), session(2)],
            &[session(1), session(2), session(3)],
            height,
            1000.0,
        );
        assert!(resort.opacity(key(3)).is_some());
        assert_eq!(resort.offset(key(1)), None);
        assert_eq!(resort.offset(key(2)), None);
    }

    #[test]
    fn a_resort_interrupting_a_glide_starts_from_the_drawn_position() {
        let mut resort = SidebarResort::default();
        let a = [session(1), session(2), session(3)];
        let b = [session(3), session(1), session(2)];
        let c = [session(1), session(3), session(2)];
        resort.rows_changed(&a, &b, height, 1000.0);
        // Row 3 sits in the top slot but is still drawn 60px down.
        resort.rows_changed(&b, &c, height, 1000.0);
        let offset = resort.offset(key(3)).expect("still gliding");
        assert!(
            (offset - 30.0).abs() < 1.0,
            "leaves from where it is drawn (60) for its new slot (30), got {offset}"
        );
        // Sent straight back to where it is drawn, it has nowhere to go.
        resort.rows_changed(&c, &a, height, 1000.0);
        assert_eq!(resort.offset(key(3)), None);
    }
}
