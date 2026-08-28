//! Headless Agent session: offscreen pixels plus in-process pointer/keyboard.
//!
//! CPU readback stays here. Product `run_runtime` present never uses this path.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

use nana_js_engine::{JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui::runtime::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityNode, AccessibilityRole,
    LayoutViewport, RuntimeDocument, StableNodeId,
};
use nana_ui::{NanaTextShaper, RuntimeInputAdapter};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use nana_ui_vue::{BridgeEvent, NodeHandle, SemanticSnapshot, VueHost};
use serde::{Deserialize, Serialize};

use crate::offscreen::{self, OffscreenSnapshots, Size};

const DEFAULT_CLEAR: [f32; 4] = [0.96, 0.96, 0.96, 1.0];

#[derive(Debug)]
pub struct AgentError(pub String);

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AgentError {}

impl From<JsEngineError> for AgentError {
    fn from(error: JsEngineError) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for AgentError {
    fn from(error: String) -> Self {
        Self(error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum AgentCommand {
    Screenshot {
        path: String,
    },
    A11y,
    Semantic,
    Click {
        #[serde(default)]
        x: Option<f32>,
        #[serde(default)]
        y: Option<f32>,
        #[serde(default)]
        node: Option<u64>,
        #[serde(default)]
        agent_id: Option<String>,
    },
    Type {
        text: String,
    },
    Pump,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<AccessibilityDumpNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widgets: Option<Vec<SemanticDumpWidget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handled: Option<bool>,
}

impl AgentReply {
    fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            path: None,
            nodes: None,
            widgets: None,
            handled: None,
        }
    }

    fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            path: None,
            nodes: None,
            widgets: None,
            handled: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoundsDump {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessibilityDumpNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub focused: bool,
    pub disabled: bool,
    pub bounds: BoundsDump,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticDumpWidget {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub agent_id: String,
}

/// Vue document driven without a winit window.
pub struct VueAgentSession<E: JsEngine> {
    host: VueHost,
    engine: E,
    gpu: Option<OffscreenSnapshots>,
    width: u32,
    height: u32,
    clear: [f32; 4],
}

impl<E: JsEngine> VueAgentSession<E> {
    pub fn new(
        mut engine: E,
        artifact: RuntimeArtifact,
        width: u32,
        height: u32,
    ) -> Result<Self, AgentError> {
        let mut host = VueHost::with_viewport(width, height, 1.0);
        host.attach_engine(&mut engine)?;
        engine.initialize(artifact)?;
        host.bind_event_bridge(&mut engine)?;
        let mut session = Self {
            host,
            engine,
            gpu: None,
            width,
            height,
            clear: DEFAULT_CLEAR,
        };
        session.pump()?;
        Ok(session)
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
        let size = Size::new(self.width, self.height);
        let clear = self.clear;
        let scene = {
            let document = self.host.document();
            let guard = document
                .lock()
                .map_err(|_| AgentError("vue document poisoned".into()))?;
            guard.scene().clone()
        };
        let gpu = self.gpu_mut()?;
        let pixels = gpu
            .paint(&scene, size, clear, None, None)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok((size, pixels))
    }

    pub fn screenshot_png(&mut self, path: impl AsRef<Path>) -> Result<(), AgentError> {
        let (size, pixels) = self.screenshot_rgba()?;
        offscreen::write_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    pub fn execute(&mut self, command: AgentCommand) -> AgentReply {
        match command {
            AgentCommand::Screenshot { path } => match self.screenshot_png(&path) {
                Ok(()) => {
                    let mut reply = AgentReply::ok();
                    reply.path = Some(path);
                    reply
                }
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::A11y => {
                let mut reply = AgentReply::ok();
                reply.nodes = Some(self.accessibility_dump());
                reply
            }
            AgentCommand::Semantic => {
                let mut reply = AgentReply::ok();
                reply.widgets = Some(self.semantic_dump());
                reply
            }
            AgentCommand::Click {
                x,
                y,
                node,
                agent_id,
            } => {
                let result = if let Some(agent_id) = agent_id {
                    self.click_agent_id(&agent_id)
                } else if let Some(node) = node {
                    self.click_node(node)
                } else if let (Some(x), Some(y)) = (x, y) {
                    self.click_xy(x, y)
                } else {
                    return AgentReply::err("click requires x/y, node, or agent_id");
                };
                match result {
                    Ok(handled) => {
                        let mut reply = AgentReply::ok();
                        reply.handled = Some(handled);
                        reply
                    }
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::Type { text } => match self.type_text(&text) {
                Ok(()) => AgentReply::ok(),
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::Pump => match self.pump() {
                Ok(()) => AgentReply::ok(),
                Err(error) => AgentReply::err(error.0),
            },
        }
    }

    pub fn run_stdio(
        &mut self,
        input: impl BufRead,
        mut output: impl Write,
    ) -> Result<(), AgentError> {
        for line in input.lines() {
            let line = line.map_err(|error| AgentError(error.to_string()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let command: AgentCommand =
                serde_json::from_str(trimmed).map_err(|error| AgentError(error.to_string()))?;
            let reply = self.execute(command);
            writeln!(
                output,
                "{}",
                serde_json::to_string(&reply).map_err(|error| AgentError(error.to_string()))?
            )
            .map_err(|error| AgentError(error.to_string()))?;
        }
        Ok(())
    }

    fn gpu_mut(&mut self) -> Result<&mut OffscreenSnapshots, AgentError> {
        if self.gpu.is_none() {
            self.gpu =
                Some(OffscreenSnapshots::new().map_err(|error| AgentError(error.to_string()))?);
        }
        Ok(self.gpu.as_mut().expect("gpu initialized"))
    }
}

/// L3 Runtime document driven without a winit window.
pub struct RuntimeAgentSession {
    document: RuntimeDocument,
    shaper: NanaTextShaper,
    gpu: Option<OffscreenSnapshots>,
    width: u32,
    height: u32,
    clear: [f32; 4],
}

impl RuntimeAgentSession {
    pub fn new(document: RuntimeDocument, width: u32, height: u32) -> Result<Self, AgentError> {
        let mut session = Self {
            document,
            shaper: NanaTextShaper::default(),
            gpu: None,
            width,
            height,
            clear: DEFAULT_CLEAR,
        };
        session.flush()?;
        Ok(session)
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    pub fn flush(&mut self) -> Result<(), AgentError> {
        self.document
            .flush(
                LayoutViewport::new(self.width as f32, self.height as f32),
                &mut self.shaper,
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    pub fn accessibility_dump(&self) -> Vec<AccessibilityDumpNode> {
        self.document
            .context()
            .world()
            .project_accessibility(self.document.document())
            .into_iter()
            .map(|node| dump_accessibility_node(node, &BTreeMap::new()))
            .collect()
    }

    pub fn click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        dispatch_runtime_pointer(&mut self.document, PointerPhase::Down, x, y)?;
        dispatch_runtime_pointer(&mut self.document, PointerPhase::Up, x, y)?;
        self.flush()?;
        Ok(true)
    }

    pub fn click_node(&mut self, id: u64) -> Result<bool, AgentError> {
        let target =
            StableNodeId::new(id).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        let document_id = self.document.document();
        let handled = self
            .document
            .context_mut()
            .apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::Click,
                },
            )
            .map_err(|error| AgentError(error.to_string()))?;
        self.flush()?;
        Ok(handled)
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), AgentError> {
        let document_id = self.document.document();
        for character in text.chars() {
            let key = character.to_string();
            RuntimeInputAdapter::default()
                .dispatch(
                    self.document.context_mut(),
                    document_id,
                    &InputEvent::Keyboard {
                        pressed: true,
                        key: key.clone(),
                        text: Some(key.clone()),
                        code: "Unidentified".into(),
                        repeat: false,
                        modifiers: InputModifiers::default(),
                    },
                )
                .map_err(|error| AgentError(error.to_string()))?;
        }
        self.flush()?;
        Ok(())
    }

    pub fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        self.flush()?;
        let size = Size::new(self.width, self.height);
        let clear = self.clear;
        let scene = self.document.scene().clone();
        let gpu = self.gpu_mut()?;
        let pixels = gpu
            .paint(&scene, size, clear, None, None)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok((size, pixels))
    }

    pub fn screenshot_png(&mut self, path: impl AsRef<Path>) -> Result<(), AgentError> {
        let (size, pixels) = self.screenshot_rgba()?;
        offscreen::write_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    fn gpu_mut(&mut self) -> Result<&mut OffscreenSnapshots, AgentError> {
        if self.gpu.is_none() {
            self.gpu =
                Some(OffscreenSnapshots::new().map_err(|error| AgentError(error.to_string()))?);
        }
        Ok(self.gpu.as_mut().expect("gpu initialized"))
    }
}

fn dispatch_runtime_pointer(
    document: &mut RuntimeDocument,
    phase: PointerPhase,
    x: f32,
    y: f32,
) -> Result<(), AgentError> {
    let document_id = document.document();
    RuntimeInputAdapter::default()
        .dispatch(
            document.context_mut(),
            document_id,
            &InputEvent::Pointer {
                phase,
                pointer_id: 1,
                pointer_type: PointerType::Mouse,
                x,
                y,
                screen_x: x,
                screen_y: y,
                button: 0,
                buttons: 0,
                pressure: 0.5,
                tangential_pressure: 0.0,
                tilt_x: 0,
                tilt_y: 0,
                twist: 0,
                is_primary: true,
                modifiers: InputModifiers::default(),
            },
        )
        .map_err(|error| AgentError(error.to_string()))?;
    Ok(())
}

fn node_click_point(host: &VueHost, id: u64) -> Option<(f32, f32)> {
    let handle = NodeHandle(id);
    let document = host.document();
    let guard = document.lock().ok()?;
    if let Some(bounds) = guard.layout_box(handle)
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

fn dump_accessibility_node(
    node: AccessibilityNode,
    agent_ids: &BTreeMap<u64, String>,
) -> AccessibilityDumpNode {
    let id = node.id.get();
    AccessibilityDumpNode {
        id,
        parent: node.parent.map(StableNodeId::get),
        children: node.children.into_iter().map(StableNodeId::get).collect(),
        role: accessibility_role_name(node.role).into(),
        label: node.label.map(|value| value.to_string()),
        value: node.value.map(|value| value.to_string()),
        focused: node.focused,
        disabled: node.disabled,
        bounds: BoundsDump {
            x: node.bounds.x,
            y: node.bounds.y,
            width: node.bounds.width,
            height: node.bounds.height,
        },
        agent_id: agent_ids.get(&id).cloned(),
    }
}

fn accessibility_role_name(role: AccessibilityRole) -> &'static str {
    match role {
        AccessibilityRole::Document => "document",
        AccessibilityRole::Text => "text",
        AccessibilityRole::Button => "button",
        AccessibilityRole::TextInput => "text-input",
        AccessibilityRole::Checkbox => "checkbox",
        AccessibilityRole::Switch => "switch",
        AccessibilityRole::Slider => "slider",
        AccessibilityRole::ComboBox => "combo-box",
        AccessibilityRole::ProgressIndicator => "progress",
        AccessibilityRole::List => "list",
        AccessibilityRole::ListItem => "list-item",
        AccessibilityRole::Table => "table",
        AccessibilityRole::Row => "row",
        AccessibilityRole::Cell => "cell",
        AccessibilityRole::ColumnHeader => "column-header",
        AccessibilityRole::TabList => "tab-list",
        AccessibilityRole::Tab => "tab",
        AccessibilityRole::RadioGroup => "radio-group",
        AccessibilityRole::Radio => "radio",
        AccessibilityRole::Separator => "separator",
        AccessibilityRole::Dialog => "dialog",
        AccessibilityRole::AlertDialog => "alert-dialog",
        AccessibilityRole::Menu => "menu",
        AccessibilityRole::MenuItem => "menu-item",
        AccessibilityRole::Tooltip => "tooltip",
        AccessibilityRole::Status => "status",
        AccessibilityRole::Image => "image",
        AccessibilityRole::Generic => "generic",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_v8::V8Engine;
    use nana_ui::runtime::{Button, DocumentId, List, Text};

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

    /// GPU-less environments skip pixel evidence, and the skip must stay observable.
    fn offscreen_gpu() -> Option<OffscreenSnapshots> {
        match OffscreenSnapshots::new() {
            Ok(gpu) => Some(gpu),
            Err(error) => {
                eprintln!("skipping offscreen GPU evidence: {error}");
                None
            }
        }
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
        if offscreen_gpu().is_none() {
            return;
        }
        let mut session =
            VueAgentSession::new(V8Engine::new(), semantic_counter_artifact(), 240, 160)
                .expect("session");
        session.click_agent_id("increment").expect("click");
        let (size, pixels) = session.screenshot_rgba().expect("screenshot");
        assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
        let unique = pixels
            .chunks_exact(4)
            .map(|pixel| u32::from_be_bytes([pixel[0], pixel[1], pixel[2], 0]))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            unique.len() > 8,
            "offscreen preview must paint UI chrome, not only a clear color ({})",
            unique.len()
        );
        assert_eq!(count_label(&session), "count = 1");
    }

    #[test]
    fn runtime_session_click_node_and_optional_preview() {
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let root = document
            .context_mut()
            .create_component(document_id, List::new().label("Agent"))
            .expect("root");
        let label = document
            .context_mut()
            .create_component(document_id, Text::new("idle"))
            .expect("label");
        let button = document
            .context_mut()
            .create_component(document_id, Button::new("Go"))
            .expect("button");
        document
            .context_mut()
            .append_child(root, label)
            .expect("append label");
        document
            .context_mut()
            .append_child(root, button)
            .expect("append button");
        let mut session = RuntimeAgentSession::new(document, 240, 160).expect("runtime session");
        assert!(
            session
                .accessibility_dump()
                .iter()
                .any(|node| node.role == "button")
        );
        let handled = session.click_node(button.stable_id().get()).expect("click");
        assert!(handled);
        if offscreen_gpu().is_some() {
            let (size, pixels) = session.screenshot_rgba().expect("preview");
            assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
            assert!(pixels.iter().any(|channel| *channel != 0));
        }
    }

    /// Real GPU evidence that `CustomRenderNode::params` reaches the painter:
    /// changing only a `GpuView` palette must change the painted pixels.
    #[test]
    fn gpu_view_palette_params_reach_the_default_painter() {
        use nana_ui::default_scene_gpu_renderers;
        use nana_ui::runtime::{GpuView, GpuViewPalette};

        const WIDTH: u32 = 96;
        const HEIGHT: u32 = 96;

        let Some(mut gpu) = offscreen_gpu() else {
            return;
        };
        let renderers = default_scene_gpu_renderers();
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let view = document
            .context_mut()
            .create_component(
                document_id,
                GpuView::new(1).palette(GpuViewPalette {
                    background: [1.0, 0.0, 0.0, 1.0],
                    accent: [1.0, 0.0, 0.0, 1.0],
                }),
            )
            .expect("gpu view");
        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");

        // `readback` returns RGBA; sum each channel over the whole frame.
        let mut paint = |session: &mut RuntimeAgentSession| {
            session.flush().expect("flush");
            let scene = session.document().scene().clone();
            let pixels = gpu
                .paint(
                    &scene,
                    Size::new(WIDTH, HEIGHT),
                    [0.0, 0.0, 0.0, 1.0],
                    None,
                    Some(&renderers),
                )
                .expect("offscreen paint with the gpu-view renderer");
            pixels.chunks_exact(4).fold([0u64; 3], |mut acc, rgba| {
                acc[0] += u64::from(rgba[0]);
                acc[1] += u64::from(rgba[1]);
                acc[2] += u64::from(rgba[2]);
                acc
            })
        };

        let red = paint(&mut session);
        assert!(
            red[0] > red[1] && red[0] > red[2],
            "the red palette must paint red-dominant, got {red:?}"
        );

        session
            .document_mut()
            .context_mut()
            .update_component(view, |view, _| {
                view.palette = GpuViewPalette {
                    background: [0.0, 1.0, 0.0, 1.0],
                    accent: [0.0, 1.0, 0.0, 1.0],
                };
                view.invalidate_content();
            })
            .expect("recolor");
        let green = paint(&mut session);
        assert!(
            green[1] > green[0] && green[1] > green[2],
            "the recolored palette must reach the painter, got {green:?}"
        );
    }

    /// Real GPU evidence that resident scrollbar chrome lands on the scrollport
    /// edge: the right-hand columns must brighten once the bar is drawn.
    #[test]
    fn resident_scrollbar_paints_pixels_on_the_scrollport_edge() {
        use nana_ui::runtime::{LengthSpec, NodeStyle, ScrollAxes, ScrollView, Text};
        use nana_ui_core::ScrollbarVisibility;

        const WIDTH: u32 = 160;
        const HEIGHT: u32 = 120;

        let Some(mut gpu) = offscreen_gpu() else {
            return;
        };
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let mut viewport = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut viewport.layout);
            layout.width = Some(LengthSpec::Px(WIDTH as f32));
            layout.height = Some(LengthSpec::Px(HEIGHT as f32));
        }
        let scroll = document
            .context_mut()
            .create_component(
                document_id,
                ScrollView::new(ScrollAxes::Vertical)
                    .scrollbars(ScrollbarVisibility::Always)
                    .style(viewport),
            )
            .expect("scroll view");
        for index in 0..8 {
            let mut row = NodeStyle::default();
            {
                let layout = std::sync::Arc::make_mut(&mut row.layout);
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(40.0));
            }
            let row = document
                .context_mut()
                .create_component(document_id, Text::new(format!("Row {index}")).style(row))
                .expect("row");
            document.context_mut().append_child(scroll, row).unwrap();
        }
        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");

        // Brightness of the rightmost track-thick band versus the same band on
        // the opposite edge, which never carries chrome.
        let mut edges = |session: &mut RuntimeAgentSession| {
            session.flush().expect("flush");
            let scene = session.document().scene().clone();
            let pixels = gpu
                .paint(
                    &scene,
                    Size::new(WIDTH, HEIGHT),
                    [0.0, 0.0, 0.0, 1.0],
                    None,
                    None,
                )
                .expect("offscreen paint");
            let band = nana_ui_core::SCROLLBAR_METRICS.thickness as u32;
            let mut right = 0u64;
            let mut left = 0u64;
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let offset = ((y * WIDTH + x) * 4) as usize;
                    let luma = u64::from(pixels[offset])
                        + u64::from(pixels[offset + 1])
                        + u64::from(pixels[offset + 2]);
                    if x >= WIDTH - band {
                        right += luma;
                    } else if x < band {
                        left += luma;
                    }
                }
            }
            (left, right)
        };

        let (left, right) = edges(&mut session);
        assert!(
            right > left,
            "the resident bar must brighten the right edge: left {left}, right {right}"
        );

        session
            .document_mut()
            .context_mut()
            .update_component(scroll, |scroll, _| {
                scroll.scrollbars = ScrollbarVisibility::Hidden;
            })
            .expect("hide bars");
        let (_, hidden_right) = edges(&mut session);
        assert!(
            hidden_right < right,
            "hiding the bar must remove those pixels: {hidden_right} vs {right}"
        );
    }

    #[test]
    fn divider_and_radio_selection_reach_pixels() {
        use nana_ui::runtime::{
            Card, Divider, LengthSpec, NodeStyle, SegmentedControl, SegmentedOption,
        };
        use nana_ui_core::FlexDirection;

        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 360;

        if offscreen_gpu().is_none() {
            return;
        }
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let mut column = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut column.layout);
            layout.width = Some(LengthSpec::Px(WIDTH as f32));
            layout.height = Some(LengthSpec::Px(HEIGHT as f32));
            layout.direction = Some(FlexDirection::Column);
            layout.gap = Some(LengthSpec::Px(12.0));
            layout.padding = Some(LengthSpec::Px(16.0));
        }
        let root = document
            .context_mut()
            .create_component(document_id, Card::new().style(column))
            .expect("root");
        let radios = document
            .context_mut()
            .create_component(document_id, SegmentedControl::radio_group())
            .expect("radios");
        let first = document
            .context_mut()
            .create_component(document_id, SegmentedOption::new("Automatic"))
            .expect("first");
        let second = document
            .context_mut()
            .create_component(document_id, SegmentedOption::new("Manual"))
            .expect("second");
        let divider = document
            .context_mut()
            .create_component(document_id, Divider::horizontal())
            .expect("divider");
        document.context_mut().append_child(root, radios).unwrap();
        document.context_mut().append_child(radios, first).unwrap();
        document.context_mut().append_child(radios, second).unwrap();
        document.context_mut().append_child(root, divider).unwrap();
        document
            .context_mut()
            .set_segmented_options(radios, vec![first, second], Some(second))
            .expect("select");

        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");
        let boxes = |session: &RuntimeAgentSession, id| {
            session
                .document()
                .context()
                .world()
                .layout_box(id)
                .expect("layout")
        };
        let rule = boxes(&session, divider.stable_id());
        let unselected = boxes(&session, first.stable_id());
        let selected = boxes(&session, second.stable_id());
        let (_, pixels) = session.screenshot_rgba().expect("pixels");
        let luma = |x: f32, y: f32| {
            let offset = ((y.round() as u32 * WIDTH + x.round() as u32) * 4) as usize;
            u32::from(pixels[offset])
                + u32::from(pixels[offset + 1])
                + u32::from(pixels[offset + 2])
        };

        let row = luma(rule.x + rule.width * 0.5, rule.y);
        let above = luma(rule.x + rule.width * 0.5, rule.y - 4.0);
        assert!(
            row > above,
            "the hairline rule must paint its own row: {row} vs {above}"
        );

        // The ring sits a fixed inset in from the option's leading edge; only
        // the selected one carries a filled dot at its center.
        let dot_x = |option: nana_ui::runtime::LayoutBox| {
            option.x
                + nana_ui_core::RADIO_ROW_INSET
                + nana_ui_core::ControlSize::Medium.indicator_size() / 2.0
        };
        let selected_dot = luma(dot_x(selected), selected.y + selected.height * 0.5);
        let empty_ring = luma(dot_x(unselected), unselected.y + unselected.height * 0.5);
        assert!(
            selected_dot > empty_ring,
            "only the selected radio fills its ring: {selected_dot} vs {empty_ring}"
        );
    }

    #[test]
    fn command_json_roundtrip() {
        let click =
            serde_json::from_str::<AgentCommand>(r#"{"cmd":"click","agent_id":"increment"}"#)
                .expect("parse");
        assert!(matches!(
            click,
            AgentCommand::Click {
                agent_id: Some(ref id),
                ..
            } if id == "increment"
        ));
    }
}
