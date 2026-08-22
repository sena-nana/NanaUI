//! Experimental Android ARM64 host — not a current NanaUI product target.
//!
//! NativeActivity + `ANativeWindow` Surface + V8 (wgpu 30 / Vulkan). Desktop
//! builds expose [`smoke_engine_only`] for host-side compile/smoke without NDK UI.
//! The control slot is NanaUI Runtime + UiScene + `SceneWgpuPainter`.

#![cfg_attr(target_os = "android", allow(clippy::unnecessary_wraps))]

mod chrome_fill;
mod control_slot;
mod engine;
#[cfg(target_os = "android")]
mod gpu;
#[cfg(target_os = "android")]
mod runtime;
mod shell;
mod slot_input;
#[cfg(target_os = "android")]
mod slot_paint;
mod slot_runtime;

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
