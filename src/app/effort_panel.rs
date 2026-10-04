//! The composer's reasoning control as an Effort slider card.
//!
//! Fork addition, a port of dsh-effort-slider's Effort panel (BSD-3-Clause.
//! Copyright (c) 2026, dsh-web-ui-custom contributors; Copyright (c) 2026,
//! dsh-effort-slider contributors. Full notice: NOTICE.md), itself after the
//! dsh-ui-web aurora skin's Claude Code–style effort card.
//!
//! The traits chip opens one card: the Effort slider on top when the model
//! offers two or more levels, then the service tier (the wire format, for the
//! built-in agent) and context window choices that used to share its menu.
//! The chip shows the level in that level's color.
//!
//! The slider drags continuously and snaps to the nearest level on release;
//! arrow keys, Home and End step it. Unlike the plugin, nothing is written
//! while dragging: choosing a level persists the session and pushes options
//! to the daemon, so the card keeps the drag to itself and commits once.
//!
//! The card is its own entity so its animation clock re-renders only the
//! card. The composer is drawn by the root view, and a lease taken there would
//! rebuild every pane at the clock's rate.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Bounds, BoxShadow, ContentMask, CursorStyle, Div, DispatchPhase, HitboxBehavior, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Role,
    Stateful,
    canvas, fill, linear_color_stop, linear_gradient, point, rgb, rgba, size,
};

use super::effort_fire::{FireFrame, paint_fire};
use super::effort_scale::{
    self as scale, NOTCH_AHEAD, NOTCH_OUTLINE, NOTCH_REACHED, PANEL_PAD_X, PANEL_W, THUMB,
    THUMB_FACE, THUMB_MARK, TICK_INACTIVE, TITLE, TRACK_BOTTOM, TRACK_H, TRACK_TOP, TRACK_W,
    fire_front, nearest_stop, ratio_at, resolve_traits, spring_step, stop_center_x, stop_ratio,
    thumb_center_x, visible_ticks,
};
use super::*;

/// The traits menu's id, shared with the handle cache.
const TRAITS_MENU_ID: &str = "model-traits";
const CARD_W: f32 = PANEL_W + 20.0;
/// How long the thumb glides to the level it snapped to.
const GLIDE: Duration = Duration::from_millis(200);
/// The plugin's ignition: cells catch over the first two and a half seconds.
const IGNITION: Duration = Duration::from_millis(2600);

/// What the card shows, rebuilt from the session on every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TraitsSpec {
    session_id: Option<Uuid>,
    /// `(id, label)` of each level, only when there are two or more.
    efforts: Vec<(String, String)>,
    /// The level the session runs with.
    effort_index: usize,
    sections: Vec<ChoiceSection>,
}

#[derive(Clone, Debug, PartialEq)]
struct ChoiceSection {
    header: SharedString,
    rows: Vec<ChoiceRow>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChoiceKind {
    Tier,
    Window,
}

#[derive(Clone, Debug, PartialEq)]
struct ChoiceRow {
    kind: ChoiceKind,
    id: String,
    label: String,
    is_default: bool,
    selected: bool,
}

impl TraitsSpec {
    fn has_slider(&self) -> bool {
        self.efforts.len() >= 2
    }

    fn rows(&self) -> impl Iterator<Item = &ChoiceRow> {
        self.sections.iter().flat_map(|section| section.rows.iter())
    }

    /// Keyboard stops: the slider, if any, then every row.
    fn stop_count(&self) -> usize {
        usize::from(self.has_slider()) + self.rows().count()
    }

    fn row_at_stop(&self, stop: usize) -> Option<&ChoiceRow> {
        let index = stop.checked_sub(usize::from(self.has_slider()))?;
        self.rows().nth(index)
    }

