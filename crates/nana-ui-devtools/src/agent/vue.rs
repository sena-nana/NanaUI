//! Vue/JS agent session. Requires the Vue renderer and a JS engine.

use std::collections::BTreeMap;
use std::path::Path;

use nana_js_engine::{JsEngine, RuntimeArtifact};
use nana_ui::HostTextureRegistry;
use nana_ui::runtime::{AccessibilityAction, AccessibilityActionRequest, StableNodeId, ThemeMode};
use nana_ui_core::SemanticColorRole;
use nana_ui_platform::InputModifiers;
use nana_ui_vue::{
    BridgeEvent, KeyboardInput, NodeHandle, PointerEventKind, PointerInput, SemanticSnapshot,
    VueHost,
};

use super::protocol::{
    DiagnosticDump, HitDump, KeyStroke, PixelStats, PointerGesture, SceneProbeDump,
    SemanticDumpWidget, SessionInfo, ThemeName,
};
use super::session::AgentSession;
use super::{AccessibilityDumpNode, AgentError, dump_accessibility_node, scene_probe};
use crate::DevtoolsSession;
use crate::offscreen::{self, OffscreenSnapshots, Size};

/// Vue document driven without a winit window.
pub struct VueAgentSession<E: JsEngine> {
    host: VueHost,
    engine: E,
    gpu: Option<OffscreenSnapshots>,
    width: u32,
    height: u32,
    scale_factor: f32,
    /// `None` follows the document's active theme.
    clear: Option<[f32; 4]>,
    host_textures: HostTextureRegistry,
    /// Vue warnings and errors are the most common cause of a blank frame that
    /// still reports `ok`. Recording them is what lets `{"cmd":"diagnostics"}`
    /// explain the screenshot instead of leaving the caller to guess.
    diagnostics: DevtoolsSession,
    /// Builds every isolate this session runs, including the first.
    ///
    /// A factory rather than an instance because `JsEngine` has no constructor
    /// in its contract, so a session cannot otherwise build the replacement a
    /// `{"cmd":"reload"}` needs -- and taking it up front makes "this session
    /// cannot reload" an unrepresentable state instead of a runtime error.
    engine_factory: std::sync::Arc<dyn Fn() -> E + Send + Sync>,
    /// The last artifact that evaluated cleanly, put back when a reload fails.
    last_good: RuntimeArtifact,
}

impl<E: JsEngine> VueAgentSession<E> {
    pub fn new(
        engine: impl Fn() -> E + Send + Sync + 'static,
        artifact: RuntimeArtifact,
        width: u32,
        height: u32,
    ) -> Result<Self, AgentError> {
        Self::new_scaled(engine, artifact, width, height, 1.0)
    }

