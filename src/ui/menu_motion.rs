//! Menu and popover motion, the click that only dismisses, and submenu hover
//! intent.
//!
//! Fork addition, translated from Zeron's popover (crates/ui/src/popover.rs,
//! crates/ui/src/popover/hover_intent.rs; MIT, Copyright (c) 2026 Wing) onto
//! the upstream menu in [`super`]:
//!
//! - **Entrance and exit.** A surface fades in travelling from its trigger's
//!   side, and on close stays drawn for [`MENU_OUT`] while it fades back
//!   toward it. The exit's progress comes from the wall clock, not from an
//!   animation element's clock, which would replay from zero if the card
//!   remounted. A closing surface lies under a transparent occluding layer,
//!   so its rows take no clicks or hover, and its key handler refuses keys.
//! - **Outside clicks only dismiss.** A press outside an open surface closes
//!   it in the capture phase, as upstream does, and the same press is then
//!   kept from the element underneath — macOS menus behave this way, and a
//!   click meant to put a menu away should not also select the sidebar row
//!   below it. Every surface's capture listener still runs, so nested
//!   surfaces all close; the swallowing happens in the bubble phase, where
//!   the card's listener, registered last, runs first. A left click on a
//!   surface's own trigger is exempt, as before: its toggle has to see it.
//! - **Hover intent.** Moving from a row with an open submenu toward that
//!   submenu crosses other rows. While the pointer is inside the corridor from
//!   where it last was on the parent row to the submenu's near edge, a newly
//!   hovered row waits [`HOVER_GRACE`] before it takes over; reaching the
//!   submenu cancels the wait, and pausing on the row lets it happen.

use std::time::{Duration, Instant};

use gpui::{AppContext as _, Div, DispatchPhase, MouseMoveEvent, Stateful};

use super::*;
use crate::ui::motion_kit::{MENU_OUT, menu_in_from, menu_out_toward};

/// How long a row crossed on the way to a submenu waits before taking over.
const HOVER_GRACE: Duration = Duration::from_millis(300);

/// A dropdown opening below its trigger starts this far above its resting
/// place; one opening upward starts as far below it.
pub(super) const DROPDOWN_TRAVEL: f32 = 4.0;
/// A context menu's travel: it opens at the pointer, not out of a trigger.
pub(super) const CONTEXT_MENU_TRAVEL: f32 = -2.0;

/// The entrance offset of a dropdown: from above when it drops down, from
/// below when it opens upward.
pub(super) fn dropdown_travel(align: MenuAlign) -> f32 {
    if align.above() {
        DROPDOWN_TRAVEL
    } else {
        -DROPDOWN_TRAVEL
    }
}

#[derive(Debug, Default)]
pub(super) struct MenuMotion {
    /// Set while a just-closed surface fades out.
    closing: Option<Closing>,
    /// The press that closed this surface; its bubble phase stops at the card.
    swallow_press: bool,
    /// Where the pointer last was over the row whose submenu is open.
    submenu_origin: Option<Point<Pixels>>,
    /// The open submenu card, as of the last frame.
    submenu_bounds: Option<Bounds<Pixels>>,
    /// A row waiting out [`HOVER_GRACE`], with the generation that set it.
    pending: Option<(PendingHover, u64)>,
    generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct Closing {
    position: Point<Pixels>,
    started: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingHover {
    Highlight(usize),
    Submenu(usize),
}

/// From [`ContextMenuHandle::close`], before the open position is cleared.
pub(super) fn remember_closing(state: &mut MenuState, cx: &App) {
    state.motion.pending = None;
    state.motion.submenu_origin = None;
    state.motion.closing = state
        .open
        .filter(|_| !cx.reduce_motion())
        .map(|position| Closing {
            position,
            started: Instant::now(),
        });
}

/// From [`ContextMenuHandle::close`], once closed: one more frame after the
/// exit, so the faded card leaves the tree.
pub(super) fn after_close(handle: &ContextMenuHandle, window: &mut Window, cx: &mut App) {
    if handle.state.borrow().motion.closing.is_none() {
        return;
    }
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(MENU_OUT.total() + Duration::from_millis(16))
            .await;
        let _ = cx.update_window(window_handle, |_, window, _| window.refresh());
    })
    .detach();
}

