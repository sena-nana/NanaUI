//! Platform-neutral core of the NanaUI mobile hosts (experimental).
//!
//! A mobile host is a native shell (UIKit view, GameActivity) around one
//! retained NanaUI document. This crate is the part both shells share, so the
//! iOS and Android hosts only lower native events and own their surface:
//!
//! - [`MobileRuntime`] owns the [`nana_ui::runtime::RuntimeDocument`] the
//!   application mounts its view into, stamps and routes input, keeps the
//!   IME lifecycle, pads the root by the safe area, and answers
//!   [`FrameDemand`] so the shell runs its display link only while something
//!   is due — an idle document presents no frames.
//! - [`MobileRenderer`] owns the one Device/Queue across surface loss: a
//!   backgrounded app drops the surface and keeps the device, the document
//!   and its state.
//!
//! Native behaviour stays native (`docs/reference/mobile-host.md`): the shell
//! drives the platform text system, accessibility and system sheets, and
//! NanaUI draws the pixels. Native views are layered whole above or below
//! the NanaUI surface, never placed inside its layout.

mod render;
mod runtime;
mod schedule;
mod viewport;

pub use render::{MobileRenderer, SurfaceOptions};
pub use runtime::{MobileAppearance, MobilePlatform, MobileRuntime, TextInputSnapshot, TouchPhase};
pub use schedule::FrameDemand;
pub use viewport::{MobileViewport, SafeArea};

pub use nana_ui::runtime::ScrollPhysics;

/// Install the waker the view task executor calls when a future is ready.
/// It may run on any thread; the shell answers it by requesting a frame on
/// the main thread, where [`MobileRuntime::prepare_frame`] polls the tasks.
pub fn set_task_wake(wake: impl Fn() + Send + Sync + 'static) {
    nana_ui::runtime::view::set_task_wake(wake);
}