    /// Width and height are logical pixels; PNG dimensions include the scale.
    ///
    /// A 1x-only Vue session cannot reproduce any HiDPI rounding defect, which
    /// is the class of bug most likely to need a screenshot in the first place.
    pub fn new_scaled(
        engine: impl Fn() -> E + Send + Sync + 'static,
        artifact: RuntimeArtifact,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<Self, AgentError> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(AgentError(
                "snapshot scale must be finite and positive".into(),
            ));
        }
        let engine_factory: std::sync::Arc<dyn Fn() -> E + Send + Sync> =
            std::sync::Arc::new(engine);
        let mut engine = engine_factory();
        let diagnostics = DevtoolsSession::default();
        let mut host = VueHost::with_viewport(width, height, scale_factor);
        // `JsEngine` exposes no diagnostics hook, so this captures Vue
        // warnings/errors and resource lifecycle. An engine-level exception
        // surfaces as the failing command's own error instead.
        host.set_diagnostics(Some(diagnostics.js_sink()), None);
        // Kept so a failed reload can put the working build back rather than
        // leaving the developer with a blank window.
        let last_good = artifact.clone();
        host.initialize_with_web_api(&mut engine, artifact)?;
        host.bind_event_bridge(&mut engine)?;
        let mut session = Self {
            host,
            engine,
            gpu: None,
            width,
            height,
            scale_factor,
            clear: None,
            host_textures: HostTextureRegistry::new(),
            diagnostics,
            engine_factory,
            last_good,
        };
        session.pump()?;
        Ok(session)
    }

    /// Read a UTF-8 artifact or stylesheet from disk for a reload.
    ///
    /// An empty file is refused rather than loaded: it is nearly always a save
    /// caught between truncate and write, and evaluating it would replace a
    /// working app with a blank one for no reason the developer can see.
    fn read_reload_source(path: &Path) -> Result<String, AgentError> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| AgentError(format!("cannot read {}: {error}", path.display())))?;
        if source.trim().is_empty() {
            return Err(AgentError(format!(
                "{} is empty; refusing to reload a half-written file",
                path.display()
            )));
        }
        Ok(source)
    }

    /// Tear the host down and swap in a brand-new isolate.
    ///
    /// The old engine is shut down *before* the new one is built. V8 enters an
    /// isolate on creation and exits it on drop, strictly LIFO, so constructing
    /// the replacement while the previous isolate is still live aborts the
    /// process on the first reload.
    ///
    /// Returns whatever the outgoing artifact published for its successor.
    fn swap_engine(&mut self) -> Result<Option<String>, AgentError> {
        let state = nana_ui_vue::dev::save_state(&mut self.engine);
        self.host.dev_teardown().map_err(AgentError::from)?;
        self.engine.shutdown();
        self.engine = (self.engine_factory)();
        self.host
            .set_diagnostics(Some(self.diagnostics.js_sink()), None);
        Ok(state)
    }

    /// Evaluate `artifact` on the current (already torn-down) host and isolate.
    fn evaluate(
        &mut self,
        artifact: RuntimeArtifact,
        state: Option<&str>,
    ) -> Result<(), AgentError> {
        nana_ui_vue::dev::publish_restore_state(&mut self.engine, state)?;
        self.host
            .initialize_with_web_api(&mut self.engine, artifact)?;
        self.host.bind_event_bridge(&mut self.engine)?;
        Ok(())
    }

    /// Host textures sampled by `nana.host-texture` nodes during a screenshot.
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }

    /// Recorder wired to the JS engine and the Vue host.
    pub fn diagnostics_session(&self) -> &DevtoolsSession {
        &self.diagnostics
    }

    /// Background actually used for the next screenshot.
    pub fn clear_color(&self) -> [f32; 4] {
        self.clear.unwrap_or_else(|| {
            let document = self.host.document();
            let Ok(guard) = document.lock() else {
                return [0.0, 0.0, 0.0, 1.0];
            };
            let color = guard
                .world()
                .style_model()
                .color(SemanticColorRole::Background);
            [color.r, color.g, color.b, color.a]
        })
    }

    pub fn host(&self) -> &VueHost {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut VueHost {
        &mut self.host
    }

    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    pub fn pump(&mut self) -> Result<(), AgentError> {
        self.engine.run_microtasks()?;
        self.host.pump_frame(&mut self.engine)?;
        let _ = self.host.semantic_snapshot();
        self.host
            .flush_scene_frame()
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    pub fn accessibility_dump(&self) -> Vec<AccessibilityDumpNode> {
        let agent_ids = agent_ids_from_snapshot(&self.host.semantic_snapshot());
        let document = self.host.document();
        let Ok(guard) = document.lock() else {
            return Vec::new();
        };
        guard
            .accessibility_snapshot()
            .into_iter()
            .map(|node| dump_accessibility_node(node, &agent_ids))
            .collect()
    }

    pub fn semantic_dump(&self) -> Vec<SemanticDumpWidget> {
        semantic_dump_from_snapshot(&self.host.semantic_snapshot())
    }

    pub fn click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        let handled = self.host.pointer_click(&mut self.engine, x, y)?;
        self.pump()?;
        Ok(handled)
    }

    pub fn click_node(&mut self, id: u64) -> Result<bool, AgentError> {
        if let Some((x, y)) = node_click_point(&self.host, id) {
            return self.click_xy(x, y);
        }
        let handled = self
            .host
            .dispatch_bridge_event(&mut self.engine, BridgeEvent::Press { id })?;
        self.pump()?;
        Ok(handled)
    }

    pub fn click_agent_id(&mut self, agent_id: &str) -> Result<bool, AgentError> {
        let id = self
            .host
            .semantic_snapshot()
            .widgets
            .iter()
            .find(|widget| widget.props.agent_id == agent_id)
            .map(|widget| widget.id)
            .ok_or_else(|| AgentError(format!("unknown agent_id {agent_id}")))?;
        self.click_node(id)
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), AgentError> {
        for character in text.chars() {
            let key = character.to_string();
            self.host
                .dispatch_key(&mut self.engine, &key, "Unidentified", None)?;
        }
        self.pump()?;
        Ok(())
    }

    pub fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        self.pump()?;
        let size = Size::new(
            (self.width as f32 * self.scale_factor).round() as u32,
            (self.height as f32 * self.scale_factor).round() as u32,
        );
        let scale = self.scale_factor;
        let clear = self.clear_color();
        let scene = {
            let document = self.host.document();
            let guard = document
                .lock()
                .map_err(|_| AgentError("vue document poisoned".into()))?;
            guard.scene().clone()
        };
        let textures = self.host_textures.clone();
        let gpu = self.gpu_mut()?;
        let renderers = gpu.default_gpu_renderers();
        let pixels = gpu
            .paint_layers_scaled(
                &[(&scene, true)],
                size,
                scale,
                clear,
                Some(&textures),
                Some(&renderers),
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Ok((size, pixels))
    }

    /// Returns the frame's own verdict, so a caller never has to decide by eye
    /// whether anything painted.
    pub fn screenshot_png(&mut self, path: impl AsRef<Path>) -> Result<PixelStats, AgentError> {
        let clear = self.clear_color();
        let (size, pixels) = self.screenshot_rgba()?;
        offscreen::write_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(super::pixels::pixel_stats(size, &pixels, clear))
    }

    fn document_world<R>(
        &self,
        read: impl FnOnce(&nana_ui_vue::NanaTreeDocument) -> R,
    ) -> Option<R> {
        let document = self.host.document();
        let guard = document.lock().ok()?;
        Some(read(&guard))
    }

    fn gpu_mut(&mut self) -> Result<&mut OffscreenSnapshots, AgentError> {
        if self.gpu.is_none() {
            self.gpu =
                Some(OffscreenSnapshots::new().map_err(|error| AgentError(error.to_string()))?);
        }
        Ok(self.gpu.as_mut().expect("gpu initialized"))
    }
}
impl<E: JsEngine> AgentSession for VueAgentSession<E> {
    fn describe(&self) -> SessionInfo {
        SessionInfo {
            kind: "vue".into(),
            width: self.width,
            height: self.height,
            scale: self.scale_factor,
            clear: self.clear_color(),
        }
    }

