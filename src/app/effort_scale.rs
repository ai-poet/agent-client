//! The Effort card's scale: level colors, slider geometry and snapping, which
//! tick labels fit, the fire's spring and intensity. Pure functions, so the
//! card's behaviour is tested without a window.
//!
//! Derived from dsh-effort-slider (BSD-3-Clause). Copyright (c) 2026,
//! dsh-web-ui-custom contributors; Copyright (c) 2026, dsh-effort-slider
//! contributors. Full notice: NOTICE.md.
//!
//! Departures from the plugin: levels are colored by what they are rather
//! than by their position (our ladders run low…max, low/high/max or up to
//! ultracode, not always off…max), tick labels are the real level names
//! instead of a forced "OFF"/"MAX" at the ends, and the two plugin colors that
//! fell short of a 4.5:1 contrast on the dark panel are raised.

use gpui::{Hsla, Rgba, rgb, rgba};

use super::{AgentSession, ProviderModel};

/// Thumb edge, and the panel's width and horizontal padding (the plugin's
/// `metrics.ts`).
pub(super) const THUMB: f32 = 28.0;
pub(super) const PANEL_W: f32 = 280.0;
pub(super) const PANEL_PAD_X: f32 = 16.0;
pub(super) const TRACK_W: f32 = PANEL_W - 2.0 * PANEL_PAD_X;
pub(super) const TRACK_H: f32 = 32.0;

/// The dark panel's surface, which the panel colors are measured against.
#[cfg(test)]
pub(super) const PANEL_SURFACE: u32 = 0x100b18;
/// Inactive tick labels; the plugin's `#6a6080` read at about 3.3:1.
pub(super) const TICK_INACTIVE: u32 = 0x8a80a4;
pub(super) const TICK_ACTIVE: u32 = 0xc084fc;
pub(super) const TITLE: u32 = 0x8880a0;

/// What a level is, whatever its id or position in the ladder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Tone {
    Off,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Unknown,
}

pub(super) fn tone(id: &str) -> Tone {
    match id.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "minimal" | "disabled" => Tone::Off,
        "low" => Tone::Low,
        "medium" | "med" | "normal" => Tone::Medium,
        "high" => Tone::High,
        "xhigh" | "x-high" | "extra-high" | "extra_high" => Tone::XHigh,
        "max" | "maximum" | "ultra" | "ultracode" => Tone::Max,
        _ => Tone::Unknown,
    }
}

/// A soft colored halo behind a level name; gpui has no text-shadow, so it is
/// painted as a blurred box shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Glow {
    pub color: Hsla,
    pub blur: f32,
}

fn purple_glow(alpha: f32, blur: f32) -> Glow {
    Glow {
        color: Rgba {
            a: alpha,
            ..rgb(0xa855f7)
        }
        .into(),
        blur,
    }
}

/// A level's color on the dark panel, and its glow if it has one.
pub(super) fn panel_color(tone: Tone) -> (Hsla, Option<Glow>) {
    match tone {
        Tone::Off => (rgb(0xc882a0).into(), None),
        Tone::Low => (rgb(0xc8aa82).into(), None),
        Tone::Medium => (rgb(0x82aac8).into(), None),
        Tone::High => (rgb(0xc084fc).into(), Some(purple_glow(0.7, 10.0))),
        Tone::XHigh => (rgb(0xcc9cfd).into(), Some(purple_glow(0.85, 11.0))),
        Tone::Max => (rgb(0xd8b4fe).into(), Some(purple_glow(1.0, 12.0))),
        Tone::Unknown => (rgb(0xc084fc).into(), None),
    }
}

/// The color and glow a level shows on the panel. The ladder's top level
/// always glows — in its own color when its tone has no glow — so the
/// strongest setting reads as such on a short ladder too.
pub(super) fn level_style(tone: Tone, is_top: bool) -> (Hsla, Option<Glow>) {
    let (color, glow) = panel_color(tone);
    let glow = glow.or_else(|| {
        is_top.then_some(Glow {
            color: color.opacity(0.6),
            blur: 10.0,
        })
    });
    (color, glow)
}

/// A level's color on the composer chip, legible on both themes.
pub(super) fn chip_color(tone: Tone, is_dark: bool) -> Hsla {
    if is_dark {
        return panel_color(tone).0;
    }
    match tone {
        Tone::Off => rgb(0xa0567a),
        Tone::Low => rgb(0x8a6a3e),
        Tone::Medium => rgb(0x3f6f96),
        Tone::High | Tone::Unknown => rgb(0x7e3fc9),
        Tone::XHigh => rgb(0x7330b8),
        Tone::Max => rgb(0x6b21a8),
    }
    .into()
}

