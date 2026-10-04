//! The fire along the Effort slider's track.
//!
//! Derived from dsh-effort-slider (BSD-3-Clause). Copyright (c) 2026,
//! dsh-web-ui-custom contributors; Copyright (c) 2026, dsh-effort-slider
//! contributors. Full notice: NOTICE.md.
//!
//! The plugin draws it with a WebGL2 shader over a 72×6 grid of cells, a blur
//! pass and a tone-mapped composite. gpui has no user shaders, so this is the
//! same per-cell math evaluated on the CPU at each cell's centre and painted
//! as one small rounded quad per lit cell (at most 432), with the blur pass
//! approximated by blurred drop shadows — a glow over the burning body and a
//! white-hot core at the leading edge. The feedback trail and screen blending
//! are left out; a lit cell's alpha is its brightest channel, which over the
//! near-black track comes close to adding light.
//!
//! The caller drives time from the pulse clock (≤30 Hz) only while the card
//! is open, and passes a still frame when the system asks for reduced motion.

use gpui::{
    Bounds, BoxShadow, ContentMask, Hsla, Pixels, Rgba, Window, fill, point, px, size,
};

use super::effort_scale::{fire_intensity, smoothstep};

const COLUMNS: usize = 72;
const ROWS: usize = 6;
/// A cell's lit quad, as a share of the cell.
const CELL_FILL: f32 = 0.65;
/// The plugin's CSS mask: the fire shows up to the slider and fades over
/// this share of the track on either side of it.
const REVEAL_FADE: f32 = 0.015;

const EMBER: [f32; 3] = [0.28, 0.10, 0.58];
const PURPLE: [f32; 3] = [0.62, 0.32, 1.0];
const WHITE: [f32; 3] = [1.0, 0.94, 0.98];

/// One frame's inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FireFrame {
    /// The leading edge, 0..=1, already sprung toward the slider.
    pub front: f32,
    /// The slider's own position, 0..=1: the fire shows up to here.
    pub reveal: f32,
    /// Seconds on the animation clock.
    pub time: f32,
    /// Seconds since the card opened; the fire ignites and spreads over the
    /// first two and a half.
    pub elapsed: f32,
}

impl FireFrame {
    /// A frame for reduced motion: no flicker, fully lit.
    pub(super) fn still(front: f32, reveal: f32) -> Self {
        Self {
            front,
            reveal,
            time: 0.0,
            elapsed: 4.0,
        }
    }
}

/// A lit cell: its grid position, color and coverage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FireCell {
    pub column: usize,
    pub row: usize,
    pub color: [f32; 3],
    pub alpha: f32,
}