    fn flush(&mut self) -> Result<(), AgentError> {
        self.pump()
    }

    /// Re-evaluate the artifact in a fresh isolate and rebuild the tree.
    ///
    /// The document scaffold, the `UiWorld` and every offscreen GPU resource
    /// survive: node ids keep rising rather than restarting, which is what keeps
    /// the accessibility projection's monotonic generation honest.
    ///
    /// A failed evaluation puts the last good artifact back. The developer sees
    /// the app they had plus the error, rather than a blank frame.
    fn reload_artifact(&mut self, path: &Path) -> Result<(), AgentError> {
        let source = Self::read_reload_source(path)?;
        let artifact = RuntimeArtifact::from_source(path.to_string_lossy().as_ref(), source);
        let state = self.swap_engine()?;
        match self.evaluate(artifact.clone(), state.as_deref()) {
            Ok(()) => {
                self.last_good = artifact;
                Ok(())
            }
            Err(error) => {
                // The isolate the failed evaluation ran in is still fresh and
                // the host is still torn down, so the previous build goes back
                // in place without building a second one.
                let previous = self.last_good.clone();
                let _ = self.evaluate(previous, state.as_deref());
                Err(error)
            }
        }
    }

    /// Swap one keyed stylesheet. No node is created or destroyed.
    fn reload_stylesheet(&mut self, key: &str, path: &Path) -> Result<(), AgentError> {
        let css = Self::read_reload_source(path)?;
        self.host.replace_stylesheet(key, &css);
        Ok(())
    }

    fn accessibility_nodes(&self) -> Vec<AccessibilityDumpNode> {
        self.accessibility_dump()
    }

    fn semantic_widgets(&self) -> Option<Vec<SemanticDumpWidget>> {
        Some(self.semantic_dump())
    }

    fn diagnostics(&self) -> Vec<DiagnosticDump> {
        self.diagnostics
            .snapshot()
            .events
            .into_iter()
            .map(super::diagnostic_dump)
            .collect()
    }

