//! V8 bootstrap shared by Android runtime and host-side smoke checks.

#[cfg(feature = "engine-v8")]
use nana_js_engine::probe::{probe_host_registry, vue_runtime_probe_artifact};
#[cfg(feature = "engine-v8")]
use nana_js_engine::{HostValue, JsEngine};
use nana_ui_platform::PlatformCapabilities;
#[cfg(feature = "engine-v8")]
use nana_ui_vue::VueHost;

/// Result of a one-shot V8 + VueHost bring-up (no GPU).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineBootReport {
    pub ok: bool,
    pub count: f64,
    pub create_element: u64,
    pub capabilities: PlatformCapabilities,
}

/// Load V8, attach VueHost, run the shared Vue runtime-core probe.
///
/// Used on host CI without a Surface. Android ARM64 links V8 when
/// `RUSTY_V8_ARCHIVE` points at the GitHub Actions package
/// (`docs/android.md`).
pub fn smoke_engine_only() -> Result<EngineBootReport, String> {
    #[cfg(feature = "engine-v8")]
    {
        use nana_js_v8::V8Engine;

        let caps = PlatformCapabilities::android_mvp();
        let mut host = VueHost::with_viewport(720, 1280, 2.0);
        let mut engine = V8Engine::new();
        host.attach_engine(&mut engine)
            .map_err(|e| format!("attach vue host: {e}"))?;

        let (api, state) = probe_host_registry();
        engine
            .register_host_api(&api)
            .map_err(|e| format!("register host api: {e}"))?;
        engine
            .initialize(vue_runtime_probe_artifact())
            .map_err(|e| format!("initialize probe: {e}"))?;

        let run = engine
            .resolve_function("__nanaProbe.run")
            .map_err(|e| format!("resolve probe: {e}"))?;
        let result = engine
            .invoke(run, &[])
            .map_err(|e| format!("invoke probe: {e}"))?;
        engine
            .run_microtasks()
            .map_err(|e| format!("microtasks: {e}"))?;

        let object = result
            .as_object()
            .ok_or_else(|| "probe result is not an object".to_string())?;
        let guard = state
            .lock()
            .map_err(|_| "probe state poisoned".to_string())?;

        let report = EngineBootReport {
            ok: object
                .get("ok")
                .and_then(HostValue::as_bool)
                .unwrap_or(false),
            count: object
                .get("count")
                .and_then(HostValue::as_f64)
                .unwrap_or(0.0),
            create_element: guard.create_element,
            capabilities: caps,
        };
        engine.shutdown();
        Ok(report)
    }

    #[cfg(not(feature = "engine-v8"))]
    {
        let _ = PlatformCapabilities::android_mvp();
        Err(
            "nana-android-host requires feature `engine-v8` (desktop smoke, or Android with RUSTY_V8_ARCHIVE)"
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "engine-v8")]
    #[test]
    fn smoke_engine_boots_on_host() {
        let report = smoke_engine_only().expect("v8 smoke");
        assert!(report.ok);
        assert!(report.create_element > 0);
        assert!(report.capabilities.vulkan_surface);
        assert!(report.capabilities.shell_chrome_fill);
        assert!(report.capabilities.control_slot);
        assert!(report.capabilities.control_widget);
        assert!(report.capabilities.control_input);
        assert!(!report.capabilities.desktop_shell);
        assert!(!report.capabilities.ime);
        assert!(!report.capabilities.clipboard);
    }
}
