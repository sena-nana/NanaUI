//! Backend-neutral dock chrome. Application pane bodies stay host-mounted slots.

mod assemble;
mod commands;
mod component;
mod model;

pub(crate) use assemble::DOCK_HOOKS;
pub use commands::{DockCommand, DockCommandOutcome, DockItemLimits};
pub use component::{Dock, DockPanel};
pub use model::{
    DOCK_DIVIDER_HIT_SIZE, DOCK_SPLIT_KEYBOARD_STEP, DockAxis, DockBoundsPersist, DockDropZone,
    DockFloatingSurface, DockNode, DockSurfaceSpec, DockWorkspace, DockWorkspaceEvent,
    MAIN_SURFACE_ID, MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, clamp_ratio, dock_nudge_split_ratio,
    dock_split_child_lengths, dock_split_ratio_from_pointer, dock_surface_window_key,
};

use std::sync::Arc;

use model::{extract_item, finite, insert_dock_item};
