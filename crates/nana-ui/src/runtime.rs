//! Canonical application API.
//!
//! Prefer this module over crate-root widget re-exports. New trees use
//! `AppContext::mount_view_root` and `AppContext::mount_view`. See
//! [`docs/reference/l3-authoring.md`](../../../docs/reference/l3-authoring.md) and
//! [`docs/reference/application-api.md`](../../../docs/reference/application-api.md).
//!
//! Typed views and `register_component` live here. Scene types are also under
//! [`host`]; frame counters under [`perf`]. [`internal`] is reserved for host
//! adapters and migration checks — not a second product contract.

/// Scene host: retained document, render scene, opaque GPU slot keys.
pub mod host {
    #[cfg(feature = "graph-canvas")]
    pub use nana_ui_runtime::GRAPH_CANVAS_RENDERER;
    pub use nana_ui_runtime::{
        CustomRenderNode, ExtractedNode, ExtractedTextSpan, GPU_TEXTURE_VIEW_RENDERER,
        GPU_VIEW_RENDERER, HOST_TEXTURE_RENDERER, pack_gpu_revision, unpack_gpu_revision,
    };
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

/// Rich text vocabulary: the application-owned value [`RichTextView`] shows,
/// its sparse span styles, and the authoring-space colour they paint in.
///
/// Its own module because `PaintColor` here is the Style Model's colour value,
/// not the painter's [`crate::runtime::PaintColor`] (which also takes theme
/// roles).
pub mod rich {
    pub use nana_ui_core::{
        AttributedRanges, MAX_TEXT_SHADOWS, OBJECT_REPLACEMENT, PaintColor, RichObject,
        RichObjectContent, RichPaintStyle, RichShapeStyle, RichSpanStyle, RichText,
        RichTextBuilder, RichTextShadow, RichTextStroke, TextDecorationLine, TextStrokeJoin,
        TextStrokePlacement,
    };
}

/// Work counters and frame profiler for benches and Issue #8 — not view state.
pub mod perf {
    pub use nana_ui_runtime::{
        FrameProfile, FrameProfiler, FrameStage, GpuWorkObservation, StageStatus, StageTiming,
        SystemWork, WorkCounters,
    };
}

/// Full `nana-ui-runtime` surface for host adapters and migration checks.
///
/// This is a compatibility escape hatch, not a second application API. New
/// product code should import from [`crate::runtime`] directly.
#[doc(hidden)]
pub mod internal {
    pub use nana_ui_runtime::*;
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

pub use nana_ui_runtime::*;
pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
