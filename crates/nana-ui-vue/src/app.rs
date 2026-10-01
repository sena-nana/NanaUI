//! 稳定公开入口：把 Vue 运行时挂成标准 NanaUI 应用宿主。

use nana_js_engine::{JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui_core::{CompiledTheme, ThemeAppearance, ThemeId, ThemeRegistry};
use nana_ui_web_api::{SharedFetchHost, SharedWebSocketHost};
use std::sync::Arc;

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
    pub theme: ThemeAppearance,
    /// Optional application-registry resolved theme. When present it is
    /// installed into the retained document after the preset seed is applied.
    pub custom_theme: Option<Arc<CompiledTheme>>,
    pub theme_id: Option<ThemeId>,
    pub theme_registry: Option<Arc<ThemeRegistry>>,
    /// Optional application-owned, policy-gated HTTP(S) backend for this
    /// mount. Governs its JS `fetch()`; [`NanaVueApp::fetch_host`] returns it
    /// for the painter, so the document's `url(...)` images follow the same
    /// policy. Without one, both paths deny every origin.
    pub fetch_host: Option<SharedFetchHost>,
    /// Optional application-owned WebSocket transport. When omitted, the
    /// default native host is used with a deny-all `SocketPolicy`.
    pub socket_host: Option<SharedWebSocketHost>,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: ThemeAppearance::Light,
            custom_theme: None,
            theme_id: None,
            theme_registry: None,
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
    let custom_theme = options.custom_theme.clone().or_else(|| {
        options.theme_id.as_ref().and_then(|id| {
            options
                .theme_registry
                .as_ref()
                .map(|registry| registry.resolve(id).theme)
        })
    });
    if let Some(theme) = custom_theme.clone() {
        let _ = app
            .document()
            .lock()
            .expect("vue doc")
            .context_mut()
            .set_theme_tokens(theme);
    }
    if let Some(socket_host) = options.socket_host
        && let Ok(mut web) = app.web_api().lock()
    {
        web.set_socket_host(Some(socket_host));
    }
    {
        let bridge = app.bridge();
        let mut guard = bridge.lock().expect("vue bridge");
        if let Some(registry) = options.theme_registry.clone() {
            guard.set_theme_registry(registry);
        }
        if let Some(theme) = custom_theme {
            guard.set_theme_tokens(theme);
        } else {
            guard.set_preset_theme(options.theme);
        }
        if let Some(theme_id) = options.theme_id.clone() {
            let resolved_id = options
                .theme_registry
                .as_ref()
                .map(|registry| registry.resolve(&theme_id))
                .filter(|resolution| resolution.fell_back_to_light)
                .map(|_| ThemeId::new("nana.light"))
                .unwrap_or(theme_id);
            guard.set_theme_id(resolved_id);
        } else {
            let id = guard.theme_tokens().id();
            guard.set_theme_id(id);
        }
    }
    {
        let label = options
            .theme_id
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .unwrap_or_else(|| {
                match options.theme {
                    ThemeAppearance::Light => "light",
                    ThemeAppearance::Dark => "dark",
                    ThemeAppearance::Custom => "custom",
                }
                .to_owned()
            });
        let doc = app.document();
        doc.lock().expect("vue doc").set_document_theme(&label);
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
            theme: ThemeAppearance::Dark,
            ..Default::default()
        });
        assert_eq!(app.theme, ThemeAppearance::Dark);
        let snap = app.semantic_snapshot();
        assert_eq!(snap.theme_appearance, ThemeAppearance::Dark);
    }
}
