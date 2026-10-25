//! Reduce motion: follow the system, always, or never, and optionally hold
//! still while the window is in the background.
//!
//! Fork addition, after Zeron's motion preference (crates/ui/src/motion.rs;
//! MIT, Copyright (c) 2026 Wing). GPUI keeps one app-wide flag that every
//! animation already reads (`cx.reduce_motion()`), so the preference only
//! decides what that flag holds. Under "System" the operating system is asked
//! again whenever the window becomes active, which upstream did only at
//! launch: turning the system setting on while the app sat behind another
//! window used to need a restart.

use gpui::Global;
use sub2api::ui_prefs::{self, ReduceMotion, UiPrefs};

use super::*;

struct MotionPrefs(UiPrefs);

impl Global for MotionPrefs {}

/// At launch, in place of upstream's one-time read of the system setting.
pub fn init(cx: &mut App) {
    cx.set_global(MotionPrefs(ui_prefs::load()));
    apply(true, cx);
}

/// Put the preference into GPUI's flag. "System" re-reads the operating
/// system, which on Linux answers off the UI thread.
fn apply(window_active: bool, cx: &mut App) {
    match cx.global::<MotionPrefs>().0.fixed_reduce_motion(window_active) {
        Some(reduce) => cx.set_reduce_motion(reduce),
        None => crate::platform::init_reduce_motion(cx),
    }
}

fn reduce_motion_label(choice: ReduceMotion) -> String {
    match choice {
        ReduceMotion::System => tr!("settings.reduce_motion_system"),
        ReduceMotion::On => tr!("settings.reduce_motion_on"),
        ReduceMotion::Off => tr!("settings.reduce_motion_off"),
    }
}

impl Waku {
    /// From the window-activation observer.
    pub(super) fn motion_window_activation_changed(&mut self, active: bool, cx: &mut App) {
        apply(active, cx);
    }

    fn update_motion_prefs(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut UiPrefs),
    ) {
        let mut prefs = cx.global::<MotionPrefs>().0.clone();
        change(&mut prefs);
        // A one-shot click writing a few bytes: done in place so two quick
        // changes cannot land on disk out of order.
        if let Err(error) = ui_prefs::save(&prefs) {
            self.show_toast(error.to_string());
        }
        cx.set_global(MotionPrefs(prefs));
        apply(window.is_window_active(), cx);
        cx.notify();
    }

    /// The rows appended to Settings → Appearance.
    pub(super) fn render_motion_settings_rows(&self, cx: &mut Context<Self>) -> [AnyElement; 4] {
        let theme = Theme::current(cx);
        let prefs = cx.global::<MotionPrefs>().0.clone();
        let selected = prefs.reduce_motion;
        let weak = cx.entity().downgrade();
        let handle = self.menu_handle("reduce-motion-selector", cx);
        let selector = dropdown_menu(
            MenuChip::new("reduce-motion-selector")
                .label(reduce_motion_label(selected))
                .outlined()
                .selected(handle.is_open())
                .w(px(116.0))
                .justify_between(),
            "reduce-motion-selector-menu",
            &handle,
            MenuAlign::BelowRight,
            move |_| {
                ReduceMotion::ALL
                    .into_iter()
                    .map(|choice| {
                        let weak = weak.clone();
                        MenuItem::new(reduce_motion_label(choice), move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.update_motion_prefs(window, cx, |prefs| {
                                    prefs.reduce_motion = choice;
                                });
                            });
                        })
                        .selected(choice == selected)
                    })
                    .collect()
            },
        );
        let paused = prefs.pause_motion_in_background;
        let pause_switch = toggle_switch(
            "pause-motion-in-background",
            paused,
            false,
            theme,
            cx,
            move |this, window, cx| {
                this.update_motion_prefs(window, cx, |prefs| {
                    prefs.pause_motion_in_background = !paused;
                });
            },
        );

        [
            settings_divider(theme),
            settings_row(
                theme,
                tr!("settings.reduce_motion"),
                tr!("settings.reduce_motion_description"),
                selector.into_any_element(),
            ),
            settings_divider(theme),
            settings_row(
                theme,
                tr!("settings.pause_motion_background"),
                tr!("settings.pause_motion_background_description"),
                pause_switch.into_any_element(),
            ),
        ]
    }
}

fn settings_divider(theme: Theme) -> AnyElement {
    div()
        .mx(px(20.0))
        .h(px(1.0))
        .bg(theme.border)
        .into_any_element()
}

/// One row of the Appearance card, laid out like its upstream rows.
fn settings_row(
    theme: Theme,
    title: String,
    description: String,
    control: AnyElement,
) -> AnyElement {
    div()
        .w_full()
        .min_h(px(60.0))
        .px(px(20.0))
        .py(px(12.0))
        .flex()
        .items_center()
        .gap(px(24.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(sp(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(5.0))
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_secondary)
                        .child(description),
                ),
        )
        .child(control)
        .into_any_element()
}
