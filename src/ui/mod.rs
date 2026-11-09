use gpui::{
    AnyElement, App, Context, Div, ElementId, Hsla, Img, InteractiveElement, Interactivity,
    ParentElement, PathBuilder, Pixels, RenderOnce, ScrollHandle, SharedString, Stateful,
    StyleRefinement, Styled, Svg, Window, canvas, div, point, prelude::*, px, rgb,
};

pub mod menu;
// Fork: shared backdrop-blurred floating surfaces.
pub mod frost;
pub mod motion;
// Fork: named curves, durations and entrance/exit helpers.
pub mod motion_kit;
pub mod scrollbar;
pub mod text_field;
pub mod tooltip;

pub use client_ui::ActivationExt;
use client_ui::UiTheme;

use crate::model::{ActivityKind, ProviderKind, SessionStatus};
use crate::theme::{Theme, sp};

pub(crate) fn ui_theme(theme: Theme) -> UiTheme {
    UiTheme {
        is_dark: theme.is_dark,
        overlay: theme.overlay,
        overlay_strong: theme.overlay_strong,
        raised: theme.raised,
        inset: theme.inset,
        border_strong: theme.border_strong,
        border: theme.border,
        text: theme.text,
        text_secondary: theme.text_secondary,
        text_tertiary: theme.text_tertiary,
        text_ghost: theme.text_ghost,
        accent: theme.accent,
        gauge: theme.gauge,
        warning: theme.warning,
        success: theme.success,
        danger: theme.danger,
        inverse: theme.inverse,
        on_inverse: theme.on_inverse,
    }
}

/// A monochrome icon from the embedded set, tinted via text color. Sized in
/// `sp` so icons keep pace with the chrome text they sit beside when the UI
/// font size setting moves.
pub fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    client_ui::icon::icon(path, sp(size), color)
}

/// A polychrome file icon rendered as an image so the SVG's authored colors
/// are preserved. GPUI's `svg()` element intentionally renders an alpha mask
/// tinted with one text color.
pub fn file_icon(path: &'static str, size: f32) -> Img {
    client_ui::icon::file_icon(path, sp(size))
}

/// A compact ghost icon button: the only button shape outside the composer's
/// bespoke send control.
pub fn icon_button(id: impl Into<ElementId>, path: &'static str, theme: Theme) -> Stateful<Div> {
    client_ui::icon_button::icon_button(id, path, sp(13.0), ui_theme(theme))
}

/// Keeps a wheel gesture in a nested scrollable while it can consume the
/// delta, then lets it chain to the ancestor at either boundary.
pub fn contain_scroll(handle: &ScrollHandle, cx: &mut App) {
    client_ui::scroll::contain_scroll(handle, cx);
}

/// The shared pill switch used by settings and automation forms.
///
/// `activate` is ignored while `disabled` is true, but the control remains in
/// the tab order so a pending operation does not move focus unexpectedly.
pub fn toggle_switch<E>(
    id: impl Into<ElementId>,
    on: bool,
    disabled: bool,
    theme: Theme,
    cx: &mut Context<E>,
    activate: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
) -> Stateful<Div>
where
    E: 'static,
{
    client_ui::toggle_switch::toggle_switch(id, on, disabled, ui_theme(theme), cx, activate)
}

/// Brand hue for each provider's official mark.
pub fn provider_color(theme: &Theme, provider: ProviderKind) -> Hsla {
    match provider {
        ProviderKind::Amp => rgb(0xF34E3F).into(),
        ProviderKind::Claude => rgb(0xD97757).into(),
        ProviderKind::DeepSeek => rgb(0x4D6BFE).into(),
        // Waku's own agent wears Waku's accent: it is the one provider that is
        // not somebody else's product.
        ProviderKind::Native => theme.accent,
        ProviderKind::Codex
        | ProviderKind::Cursor
        | ProviderKind::Fx
        | ProviderKind::OpenCode
        | ProviderKind::Grok
        | ProviderKind::Kimi
        | ProviderKind::OhMyPi
        | ProviderKind::Pi => {
            if theme.is_dark {
                rgb(0xF3F3F3).into()
            } else {
                rgb(0x34363B).into()
            }
        }
    }
}