    fn is_empty(&self) -> bool {
        !self.has_slider() && self.sections.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    ratio: f32,
    /// How far from the thumb's centre it was grabbed.
    grab_dx: f32,
}

#[derive(Clone, Copy, Debug)]
struct Glide {
    from: f32,
    to: f32,
    started: Instant,
}

impl Glide {
    fn at(&self, now: Instant) -> Option<f32> {
        let progress = now.saturating_duration_since(self.started).as_secs_f32() / GLIDE.as_secs_f32();
        if progress >= 1.0 {
            return None;
        }
        // Ease-out quint, as the app's other tweens.
        let eased = 1.0 - (1.0 - progress).powi(5);
        Some(self.from + (self.to - self.from) * eased)
    }
}

pub(super) struct EffortCard {
    waku: WeakEntity<Waku>,
    menu: Option<ContextMenuHandle>,
    spec: TraitsSpec,
    drag: Option<Drag>,
    /// A level committed whose session state has not caught up yet, so the
    /// thumb does not jump back for a frame.
    pending: Option<usize>,
    glide: Option<Glide>,
    /// The fire's leading edge and its velocity.
    spring: (f32, f32),
    last_frame: Instant,
    opened_at: Instant,
    clock: Instant,
    /// The keyboard stop: 0 is the slider when there is one, then the rows.
    cursor: usize,
    /// The last interaction was the keyboard, so the cursor shows.
    keyboard: bool,
    track: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl EffortCard {
    fn new(waku: WeakEntity<Waku>) -> Self {
        let now = Instant::now();
        Self {
            waku,
            menu: None,
            spec: TraitsSpec::default(),
            drag: None,
            pending: None,
            glide: None,
            spring: (fire_front(0.0), 0.0),
            last_frame: now,
            opened_at: now,
            clock: now,
            cursor: 0,
            keyboard: false,
            track: Rc::new(Cell::new(None)),
        }
    }

    /// Start a fresh opening: the fire re-ignites from the current level.
    fn reset_for_open(&mut self, menu: ContextMenuHandle) {
        let now = Instant::now();
        self.menu = Some(menu);
        self.drag = None;
        self.pending = None;
        self.glide = None;
        self.cursor = 0;
        self.keyboard = false;
        self.opened_at = now;
        self.last_frame = now;
        self.spring = (fire_front(self.settled_ratio()), 0.0);
    }

    /// Take the session's current traits. A drag in progress is kept unless
    /// the card now describes another session.
    fn sync(&mut self, spec: TraitsSpec) {
        if spec.session_id != self.spec.session_id || spec.efforts != self.spec.efforts {
            self.drag = None;
            self.pending = None;
            self.glide = None;
        }
        if self.pending == Some(spec.effort_index) {
            self.pending = None;
        }
        self.spec = spec;
        self.cursor = self.cursor.min(self.spec.stop_count().saturating_sub(1));
    }

    fn settled_index(&self) -> usize {
        self.pending.unwrap_or(self.spec.effort_index)
    }

    fn settled_ratio(&self) -> f32 {
        stop_ratio(self.settled_index(), self.spec.efforts.len())
    }

    fn display_ratio(&self, now: Instant) -> f32 {
        if let Some(drag) = self.drag {
            return drag.ratio;
        }
        self.glide
            .and_then(|glide| glide.at(now))
            .unwrap_or_else(|| self.settled_ratio())
    }

    fn begin_drag(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
        let Some(track) = self.track.get() else {
            return;
        };
        let left = f32::from(track.origin.x);
        let width = f32::from(track.size.width);
        let center = left + thumb_center_x(width, self.display_ratio(Instant::now()));
        // On the thumb, keep the grab point; on bare track, the thumb's
        // centre jumps to the pointer and drags from there.
        let grab_dx = if (pointer_x - center).abs() <= THUMB / 2.0 {
            pointer_x - center
        } else {
            0.0
        };
        self.drag = Some(Drag {
            ratio: ratio_at(pointer_x, left, width, grab_dx),
            grab_dx,
        });
        self.glide = None;
        self.cursor = 0;
        self.keyboard = false;
        cx.notify();
    }

    fn drag_to(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
        let (Some(drag), Some(track)) = (self.drag.as_mut(), self.track.get()) else {
            return;
        };
        let ratio = ratio_at(
            pointer_x,
            f32::from(track.origin.x),
            f32::from(track.size.width),
            drag.grab_dx,
        );
        if (ratio - drag.ratio).abs() > f32::EPSILON {
            drag.ratio = ratio;
            cx.notify();
        }
    }

    /// Snap to the nearest level and commit it.
    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let index = nearest_stop(drag.ratio, self.spec.efforts.len());
        self.glide = Some(Glide {
            from: drag.ratio,
            to: stop_ratio(index, self.spec.efforts.len()),
            started: Instant::now(),
        });
        self.commit(index, cx);
        cx.notify();
    }