fn hash(x: f32, y: f32) -> f32 {
    let value = (x * 127.1 + y * 311.7).sin() * 43_758.547;
    value - value.floor()
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

fn step(edge: f32, x: f32) -> f32 {
    if x < edge { 0.0 } else { 1.0 }
}

fn reveal_mask(x: f32, reveal: f32) -> f32 {
    let solid = (reveal - REVEAL_FADE).max(0.0);
    let clear = (reveal + REVEAL_FADE).min(1.0);
    if x <= solid {
        1.0
    } else if x >= clear {
        0.0
    } else {
        (clear - x) / (clear - solid).max(1e-4)
    }
}

/// The plugin's `FRAG_SIM` at one cell's centre, then its composite tone map.
fn cell_light(column: usize, row: usize, frame: &FireFrame) -> [f32; 3] {
    let u = (column as f32 + 0.5) / COLUMNS as f32;
    let v = (row as f32 + 0.5) / ROWS as f32;
    let s = frame.front.clamp(0.0, 1.0);
    let t = frame.time;
    let elapsed = frame.elapsed.max(0.0);
    let h = hash(column as f32, row as f32);

    let fade_mask = smoothstep(0.0, 0.4, u);
    let intensity = fire_intensity(s);
    let cell_age = (elapsed - h * 1.2).max(0.0);
    let ignited = step(0.001, cell_age);
    let cell_speed = 0.85 + h * 0.30;
    let eased = 1.0 - (1.0 - (cell_age / 2.5).clamp(0.0, 1.0)).powi(3);
    let dist = eased * s * cell_speed * ignited;
    let cell_offset = (h - 0.5) * 0.05;
    let front = (s - dist - cell_offset).max(0.02);
    let tail = (s - front).max(0.001);
    let in_zone = step(front - 0.003, u) * step(u, s + 0.003);
    let dn = ((s - u).max(0.0) / tail).clamp(0.0, 1.0);
    let mut bright = (1.0 - dn).powf(0.65);
    bright = bright.max(0.04 * ignited) * in_zone;
    bright *= 1.0 - smoothstep(0.94, 1.05, dn);
    let es = mix(0.15, 0.5, elapsed.min(1.0));
    let vy = (v - 0.5).abs() * 2.0;
    let vf = (1.0 - vy * vy * 0.45).max(0.0).powf(0.75);
    let ts = mix(0.85, 1.0, (elapsed / 1.5).min(1.0));
    let f1 = (u * 30.0 + t * 15.0 * ts + h * 6.28).sin();
    let f2 = (u * 17.0 + t * 8.0 * ts + h * 3.14).sin();
    let f3 = (u * 52.0 + t * 25.0 * ts + h * 10.0).sin();
    let flame = smoothstep(0.08, 0.92, (f1 + f2 * 0.5 + f3 * 0.25) * 0.35 + 0.5);
    let r1 = (dn * 16.0 - t * 5.0 * ts + h * 3.0).sin();
    let r2 = (dn * 8.0 - t * 2.5 * ts + h * 5.0).sin();
    let rhythm = (smoothstep(-0.15, 0.55, r1) * (r2 * 0.5 + 0.5)).max(0.0).powf(1.2);
    let average_speed = dist / cell_age.max(0.001);
    let age = (cell_age - (s - u).max(0.0) / average_speed.max(0.001)).max(0.0);
    let flash = step(0.0, age) * (-age * 3.2).exp();
    let spark_phase = {
        let value = t * (0.38 + h * 0.15) + h * 7.0;
        value - value.floor()
    };
    let spark_x = s - spark_phase * tail;
    let spark_y = 0.5 + (spark_phase * 11.0 + h * 6.28).sin() * 0.28;
    let spark = smoothstep(0.014, 0.0, (u - spark_x).abs())
        * smoothstep(0.18, 0.0, (v - spark_y).abs())
        * (1.0 - spark_phase).powi(2)
        * es;
    let mut energy = bright * vf * (flame * 0.42 + rhythm * 0.38)
        + flash * bright * vf * 0.55
        + spark * 0.7 * in_zone;
    energy *= es * intensity;
    let edge_base = (-((u - front) * 18.0).powi(2)).exp();
    let ef1 = (u * 45.0 + t * 20.0 * ts + h * 6.28).sin() * 0.5 + 0.5;
    let ef2 = (u * 28.0 + t * 11.0 * ts + h * 3.14).sin() * 0.5 + 0.5;
    let edge = edge_base * (0.25 + ef1 * ef2 * 1.5) * 1.6 * intensity * es;
    let lead_distance = front - u;
    let lead_zone = smoothstep(0.07, 0.0, lead_distance) * step(0.0, lead_distance) * vf;
    let h2 = hash(column as f32 + 99.0, row as f32 + 33.0);
    let lead_flicker = (lead_distance * 100.0 + t * 20.0 * ts + h2 * 6.28).sin() * 0.5 + 0.5;
    let lead_spark = lead_zone * step(0.6, h2) * lead_flicker * intensity * es * 0.5;
    let total = energy + edge + lead_spark;

    let temperature = 1.0 - dn;
    let mut color = mix3(EMBER, PURPLE, temperature);
    color = mix3(color, WHITE, temperature.powf(4.5));
    let pulse = (t * 2.8).sin() * 0.15 + 1.0;
    let core = (-((u - s) * 16.0).powi(2)).exp() * 2.2 * pulse * intensity * es;
    let halo = (-((u - s) * 3.5).powi(2)).exp() * 0.12 * intensity * es;
    let mask = fade_mask * reveal_mask(u, frame.reveal);
    let mut light = [0.0; 3];
    for channel in 0..3 {
        let scene =
            (color[channel] * total + WHITE[channel] * core + PURPLE[channel] * halo) * mask;
        // The composite pass's tone map, without the blurred glow term.
        light[channel] = 1.0 - (-scene.min(1.5) * 1.15).exp();
    }
    light
}

/// Every cell bright enough to draw.
pub(super) fn fire_cells(frame: &FireFrame) -> Vec<FireCell> {
    let mut cells = Vec::new();
    for column in 0..COLUMNS {
        for row in 0..ROWS {
            let light = cell_light(column, row, frame);
            let alpha = light[0].max(light[1]).max(light[2]);
            if alpha < 0.012 {
                continue;
            }
            cells.push(FireCell {
                column,
                row,
                color: light.map(|channel| (channel / alpha).min(1.0)),
                alpha,
            });
        }
    }
    cells
}

/// Paint one frame of fire into the track.
pub(super) fn paint_fire(window: &mut Window, track: Bounds<Pixels>, frame: FireFrame) {
    let width = f32::from(track.size.width);
    let height = f32::from(track.size.height);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let reveal_end = (frame.reveal + REVEAL_FADE).clamp(0.0, 1.0);
    let visible = Bounds {
        origin: track.origin,
        size: size(px(width * reveal_end), track.size.height),
    };
    let es = mix(0.15, 0.5, frame.elapsed.clamp(0.0, 1.0));
    let intensity = fire_intensity(frame.front);
    let cell_width = width / COLUMNS as f32;
    let cell_height = height / ROWS as f32;
    let cells = fire_cells(&frame);
    window.with_content_mask(Some(ContentMask { bounds: visible }), |window| {
        // The blur pass, approximated: a soft purple glow over the body…
        let body_width = width * frame.front.min(reveal_end);
        if body_width > 1.0 && intensity > 0.0 {
            window.paint_drop_shadows(
                Bounds {
                    origin: point(track.origin.x, track.origin.y + px(height * 0.3)),
                    size: size(px(body_width), px(height * 0.4)),
                },
                px(4.0).into(),
                &[BoxShadow::new(px(0.0), px(0.0), color(PURPLE, 0.25 * intensity * es))
                    .blur_radius(px(6.0))],
            );
        }
        for cell in &cells {
            let left = track.origin.x + px(cell.column as f32 * cell_width);
            let top = track.origin.y + px(cell.row as f32 * cell_height);
            let quad_width = cell_width * CELL_FILL;
            let quad_height = cell_height * CELL_FILL;
            window.paint_quad(
                fill(
                    Bounds {
                        origin: point(
                            left + px((cell_width - quad_width) / 2.0),
                            top + px((cell_height - quad_height) / 2.0),
                        ),
                        size: size(px(quad_width), px(quad_height)),
                    },
                    color(cell.color, cell.alpha),
                )
                .corner_radii(px(1.0)),
            );
        }
        // …and a white-hot core where the flame leads.
        if intensity > 0.0 {
            let pulse = (frame.time * 2.8).sin() * 0.15 + 1.0;
            let core_x = width * frame.front;
            window.paint_drop_shadows(
                Bounds {
                    origin: point(
                        track.origin.x + px(core_x - 3.0),
                        track.origin.y + px(height * 0.25),
                    ),
                    size: size(px(6.0), px(height * 0.5)),
                },
                px(3.0).into(),
                &[BoxShadow::new(
                    px(0.0),
                    px(0.0),
                    color(WHITE, (0.55 * pulse * intensity * es).min(1.0)),
                )
                .blur_radius(px(8.0))],
            );
        }
    });
}

fn color(channels: [f32; 3], alpha: f32) -> Hsla {
    Rgba {
        r: channels[0],
        g: channels[1],
        b: channels[2],
        a: alpha.clamp(0.0, 1.0),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit_reach(cells: &[FireCell]) -> f32 {
        cells
            .iter()
            .map(|cell| (cell.column as f32 + 0.5) / COLUMNS as f32)
            .fold(0.0, f32::max)
    }

    #[test]
    fn nothing_burns_past_the_slider() {
        for reveal in [0.0, 0.2, 0.5, 0.8] {
            for time in [0.0, 0.7, 3.1] {
                let frame = FireFrame {
                    front: 0.15 + 0.85 * reveal,
                    reveal,
                    time,
                    elapsed: 3.0,
                };
                let reach = lit_reach(&fire_cells(&frame));
                assert!(reach <= reveal + REVEAL_FADE + 1e-4, "{reveal} @ {time}: {reach}");
            }
        }
    }

    #[test]
    fn a_higher_level_lights_more_of_the_track() {
        let energy = |reveal: f32| {
            fire_cells(&FireFrame::still(0.15 + 0.85 * reveal, reveal))
                .iter()
                .map(|cell| cell.alpha)
                .sum::<f32>()
        };
        let (low, mid, high) = (energy(0.2), energy(0.5), energy(1.0));
        assert!(low < mid && mid < high, "{low} {mid} {high}");
        assert!(high > 0.0);
    }

    #[test]
    fn the_still_frame_is_the_same_every_time() {
        let frame = FireFrame::still(0.7, 0.65);
        assert_eq!(fire_cells(&frame), fire_cells(&frame));
    }

    #[test]
    fn cells_stay_within_a_drawable_range() {
        let frame = FireFrame {
            front: 1.0,
            reveal: 1.0,
            time: 1.3,
            elapsed: 0.6,
        };
        for cell in fire_cells(&frame) {
            assert!(cell.alpha > 0.0 && cell.alpha <= 1.0);
            assert!(cell.color.iter().all(|channel| (0.0..=1.0).contains(channel)));
        }
    }
}
