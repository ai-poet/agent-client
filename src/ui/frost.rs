//! Backdrop-blurred floating surfaces.
//!
//! Adapted from Zeron's `crates/ui/src/frost.rs` (MIT, Copyright (c) 2026
//! Wing). The whole card paints in one scene layer so its blur is always
//! behind its shadow, tint, border and content.

use gpui::{
    AnyElement, App, Bounds, Corners, Element, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window, px,
};

use crate::theme::Theme;

/// Shared blur for menus, palettes and floating status surfaces.
pub const MENU_BLUR: f32 = 16.0;
const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

/// A floating surface keeps the theme's hue while letting blurred content
/// show through. Opaque platforms keep the exact original color.
pub fn background(theme: &Theme) -> Hsla {
    if SUPPORTED {
        theme
            .raised
            .opacity(if theme.is_dark { 0.86 } else { 0.88 })
    } else {
        theme.raised
    }
}

/// Frost one floating card. The caller keeps its own translucent fill; this
/// wrapper only owns the backdrop and the card's scene-layer ordering.
pub fn frosted(corner_radius: f32, blur_radius: f32, child: impl IntoElement) -> Frosted {
    Frosted {
        corner_radius,
        blur_radius,
        child: child.into_any_element(),
    }
}

pub struct Frosted {
    corner_radius: f32,
    blur_radius: f32,
    child: AnyElement,
}

impl Element for Frosted {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if SUPPORTED && self.blur_radius > 0.0 {
            window.paint_layer(bounds, |window| {
                window.paint_backdrop_blur(
                    bounds,
                    Corners::all(px(self.corner_radius)),
                    px(self.blur_radius),
                );
                self.child.paint(window, cx);
            });
        } else {
            self.child.paint(window, cx);
        }
    }
}

impl IntoElement for Frosted {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
