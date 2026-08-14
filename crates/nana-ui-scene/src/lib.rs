//! Backend-neutral render scene and frame graph for NanaUI.
//!
//! The crate consumes Runtime extraction deltas. It owns no application state,
//! window, GPU device, or renderer objects and therefore remains usable by the
//! Iced/WGPU compatibility backend and future native backends alike.

mod graph;
mod runtime_document;
mod scene;

pub use graph::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};
pub use runtime_document::{RuntimeDocument, RuntimeFrameUpdate};
pub use scene::{
    AffineTransform, ClipRegion, PrimitiveId, SceneDelta, ScenePrimitive, ScenePrimitiveKind,
    SceneRect, UiScene,
};