    fn step(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let count = self.spec.efforts.len();
        let Some(index) = scale::key_step(self.settled_index(), count, key) else {
            return false;
        };
        self.glide = Some(Glide {
            from: self.display_ratio(Instant::now()),
            to: stop_ratio(index, count),
            started: Instant::now(),
        });
        self.commit(index, cx);
        cx.notify();
        true
    }

    /// Hand a level to the session. Deferred, so the session's update does
    /// not run while this card is leased, and dropped if the composer has
    /// moved on to another session since.
    fn commit(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((effort, _)) = self.spec.efforts.get(index).cloned() else {
            return;
        };
        if index == self.settled_index() && self.pending.is_none() {
            return;
        }
        self.pending = Some(index);
        let Some(session_id) = self.spec.session_id else {
            return;
        };
        let waku = self.waku.clone();
        cx.defer(move |cx| {
            let _ = waku.update(cx, |waku, cx| {
                if waku.selected_session().map(|session| session.id) == Some(session_id) {
                    waku.set_reasoning_effort(effort, cx);
                }
            });
        });
    }

    fn apply_row(&mut self, stop: usize, cx: &mut Context<Self>) {
        let (Some(row), Some(session_id)) = (self.spec.row_at_stop(stop).cloned(), self.spec.session_id)
        else {
            return;
        };
        let waku = self.waku.clone();
        cx.defer(move |cx| {
            let _ = waku.update(cx, |waku, cx| {
                if waku.selected_session().map(|session| session.id) != Some(session_id) {
                    return;
                }
                match row.kind {
                    ChoiceKind::Tier => waku.set_service_tier(row.id, cx),
                    ChoiceKind::Window => waku.set_context_window(row.id, cx),
                }
            });
        });
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.spec.stop_count();
        if stops == 0 {
            return;
        }
        let on_slider = self.spec.has_slider() && self.cursor == 0;
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let handled = match key {
            "left" | "right" | "home" | "end" if on_slider => {
                self.step(key, cx);
                true
            }
            "down" => {
                self.cursor = (self.cursor + 1).min(stops - 1);
                true
            }
            "up" => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            "tab" if shift => {
                self.cursor = (self.cursor + stops - 1) % stops;
                true
            }
            "tab" => {
                self.cursor = (self.cursor + 1) % stops;
                true
            }
            "enter" | "space" if on_slider => {
                if let Some(menu) = self.menu.clone() {
                    menu.close(window, cx);
                    window.refresh();
                }
                true
            }
            "enter" | "space" => {
                self.apply_row(self.cursor, cx);
                true
            }
            _ => false,
        };
        if handled {
            self.keyboard = true;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn render_panel(
        &mut self,
        ratio: f32,
        reduce_motion: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let count = self.spec.efforts.len();
        let index = nearest_stop(ratio, count);
        let (id, label) = self.spec.efforts[index].clone();
        let (color, glow) = scale::level_style(scale::tone(&id), index + 1 == count);
        let labels = self
            .spec
            .efforts
            .iter()
            .map(|(_, label)| label.to_uppercase())
            .collect::<Vec<_>>();
        let ticks = visible_ticks(&labels, index, TRACK_W);

        let header = div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .mb(px(2.0))
            .text_size(px(14.0))
            .line_height(px(20.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TITLE))
                    .child(tr!("effort_panel.title")),
            )
            .child(
                div()
                    .relative()
                    // gpui has no text-shadow: the glow is a soft blurred
                    // box behind the name.
                    .when_some(glow, |element, glow| {
                        element.child(
                            div().absolute().inset_0().rounded(px(6.0)).shadow(vec![
                                BoxShadow::new(px(0.0), px(0.0), glow.color.opacity(0.45))
                                    .blur_radius(px(glow.blur))
                                    .spread_radius(px(-2.0)),
                            ]),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .italic()
                            .font_weight(FontWeight::BOLD)
                            .font_family("Georgia")
                            .text_color(color)
                            .child(label.to_uppercase()),
                    ),
            );

        // The chosen level's label sits on a pill in its own color, so the
        // eye finds it before reading any of the others.
        let tick_row = div()
            .relative()
            .w(px(TRACK_W))
            .h(px(17.0))
            .mb(px(5.0))
            .children(ticks.into_iter().map(|tick| {
                let width = scale::tick_width(&labels[tick]) + 12.0;
                let left = (stop_center_x(TRACK_W, tick, count) - width / 2.0)
                    .clamp(-(PANEL_PAD_X - 4.0), TRACK_W + PANEL_PAD_X - 4.0 - width);
                let active = tick == index;
                div()
                    .absolute()
                    .top_0()
                    .left(px(left))
                    .w(px(width))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(5.0))
                            .rounded(px(4.0))
                            .whitespace_nowrap()
                            .text_size(px(10.0))
                            .line_height(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .when(active, |label| {
                                label
                                    .bg(color.opacity(0.3))
                                    .border_1()
                                    .border_color(color.opacity(0.75))
                                    .text_color(gpui::white())
                            })
                            .when(!active, |label| label.text_color(rgb(TICK_INACTIVE)))
                            .child(labels[tick].clone()),
                    )
            }));

        let frame = if reduce_motion {
            FireFrame::still(self.spring.0, ratio)
        } else {
            FireFrame {
                front: self.spring.0,
                reveal: ratio,
                time: self.clock.elapsed().as_secs_f32(),
                elapsed: self.opened_at.elapsed().as_secs_f32(),
            }
        };
        let paint = TrackPaint {
            ratio,
            count,
            active: index,
            dragging: self.drag.is_some(),
            focus_ring: (self.cursor == 0 && self.keyboard).then_some(theme.accent),
            frame,
        };
        let card = cx.entity().downgrade();
        let track_bounds = self.track.clone();
        let dragging = self.drag.is_some();
        let track = canvas(
            move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, _| {
                track_bounds.set(Some(bounds));
                window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
                paint_track(window, bounds, &paint);

                window.on_mouse_event({
                    let card = card.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        let x = f32::from(event.position.x);
                        let _ = card.update(cx, |card, cx| card.begin_drag(x, cx));
                    }
                });
                // The release is taken wherever the pointer is, and always
                // listened for: a quick click can come up before the frame
                // that starts listening for the drag's moves.
                window.on_mouse_event({
                    let card = card.clone();
                    move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        let _ = card.update(cx, |card, cx| card.release(cx));
                    }
                });
                if !dragging {
                    return;
                }
                // Moves likewise, so a drag carries on past the track's ends.
                window.on_mouse_event({
                    let card = card.clone();
                    move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let x = f32::from(event.position.x);
                        let released = event.pressed_button != Some(MouseButton::Left);
                        let _ = card.update(cx, |card, cx| {
                            if released {
                                // The button came up outside the window.
                                card.release(cx);
                            } else {
                                card.drag_to(x, cx);
                            }
                        });
                    }
                });
            },
        )
        .w(px(TRACK_W))
        .h(px(TRACK_H));