/// From [`ContextMenuHandle::open_at`]: reopening ends a fade in progress.
pub(super) fn reopened(state: &mut MenuState) {
    state.motion.closing = None;
    state.motion.swallow_press = false;
}

/// Where a closing surface is still drawn, or `None` once its exit is over.
pub(super) fn closing_position(handle: &ContextMenuHandle) -> Option<Point<Pixels>> {
    let mut state = handle.state.borrow_mut();
    let closing = state.motion.closing?;
    if closing.started.elapsed() >= MENU_OUT.total() {
        state.motion.closing = None;
        return None;
    }
    Some(closing.position)
}

/// Whether the surface acts on keys: a closing one does not.
pub(super) fn accepts_keys(handle: &ContextMenuHandle) -> bool {
    let mut state = handle.state.borrow_mut();
    state.motion.pending = None;
    state.open.is_some()
}

/// The card, entering or leaving. `travel` is the entrance's signed starting
/// offset; the exit retreats the same way.
pub(super) fn animate(card: AnyElement, handle: &ContextMenuHandle, travel: f32) -> AnyElement {
    let (open, closing) = {
        let state = handle.state.borrow();
        (state.open.is_some(), state.motion.closing)
    };
    match closing.filter(|_| !open) {
        None => menu_in_from("menu-in", travel, div().child(card)).into_any_element(),
        Some(closing) => {
            let t = MENU_OUT
                .progress_after(closing.started.elapsed())
                .unwrap_or(1.0);
            menu_out_toward(
                "menu-out",
                travel,
                t,
                div()
                    .relative()
                    .child(card)
                    .child(div().absolute().inset_0().occlude()),
            )
            .into_any_element()
        }
    }
}

/// From the outside-press dismissal, before it closes the surface: an open
/// surface keeps the press from what lies underneath. A surface already fading
/// out does not — the press is aimed at something else.
pub(super) fn swallow_press(handle: &ContextMenuHandle) {
    let mut state = handle.state.borrow_mut();
    state.motion.swallow_press = state.open.is_some();
}

/// A zero-size listener on the card that ends the press which closed it.
pub(super) fn outside_press_guard(handle: &ContextMenuHandle) -> impl IntoElement {
    let handle = handle.clone();
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let handle = handle.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble
                    && std::mem::take(&mut handle.state.borrow_mut().motion.swallow_press)
                {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// Records the open submenu card's bounds for the hover corridor.
pub(super) fn submenu_bounds_probe(handle: &ContextMenuHandle) -> impl IntoElement {
    let state = handle.state.clone();
    canvas(
        move |bounds: Bounds<Pixels>, _, _| state.borrow_mut().motion.submenu_bounds = Some(bounds),
        |_, _, _, _| (),
    )
    .absolute()
    .inset_0()
}

/// A submenu parent row: remember where the pointer is on it.
pub(super) fn track_submenu_origin(
    row: Stateful<Div>,
    handle: ContextMenuHandle,
    index: usize,
) -> Stateful<Div> {
    row.on_mouse_move(move |event: &MouseMoveEvent, _, _| {
        let mut state = handle.state.borrow_mut();
        if state.active_submenu == Some(index) {
            state.motion.submenu_origin = Some(event.position);
            state.motion.pending = None;
        }
    })
}

/// A submenu parent row was entered: remember where.
pub(super) fn submenu_entered(handle: &ContextMenuHandle, window: &Window) {
    handle.state.borrow_mut().motion.submenu_origin = Some(window.mouse_position());
}

/// A row inside the submenu was hovered: the pointer made it.
pub(super) fn cancel_pending(state: &mut MenuState) {
    state.motion.pending = None;
}

/// A root row is about to take the hover. `true` when it must wait, because
/// the pointer is on its way to the open submenu; the wait is scheduled here.
pub(super) fn defer_hover(
    handle: &ContextMenuHandle,
    hover: PendingHover,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let pointer = window.mouse_position();
    let generation = {
        let mut state = handle.state.borrow_mut();
        let target = match hover {
            PendingHover::Highlight(index) | PendingHover::Submenu(index) => index,
        };
        let toward_submenu = state.active_submenu.is_some_and(|active| active != target)
            && state
                .motion
                .submenu_origin
                .zip(state.motion.submenu_bounds)
                .is_some_and(|(origin, bounds)| in_corridor(origin, pointer, bounds));
        if !toward_submenu {
            state.motion.pending = None;
            return false;
        }
        state.motion.generation += 1;
        let generation = state.motion.generation;
        state.motion.pending = Some((hover, generation));
        generation
    };
    let handle = handle.clone();
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(HOVER_GRACE).await;
        let _ = cx.update_window(window_handle, |_, window, _| {
            apply_pending(&handle, generation, window);
        });
    })
    .detach();
    true
}