/// Recognizable provider marks, matching the model picker vocabulary.
pub fn provider_icon(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Amp => "icons/provider-amp.svg",
        ProviderKind::Claude => "icons/provider-claude.svg",
        ProviderKind::Codex => "icons/provider-openai.svg",
        ProviderKind::Cursor => "icons/provider-cursor.svg",
        ProviderKind::DeepSeek => "icons/provider-deepseek.svg",
        ProviderKind::Fx => "icons/provider-fx.svg",
        ProviderKind::OpenCode => "icons/provider-opencode.svg",
        ProviderKind::Grok => "icons/provider-grok.svg",
        ProviderKind::Native => "icons/provider-waku.svg",
        ProviderKind::Kimi => "icons/provider-kimi.svg",
        ProviderKind::OhMyPi => "icons/provider-ohmypi.svg",
        ProviderKind::Pi => "icons/provider-pi.svg",
    }
}

pub fn status_color(theme: &Theme, status: SessionStatus) -> Hsla {
    match status {
        SessionStatus::Idle => theme.text_ghost,
        SessionStatus::Connecting | SessionStatus::Working => theme.accent,
        SessionStatus::Waiting => theme.warning,
        SessionStatus::Failed => theme.danger,
    }
}

pub fn activity_icon(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::Reasoning => "icons/sparkle.svg",
        ActivityKind::Command => "icons/terminal.svg",
        ActivityKind::FileChange => "icons/pencil.svg",
        ActivityKind::FileRead => "icons/file.svg",
        ActivityKind::FileSearch => "icons/search.svg",
        ActivityKind::FileList => "icons/folder.svg",
        ActivityKind::Search => "icons/search.svg",
        ActivityKind::Plan => "icons/list.svg",
        ActivityKind::Tool => "icons/wrench.svg",
    }
}

/// A compact chip used as a dropdown-menu trigger. `selected` is driven by the
/// menu's open state and renders as a soft fill.
#[derive(IntoElement)]
pub struct MenuChip {
    inner: client_ui::MenuChip,
}

impl MenuChip {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            inner: client_ui::MenuChip::new(id),
        }
    }

    /// Override the chip's fixed height, for rows whose controls share a
    /// different one.
    pub fn height(mut self, height: Pixels) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    /// Fill behind an outlined chip. The default matches raised cards; a
    /// chip sitting directly on another surface passes that surface here so
    /// it doesn't read as a filled pill.
    pub fn background(mut self, background: Hsla) -> Self {
        self.inner = self.inner.background(background);
        self
    }

    pub fn icon(mut self, path: &'static str, color: Hsla) -> Self {
        self.inner = self.inner.icon(path, color);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    pub fn outlined(mut self) -> Self {
        self.inner = self.inner.outlined(true);
        self
    }

    pub fn caret(mut self, caret: bool) -> Self {
        self.inner = self.inner.caret(caret);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }

    /// Soft fill marking the chip as the open menu's trigger.
    pub fn selected(mut self, selected: bool) -> Self {
        self.inner = self.inner.selected(selected);
        self
    }
}

impl Styled for MenuChip {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl InteractiveElement for MenuChip {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl ParentElement for MenuChip {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.inner.extend(elements);
    }
}

impl RenderOnce for MenuChip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.inner
            .theme(ui_theme(Theme::current(cx)))
            .text_metrics(sp(13.0), sp(16.0))
    }
}

/// An inline, link-like dropdown trigger used for the project name in the
/// empty-state headline.
#[derive(IntoElement)]
pub struct ProjectNameSelector {
    base: Stateful<Div>,
    label: SharedString,
    selected: bool,
}

impl ProjectNameSelector {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id),
            label: label.into(),
            selected: false,
        }
    }

    /// Emphasised underline while its menu is open.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl Styled for ProjectNameSelector {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for ProjectNameSelector {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl ParentElement for ProjectNameSelector {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for ProjectNameSelector {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(cx);
        let underline_color = if self.selected {
            theme.text_secondary
        } else {
            theme.text_tertiary
        };

        self.base
            .relative()
            .flex_none()
            .max_w_full()
            .overflow_hidden()
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .child(div().min_w_0().truncate().child(self.label))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let y = bounds.origin.y + bounds.size.height - px(0.5);
                        let mut builder =
                            PathBuilder::stroke(px(1.0)).dash_array(&[px(1.0), px(2.0)]);
                        builder.move_to(point(bounds.origin.x, y));
                        builder.line_to(point(bounds.origin.x + bounds.size.width, y));
                        if let Ok(line) = builder.build() {
                            window.paint_path(line, underline_color);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
