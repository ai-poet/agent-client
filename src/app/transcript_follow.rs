//! When the transcript rejoins its tail, and when the jump-to-bottom button
//! shows.
//!
//! Fork addition, after Zeron's transcript (crates/ui/src/transcript.rs;
//! MIT, Copyright (c) 2026 Wing):
//!
//! - **Rejoining.** Upstream rejoins the tail only when a scroll comes to
//!   rest exactly on it, within half a pixel, so coming most of the way back
//!   down left the reader a few pixels short and stranded. Now a scroll that
//!   is moving down rejoins within [`REJOIN_BAND`] of the tail. A scroll
//!   moving up never does, so a small nudge up from the bottom still lets go.
//! - **The button.** It appears only once the reader is [`JUMP_SHOW_DISTANCE`]
//!   away and, once shown, stays until they are within [`JUMP_HIDE_DISTANCE`],
//!   instead of blinking on at the first pixel and off again partway back
//!   down. While the tail row is unmeasured the scrollbar's estimate stands in
//!   for the distance, so a jump far up the transcript still offers the way
//!   back.

use super::*;

/// How close to the tail a downward scroll rejoins it.
const REJOIN_BAND: f32 = 70.0;
/// How far from the tail the jump-to-bottom button appears.
const JUMP_SHOW_DISTANCE: f32 = 320.0;
/// How close to the tail a shown button hides again.
const JUMP_HIDE_DISTANCE: f32 = 2.0;

impl Waku {
    /// In place of upstream's `transcript_rests_at_tail` when a scroll asks
    /// whether to rejoin the tail. `None` while the tail is unmeasured.
    pub(super) fn transcript_rejoins_tail(
        &self,
        viewport_bottom: Pixels,
        tail_bottom: Option<Pixels>,
        end_space: Pixels,
        rows: &ListState,
    ) -> Option<bool> {
        if transcript_rests_at_tail(viewport_bottom, tail_bottom, end_space)? {
            return Some(true);
        }
        let distance = f32::from(tail_bottom? + end_space - viewport_bottom);
        let y = f32::from(rows.scroll_px_offset_for_scrollbar().y);
        let moving_down = self
            .fork_motion
            .transcript_scroll_y
            .get()
            .is_some_and(|previous| y < previous - 0.5);
        Some(moving_down && distance <= REJOIN_BAND)
    }

    /// In place of upstream's `should_show_scroll_to_bottom`, with the same
    /// first six arguments. Also records this frame's scroll position for
    /// [`Self::transcript_rejoins_tail`].
    #[allow(clippy::too_many_arguments)]
    pub(super) fn transcript_jump_visibility(
        &self,
        is_scrolled: bool,
        anchor_following: bool,
        transcript_scrollable: bool,
        viewport_bottom: Pixels,
        tail_bottom: Option<Pixels>,
        end_space: Pixels,
        rows: &ListState,
    ) -> Option<bool> {
        let y = f32::from(rows.scroll_px_offset_for_scrollbar().y);
        self.fork_motion.transcript_scroll_y.set(Some(y));
        let upstream = should_show_scroll_to_bottom(
            is_scrolled,
            anchor_following,
            transcript_scrollable,
            viewport_bottom,
            tail_bottom,
            end_space,
        );
        let measured = tail_bottom.map(|tail| f32::from(tail + end_space - viewport_bottom));
        let estimated = f32::from(rows.max_offset_for_scrollbar().y) + y;
        jump_visibility(
            self.transcript_scroll_to_bottom_visible.get(),
            upstream,
            measured,
            estimated,
        )
    }
}

/// The button's next state. `upstream` is what upstream would answer
/// (`Some(false)` whenever the button has no business showing), `measured`
/// the distance to the measured tail and `estimated` the scrollbar's guess.
fn jump_visibility(
    shown: bool,
    upstream: Option<bool>,
    measured: Option<f32>,
    estimated: f32,
) -> Option<bool> {
    match (upstream, measured) {
        (Some(false), _) => Some(false),
        (_, Some(distance)) => Some(if shown {
            distance > JUMP_HIDE_DISTANCE
        } else {
            distance > JUMP_SHOW_DISTANCE
        }),
        // The tail is unmeasured: far away for certain, or mid-remeasure
        // near it, where holding the last answer avoids a blink.
        (_, None) if !shown && estimated > JUMP_SHOW_DISTANCE * 2.0 => Some(true),
        (_, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_appears_late_and_leaves_late() {
        assert_eq!(jump_visibility(false, Some(true), Some(100.0), 100.0), Some(false));
        assert_eq!(jump_visibility(false, Some(true), Some(400.0), 400.0), Some(true));
        assert_eq!(jump_visibility(true, Some(true), Some(100.0), 100.0), Some(true));
        assert_eq!(jump_visibility(true, Some(true), Some(1.0), 1.0), Some(false));
    }

    #[test]
    fn upstream_reasons_to_hide_win() {
        assert_eq!(jump_visibility(true, Some(false), Some(900.0), 900.0), Some(false));
    }

    #[test]
    fn an_unmeasured_tail_holds_unless_the_estimate_is_far() {
        assert_eq!(jump_visibility(false, None, None, 50.0), None);
        assert_eq!(jump_visibility(true, None, None, 50.0), None);
        assert_eq!(jump_visibility(false, None, None, 5_000.0), Some(true));
    }
}