/// The panel's translucent purple, at `alpha`.
pub(super) fn purple(alpha: f32) -> Hsla {
    rgba(0xa855f700 | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32).into()
}

/// Where stop `index` of `count` sits, 0..=1.
pub(super) fn stop_ratio(index: usize, count: usize) -> f32 {
    if count < 2 {
        return 0.0;
    }
    index.min(count - 1) as f32 / (count - 1) as f32
}

/// The stop nearest a slider position.
pub(super) fn nearest_stop(ratio: f32, count: usize) -> usize {
    if count < 2 {
        return 0;
    }
    let steps = (count - 1) as f32;
    ((ratio.clamp(0.0, 1.0) * steps).round() as usize).min(count - 1)
}

/// The thumb's centre for a slider position, from the track's left edge.
/// The thumb travels from half a thumb in to half a thumb short of the end,
/// so stops and labels line up with where the thumb actually rests.
pub(super) fn thumb_center_x(track_width: f32, ratio: f32) -> f32 {
    THUMB / 2.0 + ratio.clamp(0.0, 1.0) * (track_width - THUMB).max(0.0)
}

pub(super) fn stop_center_x(track_width: f32, index: usize, count: usize) -> f32 {
    thumb_center_x(track_width, stop_ratio(index, count))
}

/// The slider position under the pointer. `grab_dx` is how far from the
/// thumb's centre it was grabbed, so the thumb does not jump under the
/// pointer; a press on bare track grabs at the centre.
pub(super) fn ratio_at(pointer_x: f32, track_left: f32, track_width: f32, grab_dx: f32) -> f32 {
    let travel = (track_width - THUMB).max(1.0);
    ((pointer_x - grab_dx - track_left - THUMB / 2.0) / travel).clamp(0.0, 1.0)
}

/// The stop a key moves to, or `None` when it moves nowhere.
pub(super) fn key_step(index: usize, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let index = index.min(last);
    let target = match key {
        "left" => index.checked_sub(1)?,
        "right" => (index < last).then_some(index + 1)?,
        "home" => 0,
        "end" => last,
        _ => return None,
    };
    (target != index).then_some(target)
}

/// Rough width of a tick label at 10px bold uppercase.
pub(super) fn tick_width(label: &str) -> f32 {
    // CJK labels (the Chinese and Japanese level names) run wider per glyph.
    let glyphs: f32 = label
        .chars()
        .map(|glyph| if glyph.is_ascii() { 7.0 } else { 11.0 })
        .sum();
    glyphs + 4.0
}

/// The tick labels that fit, by index. All of them on a ladder of five or
/// fewer whose labels do not overlap; otherwise the ends and the active one,
/// dropping an end the active label would overlap.
pub(super) fn visible_ticks(labels: &[String], active: usize, track_width: f32) -> Vec<usize> {
    let count = labels.len();
    if count == 0 {
        return Vec::new();
    }
    let active = active.min(count - 1);
    let overlaps = |a: usize, b: usize| {
        let gap = (stop_center_x(track_width, a, count) - stop_center_x(track_width, b, count)).abs();
        gap < (tick_width(&labels[a]) + tick_width(&labels[b])) / 2.0 + 4.0
    };
    if count <= 5 && (1..count).all(|index| !overlaps(index - 1, index)) {
        return (0..count).collect();
    }
    let mut shown = vec![active];
    for end in [0, count - 1] {
        if end != active && !overlaps(end, active) {
            shown.push(end);
        }
    }
    shown.sort_unstable();
    shown.dedup();
    shown
}

/// The fire's leading edge follows the slider on a spring (the plugin's
/// stiffness 7, damping 0.55). It only springs upward; lowering the slider
/// pulls the fire back at once.
pub(super) fn spring_step(value: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32) {
    const STIFFNESS: f32 = 7.0;
    const DAMPING: f32 = 0.55;
    let dt = dt.clamp(0.0, 0.05);
    if value >= target {
        return (target, 0.0);
    }
    let mut velocity = velocity + (target - value) * STIFFNESS * dt;
    velocity *= 1.0 - DAMPING * dt * 6.0;
    let value = value + velocity * dt;
    if value > target {
        (target, 0.0)
    } else {
        (value, velocity)
    }
}

/// The fire's reach for a slider position: even the lowest level has a flame.
pub(super) fn fire_front(ratio: f32) -> f32 {
    0.15 + 0.85 * ratio.clamp(0.0, 1.0)
}

pub(super) fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How fiercely the fire burns at a reach, 0..=1 (the plugin's `intensity`).
pub(super) fn fire_intensity(front: f32) -> f32 {
    let front = front.clamp(0.0, 1.0);
    smoothstep(0.0, 0.2, front) * (0.08 + (1.0 - 0.08) * front.powf(0.55))
}

/// What the session runs with: its own choice when the model offers it, else
/// the model's default, else the first option — the resolution the traits
/// menu has always drawn.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Traits {
    pub effort: Option<String>,
    /// `"default"` for the standard tier.
    pub tier: String,
    pub window: Option<String>,
}

