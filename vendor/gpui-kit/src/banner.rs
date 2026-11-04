//! Composable notice banners with semantic tones and reusable slots.
//!
//! The slot model follows T3 Code's ComposerBanner behavior. The GPUI layout
//! and styling here are Waku-owned; no React/CSS implementation is copied.

use gpui::{
    App, Context, Div, ElementId, Hsla, ParentElement, RenderOnce, SharedString, Stateful,
    StyleRefinement, Styled, Window, div, prelude::*, px, rgb,
};

use crate::{activation::ActivationExt, theme::UiTheme};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BannerTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BannerSurface {
    #[default]
    Card,
    Strip,
}

/// Root container for a notice. Content, actions and dismissal remain slots.
#[derive(IntoElement)]
pub struct Banner {
    base: Stateful<Div>,
    theme: UiTheme,
    tone: BannerTone,
    surface: BannerSurface,
}

impl Banner {
    pub fn new(id: impl Into<ElementId>, theme: UiTheme) -> Self {
        Self {
            base: div().id(id),
            theme,
            tone: BannerTone::Neutral,
            surface: BannerSurface::Card,
        }
    }

    pub fn tone(mut self, tone: BannerTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn surface(mut self, surface: BannerSurface) -> Self {
        self.surface = surface;
        self
    }

    /// Flexible text/content slot shared by card and strip banners.
    pub fn content() -> Div {
        div().flex_1().min_w_0().truncate()
    }

    /// Icon slot with fixed geometry and inherited semantic color.
    pub fn icon(child: impl IntoElement) -> Div {
        div().flex_none().child(child)
    }

    /// Action-row slot. The caller controls whether it needs a top margin.
    pub fn actions() -> Div {
        div().flex().items_center().gap(px(8.0))
    }

    /// Card-style primary or secondary action, activated by the caller.
    pub fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        primary: bool,
        theme: UiTheme,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .h(px(26.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .cursor_default()
            .text_size(gpui::rems(12.5 / 14.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .when(primary, |button| {
                button
                    .bg(theme.inverse)
                    .text_color(theme.on_inverse)
                    .hover(|button| button.opacity(0.9))
            })
            .when(!primary, |button| {
                button
                    .border_1()
                    .border_color(theme.border_strong)
                    .text_color(theme.text_secondary)
                    .hover(|button| button.bg(theme.overlay).text_color(theme.text))
            })
            .active(|button| button.opacity(0.8))
            .child(label.into())
    }

    /// Compact action on a colored strip.
    pub fn strip_action(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
        let foreground: Hsla = rgb(0xFFFFFF).into();
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(foreground))
            .h(px(24.0))
            .px(px(10.0))
            .rounded_full()
            .flex()
            .flex_none()
            .items_center()
            .cursor_default()
            .bg(foreground.opacity(0.18))
            .hover(|style| style.bg(foreground.opacity(0.28)))
            .child(label.into())
    }

    /// Dismiss slot with keyboard activation and surface-appropriate focus.
    pub fn dismiss<E>(
        id: impl Into<ElementId>,
        surface: BannerSurface,
        theme: UiTheme,
        cx: &mut Context<E>,
        activate: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
    ) -> Stateful<Div>
    where
        E: 'static,
    {
        let foreground: Hsla = rgb(0xFFFFFF).into();
        div()
            .id(id)
            .tab_index(0)
            .size(px(22.0))
            .rounded(px(6.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_default()
            .when(surface == BannerSurface::Card, |button| {
                button
                    .focus_visible(|style| style.bg(theme.overlay_strong))
                    .hover(|button| button.bg(theme.overlay_strong))
            })
            .when(surface == BannerSurface::Strip, |button| {
                button
                    .focus_visible(|style| style.border_1().border_color(foreground))
                    .hover(|style| style.bg(foreground.opacity(0.18)))
            })
            .on_activation(cx, activate)
    }

    fn strip_background(&self) -> Hsla {
        let theme = self.theme;
        match self.tone {
            BannerTone::Neutral => theme.raised,
            BannerTone::Info => theme.gauge,
            BannerTone::Success => theme.success,
            BannerTone::Warning => theme.warning,
            BannerTone::Error => theme.danger,
        }
    }
}

impl Styled for Banner {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl gpui::InteractiveElement for Banner {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl gpui::StatefulInteractiveElement for Banner {}

impl ParentElement for Banner {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Banner {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        match self.surface {
            BannerSurface::Card => {
                let border = match self.tone {
                    BannerTone::Error => theme.danger.opacity(0.35),
                    BannerTone::Warning => theme.warning.opacity(0.35),
                    BannerTone::Success => theme.success.opacity(0.35),
                    BannerTone::Info => theme.gauge.opacity(0.35),
                    BannerTone::Neutral => theme.border_strong,
                };
                self.base
                    .w_full()
                    .p(px(12.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(border)
                    .bg(theme.raised)
                    .shadow_md()
            }
            BannerSurface::Strip => {
                let foreground = if self.tone == BannerTone::Neutral {
                    theme.text
                } else {
                    rgb(0xFFFFFF).into()
                };
                let background = self.strip_background();
                self.base
                    .w_full()
                    .flex_none()
                    .h(px(34.0))
                    .px(px(14.0))
                    .bg(background)
                    .text_color(foreground)
                    .text_size(gpui::rems(12.5 / 14.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tones_use_semantic_colors() {
        let theme = UiTheme::fixture();
        let banner = |tone| Banner::new("notice", theme).tone(tone);
        assert_eq!(banner(BannerTone::Neutral).strip_background(), theme.raised);
        assert_eq!(banner(BannerTone::Info).strip_background(), theme.gauge);
        assert_eq!(
            banner(BannerTone::Success).strip_background(),
            theme.success
        );
        assert_eq!(
            banner(BannerTone::Warning).strip_background(),
            theme.warning
        );
        assert_eq!(banner(BannerTone::Error).strip_background(), theme.danger);
    }
}
