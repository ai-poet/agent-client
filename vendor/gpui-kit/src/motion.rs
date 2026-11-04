//! Shared pulse clock for the repeating loaders.
//!
//! Ported from Zeron's motion kit (<https://github.com/zeronsh/comet>, MIT).
//! A repeating `with_animation` element requests a redraw every display frame
//! for as long as it is mounted — one working row pinned the whole window at
//! 120 Hz on a ProMotion panel. Loaders instead read their phase from one
//! shared clock: it ticks at ~30 fps, notifies only views that painted a
//! loader recently, and parks itself once the last lease lapses, so a window
//! with no loader mounted schedules nothing at all. Every loader shares one
//! epoch, keeping multi-instance loaders phase-locked.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, EntityId, FontWeight, Global, Hsla, IntoElement, RenderOnce, SharedString,
    StyledText, Svg, TextRun, Transformation, Window, ease_out_quint, percentage,
};

/// Repeat-tick interval (~30 fps): visually equivalent for these chunky
/// pulses and spins at a quarter of a ProMotion display's redraws.
const PULSE_TICK: Duration = Duration::from_millis(33);

/// How long a view stays on the tick list after it last painted a loader. One
/// lease outlives a few missed frames; an unmounted loader stops renewing and
/// its view drops off, letting the clock park.
const PULSE_LEASE: Duration = Duration::from_millis(300);

/// The rotating `loader-circle` spinners' period.
const SPINNER_PERIOD: Duration = Duration::from_millis(900);

struct Lease {
    until: Instant,
    /// Notify this view every `stride`-th tick. A view's whole subtree
    /// rebuilds per notify, so a loader on an expensive surface can trade
    /// animation granularity for a cheaper cadence.
    stride: u32,
}

struct PulseClock {
    epoch: Instant,
    leases: HashMap<EntityId, Lease>,
    ticks: u64,
    running: bool,
}

impl Global for PulseClock {}

impl Default for PulseClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            leases: HashMap::new(),
            ticks: 0,
            running: false,
        }
    }
}

/// Keep `view` re-rendering at [`PULSE_TICK`] until the lease lapses. A caller
/// that stops leasing stops being notified, and the clock parks once no
/// leases remain — quiescence needs no unsubscribe step.
pub fn pulse_lease(view: EntityId, cx: &mut App) {
    pulse_lease_with_stride(view, 1, cx);
}

/// [`pulse_lease`] at every second tick (~15 fps), for animations whose view
/// is expensive to rebuild and whose motion survives the coarser step — a
/// notify re-renders the view's whole subtree, so cadence is priced per
/// tick, not per animation.
pub fn pulse_lease_slow(view: EntityId, cx: &mut App) {
    pulse_lease_with_stride(view, 2, cx);
}

fn pulse_lease_with_stride(view: EntityId, stride: u32, cx: &mut App) {
    let clock = cx.default_global::<PulseClock>();
    let until = Instant::now() + PULSE_LEASE;
    // A view hosting both a full-rate and a strided loader keeps full rate.
    clock
        .leases
        .entry(view)
        .and_modify(|lease| {
            lease.until = until;
            lease.stride = lease.stride.min(stride);
        })
        .or_insert(Lease { until, stride });
    if clock.running {
        return;
    }
    clock.running = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(PULSE_TICK).await;
            let parked = cx.update(|cx| {
                let clock = cx.default_global::<PulseClock>();
                let now = Instant::now();
                clock.ticks += 1;
                let ticks = clock.ticks;
                clock.leases.retain(|_, lease| lease.until > now);
                if clock.leases.is_empty() {
                    clock.running = false;
                    return true;
                }
                let due = clock
                    .leases
                    .iter_mut()
                    .filter(|(_, lease)| ticks % lease.stride.max(1) as u64 == 0)
                    .map(|(view, lease)| {
                        // Strides re-establish on the render this notify
                        // triggers; without the reset, one full-rate lease
                        // would drag its view's cadence down permanently.
                        lease.stride = u32::MAX;
                        *view
                    })
                    .collect::<Vec<_>>();
                for view in due {
                    cx.notify(view);
                }
                false
            });
            if parked {
                break;
            }
        }
    })
    .detach();
}

/// Phase `[0,1)` of a repeating cycle of `period`, plus a lease keeping `view`
/// re-rendering while its loader stays mounted. Under reduce-motion this is a
/// constant 0 — the cycle's first frame, matching what a repeating
/// `with_animation` held — and nothing is scheduled.
fn pulse_phase(period: Duration, stride: u32, view: EntityId, cx: &mut App) -> f32 {
    if cx.reduce_motion() {
        return 0.0;
    }
    let clock = cx.default_global::<PulseClock>();
    let phase = (clock.epoch.elapsed().as_secs_f32() / period.as_secs_f32()).fract();
    pulse_lease_with_stride(view, stride, cx);
    phase
}