pub(super) fn resolve_traits(session: &AgentSession, model: &ProviderModel) -> Traits {
    let effort = session
        .reasoning_effort
        .as_deref()
        .filter(|selected| {
            model
                .reasoning_efforts
                .iter()
                .any(|option| option.id == *selected)
        })
        .or(model.default_reasoning_effort.as_deref())
        .or_else(|| model.reasoning_efforts.first().map(|option| option.id.as_str()))
        .map(str::to_owned);
    let tier = session
        .service_tier
        .as_deref()
        .filter(|selected| {
            *selected == "default" || model.service_tiers.iter().any(|option| option.id == *selected)
        })
        .or(model.default_service_tier.as_deref())
        .unwrap_or("default")
        .to_owned();
    let window = session
        .context_window
        .as_deref()
        .filter(|selected| {
            model
                .context_windows
                .iter()
                .any(|option| option.id == *selected)
        })
        .or(model.default_context_window.as_deref())
        .or_else(|| model.context_windows.first().map(|option| option.id.as_str()))
        .map(str::to_owned);
    Traits {
        effort,
        tier,
        window,
    }
}

#[cfg(test)]
fn relative_luminance(color: Hsla) -> f32 {
    let rgba = Rgba::from(color);
    let channel = |value: f32| {
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
}

/// WCAG contrast ratio between two opaque colors.
#[cfg(test)]
pub(super) fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use waku_protocol::model::ProviderKind;

    const ALL_TONES: [Tone; 7] = [
        Tone::Off,
        Tone::Low,
        Tone::Medium,
        Tone::High,
        Tone::XHigh,
        Tone::Max,
        Tone::Unknown,
    ];

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn levels_are_toned_by_what_they_are() {
        assert_eq!(tone("off"), Tone::Off);
        assert_eq!(tone(" None "), Tone::Off);
        assert_eq!(tone("minimal"), Tone::Off);
        assert_eq!(tone("LOW"), Tone::Low);
        assert_eq!(tone("normal"), Tone::Medium);
        assert_eq!(tone("high"), Tone::High);
        assert_eq!(tone("extra-high"), Tone::XHigh);
        assert_eq!(tone("xhigh"), Tone::XHigh);
        assert_eq!(tone("ultracode"), Tone::Max);
        assert_eq!(tone("ultra"), Tone::Max);
        assert_eq!(tone("turbo"), Tone::Unknown);
    }

    #[test]
    fn the_top_level_glows_without_changing_color() {
        let (color, glow) = level_style(Tone::Medium, true);
        assert_eq!(color, panel_color(Tone::Medium).0);
        assert!(glow.is_some());
        assert!(level_style(Tone::Medium, false).1.is_none());
        assert_eq!(level_style(Tone::Max, true), panel_color(Tone::Max));
    }

    #[test]
    fn snapping_round_trips_every_stop() {
        for count in 2..=7 {
            for index in 0..count {
                assert_eq!(nearest_stop(stop_ratio(index, count), count), index);
            }
        }
        assert_eq!(nearest_stop(0.49, 3), 1);
        assert_eq!(nearest_stop(0.24, 3), 0);
        assert_eq!(nearest_stop(-1.0, 3), 0);
        assert_eq!(nearest_stop(2.0, 3), 2);
        assert_eq!(nearest_stop(0.7, 1), 0);
    }

    #[test]
    fn the_pointer_maps_onto_the_thumbs_travel() {
        // Pressing on a stop's centre lands on that stop.
        for index in 0..5 {
            let x = 100.0 + stop_center_x(TRACK_W, index, 5);
            assert_eq!(nearest_stop(ratio_at(x, 100.0, TRACK_W, 0.0), 5), index);
        }
        // A grab off-centre does not jump the thumb.
        let center = 100.0 + thumb_center_x(TRACK_W, 0.5);
        assert!((ratio_at(center + 6.0, 100.0, TRACK_W, 6.0) - 0.5).abs() < 1e-4);
        // Outside the track it clamps.
        assert_eq!(ratio_at(0.0, 100.0, TRACK_W, 0.0), 0.0);
        assert_eq!(ratio_at(1000.0, 100.0, TRACK_W, 0.0), 1.0);
    }

    #[test]
    fn keys_step_within_the_ladder() {
        assert_eq!(key_step(0, 3, "left"), None);
        assert_eq!(key_step(0, 3, "right"), Some(1));
        assert_eq!(key_step(2, 3, "right"), None);
        assert_eq!(key_step(1, 3, "home"), Some(0));
        assert_eq!(key_step(0, 3, "home"), None);
        assert_eq!(key_step(0, 3, "end"), Some(2));
        assert_eq!(key_step(1, 3, "up"), None);
        assert_eq!(key_step(0, 0, "right"), None);
    }

    #[test]
    fn short_ladders_label_every_stop() {
        let three = labels(&["Low", "High", "Max"]);
        assert_eq!(visible_ticks(&three, 1, TRACK_W), vec![0, 1, 2]);
        let five = labels(&["Low", "Medium", "High", "Extra High", "Max"]);
        let shown = visible_ticks(&five, 2, TRACK_W);
        assert!(shown.contains(&0) && shown.contains(&2) && shown.contains(&4));
        let two = labels(&["Low", "High"]);
        assert_eq!(visible_ticks(&two, 0, TRACK_W), vec![0, 1]);
    }

    #[test]
    fn long_ladders_label_the_ends_and_the_active_stop() {
        let six = labels(&["Low", "Medium", "High", "Extra High", "Max", "Ultracode"]);
        assert_eq!(visible_ticks(&six, 2, TRACK_W), vec![0, 2, 5]);
        // The active label wins over an end it would overlap.
        let shown = visible_ticks(&six, 4, TRACK_W);
        assert!(shown.contains(&4));
        assert!(!shown.contains(&5));
        assert!(shown.contains(&0));
        // The active stop being an end is shown once.
        assert_eq!(visible_ticks(&six, 0, TRACK_W), vec![0, 5]);
    }

    #[test]
    fn the_spring_never_overshoots_and_drops_at_once() {
        let (mut value, mut velocity) = (0.2_f32, 0.0_f32);
        for _ in 0..400 {
            (value, velocity) = spring_step(value, velocity, 0.9, 1.0 / 30.0);
            assert!(value <= 0.9);
        }
        assert!((value - 0.9).abs() < 0.01, "{value}");
        assert_eq!(spring_step(0.9, 0.4, 0.3, 1.0 / 30.0), (0.3, 0.0));
        // A long stall does not fling it.
        let (value, _) = spring_step(0.1, 0.0, 1.0, 5.0);
        assert!(value < 0.2);
    }

    #[test]
    fn the_fire_burns_harder_at_higher_levels() {
        let mut last = -1.0;
        for step in 0..=20 {
            let intensity = fire_intensity(fire_front(step as f32 / 20.0));
            assert!(intensity >= last);
            last = intensity;
        }
        assert!(fire_intensity(fire_front(0.0)) > 0.0);
        assert!((fire_intensity(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn every_chip_color_is_legible_on_both_composers() {
        for theme in [Theme::light(), Theme::dark()] {
            for tone in ALL_TONES {
                let ratio = contrast(chip_color(tone, theme.is_dark), theme.composer);
                assert!(ratio >= 4.5, "{tone:?} on dark={}: {ratio}", theme.is_dark);
            }
        }
    }

    #[test]
    fn every_panel_color_is_legible_on_the_panel() {
        let surface: Hsla = rgb(PANEL_SURFACE).into();
        for tone in ALL_TONES {
            let ratio = contrast(panel_color(tone).0, surface);
            assert!(ratio >= 4.5, "{tone:?}: {ratio}");
        }
        for color in [TICK_INACTIVE, TICK_ACTIVE, TITLE] {
            let ratio = contrast(rgb(color).into(), surface);
            assert!(ratio >= 4.5, "{color:06x}: {ratio}");
        }
    }

    #[test]
    fn traits_fall_back_to_the_models_default_then_its_first_option() {
        let option = |id: &str| waku_protocol::model::ProviderModelOption::new(id, id);
        let mut model = ProviderModel::new("m", "M");
        model.reasoning_efforts = vec![option("low"), option("high"), option("max")];
        model.default_reasoning_effort = Some("high".to_owned());
        model.context_windows = vec![option("200k"), option("1m")];
        let mut session = AgentSession::new(uuid::Uuid::new_v4(), ProviderKind::Claude);

        session.reasoning_effort = Some("max".to_owned());
        assert_eq!(resolve_traits(&session, &model).effort.as_deref(), Some("max"));
        // A choice the model does not offer falls back to its default.
        session.reasoning_effort = Some("xhigh".to_owned());
        assert_eq!(resolve_traits(&session, &model).effort.as_deref(), Some("high"));
        model.default_reasoning_effort = None;
        assert_eq!(resolve_traits(&session, &model).effort.as_deref(), Some("low"));
        let traits = resolve_traits(&session, &model);
        assert_eq!(traits.tier, "default");
        assert_eq!(traits.window.as_deref(), Some("200k"));
    }
}
