//! NanaUI's native Lilia-style application framework.
//!
//! [`WorkspaceController`], [`WorkspaceSlots`], and [`workspace_view`] provide
//! the reusable workspace contract. [`WorkspaceState`] is a runnable demo that
//! exercises the framework with real application state.

pub mod dialog;
pub mod gallery;
pub mod geometry;
pub mod gpu_texture;
pub mod gpu_view;
pub mod layout;
pub mod menu;
mod node_canvas;
pub mod overlay;
pub mod selection;
mod shell;
pub mod theme;
pub mod tooltip;
pub mod widgets;
pub mod workspace;
mod workspace_demo;

pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use gallery::{ContextAction, GalleryMessage, GalleryState, GalleryTab, SurfaceView};
pub use geometry::{LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
pub use gpu_texture::{GpuTextureView, HostTexture};
pub use gpu_view::{GpuView, GpuViewMode, GpuViewPalette, RenderSlot};
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
pub use overlay::ExclusiveOverlay;
pub use selection::{SelectionMove, SingleSelection};
pub use shell::{app_shell, app_title_bar};
pub use theme::{Colors, ThemeMetrics, ThemeMode, ThemeTokens};
pub use tooltip::{TooltipConfig, TooltipPlacement};
pub use widgets::{ButtonKind, CardKind};
pub use workspace::{
    WorkspaceAction, WorkspaceController, WorkspaceRegion, WorkspaceRegions, WorkspaceSlots,
    workspace_view,
};
pub use workspace_demo::{LayoutPreset, Message, Navigation, WorkspaceState};
