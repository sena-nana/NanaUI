//! Backend-neutral render scene and frame graph for NanaUI.
//!
//! The crate consumes Runtime extraction deltas. It owns no application state,
//! window, GPU device, or renderer objects. Product paint is `SceneWgpuPainter`
//! in `nana-ui`, which consumes this crate's `UiScene`.

mod graph;
mod icon;
mod runtime_document;
mod scene;

pub use graph::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};
pub use icon::{IconGeometry, IconPathCommand, IconShape, icon_geometry};
pub use runtime_document::{RuntimeDocument, RuntimeFrameUpdate};
pub use scene::{
    AffineTransform, ClipRegion, FilterGroup, OpacityGroup, PrimitiveId, QuadSurfacePaint,
    SceneDelta, ScenePrimitive, ScenePrimitiveKind, SceneRect, SceneTextSpan, StrokeCap, UiScene,
};
