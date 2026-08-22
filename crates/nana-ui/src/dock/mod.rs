//! Dock workspace: serde model, controller reduction, view projection, host adapter.
//!
//! Product dock is Runtime [`crate::DockWorkspace`]. Controller types such as
//! [`DockController`] and [`DockAction`] are public here as `nana_ui::dock::*`,
//! not at the crate root.

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
