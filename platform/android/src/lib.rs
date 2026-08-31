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
#[cfg(target_os = "android")]
mod slot_ax;
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

#[cfg(test)]
mod android_not_product {
    /// Android is experimental, not a product UI path (`run_runtime` / DesktopShell).
    #[test]
    fn android_cfg_is_experimental_host_not_product_path() {
        assert!(!crate::AndroidShellStub::desktop_shell_available());
        let caps = nana_ui_platform::PlatformCapabilities::android_mvp();
        assert!(!caps.desktop_shell);
        assert!(!caps.ime);
        assert!(!caps.clipboard);

        #[cfg(target_os = "android")]
        {
            assert!(cfg!(target_os = "android"));
            assert!(!crate::AndroidShellStub::desktop_shell_available());
        }
        #[cfg(not(target_os = "android"))]
        {
            assert!(!cfg!(target_os = "android"));
        }
    }

    /// Same `LayoutStyle` as desktop Runtime/Scene — no Android field fork.
    /// Extra L1 fields use [`Default`]; do not reconstruct the struct here.
    #[test]
    fn experimental_host_shares_current_l1_layout_fields() {
        let style = nana_ui_core::LayoutStyle::default();
        let _pointer_events: Option<nana_ui_core::PointerEventsSpec> = style.pointer_events;
        let _transform_3d: Option<nana_ui_core::PaintMat4> = style.transform_3d;
        let _logical_padding: nana_ui_core::LogicalInlineEdges = style.logical_padding;
        let _logical_margin: nana_ui_core::LogicalInlineEdges = style.logical_margin;
        let _logical_inset: nana_ui_core::LogicalInlineEdges = style.logical_inset;
    }
}
