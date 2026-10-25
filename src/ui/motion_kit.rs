//! Named motion: timing curves, durations, and the entrance and exit helpers
//! built on them.
//!
//! Fork addition. Translated from Zeron's motion catalog
//! (crates/ui/src/motion.rs; MIT, Copyright (c) 2026 Wing), the same source
//! upstream's pulse clock in [`super::motion`] came from. Everything here
//! moves an element with a relative `top` inset and opacity, which apply after
//! layout, so a moving element never shifts its siblings. Helpers built on
//! `with_animation` hold still under reduce motion on their own; the
//! hand-driven [`Tween`] does not know about it, so its callers check
//! `cx.reduce_motion()` before starting one.

use std::time::{Duration, Instant};

use gpui::{AnimationElement, AnimationExt, ElementId, IntoElement, Styled, px};

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function, endpoints fixed at
/// (0,0) and (1,1). `x(t) = input` is solved by Newton iteration with a
/// bisection fallback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    fn coefficients(a: f32, b: f32) -> (f32, f32, f32) {
        let c = 3.0 * a;
        let bb = 3.0 * (b - a) - c;
        let aa = 1.0 - c - bb;
        (aa, bb, c)
    }

    fn sample_x(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        ((a * t + b) * t + c) * t
    }

    fn sample_y(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.y1, self.y2);
        ((a * t + b) * t + c) * t
    }

    fn sample_x_derivative(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        (3.0 * a * t + 2.0 * b) * t + c
    }

    fn solve_t_for_x(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..8 {
            let error = self.sample_x(t) - x;
            if error.abs() < 1e-6 {
                return t;
            }
            let slope = self.sample_x_derivative(t);
            if slope.abs() < 1e-6 {
                break;
            }
            t -= error / slope;
        }
        // x(t) is monotonic for every valid CSS curve.
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let middle = (low + high) / 2.0;
            if self.sample_x(middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) / 2.0
    }

    /// Eased output for progress `x` in [0,1], clamped. f32 rounding can push
    /// the curve a hair past 1.0 near its end, and gpui's animation element
    /// asserts its delta stays in [0,1], so the clamp is not optional.
    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        self.sample_y(self.solve_t_for_x(x)).clamp(0.0, 1.0)
    }
}

/// Fast start, long landing — CSS `cubic-bezier(0.16, 1, 0.3, 1)`.
pub const EASE_OUT_EXPO: CubicBezier = CubicBezier::new(0.16, 1.0, 0.3, 1.0);
/// CSS `ease-out`.
pub const EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.58, 1.0);
/// CSS `ease`.
pub const EASE: CubicBezier = CubicBezier::new(0.25, 0.1, 0.25, 1.0);
/// CSS `cubic-bezier(0.22, 1, 0.36, 1)`.
pub const EASE_OUT_QUINT: CubicBezier = CubicBezier::new(0.22, 1.0, 0.36, 1.0);
/// CSS `ease-in-out`: gentle start, cruise, gentle landing.
pub const EASE_IN_OUT: CubicBezier = CubicBezier::new(0.42, 0.0, 0.58, 1.0);
/// CSS `cubic-bezier(0.2, 0, 0, 1)`.
pub const EASE_OUT_SOFT: CubicBezier = CubicBezier::new(0.2, 0.0, 0.0, 1.0);

/// One catalog entry: duration, optional delay, curve. gpui animations have
/// no delay of their own, so the delay is folded into the timeline: it runs
/// for `delay + duration` and holds 0 until the delay has passed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub curve: CubicBezier,
}

impl MotionSpec {
    pub const fn new(duration_ms: u64, curve: CubicBezier) -> Self {
        Self {
            duration_ms,
            delay_ms: 0,
            curve,
        }
    }

    pub const fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// The whole timeline, delay included.
    pub fn total(&self) -> Duration {
        Duration::from_millis(self.delay_ms + self.duration_ms)
    }