    fn set_viewport(&mut self, width: u32, height: u32, scale: f32) -> Result<(), AgentError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(AgentError("scale must be finite and positive".into()));
        }
        if width == 0 || height == 0 {
            return Err(AgentError("viewport must be non-zero".into()));
        }
        self.width = width;
        self.height = height;
        self.scale_factor = scale;
        self.host.set_viewport(width, height, scale);
        self.pump()
    }

    /// A Vue app that manages `documentElement.dataset.theme` itself can
    /// overwrite this on its next render; the command sets the host theme, it
    /// does not take authority away from the application.
    fn set_theme(&mut self, mode: ThemeName) -> Result<(), AgentError> {
        let mode = match mode {
            ThemeName::Light => ThemeMode::Light,
            ThemeName::Dark => ThemeMode::Dark,
        };
        {
            let bridge = self.host.bridge();
            let mut guard = bridge
                .lock()
                .map_err(|_| AgentError("vue bridge poisoned".into()))?;
            guard.set_theme(mode);
        }
        self.pump()
    }

    fn set_clear(&mut self, clear: Option<[f32; 4]>) {
        self.clear = clear;
    }

    fn pointer(&mut self, gesture: PointerGesture) -> Result<bool, AgentError> {
        let handled = match gesture {
            PointerGesture::Click { x, y, button: 0 } => {
                self.host.pointer_click(&mut self.engine, x, y)?
            }
            PointerGesture::Click { x, y, button } => {
                let mut down = PointerInput::mouse(PointerEventKind::Down, x, y);
                down.button = button;
                down.buttons = button_mask(button);
                let mut up = PointerInput::mouse(PointerEventKind::Up, x, y);
                up.button = button;
                let pressed = self.host.dispatch_pointer(&mut self.engine, down)?;
                let released = self.host.dispatch_pointer(&mut self.engine, up)?;
                pressed || released
            }
            PointerGesture::Hover { x, y } => self.host.dispatch_pointer(
                &mut self.engine,
                PointerInput::mouse(PointerEventKind::Move, x, y),
            )?,
            PointerGesture::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => self
                .host
                .pointer_wheel(&mut self.engine, x, y, delta_x, delta_y)?,
        };
        self.pump()?;
        Ok(handled)
    }

    fn activate(&mut self, node: u64) -> Result<bool, AgentError> {
        self.click_node(node)
    }

    fn keyboard(&mut self, stroke: KeyStroke) -> Result<(), AgentError> {
        let mut input = KeyboardInput::key_down(stroke.key, stroke.code);
        input.modifiers = InputModifiers {
            alt: stroke.alt,
            control: stroke.ctrl,
            meta: stroke.meta,
            shift: stroke.shift,
        };
        // `dispatch_keyboard` rather than `dispatch_key`: a named key must not
        // commit text, which is what separates navigation from typing.
        self.host
            .dispatch_keyboard(&mut self.engine, &input, None)?;
        self.pump()
    }

    fn type_text(&mut self, text: &str) -> Result<(), AgentError> {
        Self::type_text(self, text)
    }

    fn set_value(&mut self, node: u64, value: &str) -> Result<bool, AgentError> {
        let target =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        let handled = {
            let document = self.host.document();
            let mut guard = document
                .lock()
                .map_err(|_| AgentError("vue document poisoned".into()))?;
            // `NanaTreeDocument::id` is the Vue-local id; the accessibility
            // action is addressed by the Runtime document it projects into.
            let document_id = guard.runtime_document().document();
            guard
                .context_mut()
                .apply_accessibility_action(
                    document_id,
                    AccessibilityActionRequest {
                        target,
                        action: AccessibilityAction::SetValue(value.to_owned()),
                    },
                )
                .map_err(|error| AgentError(error.to_string()))?
        };
        self.pump()?;
        Ok(handled)
    }

    fn hit_test(&self, x: f32, y: f32) -> Vec<HitDump> {
        let candidates = self
            .document_world(|document| {
                document
                    .world()
                    .hit_test_candidates(document.runtime_document().document(), x, y)
            })
            .unwrap_or_default();
        scene_probe::hits(&self.accessibility_dump(), &candidates)
    }

    fn scene_probe(&self, node: u64) -> Option<SceneProbeDump> {
        let id = StableNodeId::new(node)?;
        let (scene, layout) = self.document_world(|document| {
            (document.scene().clone(), document.world().layout_box(id))
        })?;
        scene_probe::probe(
            id,
            &scene,
            layout,
            self.width as f32,
            self.height as f32,
            |x, y| self.hit_test(x, y),
        )
    }

    fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        Self::screenshot_rgba(self)
    }

    /// The semantic snapshot can carry a widget the accessibility projection
    /// does not, so `agent_id` is resolved there rather than over the dump.
    fn resolve_agent_id(&self, agent_id: &str) -> Option<u64> {
        self.host
            .semantic_snapshot()
            .widgets
            .iter()
            .find(|widget| widget.props.agent_id == agent_id)
            .map(|widget| widget.id)
    }
}

/// Platform button mask for a button index, matching the hosted adapter.
const fn button_mask(button: i16) -> u16 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        _ => 0,
    }
}

fn node_click_point(host: &VueHost, id: u64) -> Option<(f32, f32)> {
    let handle = NodeHandle(id);
    let document = host.document();
    let guard = document.lock().ok()?;
    // Runtime LayoutBox deliberately excludes scroll/paint transforms. Use
    // the same current projection as the painter when synthesizing a pointer.
    if let Some(bounds) = guard
        .scene()
        .draw_node_bounds(StableNodeId::try_from(handle).ok()?)
        && (bounds.width > 0.0 || bounds.height > 0.0)
    {
        return Some((
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.5,
        ));
    }
    let bounds = guard
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id.get() == id)?
        .bounds;
    if bounds.width <= 0.0 && bounds.height <= 0.0 {
        return None;
    }
    Some((
        bounds.x + bounds.width * 0.5,
        bounds.y + bounds.height * 0.5,
    ))
}

fn agent_ids_from_snapshot(snapshot: &SemanticSnapshot) -> BTreeMap<u64, String> {
    snapshot
        .widgets
        .iter()
        .filter(|widget| !widget.props.agent_id.is_empty())
        .map(|widget| (widget.id, widget.props.agent_id.clone()))
        .collect()
}

