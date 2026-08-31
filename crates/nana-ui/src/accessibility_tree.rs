//! Public AccessKit projection for hosts that own their platform adapter
//! wiring (e.g. Android via `accesskit_android`): converts Runtime's
//! backend-neutral accessibility nodes into AccessKit `TreeUpdate`s with the
//! same projector desktop hosts use. macOS/Windows/Unix hosts should prefer
//! the bundled `hosted` adapters.

use accesskit::TreeUpdate;
use nana_ui_runtime::{AccessibilityDelta, AccessibilityNode};

use crate::accessibility::AccessibilityProjector;

/// Stateful Runtime → AccessKit tree projector.
pub struct AccessTreeProjector(AccessibilityProjector);

impl AccessTreeProjector {
    /// Start projecting `nodes` as a full tree.
    pub fn new(nodes: Vec<AccessibilityNode>, interactive: bool, scale_factor: f32) -> Self {
        let (inner, _) =
            AccessibilityProjector::new_at_generation(nodes, interactive, scale_factor, None);
        Self(inner)
    }

    /// Replace the cached tree with `nodes` and produce a full update.
    pub fn synchronize_full(
        &mut self,
        nodes: Vec<AccessibilityNode>,
        scale_factor: f32,
    ) -> Option<TreeUpdate> {
        self.0.synchronize_full(nodes, scale_factor, None)
    }

    /// Apply one incremental transaction; `None` when it is stale.
    pub fn apply_delta(&mut self, delta: AccessibilityDelta) -> Option<TreeUpdate> {
        self.0.apply_delta(delta)
    }

    /// Rebuild the full tree update from the cached nodes.
    pub fn full_update(&self) -> TreeUpdate {
        self.0.full_update()
    }
}
