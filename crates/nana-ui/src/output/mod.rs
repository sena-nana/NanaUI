//! Window-independent presentation/output contracts.
//!
//! The output layer sits after [`nana_ui_scene::UiScene`].  It owns target
//! negotiation and retained external resources, while the existing
//! [`crate::SceneWgpuPainter`] remains the only Scene renderer.  A Window is
//! consequently just one consumer of a presentation; headless, nested and
//! native-host consumers can use the same contracts without constructing a
//! `winit` window.

mod embedded;
mod external;
pub mod planner;
#[cfg(feature = "hosted")]
mod window;

#[cfg(test)]
mod tests;

pub use embedded::{
    EmbeddedBindError, EmbeddedFrameBinding, EmbeddedMetadataError, EmbeddedOutputMetadata,
    EmbeddedSurfaceNode,
};
pub use external::{
    ExternalFrame, ExternalRenderOutcome, ExternalSurface, ExternalSurfaceConfig,
    ExternalSurfaceError,
};
pub use planner::{
    ConsumerId, ConsumerKind, ConsumerRoute, OutputConsumer, OutputPath, OutputPlan,
    OutputTopology, PlanError, PresenterCapabilities, RenderTargetPlanner,
    RenderTargetRequirements,
};
#[cfg(feature = "hosted")]
pub use window::WindowPresenter;