fn semantic_dump_from_snapshot(snapshot: &SemanticSnapshot) -> Vec<SemanticDumpWidget> {
    snapshot
        .widgets
        .iter()
        .map(|widget| SemanticDumpWidget {
            id: widget.id,
            kind: format!("{:?}", widget.kind),
            label: widget.props.label.clone(),
            agent_id: widget.props.agent_id.clone(),
        })
        .collect()
}
/// Semantic counter fixture used by tests and the stdio binary.
pub fn semantic_counter_source() -> &'static str {
    r#"
(function () {
  let count = 0;
  const host = globalThis.__nanaHost;
  const root = host.call("mountRoot", []);
  const col = host.call("createWidget", ["column", { style: "width:100%;height:100%;gap:8px;padding:12px;align-items:flex-start" }]);
  const title = host.call("createWidget", ["text", { label: "Agent session counter", style: "white-space:nowrap" }]);
  const text = host.call("createWidget", ["text", { label: "count = 0", "data-agent-id": "count", style: "white-space:nowrap" }]);
  const btn = host.call("createWidget", ["button", { label: "Increment", kind: "primary", "data-agent-id": "increment" }]);
  host.call("insert", [col, root, null]);
  host.call("insert", [title, col, null]);
  host.call("insert", [text, col, null]);
  host.call("insert", [btn, col, null]);
  host.call("patchProp", [btn, "onPress", true]);

  const listeners = new Map();
  function key(nid, event) { return Number(nid) + ":" + String(event).toLowerCase(); }
  function sync() {
    host.call("patchProp", [text, "label", "count = " + count]);
  }
  listeners.set(key(btn, "press"), function () { count += 1; sync(); });

  globalThis.__nanaFireEvent = function (nid, event, detail) {
    const fn = listeners.get(key(nid, event));
    if (typeof fn === "function") fn(detail || {});
    return true;
  };
  return { ok: true, app: "agent-counter", buttonId: btn, textId: text };
})();
"#
}

pub fn semantic_counter_artifact() -> RuntimeArtifact {
    RuntimeArtifact::from_source("agent-counter.js", semantic_counter_source())
}
// The Vue session can only be exercised with a real JS engine, so these tests
// live behind `agent-bin`. Keeping them off `agent` lets the Vue-free
// `runtime-agent` tier build and test without V8.
#[cfg(all(test, feature = "agent-bin"))]
mod tests {
    use super::*;
    use crate::agent::AgentCommand;
    use nana_js_v8::V8Engine;

    // --- Reload -------------------------------------------------------------
    //
    // The reload path is the one place where the host tears its own tree down
    // and rebuilds it under a live document. Everything asserted here is a
    // property a windowed session depends on but cannot check cheaply: that the
    // scaffold survives, that nothing accumulates across saves, and that node
    // identity keeps moving forward so the accessibility projection stays valid.

