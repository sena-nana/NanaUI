//! Snapshot-only offscreen Scene paint + CPU readback.
//!
//! Product windows never use this path. Shared implementation lives in
//! `nana-ui-devtools` (`offscreen` feature).

#![allow(unused_imports)]

pub use nana_ui_devtools::offscreen::{FORMAT, OffscreenSnapshots, readback, write_scene};
