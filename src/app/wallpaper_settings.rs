//! Appearance controls for the new-task wallpaper.

use gpui::MouseButton;

use super::*;
use crate::persistence::WallpaperEffect;
use crate::ui::ActivationExt;

fn wallpaper_effect_label(effect: WallpaperEffect) -> String {
    match effect {
        WallpaperEffect::None => tr!("wallpaper.effect.none"),
        WallpaperEffect::Dither => tr!("wallpaper.effect.dither"),
        WallpaperEffect::Ascii => tr!("wallpaper.effect.ascii"),
        WallpaperEffect::Halftone => tr!("wallpaper.effect.halftone"),
        WallpaperEffect::Scanlines => tr!("wallpaper.effect.scanlines"),
    }
}

fn wallpaper_button(
    id: &'static str,
    label: String,
    enabled: bool,
    theme: Theme,
    cx: &mut Context<Waku>,
    activate: impl Fn(&mut Waku, &mut Context<Waku>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .tab_index(0)
        .h(px(29.0))
        .px(px(11.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .text_size(sp(12.5))
        .text_color(theme.text_secondary)
        .opacity(if enabled { 1.0 } else { 0.45 })
        .focus_visible(|style| style.border_color(theme.accent));
    let button = if enabled {
        button
            .hover(|element| element.bg(theme.overlay))
            .on_activation(cx, move |this, _, cx| activate(this, cx))
    } else {
        button
    };
    button.child(label)
}

fn wallpaper_row_separator(theme: Theme) -> Div {
    div().mx(px(20.0)).h(px(1.0)).bg(theme.border)
}

impl Waku {
    pub(super) fn render_wallpaper_settings_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let settings = self.state.wallpaper.clone();
        let loading = self.wallpaper.loading;
        let choose_label = if loading {
            tr!("wallpaper.loading")
        } else if settings.is_some() {
            tr!("wallpaper.replace")
        } else {
            tr!("wallpaper.choose")
        };
        let choose = wallpaper_button(
            "wallpaper-choose",
            choose_label,
            !loading,
            theme,
            cx,
            |this, cx| this.pick_wallpaper(cx),
        );
        let description = settings.as_ref().map_or_else(
            || tr!("wallpaper.description"),
            |settings| settings.name.clone(),
        );
        let mut rows = div()
            .w_full()
            .flex()
            .flex_col()
            .child(wallpaper_row_separator(theme))
            .child(
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
                                    .child(tr!("wallpaper.title")),
                            )
                            .child(
                                div()
                                    .mt(px(5.0))
                                    .text_size(sp(12.5))
                                    .line_height(sp(18.0))
                                    .text_color(theme.text_secondary)
                                    .truncate()
                                    .child(description),
                            ),
                    )
                    .child(choose),
            );

        if let Some(settings) = settings {
            let selected_effect = settings.effect;
            let weak = cx.entity().downgrade();
            let effect_handle = self.menu_handle("wallpaper-effect-selector", cx);
            let effect_selector = dropdown_menu(
                MenuChip::new("wallpaper-effect-selector")
                    .label(wallpaper_effect_label(selected_effect))
                    .outlined()
                    .selected(effect_handle.is_open())
                    .w(px(116.0))
                    .justify_between(),
                "wallpaper-effect-selector-menu",
                &effect_handle,
                MenuAlign::BelowRight,
                move |_| {
                    WallpaperEffect::ALL
                        .into_iter()
                        .map(|effect| {
                            let weak = weak.clone();
                            MenuItem::new(wallpaper_effect_label(effect), move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_wallpaper_effect(effect, cx);
                                });
                            })
                            .selected(effect == selected_effect)
                        })
                        .collect()
                },
            );
            let adjust = wallpaper_button(
                "wallpaper-adjust",
                tr!("wallpaper.adjust"),
                true,
                theme,
                cx,
                |this, cx| this.open_wallpaper_adjustment(cx),
            );
            let remove = wallpaper_button(
                "wallpaper-remove",
                tr!("wallpaper.remove"),
                !loading,
                theme,
                cx,
                |this, cx| this.remove_wallpaper(cx),
            );
            rows = rows.child(wallpaper_row_separator(theme)).child(
                div()
                    .w_full()
                    .min_h(px(60.0))
                    .px(px(20.0))
                    .py(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(13.0))
                            .text_color(theme.text_secondary)
                            .child(tr!("wallpaper.effect")),
                    )
                    .child(effect_selector)
                    .child(adjust)
                    .child(remove),
            );

            let theme_colors = settings.theme_colors;
            let tint_toggle = toggle_switch(
                "wallpaper-theme-colors",
                theme_colors,
                false,
                theme,
                cx,
                move |this, _, cx| this.set_wallpaper_theme_colors(!theme_colors, cx),
            );
            rows = rows.child(wallpaper_row_separator(theme)).child(
                div()
                    .w_full()
                    .min_h(px(54.0))
                    .px(px(20.0))
                    .py(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(24.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(sp(13.0))
                                    .text_color(theme.text)
                                    .child(tr!("wallpaper.theme_colors")),
                            )
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_secondary)
                                    .child(tr!("wallpaper.theme_colors_description")),
                            ),
                    )
                    .child(tint_toggle),
            );
        }

        if let Some(error) = self.wallpaper.error.as_ref() {
            rows = rows.child(
                div()
                    .px(px(20.0))
                    .pb(px(10.0))
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        if let Some(adjustment) = self.render_wallpaper_adjustment(cx) {
            rows = rows.child(adjustment);
        }
        rows.into_any_element()
    }

    fn render_wallpaper_adjustment(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.wallpaper.adjustment_dialog.as_ref()?;
        let theme = Theme::current(cx);
        let draft = dialog.draft;
        let focus = dialog.focus.clone();
        let source = self.wallpaper_preview();
        let measured = self.wallpaper.adjustment_preview_bounds.clone();
        let paint_measured = measured.clone();
        let paint_source = source.clone();
        let preview = div()
            .id("wallpaper-adjustment-preview")
            .track_focus(&focus)
            .tab_index(0)
            .relative()
            .w_full()
            .h(px(220.0))
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.inset)
            .cursor_default()
            .focus_visible(|style| style.border_color(theme.accent))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.key_wallpaper_adjustment(event, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.begin_wallpaper_adjustment_drag(event.position, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.move_wallpaper_adjustment_drag(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.end_wallpaper_adjustment_drag();
                }),
            )
            .child(
                canvas(
                    move |bounds, _, _| paint_measured.set(Some(bounds)),
                    move |bounds, _, window, _| {
                        if let Some(source) = paint_source.clone() {
                            super::wallpaper_view::paint_wallpaper_preview(
                                source, bounds, draft, window,
                            );
                        }
                    },
                )
                .size_full(),
            )
            .when(source.is_none(), |preview| {
                preview.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("wallpaper.loading")),
                )
            });

        let zoom_out = wallpaper_button(
            "wallpaper-zoom-out",
            "−".to_owned(),
            draft.zoom > crate::persistence::WallpaperAdjustment::MIN_ZOOM,
            theme,
            cx,
            |this, cx| this.zoom_wallpaper_adjustment(-0.1, cx),
        );
        let zoom_in = wallpaper_button(
            "wallpaper-zoom-in",
            "+".to_owned(),
            draft.zoom < crate::persistence::WallpaperAdjustment::MAX_ZOOM,
            theme,
            cx,
            |this, cx| this.zoom_wallpaper_adjustment(0.1, cx),
        );
        let reset = wallpaper_button(
            "wallpaper-adjustment-reset",
            tr!("wallpaper.reset"),
            true,
            theme,
            cx,
            |this, cx| this.reset_wallpaper_adjustment(cx),
        );
        let cancel = wallpaper_button(
            "wallpaper-adjustment-cancel",
            tr!("wallpaper.cancel"),
            true,
            theme,
            cx,
            |this, cx| this.cancel_wallpaper_adjustment(cx),
        );
        let apply = wallpaper_button(
            "wallpaper-adjustment-apply",
            tr!("wallpaper.apply"),
            true,
            theme,
            cx,
            |this, cx| this.apply_wallpaper_adjustment(cx),
        );
        Some(
            div()
                .mx(px(20.0))
                .mb(px(14.0))
                .p(px(12.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.inset.opacity(0.72))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .text_size(sp(12.0))
                                .text_color(theme.text_secondary)
                                .child(tr!("wallpaper.adjust_hint")),
                        )
                        .child(zoom_out)
                        .child(
                            div()
                                .w(px(44.0))
                                .text_center()
                                .text_size(sp(12.0))
                                .text_color(theme.text_secondary)
                                .child(format!("{:.1}×", draft.zoom)),
                        )
                        .child(zoom_in),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(reset)
                        .child(div().flex_1())
                        .child(cancel)
                        .child(apply),
                )
                .into_any_element(),
        )
    }
}
