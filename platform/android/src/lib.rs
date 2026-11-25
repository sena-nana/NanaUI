//! Experimental Android ARM64 host — not a current NanaUI product target.
//!
//! GameActivity + GameTextInput InputConnection + `ANativeWindow` Surface
//! (wgpu 30 / Vulkan) over [`nana_mobile_host`], the core it shares with the
//! iOS host. The window is one NanaUI Runtime document painted through
//! UiScene + `SceneWgpuPainter`, not a second product paint kernel.
//!
//! The host is an rlib as well: an app crate builds the one cdylib its
//! Gradle project packages, puts its view in through [`AndroidHostApp`] and
//! exports `android_main` with [`export_android_app!`], with this crate's
//! default `probe` feature off. With it on (this crate's own APK), the
//! window is the mobile probe page shared with the iOS host.

#![cfg_attr(target_os = "android", allow(clippy::unnecessary_wraps))]

#[cfg_attr(all(not(target_os = "android"), not(test)), allow(dead_code))]
mod choreographer;
#[cfg(target_os = "android")]
mod host;
#[cfg(all(target_os = "android", feature = "probe"))]
mod probe;
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
pub use android_activity;
#[cfg(target_os = "android")]
pub use host::{AndroidHostApp, Haptic, Lifecycle, Shell};
#[cfg(target_os = "android")]
pub use nana_mobile_host::{FrameDemand, MobileRenderer, MobileRuntime};
#[cfg(target_os = "android")]
pub use nana_ui::runtime::FrameworkError;

/// Run the activity around `app` until it is destroyed: the body of
/// [`export_android_app!`]'s `android_main`. Logs to logcat under `tag`.
#[cfg(target_os = "android")]
pub fn main(activity: android_activity::AndroidApp, app: Box<dyn AndroidHostApp>, tag: &str) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag(tag),
    );
    log::info!("nana-android-host: android_main (no WebView)");
    if let Err(err) = host::run(activity, app) {
        log::error!("nana-android-host: fatal: {err}");
    }
}

/// Export the GameActivity entry `android_main`, building the host around
/// `$app` (an expression of a type implementing [`AndroidHostApp`]) and
/// logging under `$tag`. Use it once, in the crate that builds the cdylib
/// the manifest's `android.app.lib_name` names.
#[macro_export]
macro_rules! export_android_app {
    ($app:expr, $tag:expr) => {
        #[unsafe(no_mangle)]
        fn android_main(activity: $crate::android_activity::AndroidApp) {
            $crate::main(activity, ::std::boxed::Box::new($app), $tag);
        }
    };
}

#[cfg(all(target_os = "android", feature = "probe"))]
export_android_app!(probe::ProbeApp::default(), "nana-android-host");

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
        let _transform_3d: Option<nana_ui_core::PaintMat4> = style.transforms.transform_3d;
        let _logical_padding: nana_ui_core::LogicalEdges = style.logical_edges.padding;
        let _logical_margin: nana_ui_core::LogicalEdges = style.logical_edges.margin;
        let _logical_inset: nana_ui_core::LogicalEdges = style.logical_edges.inset;
    }
}