    /// An app that renders one labelled button, so a reload is visible as a
    /// label change rather than as a count.
    fn labelled_app(label: &str) -> String {
        format!(
            r#"
(function () {{
  const host = globalThis.__nanaHost;
  const root = host.call("mountRoot", []);
  const col = host.call("createWidget", ["column", {{ style: "width:100%;height:100%" }}]);
  const btn = host.call("createWidget", ["button", {{ label: "{label}", "data-agent-id": "only" }}]);
  host.call("insert", [col, root, null]);
  host.call("insert", [btn, col, null]);
  globalThis.__nanaFireEvent = function () {{ return true; }};
  return {{ ok: true }};
}})();
"#
        )
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("nana-ui-devtools-reload")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn write(path: &std::path::Path, contents: &str) {
        std::fs::write(path, contents).expect("write scratch file");
    }

    fn reload_session(source: &str) -> VueAgentSession<V8Engine> {
        VueAgentSession::new(
            V8Engine::new,
            RuntimeArtifact::from_source("reload-fixture.js", source),
            480,
            320,
        )
        .expect("session")
    }

    fn widget_ids(session: &VueAgentSession<V8Engine>) -> Vec<u64> {
        let mut ids: Vec<u64> = session.semantic_dump().into_iter().map(|w| w.id).collect();
        ids.sort_unstable();
        ids
    }

    fn only_label(session: &VueAgentSession<V8Engine>) -> String {
        session
            .semantic_dump()
            .into_iter()
            .find(|widget| widget.agent_id == "only")
            .map(|widget| widget.label)
            .unwrap_or_default()
    }

    fn world_generation(session: &VueAgentSession<V8Engine>) -> u64 {
        session
            .host()
            .document()
            .lock()
            .expect("document")
            .world()
            .generation()
    }

    #[test]
    fn a_reload_swaps_the_tree_and_keeps_the_document_scaffold() {
        let dir = scratch_dir("swap");
        let app = dir.join("app.js");
        write(&app, &labelled_app("Before"));
        let mut session = reload_session(&labelled_app("Before"));
        session.flush().expect("flush");

        let (html, body) = {
            let document = session.host().document();
            let guard = document.lock().expect("document");
            (guard.html_root(), guard.mount_root())
        };
        // The scaffold survives by design and keeps its ids, so it is excluded
        // from the identity comparison below; everything else is app-created.
        let scaffold = [html.0, body.0];
        let app_ids = |session: &VueAgentSession<V8Engine>| -> Vec<u64> {
            widget_ids(session)
                .into_iter()
                .filter(|id| !scaffold.contains(id))
                .collect()
        };
        let before_ids = app_ids(&session);
        assert!(!before_ids.is_empty(), "the fixture must create widgets");
        assert_eq!(only_label(&session), "Before");

        write(&app, &labelled_app("After"));
        session.reload_artifact(&app).expect("reload");
        session.flush().expect("flush");

        assert_eq!(only_label(&session), "After");
        assert_eq!(
            session
                .semantic_dump()
                .iter()
                .filter(|w| w.agent_id == "only")
                .count(),
            1,
            "the old tree must be gone, not layered under the new one"
        );

        let document = session.host().document();
        let guard = document.lock().expect("document");
        assert_eq!(guard.html_root(), html, "the html scaffold must survive");
        assert_eq!(guard.mount_root(), body, "the body scaffold must survive");
        drop(guard);

        // Node ids are retired permanently, never recycled. Anything the host
        // caches by node id therefore cannot alias a reloaded node -- which is
        // exactly why the tree is rebuilt in place instead of swapping in a
        // fresh `UiWorld`.
        let after_ids = app_ids(&session);
        let highest_before = before_ids.iter().copied().max().expect("ids before");
        assert!(
            after_ids.iter().all(|id| *id > highest_before),
            "reloaded ids {after_ids:?} must all exceed {highest_before}"
        );
    }

    #[test]
    fn repeated_reloads_do_not_accumulate_widgets_or_stylesheets() {
        // A missed reset is invisible for the first few saves and unmistakable
        // after twenty. Five is enough to catch any per-reload growth.
        let dir = scratch_dir("accumulate");
        let app = dir.join("app.js");
        let with_sheet = |label: &str| {
            format!(
                r#"
(function () {{
  const host = globalThis.__nanaHost;
  host.call("injectStylesheet", [".only {{ width: 40px; }}", "app.css"]);
  const root = host.call("mountRoot", []);
  const btn = host.call("createWidget", ["button", {{ label: "{label}", class: "only", "data-agent-id": "only" }}]);
  host.call("insert", [btn, root, null]);
  globalThis.__nanaFireEvent = function () {{ return true; }};
  return {{ ok: true }};
}})();
"#
            )
        };
        let mut session = reload_session(&with_sheet("0"));
        session.flush().expect("flush");
        let baseline_widgets = session.semantic_dump().len();
        let baseline_sheets = session
            .host()
            .document()
            .lock()
            .expect("document")
            .stylesheet_count();

        for round in 1..=5 {
            write(&app, &with_sheet(&round.to_string()));
            session.reload_artifact(&app).expect("reload");
            session.flush().expect("flush");
            assert_eq!(
                session.semantic_dump().len(),
                baseline_widgets,
                "widget count grew on reload {round}"
            );
            assert_eq!(
                session
                    .host()
                    .document()
                    .lock()
                    .expect("document")
                    .stylesheet_count(),
                baseline_sheets,
                "stylesheet count grew on reload {round}"
            );
        }
        assert_eq!(only_label(&session), "5");
    }

    #[test]
    fn a_reload_keeps_the_world_generation_moving_forward() {
        // `AccessibilityProjector` rejects any update whose generation does not
        // exceed the last one it applied, and it is built once per window and
        // never rebuilt. A reload that reset the generation would leave screen
        // readers announcing the pre-reload tree for the life of the process.
        let dir = scratch_dir("generation");
        let app = dir.join("app.js");
        let mut session = reload_session(&labelled_app("Before"));
        session.flush().expect("flush");
        let before = world_generation(&session);

        write(&app, &labelled_app("After"));
        session.reload_artifact(&app).expect("reload");
        session.flush().expect("flush");

        assert!(
            world_generation(&session) > before,
            "generation must rise across a reload, went {before} -> {}",
            world_generation(&session)
        );
    }

    #[test]
    fn a_reload_drops_focus_held_by_the_tree_it_replaced() {
        let dir = scratch_dir("focus");
        let app = dir.join("app.js");
        let field = r#"
(function () {
  const host = globalThis.__nanaHost;
  const root = host.call("mountRoot", []);
  const input = host.call("createWidget", ["input", { value: "typed", "data-agent-id": "field" }]);
  host.call("insert", [input, root, null]);
  globalThis.__nanaFireEvent = function () { return true; };
  return { ok: true };
})();
"#;
        let mut session = reload_session(field);
        session.flush().expect("flush");
        let node = session
            .accessibility_dump()
            .into_iter()
            .find(|n| n.agent_id.as_deref() == Some("field"))
            .expect("field projects")
            .id;
        session.activate(node).expect("activate");
        session.flush().expect("flush");

        write(&app, &labelled_app("After"));
        session.reload_artifact(&app).expect("reload");
        session.flush().expect("flush");

        let document = session.host().document();
        let focused = document.lock().expect("document").focused();
        assert!(
            focused.is_none(),
            "focus survived on a node that no longer exists: {focused:?}"
        );
    }

    #[test]
    fn a_failed_reload_puts_the_previous_build_back() {
        // A blank window plus a syntax error that points at nothing the
        // developer wrote is the worst outcome a dev loop can produce, and the
        // easiest one to ship by accident.
        let dir = scratch_dir("broken");
        let app = dir.join("app.js");
        let mut session = reload_session(&labelled_app("Working"));
        session.flush().expect("flush");

        write(&app, "this is not ( valid javascript");
        let error = session.reload_artifact(&app).expect_err("broken source");
        session.flush().expect("flush");

        assert_eq!(
            only_label(&session),
            "Working",
            "the working build must still be on screen after a failed reload"
        );
        assert!(!error.0.is_empty(), "the failure must be reported");
    }

    #[test]
    fn an_empty_file_is_refused_rather_than_evaluated() {
        let dir = scratch_dir("truncated");
        let app = dir.join("app.js");
        let mut session = reload_session(&labelled_app("Working"));
        session.flush().expect("flush");

        write(&app, "");
        let error = session.reload_artifact(&app).expect_err("empty file");
        assert!(
            error.0.contains("empty"),
            "a truncated save must say so, got {error:?}"
        );
        assert_eq!(only_label(&session), "Working");
    }

    #[test]
    fn a_stylesheet_reload_restyles_without_touching_the_tree() {
        let dir = scratch_dir("css");
        let sheet = dir.join("app.css");
        let styled = r#"
(function () {
  const host = globalThis.__nanaHost;
  host.call("injectStylesheet", [".only { width: 40px; }", "app.css"]);
  const root = host.call("mountRoot", []);
  const btn = host.call("createWidget", ["button", { label: "Styled", class: "only", "data-agent-id": "only" }]);
  host.call("insert", [btn, root, null]);
  globalThis.__nanaFireEvent = function () { return true; };
  return { ok: true };
})();
"#;
        let mut session = reload_session(styled);
        session.flush().expect("flush");
        let before_ids = widget_ids(&session);
        let generation = world_generation(&session);

        write(&sheet, ".only { width: 90px; }");
        session
            .reload_stylesheet("app.css", &sheet)
            .expect("stylesheet reload");
        session.flush().expect("flush");

        assert_eq!(
            widget_ids(&session),
            before_ids,
            "a stylesheet swap must not create or destroy a node"
        );
        assert_eq!(only_label(&session), "Styled");
        assert!(
            world_generation(&session) >= generation,
            "the generation must not go backwards"
        );
        assert_eq!(
            session
                .host()
                .document()
                .lock()
                .expect("document")
                .stylesheet_count(),
            1,
            "a keyed swap must replace the sheet, not stack another copy"
        );
    }

    #[test]
    fn the_reload_command_reports_what_it_cannot_do() {
        let mut session = reload_session(&labelled_app("Working"));
        let reply = session.dispatch(AgentCommand::Reload {
            js: None,
            css: None,
            css_key: None,
        });
        assert!(!reply.ok, "an empty reload must be refused: {reply:?}");

        let missing = session.dispatch(AgentCommand::Reload {
            js: Some("/definitely/not/here.js".into()),
            css: None,
            css_key: None,
        });
        assert!(!missing.ok, "a missing artifact must be refused");
        assert_eq!(only_label(&session), "Working");
    }

    fn count_label(session: &VueAgentSession<V8Engine>) -> String {
        session
            .semantic_dump()
            .into_iter()
            .find(|widget| widget.agent_id == "count")
            .map(|widget| widget.label)
            .unwrap_or_default()
    }

    /// The counter as the Runtime projects it, which is what a screen reader
    /// reads. [`count_label`] reads the JS-side bridge props instead, so the two
    /// together separate a dropped press from a stale projection.
    fn count_a11y_label(session: &VueAgentSession<V8Engine>) -> String {
        session
            .accessibility_dump()
            .into_iter()
            .find(|node| node.agent_id.as_deref() == Some("count"))
            .and_then(|node| node.label)
            .unwrap_or_default()
    }

    #[test]
    fn a_widget_keeps_its_projected_geometry_across_a_bare_pump() {
        let mut session =
            VueAgentSession::new(V8Engine::new, semantic_counter_artifact(), 480, 320)
                .expect("session");
        let button = session
            .accessibility_dump()
            .into_iter()
            .find(|node| node.agent_id.as_deref() == Some("increment"))
            .expect("increment in a11y dump");
        let handle = NodeHandle(button.id);
        let projected = {
            let document = session.host().document();
            let guard = document.lock().expect("doc");
            guard.layout_box(handle).expect("projected button box")
        };

        // A pump runs the CSS cascade writeback without a semantic sync. The
        // button's padding and min-height come from its Runtime component, so
        // the cascade must leave them alone; otherwise the box collapses to the
        // bare text and the pointer falls through to the column behind it.
        session.host_mut().resolve_layout();

        let document = session.host().document();
        let guard = document.lock().expect("doc");
        assert_eq!(
            guard.layout_box(handle),
            Some(projected),
            "cascade writeback must not overwrite component-projected geometry"
        );
    }

    /// `createWidget` seeds the label as an attribute only. A `#text` child
    /// would announce a second copy that `patchProp` never refreshes, so every
    /// label the mounted app exposes must belong to exactly one a11y node.
    #[test]
    fn mounted_widgets_announce_each_label_once() {
        let mut session =
            VueAgentSession::new(V8Engine::new, semantic_counter_artifact(), 480, 320)
                .expect("session");
        session.click_agent_id("increment").expect("click");

        let labels: Vec<_> = session
            .accessibility_dump()
            .into_iter()
            .filter_map(|node| node.label)
            .collect();
        let mut unique = labels.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            labels.len(),
            unique.len(),
            "each label must come from one retained node, got {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "count = 1"),
            "the counter label must track the click, got {labels:?}"
        );
    }

