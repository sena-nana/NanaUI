//! Dock workspace: serde model, controller reduction, view projection, host adapter.

mod model;
mod view;
mod controller;
#[cfg(feature = "hosted")]
mod host;

pub use controller::*;
#[cfg(feature = "hosted")]
pub use host::*;
pub use model::*;
pub use view::*;
