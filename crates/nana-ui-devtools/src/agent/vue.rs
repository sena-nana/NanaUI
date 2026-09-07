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
}

impl<E: JsEngine> VueAgentSession<E> {
    pub fn new(
        engine: E,
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
        mut engine: E,
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
        let diagnostics = DevtoolsSession::default();
        let mut host = VueHost::with_viewport(width, height, scale_factor);
        // `JsEngine` exposes no diagnostics hook, so this captures Vue
        // warnings/errors and resource lifecycle. An engine-level exception
        // surfaces as the failing command's own error instead.
        host.set_diagnostics(Some(diagnostics.js_sink()), None);
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
        };
        session.pump()?;
        Ok(session)
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
            .flush_scene_frame(self.width as f32, self.height as f32)
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
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 480, 320)
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
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 480, 320)
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
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 480, 320)
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
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 480, 320)
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
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 240, 160)
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