/// A loader element styled from the shared clock's phase. Resolving the phase
/// is deferred to render, where the owning view is known, so call sites need
/// neither a `Window` nor an `EntityId` in scope.
pub fn pulse(period: Duration, render: impl FnOnce(f32) -> AnyElement + 'static) -> Pulse {
    Pulse {
        period,
        stride: 1,
        render: Box::new(render),
    }
}

/// A rotating loader icon riding the shared clock.
pub fn spin(icon: Svg) -> AnyElement {
    spin_with_stride(icon, 1)
}

/// A rotating loader at every second tick (~15 fps — the classic
/// discrete-step spinner cadence). For loaders on expensive surfaces: the
/// sidebar rebuilds its whole subtree per notify, and a session row's working
/// spinner is not worth pricing that at full rate.
pub fn spin_slow(icon: Svg) -> AnyElement {
    spin_with_stride(icon, 2)
}

fn spin_with_stride(icon: Svg, stride: u32) -> AnyElement {
    let mut pulse = pulse(SPINNER_PERIOD, move |phase| {
        icon.with_transformation(Transformation::rotate(percentage(phase)))
            .into_any_element()
    });
    pulse.stride = stride;
    pulse.into_any_element()
}

#[derive(IntoElement)]
pub struct Pulse {
    period: Duration,
    stride: u32,
    render: Box<dyn FnOnce(f32) -> AnyElement>,
}

impl Pulse {
    /// Tick every `stride`-th pulse instead of every one. A view's whole
    /// subtree rebuilds per notify — the pane ticks at the fastest of its
    /// lessees — so a loader mounted for a whole turn on an expensive
    /// surface should ride the coarser cadence.
    pub fn every(mut self, stride: u32) -> Self {
        self.stride = stride.max(1);
        self
    }
}

impl RenderOnce for Pulse {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let phase = pulse_phase(self.period, self.stride, window.current_view(), cx);
        (self.render)(phase)
    }
}

/// One shimmer cycle: the bright band crosses the text, then rests.
const SHIMMER_PERIOD: Duration = Duration::from_millis(2400);

/// The share of a cycle the band spends crossing; the rest is a pause, so the
/// motion reads as a glint rather than a scrolling marquee.
const SHIMMER_SWEEP: f32 = 0.6;

/// Brightness steps. Each step is a text run of its own, so fewer steps keep
/// a long label from splitting into one run per character.
const SHIMMER_STEPS: u8 = 7;

/// A label that is still happening — "正在读取", "思考中" — drawn with a band of
/// light passing across it on the shared clock, instead of a spinner per row.
/// Under reduce-motion it is plain text.
pub fn shimmer(text: impl Into<SharedString>, base: Hsla, highlight: Hsla) -> Shimmer {
    Shimmer {
        text: text.into(),
        base,
        highlight,
        weight: FontWeight::NORMAL,
    }
}

#[derive(IntoElement)]
pub struct Shimmer {
    text: SharedString,
    base: Hsla,
    highlight: Hsla,
    weight: FontWeight,
}

impl Shimmer {
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }
}

impl RenderOnce for Shimmer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Every second tick is plenty for a slow sweep, and it halves what a
        // transcript full of live rows costs to rebuild.
        let phase = pulse_phase(SHIMMER_PERIOD, 2, window.current_view(), cx);
        let mut font = window.text_style().font();
        font.weight = self.weight;
        let levels = shimmer_levels(self.text.chars().count(), phase);
        let mut runs: Vec<TextRun> = Vec::new();
        let mut current: Option<(u8, usize)> = None;
        for (character, level) in self.text.chars().zip(levels) {
            match &mut current {
                Some((run_level, len)) if *run_level == level => *len += character.len_utf8(),
                _ => {
                    if let Some((run_level, len)) = current.take() {
                        runs.push(shimmer_run(len, run_level, &self, &font));
                    }
                    current = Some((level, character.len_utf8()));
                }
            }
        }
        if let Some((run_level, len)) = current {
            runs.push(shimmer_run(len, run_level, &self, &font));
        }
        StyledText::new(self.text.clone()).with_runs(runs)
    }
}