        div()
            .w(px(PANEL_W))
            .flex()
            .flex_col()
            .pt(px(14.0))
            .px(px(PANEL_PAD_X))
            .pb(px(12.0))
            .rounded(px(13.0))
            .border_1()
            .border_color(scale::purple(0.12))
            .bg(linear_gradient(
                160.0,
                linear_color_stop(rgb(0x0e0a16), 0.0),
                linear_color_stop(rgb(0x0c0818), 1.0),
            ))
            // The plugin's blurred halo behind the card.
            .shadow(vec![
                BoxShadow::new(px(0.0), px(0.0), scale::purple(0.18))
                    .blur_radius(px(8.0))
                    .spread_radius(px(3.0)),
                BoxShadow::new(px(0.0), px(0.0), rgba(0x3b82f614).into())
                    .blur_radius(px(8.0))
                    .spread_radius(px(3.0)),
            ])
            .child(header)
            .child(tick_row)
            .child(
                // Screen readers hear the level's name, not a number.
                div()
                    .id("effort-slider")
                    .role(Role::Slider)
                    .aria_label(tr!("effort_panel.slider_label"))
                    .aria_value(label)
                    .child(track),
            )
    }

    fn render_sections(&self, theme: Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut stop = usize::from(self.spec.has_slider());
        let mut sections = Vec::new();
        for section in &self.spec.sections {
            let mut column = div().flex().flex_col().child(
                div()
                    .px(px(10.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .text_size(sp(12.5))
                    .line_height(sp(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_tertiary)
                    .child(section.header.clone()),
            );
            for row in &section.rows {
                let at = stop;
                stop += 1;
                let highlighted = self.keyboard && self.cursor == at;
                column = column.child(
                    div()
                        .id(("effort-card-row", at))
                        .h(px(28.0))
                        .px(px(10.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(if highlighted {
                            theme.accent
                        } else {
                            gpui::transparent_black()
                        })
                        .when(highlighted, |element| element.bg(theme.overlay))
                        .hover(|element| element.bg(theme.overlay))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(sp(13.0))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cursor = at;
                            this.keyboard = false;
                            this.apply_row(at, cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_color(theme.text_secondary)
                                .child(row.label.clone()),
                        )
                        .when(row.is_default, |element| {
                            element.child(
                                div()
                                    .h(px(18.0))
                                    .px(px(5.0))
                                    .flex_none()
                                    .rounded(px(4.0))
                                    .border_1()
                                    .border_color(theme.border_strong)
                                    .bg(theme.overlay)
                                    .flex()
                                    .items_center()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_tertiary)
                                    .child(tr!("common.default")),
                            )
                        })
                        .when(row.selected, |element| {
                            element.child(icon("icons/check.svg", 11.0, theme.text_tertiary))
                        }),
                );
            }
            sections.push(column.into_any_element());
        }
        sections
    }
}

impl Render for EffortCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let now = Instant::now();
        let reduce_motion = cx.reduce_motion();
        if self.glide.is_some_and(|glide| glide.at(now).is_none()) {
            self.glide = None;
        }
        let ratio = self.display_ratio(now);
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let target = fire_front(ratio);
        self.spring = if reduce_motion {
            (target, 0.0)
        } else {
            spring_step(self.spring.0, self.spring.1, target, dt)
        };

        if self.spec.has_slider() && !reduce_motion {
            // Full rate while something moves; the idle flicker is fine at
            // half. Only this card is leased, so panes replay from cache.
            let moving = self.drag.is_some()
                || self.glide.is_some()
                || self.spring.0 < target - 1e-3
                || self.opened_at.elapsed() < IGNITION;
            if moving {
                motion::pulse_lease(window.current_view(), cx);
            } else {
                motion::pulse_lease_slow(window.current_view(), cx);
            }
        }

        let panel = self
            .spec
            .has_slider()
            .then(|| self.render_panel(ratio, reduce_motion, theme, cx));
        let sections = self.render_sections(theme, cx);
        div()
            .when_some(self.menu.clone(), |element, menu| {
                element.track_focus(menu.focus_handle())
            })
            .on_key_down(cx.listener(Self::on_key))
            .w(px(CARD_W))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(13.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.raised)
            .shadow_lg()
            .children(panel)
            .children(sections)
    }
}

/// What the track paints in one frame.
struct TrackPaint {
    ratio: f32,
    count: usize,
    active: usize,
    dragging: bool,
    focus_ring: Option<Hsla>,
    frame: FireFrame,
}

fn paint_track(window: &mut Window, bounds: Bounds<Pixels>, paint: &TrackPaint) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let middle = bounds.origin.y + px(height / 2.0);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        // A neutral near-black track, so the stops, the thumb and the fire
        // all read against it.
        window.paint_quad(
            fill(
                bounds,
                linear_gradient(
                    180.0,
                    linear_color_stop(rgb(TRACK_TOP), 0.0),
                    linear_color_stop(rgb(TRACK_BOTTOM), 1.0),
                ),
            )
            .corner_radii(px(8.0))
            .border_widths(px(1.0))
            .border_color(rgba(0xffffff26)),
        );

        paint_fire(window, bounds, paint.frame);

        // The stops, over the fire: a notch at each, bright up to the
        // chosen level and dim past it, outlined in near-black so a notch
        // reads on the flame as well as on the bare track.
        for stop in 0..paint.count {
            let center = bounds.origin.x + px(stop_center_x(width, stop, paint.count));
            let reached = stop <= paint.active;
            let (notch_w, notch_h) = if reached { (3.0, 14.0) } else { (2.0, 10.0) };
            window.paint_quad(
                fill(
                    Bounds {
                        origin: point(
                            center - px(notch_w / 2.0 + 1.0),
                            middle - px(notch_h / 2.0 + 1.0),
                        ),
                        size: size(px(notch_w + 2.0), px(notch_h + 2.0)),
                    },
                    rgba(NOTCH_OUTLINE << 8 | 0xb3),
                )
                .corner_radii(px(2.0)),
            );
            window.paint_quad(
                fill(
                    Bounds {
                        origin: point(center - px(notch_w / 2.0), middle - px(notch_h / 2.0)),
                        size: size(px(notch_w), px(notch_h)),
                    },
                    rgb(if reached { NOTCH_REACHED } else { NOTCH_AHEAD }),
                )
                .corner_radii(px(1.0)),
            );
        }

        let thumb_center = bounds.origin.x + px(thumb_center_x(width, paint.ratio));
        if paint.dragging {
            // A light that follows the pointer while it holds the thumb.
            window.paint_drop_shadows(
                Bounds {
                    origin: point(thumb_center - px(16.0), middle - px(16.0)),
                    size: size(px(32.0), px(32.0)),
                },
                px(16.0).into(),
                &[BoxShadow::new(px(0.0), px(0.0), scale::purple(0.2)).blur_radius(px(30.0))],
            );
        }

        let thumb = Bounds {
            origin: point(thumb_center - px(THUMB / 2.0), middle - px(THUMB / 2.0)),
            size: size(px(THUMB), px(THUMB)),
        };
        let mut shadows = vec![
            BoxShadow::new(px(0.0), px(2.0), rgba(0x00000080).into()).blur_radius(px(8.0)),
        ];
        if paint.dragging {
            shadows.push(
                BoxShadow::new(px(0.0), px(0.0), scale::purple(0.35)).blur_radius(px(20.0)),
            );
        }
        if let Some(ring) = paint.focus_ring {
            shadows.push(BoxShadow::new(px(0.0), px(0.0), ring).spread_radius(px(2.0)));
        }
        window.paint_drop_shadows(thumb, px(8.0).into(), &shadows);
        window.paint_quad(
            fill(
                thumb,
                linear_gradient(
                    180.0,
                    linear_color_stop(rgb(THUMB_FACE), 0.0),
                    linear_color_stop(rgb(0xc4b8dc), 1.0),
                ),
            )
            .corner_radii(px(8.0))
            .border_widths(px(1.0))
            .border_color(rgba(0x0000008c)),
        );
        // A dark line down the thumb's middle: exactly where it sits, which
        // the stop it covers can no longer say.
        window.paint_quad(
            fill(
                Bounds {
                    origin: point(thumb_center - px(1.0), middle - px(8.0)),
                    size: size(px(2.0), px(16.0)),
                },
                rgb(THUMB_MARK),
            )
            .corner_radii(px(1.0)),
        );
    });
}