fn apply_pending(handle: &ContextMenuHandle, generation: u64, window: &mut Window) {
    let pointer = window.mouse_position();
    let hover = {
        let mut state = handle.state.borrow_mut();
        // Resting on the submenu card's padding still counts as arriving.
        if state
            .motion
            .submenu_bounds
            .is_some_and(|bounds| bounds.contains(&pointer))
        {
            state.motion.pending = None;
            return;
        }
        match state.motion.pending {
            Some((hover, pending)) if pending == generation && state.open.is_some() => {
                state.motion.pending = None;
                hover
            }
            _ => return,
        }
    };
    match hover {
        PendingHover::Submenu(index) => open_submenu(handle, index, false),
        PendingHover::Highlight(index) => {
            let mut state = handle.state.borrow_mut();
            state.highlighted = Some(index);
            state.active_submenu = None;
            state.submenu_highlighted = None;
            state.submenu_focused = false;
        }
    }
    window.refresh();
}

/// Whether `pointer` is inside the triangle from `origin` to the near (left)
/// edge of the submenu, widened by 8px above and below.
fn in_corridor(origin: Point<Pixels>, pointer: Point<Pixels>, submenu: Bounds<Pixels>) -> bool {
    let distance = f32::from(submenu.left() - origin.x);
    let advance = f32::from(pointer.x - origin.x);
    if distance <= 0.0 || advance <= 0.0 || advance > distance + 8.0 {
        return false;
    }
    let fraction = (advance / distance).min(1.0);
    let top = origin.y + (submenu.top() - px(8.0) - origin.y) * fraction;
    let bottom = origin.y + (submenu.bottom() + px(8.0) - origin.y) * fraction;
    pointer.y >= top && pointer.y <= bottom
}

