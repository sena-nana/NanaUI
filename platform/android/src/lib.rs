//! Experimental Android ARM64 host — not a current NanaUI product target.
//!
//! GameActivity + GameTextInput InputConnection + `ANativeWindow` Surface
//! (wgpu 30 / Vulkan) over [`nana_mobile_host`], the core it shares with the
//! iOS host. The window is one NanaUI Runtime document — the mobile probe
//! page — painted through UiScene + `SceneWgpuPainter`, not a second product
//! paint kernel.

#![cfg_attr(target_os = "android", allow(clippy::unnecessary_wraps))]

#[cfg(target_os = "android")]
mod host;
mod shell;
#[cfg(target_os = "android")]
mod slot_ax;
// Host-tested, but only the Android-only `runtime` uses it outside tests.
#[cfg_attr(all(not(target_os = "android"), not(test)), allow(dead_code))]
mod slot_ime;
mod slot_input;

pub use shell::{
    ANDROID_MDPI_DPI, AndroidShellStub, DEFAULT_ANDROID_SCALE_FACTOR, SystemInsets, primary_rect,
    scale_factor_from_density_dpi,
};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;

/// GameActivity entry — linked from the AndroidManifest `android.app.lib_name`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("nana-android-host"),
    );
    log::info!("nana-android-host: android_main (no WebView)");
    if let Err(err) = host::run(app) {
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
        assert!(caps.ime);
        assert!(caps.clipboard);
    }

    /// Same `LayoutStyle` as desktop Runtime/Scene — no Android field fork.
    /// Extra fields use [`Default`]; do not reconstruct the struct here.
    #[test]
    fn experimental_host_shares_current_layout_fields() {
        let style = nana_ui_core::LayoutStyle::default();
        let _pointer_events: Option<nana_ui_core::PointerEventsSpec> = style.pointer_events;
        let _transform_3d: Option<nana_ui_core::PaintMat4> = style.transform_3d;
        let _logical_padding: nana_ui_core::LogicalEdges = *style.logical_padding;
        let _logical_margin: nana_ui_core::LogicalEdges = *style.logical_margin;
        let _logical_inset: nana_ui_core::LogicalEdges = *style.logical_inset;
    }
}