    #[test]
    fn repeated_clicks_advance_the_counter_in_both_projections() {
        let mut session =
            VueAgentSession::new(V8Engine::new, semantic_counter_artifact(), 480, 320)
                .expect("session");
        assert_eq!(count_label(&session), "count = 0");
        assert_eq!(count_a11y_label(&session), "count = 0");

        for expected in 1..=3 {
            session.click_agent_id("increment").expect("click");
            let expected = format!("count = {expected}");
            assert_eq!(
                count_label(&session),
                expected,
                "bridge props must record every press"
            );
            assert_eq!(
                count_a11y_label(&session),
                expected,
                "a11y projection must not lag the press it already handled"
            );
        }
    }

    #[test]
    fn vue_session_click_updates_semantic_and_a11y() {
        let mut session =
            VueAgentSession::new(V8Engine::new, semantic_counter_artifact(), 480, 320)
                .expect("session");
        assert_eq!(count_label(&session), "count = 0");
        let increment = session
            .accessibility_dump()
            .into_iter()
            .find(|node| node.agent_id.as_deref() == Some("increment"))
            .expect("increment in a11y dump");
        assert!(
            increment.bounds.width > 8.0 && increment.bounds.height > 8.0,
            "headless layout must size the increment button, got {:?}",
            increment.bounds
        );
        let handled = session
            .click_xy(
                increment.bounds.x + increment.bounds.width * 0.5,
                increment.bounds.y + increment.bounds.height * 0.5,
            )
            .expect("click");
        assert!(handled);
        assert_eq!(count_label(&session), "count = 1");
        assert!(
            session
                .semantic_dump()
                .iter()
                .any(|widget| widget.agent_id == "increment"),
            "increment agent_id remains after click"
        );
    }

