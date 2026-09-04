//! Direct entries for the right panel's surfaces.
//!
//! Fork addition. Upstream reaches Terminal, Files, Browser and Review only
//! through the right panel itself: open it (a small toggle, ⇧⌘B, the View
//! menu or the palette), then pick one of four cards or the "+" menu in its
//! header. This puts the four surfaces in the window header as always-visible
//! buttons — one click opens, switches to, or puts away each one — and gives
//! them a shortcut, a View-menu item and a palette command apiece.
//!
//! The panel is untouched: every button ends in `open_right_panel_surface`
//! or `set_right_panel_visible`, the same calls the panel's own chrome makes,
//! and a second terminal or browser still comes from the panel's "+" menu.

use gpui::{Action, KeyBinding, MenuItem};

use crate::ui::ActivationExt as _;

use super::*;

/// The four surfaces a header button stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceKind {
    Terminal,
    Files,
    Browser,
    Review,
}

impl SurfaceKind {
    /// Header order.
    pub const ALL: [SurfaceKind; 4] = [Self::Terminal, Self::Files, Self::Browser, Self::Review];

    fn element_id(self) -> &'static str {
        match self {
            Self::Terminal => "surface-bar-terminal",
            Self::Files => "surface-bar-files",
            Self::Browser => "surface-bar-browser",
            Self::Review => "surface-bar-review",
        }
    }

    pub(super) fn icon_path(self) -> &'static str {
        match self {
            Self::Terminal => "icons/terminal.svg",
            Self::Files => "icons/folder.svg",
            Self::Browser => "icons/globe.svg",
            Self::Review => "icons/file-diff.svg",
        }
    }

    /// The panel's own tab titles, so the header and the panel agree.
    pub(super) fn label(self) -> String {
        match self {
            Self::Terminal => tr!("right_panel.terminal"),
            Self::Files => tr!("right_panel.files"),
            Self::Browser => tr!("right_panel.browser"),
            Self::Review => tr!("right_panel.diff"),
        }
    }

    fn keystrokes(self) -> &'static str {
        match self {
            Self::Terminal => "secondary-shift-t",
            Self::Files => "secondary-shift-e",
            Self::Browser => "secondary-shift-o",
            Self::Review => "secondary-shift-d",
        }
    }

    pub(super) fn shortcut_label(self) -> &'static str {
        match self {
            Self::Terminal => crate::platform::primary_shortcut("⇧⌘T", "Ctrl+Shift+T"),
            Self::Files => crate::platform::primary_shortcut("⇧⌘E", "Ctrl+Shift+E"),
            Self::Browser => crate::platform::primary_shortcut("⇧⌘O", "Ctrl+Shift+O"),
            Self::Review => crate::platform::primary_shortcut("⇧⌘D", "Ctrl+Shift+D"),
        }
    }

    /// Whether a tab belongs to this button. Files means the explorer only:
    /// with an editor tab in front, the Files button should bring the tree
    /// forward, not put the panel away.
    fn matches(self, surface: &RightPanelSurface) -> bool {
        matches!(
            (self, surface),
            (Self::Terminal, RightPanelSurface::Terminal(_))
                | (Self::Files, RightPanelSurface::Files)
                | (Self::Browser, RightPanelSurface::Browser(_))
                | (Self::Review, RightPanelSurface::Diff)
        )
    }

    fn new_surface(self) -> RightPanelSurface {
        match self {
            Self::Terminal => RightPanelSurface::Terminal(Uuid::new_v4()),
            Self::Files => RightPanelSurface::Files,
            Self::Browser => RightPanelSurface::Browser(Uuid::new_v4()),
            Self::Review => RightPanelSurface::Diff,
        }
    }

    pub(super) fn palette_label(self) -> String {
        tr!("command_palette.open_surface", surface = self.label())
    }

    pub(super) fn palette_keywords(self) -> &'static str {
        match self {
            Self::Terminal => "open terminal shell console right panel",
            Self::Files => "open files tree explorer editor right panel",
            Self::Browser => "open browser web url preview right panel",
            Self::Review => "open review diff changes git right panel",
        }
    }
}

/// The one action behind every entry: header button, shortcut, View menu
/// and palette command.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = waku, no_json)]
pub struct OpenSurface {
    pub kind: SurfaceKind,
}

/// What pressing a button does, given the panel as it stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SurfacePlan {
    /// No tab of this kind exists: open one.
    OpenNew,
    /// A tab of this kind exists: bring this index forward and show the panel.
    Activate(usize),
    /// This kind is already in front of a visible panel: put the panel away.
    Hide,
}

