//! Development-only diagnostics for NanaUI's hosted rendering stack. The crate
//! deliberately does not provide product-facing UI.
//!
//! Optional `offscreen` / `runtime-agent` features add snapshot CPU readback and
//! a headless Agent session. Those paths are tooling-only and must not be wired
//! into product Surface present.
//!
//! The tiers nest: `offscreen` is paint plus readback; `runtime-agent` adds the
//! JSON protocol, the `agent::AgentSession` trait, the Rust Runtime session
//! and the `nana-runtime-agent` binary; `headless-host` drives a whole
//! `RuntimeProgram` offscreen. `inspect-overlay` stands apart: runtime only,
//! it turns a live document into draw data for a development host's
//! inspection overlay (`inspect::InspectOverlay`), which the host paints.

#[cfg(feature = "runtime-agent")]
pub mod agent;
#[cfg(feature = "headless-host")]
pub mod headless;
#[cfg(feature = "inspect-overlay")]
pub mod inspect;
#[cfg(feature = "offscreen")]
pub mod offscreen;