    #[test]
    fn vue_session_screenshot_matches_semantic_after_click() {
        if offscreen::optional().is_none() {
            return;
        }
        let mut session =
            VueAgentSession::new(V8Engine::new, semantic_counter_artifact(), 240, 160)
                .expect("session");
        session.click_agent_id("increment").expect("click");
        let (size, pixels) = session.screenshot_rgba().expect("screenshot");
        assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
        let unique = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| u32::from_be_bytes([pixel[0], pixel[1], pixel[2], 0]))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            unique.len() > 8,
            "offscreen preview must paint UI chrome, not only a clear color ({})",
            unique.len()
        );
        assert_eq!(count_label(&session), "count = 1");
    }
    /// The wire form the Python drivers and the skill already use must keep
    /// parsing after the selector moved into a shared `Target`.
    #[test]
    fn the_existing_click_wire_form_still_parses() {
        let click =
            serde_json::from_str::<AgentCommand>(r#"{"cmd":"click","agent_id":"increment"}"#)
                .expect("parse");
        let AgentCommand::Click { target, button } = click else {
            panic!("expected a click");
        };
        assert_eq!(target.agent_id.as_deref(), Some("increment"));
        assert_eq!(button, None);

        let xy = serde_json::from_str::<AgentCommand>(r#"{"cmd":"click","x":10,"y":20}"#)
            .expect("parse");
        let AgentCommand::Click { target, .. } = xy else {
            panic!("expected a click");
        };
        assert_eq!(target.point(), Some((10.0, 20.0)));
    }
}