impl Waku {
    /// The traits chip and its card, in place of the traits dropdown.
    pub(super) fn render_effort_card_control(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = Theme::current(cx);
        let session = self.selected_session()?;
        let model = self.model_metadata_for_session(session)?;
        if model.reasoning_efforts.is_empty()
            && model.service_tiers.is_empty()
            && model.context_windows.is_empty()
        {
            return None;
        }
        let traits = resolve_traits(session, model);
        // The built-in agent's tier slot carries the wire format: the one
        // API the model goes over, with no "standard" among them.
        let tiers_are_wire_formats = session.provider.is_builtin();

        let effort = traits.effort.as_deref().and_then(|selected| {
            model
                .reasoning_efforts
                .iter()
                .position(|option| option.id == selected)
                .map(|index| (index, &model.reasoning_efforts[index]))
        });
        let tier_label = if traits.tier == "default" {
            tr!("models.standard")
        } else {
            model
                .service_tiers
                .iter()
                .find(|option| option.id == traits.tier)
                .map(|option| option.label.clone())
                .unwrap_or_else(|| traits.tier.clone())
        };
        // A non-default window changes what the session costs and holds, so
        // it reads on the chip rather than only inside the card.
        let window_label = traits
            .window
            .as_deref()
            .filter(|selected| model.default_context_window.as_deref() != Some(selected))
            .and_then(|selected| {
                model
                    .context_windows
                    .iter()
                    .find(|option| option.id == selected)
                    .map(|option| option.label.clone())
            });
        let fast = traits.tier == "fast" || tier_label.eq_ignore_ascii_case("fast");

        let mut sections = Vec::new();
        if !model.service_tiers.is_empty() {
            let mut rows = Vec::new();
            let default_tier = model
                .default_service_tier
                .clone()
                .unwrap_or_else(|| "default".to_owned());
            if !tiers_are_wire_formats {
                rows.push(ChoiceRow {
                    kind: ChoiceKind::Tier,
                    id: "default".to_owned(),
                    label: tr!("models.standard"),
                    is_default: default_tier == "default",
                    selected: traits.tier == "default",
                });
            }
            rows.extend(model.service_tiers.iter().map(|option| ChoiceRow {
                kind: ChoiceKind::Tier,
                id: option.id.clone(),
                label: option.label.clone(),
                is_default: default_tier == option.id,
                selected: traits.tier == option.id,
            }));
            sections.push(ChoiceSection {
                header: if tiers_are_wire_formats {
                    tr!("models.wire_format")
                } else {
                    tr!("models.service_tier")
                }
                .into(),
                rows,
            });
        }
        if !model.context_windows.is_empty() {
            sections.push(ChoiceSection {
                header: tr!("models.context_window").into(),
                rows: model
                    .context_windows
                    .iter()
                    .map(|option| ChoiceRow {
                        kind: ChoiceKind::Window,
                        id: option.id.clone(),
                        label: option.label.clone(),
                        is_default: model.default_context_window.as_deref()
                            == Some(option.id.as_str()),
                        selected: traits.window.as_deref() == Some(option.id.as_str()),
                    })
                    .collect(),
            });
        }
        let spec = TraitsSpec {
            session_id: Some(session.id),
            efforts: if model.reasoning_efforts.len() >= 2 {
                model
                    .reasoning_efforts
                    .iter()
                    .map(|option| (option.id.clone(), option.label.clone()))
                    .collect()
            } else {
                Vec::new()
            },
            effort_index: effort.map_or(0, |(index, _)| index),
            sections,
        };

        let weak = cx.entity().downgrade();
        let handle = self.menu_handle_with(TRAITS_MENU_ID, cx, move |open, window, cx| {
            if open {
                let mut card_focus = None;
                let _ = weak.update(cx, |this, cx| {
                    let Some(menu) = this.menus.borrow().get(TRAITS_MENU_ID).cloned() else {
                        return;
                    };
                    let waku = cx.entity().downgrade();
                    let card = this
                        .effort_card
                        .get_or_insert_with(|| cx.new(|_| EffortCard::new(waku)))
                        .clone();
                    card_focus = Some(menu.focus_handle().clone());
                    card.update(cx, |card, _| card.reset_for_open(menu));
                    cx.notify();
                });
                // The card is deferred, so it joins the dispatch tree a frame
                // late — the menus' two-frame wait. Focused, its menu context
                // is what lets `escape` dismiss it and the arrows reach the
                // slider.
                if let Some(focus) = card_focus {
                    window.on_next_frame(move |window, _| {
                        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
                    });
                }
            } else {
                let mut composer_focus = None;
                let _ = weak.update(cx, |this, cx| {
                    composer_focus = Some(this.composer.read(cx).focus());
                    cx.notify();
                });
                if let Some(focus) = composer_focus {
                    window.focus(&focus, cx);
                }
            }
        });

        let open = handle.is_open();
        let chip = effort_chip(
            theme,
            effort.map(|(_, option)| (option.id.as_str(), option.label.clone())),
            tier_label,
            window_label,
            fast,
            open,
        );
        // One level and nothing else to choose: there is no card to open.
        if spec.is_empty() {
            return Some(chip.into_any_element());
        }
        if let Some(card) = &self.effort_card {
            card.update(cx, |card, _| card.sync(spec));
        }
        let card = self.effort_card.clone();
        Some(popover(chip, &handle, MenuAlign::AboveLeft, move |_, _, _| {
            card.clone()
                .map(|card| card.into_any_element())
                .unwrap_or_else(|| div().into_any_element())
        }))
    }
}

