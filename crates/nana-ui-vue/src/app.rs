//! 稳定公开入口：把 Vue 运行时挂成标准 NanaUI 应用宿主。

use nana_js_engine::{JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui_core::ThemeMode;
use nana_ui_web_api::{SharedFetchHost, SharedWebSocketHost};

use crate::VueHost;
use crate::bridge::SemanticSnapshot;

/// 系统化 Vue→Nana 宿主（稳定公开名）。
///
/// 当前实现即 [`VueHost`]：拥有 Vue facade、语义投影与 web-api 状态。
pub type NanaVueApp = VueHost;

/// [`mount_vue_as_nana`] 视口选项。
#[derive(Debug, Clone)]
pub struct MountOptions {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub theme: ThemeMode,
    /// Optional application-owned, policy-gated HTTP(S) backend.
    pub fetch_host: Option<SharedFetchHost>,
    /// Optional application-owned WebSocket transport. The framework reserves
    /// the interface only — without a host here, the JS `WebSocket` constructor
    /// reports itself unavailable.
    pub socket_host: Option<SharedWebSocketHost>,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: ThemeMode::Light,
            fetch_host: None,
            socket_host: None,
        }
    }
}

/// 创建已就绪视口的 [`NanaVueApp`]（尚未绑定 JS 引擎）。
///
/// 典型后续：
/// ```ignore
/// let mut app = mount_vue_as_nana(MountOptions::default());
/// app.attach_engine(&mut engine)?;
/// app.initialize_with_web_api(&mut engine, artifact)?;
/// app.bind_event_bridge(&mut engine)?;
/// let snap = app.semantic_snapshot();
/// ```
pub fn mount_vue_as_nana(options: MountOptions) -> NanaVueApp {
    let mut app = match options.fetch_host {
        Some(fetch_host) => NanaVueApp::with_web_api_state(
            options.width,
            options.height,
            options.scale_factor,
            nana_ui_web_api::shared_web_api_state_with_fetch(fetch_host),
        ),
        None => NanaVueApp::with_viewport(options.width, options.height, options.scale_factor),
    };
    // Theme is applied fully once an engine is bound; seed bridge/document/web-api here.
    app.theme = options.theme;
    if let Some(socket_host) = options.socket_host
        && let Ok(mut web) = app.web_api().lock()
    {
        web.set_socket_host(Some(socket_host));
    }
    {
        let bridge = app.bridge();
        let mut guard = bridge.lock().expect("vue bridge");
        guard.set_theme(options.theme);
    }
    {
        let label = match options.theme {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        let doc = app.document();
        doc.lock().expect("vue doc").set_document_theme(label);
        if let Ok(mut web) = app.web_api().lock() {
            web.set_document_dataset("theme", label);
        }
    }
    app
}

/// 一站式：创建宿主、挂引擎、加载产物并绑定事件桥。
pub fn mount_vue_as_nana_with_engine<E: JsEngine + ?Sized>(
    options: MountOptions,
    engine: &mut E,
    artifact: RuntimeArtifact,
) -> Result<NanaVueApp, JsEngineError> {
    let mut app = mount_vue_as_nana(options);
    app.initialize_with_web_api(engine, artifact)?;
    app.bind_event_bridge(engine)?;
    Ok(app)
}

/// 便利：取当前语义快照（供 Scene host / 测试）。
pub fn semantic_snapshot_of(app: &NanaVueApp) -> SemanticSnapshot {
    app.semantic_snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_seeds_theme_and_empty_snapshot() {
        let app = mount_vue_as_nana(MountOptions {
            width: 320,
            height: 240,
            theme: ThemeMode::Dark,
            ..Default::default()
        });
        assert_eq!(app.theme, ThemeMode::Dark);
        let snap = app.semantic_snapshot();
        assert_eq!(snap.theme, ThemeMode::Dark);
    }
}
