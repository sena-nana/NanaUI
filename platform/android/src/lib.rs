//! Minimal Android ARM64 host — no System WebView.
//!
//! Lifecycle + `ANativeWindow` Surface + QuickJS (wgpu 30 / Vulkan; no Blitz).
//! Desktop builds of this crate expose [`smoke_engine_only`] for CI without NDK UI.

#![cfg_attr(target_os = "android", allow(clippy::unnecessary_wraps))]

mod chrome_fill;
mod engine;
#[cfg(target_os = "android")]
mod gpu;
mod iced_control;
mod iced_slot;
mod iced_slot_input;
#[cfg(target_os = "android")]
mod iced_slot_paint;
#[cfg(target_os = "android")]
mod runtime;
mod shell;

pub use chrome_fill::{band_draw_list, clamp_scissor};

pub use shell::{
    ANDROID_MDPI_DPI, AndroidShellStub, DEFAULT_ANDROID_SCALE_FACTOR, scale_factor_from_density_dpi,
};

pub use engine::{EngineBootReport, smoke_engine_only};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;

/// NativeActivity entry — linked from the AndroidManifest `android.app.lib_name`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("nana-android-host"),
    );
    log::info!("nana-android-host: android_main (no WebView)");
    if let Err(err) = runtime::run(app) {
        log::error!("nana-android-host: fatal: {err}");
    }
}
