#![cfg(windows)]

#[path = "../support/mod.rs"]
mod support;

mod dx12_partial_present;
mod surface_reconfigure;
mod vulkan_present_tracking;