/// The traits chip: [`MenuChip`]'s metrics, with the level drawn in its own
/// color — the closed control says how hard the model will think.
fn effort_chip(
    theme: Theme,
    effort: Option<(&str, String)>,
    tier_label: String,
    window_label: Option<String>,
    fast: bool,
    open: bool,
) -> Stateful<Div> {
    let (label, color) = match effort {
        Some((id, label)) => (label, scale::chip_color(scale::tone(id), theme.is_dark)),
        None => (tier_label, theme.text_secondary),
    };
    div()
        .id("model-traits")
        .h(px(26.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(sp(13.0))
        .line_height(sp(16.0))
        .cursor_default()
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .when(open, |element| element.bg(theme.overlay))
        .hover(|element| element.bg(theme.overlay))
        .when(fast, |element| {
            element.child(icon("icons/zap.svg", 12.0, theme.text_secondary))
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .flex()
                .child(div().text_color(color).child(label))
                .when_some(window_label, |element, window| {
                    element.child(
                        div()
                            .text_color(theme.text_secondary)
                            .child(format!(" · {window}")),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: ChoiceKind, id: &str) -> ChoiceRow {
        ChoiceRow {
            kind,
            id: id.to_owned(),
            label: id.to_owned(),
            is_default: false,
            selected: false,
        }
    }

    fn spec(levels: usize, tiers: usize, windows: usize) -> TraitsSpec {
        let mut sections = Vec::new();
        if tiers > 0 {
            sections.push(ChoiceSection {
                header: "Tier".into(),
                rows: (0..tiers).map(|i| row(ChoiceKind::Tier, &format!("t{i}"))).collect(),
            });
        }
        if windows > 0 {
            sections.push(ChoiceSection {
                header: "Window".into(),
                rows: (0..windows)
                    .map(|i| row(ChoiceKind::Window, &format!("w{i}")))
                    .collect(),
            });
        }
        TraitsSpec {
            session_id: Some(Uuid::nil()),
            efforts: (0..levels)
                .map(|i| (format!("e{i}"), format!("E{i}")))
                .collect(),
            effort_index: 0,
            sections,
        }
    }

    #[test]
    fn the_slider_is_the_first_keyboard_stop_when_there_is_one() {
        let with_slider = spec(3, 2, 2);
        assert!(with_slider.has_slider());
        assert_eq!(with_slider.stop_count(), 5);
        assert_eq!(with_slider.row_at_stop(0), None);
        assert_eq!(with_slider.row_at_stop(1).map(|row| row.id.as_str()), Some("t0"));
        assert_eq!(with_slider.row_at_stop(4).map(|row| row.id.as_str()), Some("w1"));
        assert_eq!(with_slider.row_at_stop(5), None);

        let rows_only = spec(1, 2, 0);
        assert!(!rows_only.has_slider());
        assert_eq!(rows_only.stop_count(), 2);
        assert_eq!(rows_only.row_at_stop(0).map(|row| row.id.as_str()), Some("t0"));
    }

    #[test]
    fn one_level_and_nothing_else_has_no_card() {
        assert!(spec(1, 0, 0).is_empty());
        assert!(spec(0, 0, 0).is_empty());
        assert!(!spec(2, 0, 0).is_empty());
        assert!(!spec(1, 1, 0).is_empty());
    }

    #[test]
    fn the_glide_eases_to_its_stop_and_ends() {
        let started = Instant::now();
        let glide = Glide {
            from: 0.0,
            to: 1.0,
            started,
        };
        let early = glide.at(started + Duration::from_millis(40)).expect("moving");
        let late = glide.at(started + Duration::from_millis(160)).expect("moving");
        assert!(early > 0.0 && early < late && late < 1.0);
        assert_eq!(glide.at(started + GLIDE), None);
    }
}
