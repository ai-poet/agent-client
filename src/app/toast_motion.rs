//! The notice toast fades out instead of vanishing, and its countdown holds
//! while the window is in the background.
//!
//! Fork addition, after Zeron's toasts (crates/ui/src/toast.rs; MIT,
//! Copyright (c) 2026 Wing). Upstream's toast enters with a slide and leaves
//! in a single frame. A dismissed or expired toast is now kept on screen for
//! [`TOAST_OUT`], drawn by upstream's own `render_toast` and fading to nothing,
//! then dropped. A toast replaced by a newer one does not fade: the new one
//! takes its place. A notice that counted down while another app was in front
//! was usually gone before anyone saw it, so leaving the window pauses the
//! countdown the way hovering the toast does.

use std::cell::{Cell, RefCell};

use super::*;
use crate::ui::motion_kit::{EASE, MotionSpec};

/// How long a leaving toast takes to fade out.
const TOAST_OUT: MotionSpec = MotionSpec::new(200, EASE);

#[derive(Default)]
pub(super) struct ToastMotion {
    leaving: RefCell<Option<LeavingToast>>,
    /// The countdown is held because the window lost focus, not the pointer.
    paused_by_window: Cell<bool>,
}

struct LeavingToast {
    message: String,
    tone: ToastTone,
    id: u64,
    started: Instant,
    /// The wake-up that drops it once faded has been armed.
    armed: bool,
}

impl ToastMotion {
    /// From `hide_toast`, as the toast leaves.
    pub(super) fn leave(&self, toast: &ToastState) {
        *self.leaving.borrow_mut() = Some(LeavingToast {
            message: toast.message.clone(),
            tone: toast.tone,
            id: toast.id,
            started: Instant::now(),
            armed: false,
        });
    }

    /// A new toast has the stage; the old one stops fading.
    fn replaced(&self) {
        self.leaving.borrow_mut().take();
    }
}

impl Waku {
    /// The leaving toast, while it fades. From `render_active_toast` when no
    /// toast is showing.
    pub(super) fn render_leaving_toast(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (message, tone, id, elapsed, arm) = {
            let mut leaving = self.fork_motion.toast.leaving.borrow_mut();
            let toast = leaving.as_mut()?;
            let elapsed = toast.started.elapsed();
            if cx.reduce_motion() || elapsed >= TOAST_OUT.total() {
                *leaving = None;
                return None;
            }
            let arm = !std::mem::replace(&mut toast.armed, true);
            (toast.message.clone(), toast.tone, toast.id, elapsed, arm)
        };
        if arm {
            let remaining = TOAST_OUT.total().saturating_sub(elapsed);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(remaining + Duration::from_millis(16))
                    .await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        let fade = 1.0 - TOAST_OUT.progress_after(elapsed).unwrap_or(1.0);
        Some(
            div()
                .opacity(fade)
                .child(self.render_toast(message, tone, id, cx))
                .with_animation(
                    SharedString::from(format!("toast-leave-{id}")),
                    TOAST_OUT.animation(),
                    |element, _| element,
                )
                .into_any_element(),
        )
    }

    /// From the window-activation observer.
    pub(super) fn toast_window_activation_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        let paused = &self.fork_motion.toast.paused_by_window;
        if active {
            if paused.replace(false) {
                self.set_toast_hovered(false, cx);
            }
        } else if self.toast.as_ref().is_some_and(|toast| !toast.hovered) {
            paused.set(true);
            self.set_toast_hovered(true, cx);
        }
    }

    /// From `show_toast_with_tone`, as a new toast takes the stage.
    pub(super) fn toast_shown(&mut self) {
        self.fork_motion.toast.replaced();
        self.fork_motion.toast.paused_by_window.set(false);
    }
}