/// The button's plan, from the selected session's tabs and the panel's
/// visibility. A stale active index counts as no active tab.
pub(super) fn surface_plan(
    kind: SurfaceKind,
    surfaces: &[RightPanelSurface],
    active: Option<usize>,
    visible: bool,
) -> SurfacePlan {
    let active = active.filter(|index| {
        surfaces
            .get(*index)
            .is_some_and(|surface| kind.matches(surface))
    });
    match active {
        Some(_) if visible => SurfacePlan::Hide,
        Some(index) => SurfacePlan::Activate(index),
        None => surfaces
            .iter()
            .rposition(|surface| kind.matches(surface))
            .map_or(SurfacePlan::OpenNew, SurfacePlan::Activate),
    }
}

/// Key bindings for the four surfaces, global like `ToggleRightPanel`.
pub(crate) fn surface_key_bindings() -> Vec<KeyBinding> {
    let mut bindings: Vec<KeyBinding> = SurfaceKind::ALL
        .into_iter()
        .map(|kind| KeyBinding::new(kind.keystrokes(), OpenSurface { kind }, None))
        .collect();
    // `src/input.rs` binds ctrl-shift-e to SelectToEnd inside a text input,
    // and that deeper binding outranks a global one while the composer has
    // focus. The same second binding upstream adds for OpenFind keeps
    // Ctrl+Shift+E reaching Files from the composer.
    bindings.push(KeyBinding::new(
        SurfaceKind::Files.keystrokes(),
        OpenSurface {
            kind: SurfaceKind::Files,
        },
        Some("Waku > TextInput"),
    ));
    bindings
}

/// View-menu items for the four surfaces.
pub(crate) fn surface_menu_items() -> Vec<MenuItem> {
    SurfaceKind::ALL
        .into_iter()
        .map(|kind| MenuItem::action(kind.label(), OpenSurface { kind }))
        .collect()
}

impl Waku {
    /// Open, switch to, or put away the surface behind a button.
    pub(super) fn activate_surface_kind(&mut self, kind: SurfaceKind, cx: &mut Context<Self>) {
        let plan = surface_plan(
            kind,
            &self.right_panel_surfaces,
            self.right_panel_active_surface,
            self.right_panel_visible,
        );
        match plan {
            SurfacePlan::Hide => self.set_right_panel_visible(false, cx),
            SurfacePlan::OpenNew => self.open_right_panel_surface(kind.new_surface(), cx),
            SurfacePlan::Activate(index) => match kind {
                // Single-instance tabs: the panel reuses the existing one and
                // refreshes the tree or the diff on the way.
                SurfaceKind::Files | SurfaceKind::Review => {
                    self.open_right_panel_surface(kind.new_surface(), cx)
                }
                SurfaceKind::Terminal | SurfaceKind::Browser => {
                    self.right_panel_active_surface = Some(index);
                    // What the panel's private `reveal_right_panel_tab` does.
                    self.right_panel_pending_tab_reveal = Some(index);
                    self.right_panel_tabs_scroll_handle.scroll_to_item(index);
                    self.request_active_terminal_focus();
                    self.request_active_browser_focus();
                    self.set_right_panel_visible(true, cx);
                    cx.notify();
                }
            },
        }
    }

    pub(super) fn open_surface_action(
        &mut self,
        action: &OpenSurface,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The settings page never shows the right panel; leave it first, the
        // way the palette does when it selects a task.
        if self.settings_page.is_some() {
            self.settings_page = None;
        }
        self.activate_surface_kind(action.kind, cx);
    }

