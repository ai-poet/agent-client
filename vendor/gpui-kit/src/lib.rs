//! Reusable GPUI components and interaction primitives for Waku.
//!
//! This crate owns behavior and presentation only. Application state, provider
//! models, persistence and platform policy stay in the caller.

pub mod activation;
pub mod command;
pub mod diff;
pub mod frost;
pub mod icon;
pub mod icon_button;
pub mod menu;
pub mod menu_chip;
pub mod message_scroller;
pub mod motion;
pub mod motion_kit;
pub mod scroll;
pub mod scrollbar;
pub mod text_field;
pub mod theme;
pub mod toggle_switch;
pub mod tooltip;

pub use activation::ActivationExt;
pub use menu_chip::MenuChip;
pub use text_field::TextField;
pub use theme::UiTheme;
