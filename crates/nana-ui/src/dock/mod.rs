//! Host-adapter dock: pointer/dwell/frame → [`DockMutation`]; geometry via [`DockController::surface_layout`].
//!
//! Product dock is Runtime [`crate::DockWorkspace`], re-exported at the crate
//! root. [`DockController`] does not own a second split-ratio formula: resize
//! and keyboard nudge call `nana_ui_runtime::{dock_split_ratio_from_pointer,
//! dock_nudge_split_ratio, dock_split_child_lengths}`. Split pixels come only
//! from [`DockController::surface_layout`]. Controller types such as
//! [`DockController`] and [`DockAction`] are public here as `nana_ui::dock::*`,
//! not at the crate root.
//!
//! Remaining adapter-only APIs (not product dock): [`DockLayout`] serde,
//! [`DockItemSpec`] size limits, monitor clamp, drag/dwell, hosted window
//! effects, and [`DockAction`] → [`DockMutation`] conversion.

mod controller;
#[cfg(feature = "hosted")]
mod host;
mod model;
mod view;

pub use controller::*;
#[cfg(feature = "hosted")]
pub use host::*;
pub use model::*;
pub use view::*;

#[cfg(test)]
mod tests {
    #[test]
    fn dock_controller_is_public_on_the_module_path() {
        let layout = crate::dock::DockLayout::new(crate::dock::DockNode::item("center"));
        assert!(
            crate::dock::DockController::new(
                "center",
                [crate::dock::DockItemSpec::new("center", "Center")],
                layout,
            )
            .is_ok()
        );
    }
}