    /// The four buttons for the window header.
    pub(super) fn render_surface_bar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = Theme::current(cx);
        div()
            .id("surface-bar")
            .tab_group()
            .tab_stop(false)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.0))
            .children(
                SurfaceKind::ALL
                    .into_iter()
                    .map(|kind| self.render_surface_button(kind, theme, cx)),
            )
    }

    fn render_surface_button(
        &self,
        kind: SurfaceKind,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let plan = surface_plan(
            kind,
            &self.right_panel_surfaces,
            self.right_panel_active_surface,
            self.right_panel_visible,
        );
        let shown = plan == SurfacePlan::Hide;
        let open_behind = matches!(plan, SurfacePlan::Activate(_));
        let label = kind.label();
        let shortcut = kind.shortcut_label();
        // The words carry the state; the colour only echoes it.
        let tooltip: SharedString = match plan {
            SurfacePlan::Hide => tr!("surface_bar.hide", surface = label, shortcut = shortcut),
            SurfacePlan::Activate(_) => {
                tr!("surface_bar.switch", surface = label, shortcut = shortcut)
            }
            SurfacePlan::OpenNew => tr!("surface_bar.open", surface = label, shortcut = shortcut),
        }
        .into();

        div()
            .id(kind.element_id())
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .w(px(26.0))
            .h(px(26.0))
            .flex_none()
            .relative()
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .when(shown, |element| element.bg(theme.overlay))
            .hover(|element| element.bg(theme.overlay))
            .active(|element| element.bg(theme.overlay_strong))
            .child(icon(
                kind.icon_path(),
                14.0,
                if shown {
                    theme.accent
                } else {
                    theme.text_tertiary
                },
            ))
            .when(open_behind, |element| {
                element.child(
                    div()
                        .absolute()
                        .bottom(px(3.0))
                        .left(px(11.5))
                        .w(px(3.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(theme.text_tertiary),
                )
            })
            .tooltip(Tooltip::text(tooltip))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_activation(cx, move |this, _, cx| this.activate_surface_kind(kind, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> RightPanelSurface {
        RightPanelSurface::Terminal(Uuid::new_v4())
    }

    fn browser() -> RightPanelSurface {
        RightPanelSurface::Browser(Uuid::new_v4())
    }

    #[test]
    fn hides_when_the_current_tab_is_of_this_kind() {
        let surfaces = vec![terminal(), RightPanelSurface::Files];
        assert_eq!(
            surface_plan(SurfaceKind::Terminal, &surfaces, Some(0), true),
            SurfacePlan::Hide
        );
        assert_eq!(
            surface_plan(SurfaceKind::Files, &surfaces, Some(1), true),
            SurfacePlan::Hide
        );
    }

    #[test]
    fn reveals_the_current_tab_when_the_panel_is_hidden() {
        let surfaces = vec![terminal(), RightPanelSurface::Files];
        assert_eq!(
            surface_plan(SurfaceKind::Terminal, &surfaces, Some(0), false),
            SurfacePlan::Activate(0)
        );
    }

    #[test]
    fn activates_the_last_tab_of_the_kind_when_another_tab_is_current() {
        let surfaces = vec![terminal(), terminal(), RightPanelSurface::Files];
        assert_eq!(
            surface_plan(SurfaceKind::Terminal, &surfaces, Some(2), true),
            SurfacePlan::Activate(1)
        );
        assert_eq!(
            surface_plan(SurfaceKind::Terminal, &surfaces, Some(2), false),
            SurfacePlan::Activate(1)
        );
    }

    #[test]
    fn opens_a_new_tab_when_none_of_the_kind_exists() {
        for kind in SurfaceKind::ALL {
            assert_eq!(surface_plan(kind, &[], None, false), SurfacePlan::OpenNew);
            assert_eq!(surface_plan(kind, &[], None, true), SurfacePlan::OpenNew);
        }
        let surfaces = vec![RightPanelSurface::Files];
        assert_eq!(
            surface_plan(SurfaceKind::Browser, &surfaces, Some(0), true),
            SurfacePlan::OpenNew
        );
    }

    #[test]
    fn review_matches_the_diff_tab_and_files_only_the_explorer() {
        let surfaces = vec![
            RightPanelSurface::File("src/main.rs".into()),
            RightPanelSurface::Diff,
        ];
        assert_eq!(
            surface_plan(SurfaceKind::Files, &surfaces, Some(0), true),
            SurfacePlan::OpenNew
        );
        assert_eq!(
            surface_plan(SurfaceKind::Review, &surfaces, Some(0), true),
            SurfacePlan::Activate(1)
        );
        assert_eq!(
            surface_plan(SurfaceKind::Review, &surfaces, Some(1), true),
            SurfacePlan::Hide
        );
    }

    #[test]
    fn tolerates_a_stale_active_index() {
        let surfaces = vec![RightPanelSurface::Files];
        assert_eq!(
            surface_plan(SurfaceKind::Files, &surfaces, Some(5), true),
            SurfacePlan::Activate(0)
        );
        assert_eq!(
            surface_plan(SurfaceKind::Terminal, &surfaces, Some(5), true),
            SurfacePlan::OpenNew
        );
    }

    #[test]
    fn each_kind_opens_a_surface_it_matches() {
        for kind in SurfaceKind::ALL {
            assert!(kind.matches(&kind.new_surface()), "{kind:?}");
        }
        assert_ne!(SurfaceKind::Terminal.new_surface(), terminal());
        assert_ne!(SurfaceKind::Browser.new_surface(), browser());
        assert_eq!(SurfaceKind::Files.new_surface(), RightPanelSurface::Files);
        assert_eq!(SurfaceKind::Review.new_surface(), RightPanelSurface::Diff);
    }

    #[test]
    fn background_work_tabs_belong_to_no_kind() {
        let surfaces = vec![RightPanelSurface::BackgroundWork {
            key: BackgroundWorkKey::new(BackgroundWorkKind::Process, "process-1"),
            title: "build".into(),
        }];
        for kind in SurfaceKind::ALL {
            assert_eq!(
                surface_plan(kind, &surfaces, Some(0), true),
                SurfacePlan::OpenNew,
                "{kind:?}"
            );
        }
    }
}