fn shimmer_run(len: usize, level: u8, shimmer: &Shimmer, font: &gpui::Font) -> TextRun {
    let strength = f32::from(level) / f32::from(SHIMMER_STEPS);
    TextRun {
        len,
        font: font.clone(),
        color: shimmer.base.blend(shimmer.highlight.opacity(strength)),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Brightness of each of `chars` characters at `phase` of a cycle, from 0
/// (the base colour) to [`SHIMMER_STEPS`]. The band enters from the left
/// edge, leaves past the right, and is gone for the rest of the cycle.
pub fn shimmer_levels(chars: usize, phase: f32) -> Vec<u8> {
    let width = (chars as f32 / 3.0).max(3.0);
    if phase >= SHIMMER_SWEEP {
        return vec![0; chars];
    }
    let travel = phase / SHIMMER_SWEEP;
    let center = -width + travel * (chars as f32 + 2.0 * width);
    (0..chars)
        .map(|index| {
            let distance = (index as f32 + 0.5 - center).abs();
            let strength = (1.0 - distance / width).max(0.0);
            (strength * f32::from(SHIMMER_STEPS)).round() as u8
        })
        .collect()
}

/// How long a side panel takes to slide open or shut. Zeron's panel
/// transition (`crates/ui/src/motion.rs` `RESIZE`) is 200ms — long enough to
/// read as travel rather than a jump cut, short enough that the layout is
/// settled before the pointer arrives anywhere else.
pub const PANEL_SLIDE: Duration = Duration::from_millis(200);

/// A one-shot width slide, evaluated from `render` instead of wrapped around
/// an element.
///
/// `with_animation` cannot drive this. The width feeds the flex layout of the
/// panel's *siblings* — the transcript column takes whatever the panels leave
/// — and gpui keys an animation element by its element-id path, so a wrapper
/// that remounts would replay the slide from zero. Evaluating by hand keeps
/// the element tree's shape constant: a finished or dropped tween is exactly
/// the steady state.
#[derive(Clone, Copy, Debug)]
pub struct WidthTween {
    from: f32,
    started: Instant,
}

impl WidthTween {
    /// Start a slide from the width the panel currently occupies, so a toggle
    /// mid-slide reverses from where the edge actually is instead of jumping
    /// back to the far end.
    pub fn new(from: f32) -> Self {
        Self {
            from,
            started: Instant::now(),
        }
    }

    /// Eased width on the way to `target`, or `None` once the slide is over —
    /// the caller then drops the tween and reads `target` directly, which is
    /// also what retires a closed panel from the element tree.
    pub fn width_toward(&self, target: f32) -> Option<f32> {
        width_at(self.from, target, self.started.elapsed())
    }
}

fn width_at(from: f32, target: f32, elapsed: Duration) -> Option<f32> {
    let progress = elapsed.as_secs_f32() / PANEL_SLIDE.as_secs_f32();
    (progress < 1.0).then(|| from + (target - from) * ease_out_quint()(progress.max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shimmer_band_crosses_the_text_and_then_rests() {
        // Before it enters and after it has passed, nothing is lit.
        assert!(shimmer_levels(12, 0.0).iter().all(|level| *level == 0));
        assert!(shimmer_levels(12, 0.9).iter().all(|level| *level == 0));
        // Mid-sweep the middle is brightest and the edges are dark.
        let middle = shimmer_levels(12, SHIMMER_SWEEP / 2.0);
        let peak = middle.iter().copied().max().unwrap();
        assert!(peak >= SHIMMER_STEPS - 1, "{middle:?}");
        assert_eq!(middle[0], 0, "{middle:?}");
        assert_eq!(middle[11], 0, "{middle:?}");
        assert!(shimmer_levels(0, 0.3).is_empty());
    }

    #[test]
    fn a_slide_eases_out_and_then_retires() {
        let start = width_at(0.0, 260.0, Duration::ZERO).expect("a fresh slide is in flight");
        assert!(start.abs() < 0.01, "the slide opens from its start width");

        let half = width_at(0.0, 260.0, PANEL_SLIDE / 2).expect("halfway is in flight");
        assert!(
            half > 130.0,
            "ease-out covers most of the distance early, got {half}"
        );

        assert_eq!(
            width_at(0.0, 260.0, PANEL_SLIDE),
            None,
            "an elapsed slide reports no width so the caller settles on the target"
        );
    }

    #[test]
    fn a_slide_reversed_mid_flight_leaves_from_where_it_is() {
        let interrupted = width_at(0.0, 260.0, PANEL_SLIDE / 4).expect("in flight");
        let reversed = width_at(interrupted, 0.0, Duration::ZERO).expect("in flight");
        assert!(
            (reversed - interrupted).abs() < 0.01,
            "the reversed slide starts at the interrupted width"
        );
    }
}
