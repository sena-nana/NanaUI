//! Canonical L3 application API.
//!
//! Prefer this module over crate-root widget re-exports. See
//! [`docs/application-api.md`](../../../docs/application-api.md).
//!
//! Typed views and `register_component` live here. Scene types are also under
//! [`host`]; Issue #8 counters under [`perf`]. [`internal`] is the same Runtime
//! crate for Gallery and host adapters — not a second contract.

/// Scene host: retained document, render scene, opaque GPU slot keys.
pub mod host {
    pub use nana_ui_runtime::{
        CustomRenderNode, ExtractedNode, ExtractedTextSpan, GPU_TEXTURE_VIEW_RENDERER,
        GPU_VIEW_RENDERER, GRAPH_CANVAS_RENDERER, HOST_TEXTURE_RENDERER, pack_gpu_revision,
        unpack_gpu_revision,
    };
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

/// Work counters and frame profiler for benches and Issue #8 — not view state.
pub mod perf {
    pub use nana_ui_runtime::{
        FrameProfile, FrameProfiler, FrameStage, GpuWorkObservation, StageStatus, StageTiming,
        SystemWork, WorkCounters,
    };
}

/// Full `nana-ui-runtime` surface. Prefer the parent module for new applications.
pub mod internal {
    pub use nana_ui_runtime::*;
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

pub use nana_ui_runtime::*;
pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