    /// Eased progress for raw progress `raw` (0..1 across [`Self::total`]).
    pub fn progress(&self, raw: f32) -> f32 {
        let total = (self.delay_ms + self.duration_ms) as f32;
        if total <= 0.0 || self.duration_ms == 0 {
            return 1.0;
        }
        let t = (raw.clamp(0.0, 1.0) * total - self.delay_ms as f32) / self.duration_ms as f32;
        self.curve.eval(t.clamp(0.0, 1.0))
    }

    /// Eased progress `elapsed` into the timeline, or `None` once it is over.
    pub fn progress_after(&self, elapsed: Duration) -> Option<f32> {
        let total = self.total();
        (elapsed < total).then(|| self.progress(elapsed.as_secs_f32() / total.as_secs_f32()))
    }

    /// A one-shot gpui animation for this spec.
    pub fn animation(&self) -> gpui::Animation {
        let spec = *self;
        gpui::Animation::new(spec.total()).with_easing(move |raw| spec.progress(raw))
    }
}

/// Entrances: a 500ms fade with a 4px rise.
pub const FADE_IN: MotionSpec = MotionSpec::new(500, EASE_OUT_EXPO);
/// A quick 150ms fade.
pub const FADE_QUICK: MotionSpec = MotionSpec::new(150, EASE);
/// A menu or popover opening.
pub const MENU_IN: MotionSpec = MotionSpec::new(140, EASE);
/// A menu closing: quicker than it opened, so it gets out of the way.
pub const MENU_OUT: MotionSpec = MotionSpec::new(100, EASE);
/// A dialog appearing.
pub const DIALOG_IN: MotionSpec = MotionSpec::new(180, EASE);
/// A section folding open or shut.
pub const COLLAPSE: MotionSpec = MotionSpec::new(180, EASE_OUT);
/// A selection indicator gliding between segments.
pub const INDICATOR_SLIDE: MotionSpec = MotionSpec::new(260, EASE_OUT_SOFT);
/// List rows gliding to their new places after a reorder.
pub const RESORT: MotionSpec = MotionSpec::new(260, EASE_OUT_QUINT);
/// A scroll that jumps a long way: a fixed-length gentle glide.
pub const SCROLL_GLIDE: MotionSpec = MotionSpec::new(500, EASE_IN_OUT);

/// Entrance: fade in while rising 4px into place over [`FADE_IN`].
pub fn fade_in<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, FADE_IN.animation(), |element, t| {
        element.relative().opacity(t).top(px(4.0 * (1.0 - t)))
    })
}

/// Opacity alone over [`FADE_QUICK`].
pub fn fade_quick<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, FADE_QUICK.animation(), |element, t| element.opacity(t))
}

/// A menu opening from its trigger's side over [`MENU_IN`]. `from` is the
/// signed starting offset: negative above the resting place for a menu that
/// drops down, positive below it for one that opens upward. gpui divs have no
/// scale transform, so Zeron's 0.96 → 1 scale is approximated by the travel.
pub fn menu_in_from<E>(id: impl Into<ElementId>, from: f32, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, MENU_IN.animation(), move |element, t| {
        element
            .relative()
            .opacity(0.3 + 0.7 * t)
            .top(px(from * (1.0 - t)))
    })
}

/// A closing menu retreating toward its trigger by half the entrance travel.
/// The eased progress `t` comes from the caller's wall clock: an
/// `with_animation` clock is keyed by element id and replays from zero when
/// the element remounts, and a replay mid-exit flashes the menu back to full
/// opacity. The animation wrapper here only keeps frames coming for the exit.
pub fn menu_out_toward<E>(
    id: impl Into<ElementId>,
    toward: f32,
    t: f32,
    element: E,
) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, MENU_OUT.animation(), move |element, _| {
        element
            .relative()
            .opacity(1.0 - t)
            .top(px(toward * 0.5 * t))
    })
}

