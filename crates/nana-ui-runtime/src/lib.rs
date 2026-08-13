//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! generational entity representation is deliberately not part of the public
//! contract, so changing ECS implementations cannot invalidate JS handles,
//! diagnostics, snapshots, or persisted data.

mod animation;
mod components;
mod framework;
mod mutation;
mod schedule;
mod world;

pub use animation::{AnimationFrame, AnimationId, AnimationSample, AnimationSpec, Easing};
pub use components::{
    AccessibilityNode, AccessibilityRole, AccessibilityState, ComputedStyle, CustomRenderNode,
    EventRoute, ExtractedNode, ImeComposition, InteractionState, LayoutBox, LayoutInput, NodeStyle,
    PointerCaptureChange, TextContent, TextInputState, TextMetrics, TextSelection, TextShaper,
};
pub use framework::{
    AppContext, Entity, ExtensionRegistrar, FrameworkError, Subscription, Task, UiExtension, View,
    ViewContext,
};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{ActionId, ContextPredicate, KeyContext};
pub use schedule::SystemWork;
pub use world::{
    CommitReport, DocumentId, NodeKind, NodeSnapshot, StableNodeId, UiWorld, UiWorldError,
};