#[cfg(test)]
mod motion_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use gpui::{Context, Modifiers, Render, TestAppContext, point, size};

    /// A dropdown at the window origin (120×32, card hanging below) beside a
    /// counter far from both.
    struct Harness {
        handle: ContextMenuHandle,
        presses: Rc<Cell<usize>>,
        activated: Rc<Cell<bool>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let presses = self.presses.clone();
            let activated = self.activated.clone();
            div()
                .size_full()
                .child(dropdown_menu(
                    div().w(px(120.0)).h(px(32.0)),
                    "motion-dropdown",
                    &self.handle,
                    MenuAlign::BelowLeft,
                    move |_| {
                        let activated = activated.clone();
                        vec![MenuItem::new("Entry", move |_, _| activated.set(true))]
                    },
                ))
                .child(
                    div()
                        .absolute()
                        .left(px(400.0))
                        .top(px(300.0))
                        .w(px(100.0))
                        .h(px(100.0))
                        .on_mouse_down(MouseButton::Left, move |_, _, _| {
                            presses.set(presses.get() + 1);
                        }),
                )
        }
    }

    fn harness(cx: &mut TestAppContext) -> (Harness, Rc<Cell<usize>>, Rc<Cell<bool>>) {
        let handle = cx.update(ContextMenuHandle::new);
        let presses = Rc::new(Cell::new(0));
        let activated = Rc::new(Cell::new(false));
        (
            Harness {
                handle,
                presses: presses.clone(),
                activated: activated.clone(),
            },
            presses,
            activated,
        )
    }

    const TRIGGER: (f32, f32) = (10.0, 10.0);
    const ENTRY: (f32, f32) = (20.0, 50.0);
    const ELSEWHERE: (f32, f32) = (450.0, 350.0);

    fn at((x, y): (f32, f32)) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[gpui::test]
    fn a_press_that_dismisses_does_not_reach_what_lies_underneath(cx: &mut TestAppContext) {
        let (harness, presses, _) = harness(cx);
        let handle = harness.handle.clone();
        let (_view, cx) = cx.add_window_view(|_, _| harness);

        cx.simulate_mouse_down(at(TRIGGER), MouseButton::Left, Modifiers::none());
        assert!(handle.is_open());
        cx.run_until_parked();

        cx.simulate_mouse_down(at(ELSEWHERE), MouseButton::Left, Modifiers::none());
        assert!(!handle.is_open(), "the press dismisses");
        assert_eq!(presses.get(), 0, "and goes no further");

        cx.simulate_mouse_down(at(ELSEWHERE), MouseButton::Left, Modifiers::none());
        assert_eq!(presses.get(), 1, "with the menu gone, presses arrive");
    }

    #[gpui::test]
    fn a_closing_menu_takes_no_clicks_and_reopens_from_its_trigger(cx: &mut TestAppContext) {
        let (harness, _, activated) = harness(cx);
        let handle = harness.handle.clone();
        let (_view, cx) = cx.add_window_view(|_, _| harness);

        cx.simulate_mouse_down(at(TRIGGER), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_down(at(TRIGGER), MouseButton::Left, Modifiers::none());
        assert!(!handle.is_open());
        assert!(
            handle.state.borrow().motion.closing.is_some(),
            "the card fades out instead of vanishing"
        );

        cx.simulate_mouse_down(at(ENTRY), MouseButton::Left, Modifiers::none());
        assert!(!activated.get(), "a fading card's rows take no clicks");

        cx.simulate_mouse_down(at(TRIGGER), MouseButton::Left, Modifiers::none());
        assert!(handle.is_open(), "the trigger reopens it mid-fade");
        assert!(handle.state.borrow().motion.closing.is_none());
    }

    fn submenu() -> Bounds<Pixels> {
        Bounds::new(point(px(200.0), px(80.0)), size(px(160.0), px(160.0)))
    }

    #[test]
    fn the_corridor_covers_diagonal_travel_toward_the_submenu() {
        let origin = point(px(100.0), px(100.0));
        assert!(in_corridor(origin, point(px(150.0), px(140.0)), submenu()));
        assert!(in_corridor(origin, point(px(195.0), px(230.0)), submenu()));
    }

    #[test]
    fn the_corridor_excludes_moving_away_or_off_its_slope() {
        let origin = point(px(100.0), px(100.0));
        // Backwards, or straight down without moving toward it.
        assert!(!in_corridor(origin, point(px(90.0), px(120.0)), submenu()));
        assert!(!in_corridor(origin, point(px(100.0), px(140.0)), submenu()));
        // Steeper than the line to the submenu's bottom corner.
        assert!(!in_corridor(origin, point(px(110.0), px(160.0)), submenu()));
        // Past the submenu's near edge.
        assert!(!in_corridor(origin, point(px(260.0), px(120.0)), submenu()));
    }

    #[test]
    fn a_submenu_on_the_wrong_side_has_no_corridor() {
        let origin = point(px(300.0), px(100.0));
        assert!(!in_corridor(origin, point(px(320.0), px(110.0)), submenu()));
    }
}
