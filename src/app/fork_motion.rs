//! Where the fork's motion keeps its state between frames.
//!
//! Fork file. One field on [`Waku`] holds everything the ported motion needs
//! (sidebar_resort.rs, toast_motion.rs, transcript_follow.rs,
//! transcript_memory.rs, prompt_clamp.rs), so upstream's struct carries a single
//! line for all of it. Render paths read through `&self`, hence the cells.

use std::cell::{Cell, RefCell};

use super::sidebar_resort::SidebarResort;
use super::toast_motion::ToastMotion;
use super::transcript_memory::TranscriptMemory;

#[derive(Default)]
pub(super) struct ForkMotion {
    pub(super) sidebar_resort: RefCell<SidebarResort>,
    pub(super) toast: ToastMotion,
    /// The transcript's scroll offset as of the last frame
    /// (transcript_follow.rs).
    pub(super) transcript_scroll_y: Cell<Option<f32>>,
    pub(super) transcript_memory: RefCell<TranscriptMemory>,
    /// User prompts unfolded past their clamp (prompt_clamp.rs).
    pub(super) expanded_prompts: RefCell<std::collections::HashSet<uuid::Uuid>>,
    /// Whether each prompt is long, by (length, width, font) (prompt_clamp.rs).
    #[allow(clippy::type_complexity)]
    pub(super) prompt_lengths:
        RefCell<std::collections::HashMap<uuid::Uuid, ((usize, u32, u32), bool)>>,
}