/// A dialog appearing over [`DIALOG_IN`]: a fade with a 2px rise standing in
/// for Zeron's scale.
pub fn dialog_in<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, DIALOG_IN.animation(), |element, t| {
        element.relative().opacity(t).top(px(2.0 * (1.0 - t)))
    })
}

/// A one-shot move of one value along a [`MotionSpec`], evaluated from render
/// by wall clock rather than wrapped around an element — the pattern of
/// [`super::motion::WidthTween`], for any value and curve. It survives the
/// element remounting, and a tween replaced mid-flight starts from where the
/// value is drawn, so a reversal never jumps.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    started: Instant,
    spec: MotionSpec,
}

impl Tween {
    pub fn new(from: f32, spec: MotionSpec) -> Self {
        Self {
            from,
            started: Instant::now(),
            spec,
        }
    }

    /// The value on the way to `target`, or `None` once the move is over and
    /// the caller should drop the tween and read `target` directly.
    pub fn value_toward(&self, target: f32) -> Option<f32> {
        tween_at(self.from, target, self.spec, self.started.elapsed())
    }

    /// Whether the move is still under way.
    pub fn is_running(&self) -> bool {
        self.started.elapsed() < self.spec.total()
    }
}

fn tween_at(from: f32, target: f32, spec: MotionSpec, elapsed: Duration) -> Option<f32> {
    spec.progress_after(elapsed)
        .map(|progress| from + (target - from) * progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_zero_end_at_one_and_never_leave_the_range() {
        for curve in [
            EASE_OUT_EXPO,
            EASE_OUT,
            EASE,
            EASE_OUT_QUINT,
            EASE_IN_OUT,
            EASE_OUT_SOFT,
        ] {
            assert_eq!(curve.eval(0.0), 0.0);
            assert_eq!(curve.eval(1.0), 1.0);
            assert_eq!(curve.eval(-0.5), 0.0);
            assert_eq!(curve.eval(1.5), 1.0);
            let mut previous = 0.0;
            for step in 1..=200 {
                let value = curve.eval(step as f32 / 200.0);
                assert!((0.0..=1.0).contains(&value), "{curve:?} left the range");
                assert!(value + 1e-5 >= previous, "{curve:?} went backwards");
                previous = value;
            }
        }
    }

    #[test]
    fn a_curve_matches_its_css_shape() {
        // ease-in-out is symmetric about its middle.
        assert!((EASE_IN_OUT.eval(0.5) - 0.5).abs() < 1e-3);
        // ease-out-expo covers most of the way early.
        assert!(EASE_OUT_EXPO.eval(0.25) > 0.75);
    }

    #[test]
    fn a_delay_holds_the_start_and_then_runs_the_curve() {
        let spec = MotionSpec::new(100, EASE).with_delay(100);
        assert_eq!(spec.total(), Duration::from_millis(200));
        assert_eq!(spec.progress(0.25), 0.0);
        assert_eq!(spec.progress(0.5), 0.0);
        assert!(spec.progress(0.75) > 0.0);
        assert_eq!(spec.progress(1.0), 1.0);
        assert_eq!(spec.progress_after(Duration::from_millis(200)), None);
    }

    #[test]
    fn a_tween_eases_toward_its_target_and_then_retires() {
        let start = tween_at(0.0, 100.0, RESORT, Duration::ZERO).expect("in flight");
        assert!(start.abs() < 0.01);
        let middle = tween_at(0.0, 100.0, RESORT, RESORT.duration() / 2).expect("in flight");
        assert!(middle > 50.0, "ease-out is past halfway at half time, got {middle}");
        assert_eq!(tween_at(0.0, 100.0, RESORT, RESORT.duration()), None);
    }

    #[test]
    fn a_tween_reversed_mid_flight_leaves_from_where_it_is() {
        let interrupted = tween_at(0.0, 100.0, RESORT, RESORT.duration() / 4).expect("in flight");
        let reversed = tween_at(interrupted, 0.0, RESORT, Duration::ZERO).expect("in flight");
        assert!((reversed - interrupted).abs() < 0.01);
    }
}
