//! Window-independent presentation/output contracts.
//!
//! The output layer sits after [`nana_ui_scene::UiScene`].  It owns retained
//! external resources, while the existing [`crate::SceneWgpuPainter`] remains
//! the only Scene renderer.  A Window is consequently just one consumer of a
//! presentation; headless, nested and native-host consumers can use the same
//! contracts without constructing a `winit` window.

mod adapters;
pub(crate) mod driver;
mod embedded;
mod external;
mod planner;
pub(crate) mod provenance;
mod trace;
pub(crate) mod transfer;
pub use trace::{
    OutputRecreateReason, OutputTraceDeferral, OutputTraceRole, OutputTraceSnapshot,
    OutputTraceState, OutputTraceWait, OutputWorkObservation, PresentationWork,
};
#[cfg(feature = "hosted")]
pub(crate) mod window_output;
#[cfg(feature = "hosted")]
mod window_trace;

#[cfg(test)]
mod cycle_tests;
#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod tests;

pub use adapters::{
    OverlayAdapterError, OverlayAdapterState, OverlayFramePlan, OverlayHostAdapter, OverlayRelease,
    OverlayTransport, OverlayTransportError,
};
pub use driver::{
    DocumentFrame, MediaSampler, PresentationDriver, PresentationError, PresentationFrameTime,
};
pub use embedded::{
    EmbeddedBindError, EmbeddedFrameBinding, EmbeddedInputMapping, EmbeddedMetadataError,
    EmbeddedOutputMetadata, EmbeddedSurfaceNode,
};
pub use external::{
    ExternalFrame, ExternalFramePlan, ExternalPrepare, ExternalRenderOutcome, ExternalSurface,
    ExternalSurfaceConfig, ExternalSurfaceError,
};
pub use planner::{
    OutputAlpha, OutputConsumer, OutputConsumerBinding, OutputConsumerId, OutputConsumerKind,
    OutputIncompatibility, OutputIncompatibilityReason, OutputOwnership, OutputPath, OutputPlan,
    OutputTargetGroup, OutputTextAa, OutputUnsupportedReason, PresentationId,
    PresenterCapabilities, RenderTargetPlanner, RenderTargetRequirements,
};
pub use provenance::{OutputProvenance, OutputSurfaceId};
#[cfg(feature = "hosted")]
pub use window_output::{
    WINDOW_OUTPUT_FORMAT, WINDOW_OUTPUT_HIDDEN_FPS, WindowOutputAlpha, WindowOutputConfig,
    WindowOutputExport, WindowOutputExtent, WindowOutputFit, WindowOutputFrame, WindowOutputLease,
    WindowOutputStatus,
};
