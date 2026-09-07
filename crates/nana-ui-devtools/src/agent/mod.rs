//! Headless Agent session: offscreen pixels plus in-process pointer/keyboard.
//!
//! CPU readback stays here. Product `run_runtime` present never uses this path.
//!
//! [`runtime::RuntimeAgentSession`] drives a plain L3 `RuntimeDocument` and is
//! available under `runtime-agent` alone: a Rust product must not pull the Vue
//! renderer and a JS engine into its dependency graph just to click a button
//! without a window. [`vue::VueAgentSession`] needs `agent`.
//!
//! Both implement [`session::AgentSession`], so command dispatch and the stdio
//! loop are written once in [`session`] and the wire types in [`protocol`] name
//! no Vue type. [`cli`] drives a `&mut dyn AgentSession`, which is also how a
//! consuming product joins with its own session and its own document.

#[cfg(feature = "agent")]
pub mod vue;

pub mod cli;
pub mod fixtures;
pub mod pixels;
pub mod protocol;
pub mod runtime;
pub(crate) mod scene_probe;
pub mod session;

pub use protocol::{
    A11yFilter, AgentCommand, AgentReply, DiagnosticDump, GpuDump, HitDump, KeyStroke, PixelDiff,
    PixelStats, PointerGesture, RectDump, SceneProbeDump, SemanticDumpWidget, SessionInfo, Target,
    ThemeName,
};
pub use runtime::RuntimeAgentSession;
pub use session::{AgentSession, run_stdio};
#[cfg(feature = "agent")]
pub use vue::{VueAgentSession, semantic_counter_artifact, semantic_counter_source};

use std::collections::BTreeMap;

use nana_ui::runtime::{
    AccessibilityNode, AccessibilityRole, SelectionOrientation, StableNodeId, TextSelection,
};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct AgentError(pub String);

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AgentError {}

#[cfg(feature = "agent")]
impl From<nana_js_engine::JsEngineError> for AgentError {
    fn from(error: nana_js_engine::JsEngineError) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for AgentError {
    fn from(error: String) -> Self {
        Self(error)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoundsDump {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// `nana-ui-runtime` deliberately carries no serde derives, so the wire shape of
/// a text selection is defined here rather than by adding a serialization
/// surface to a product crate for a dev-only consumer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectionDump {
    pub anchor: usize,
    pub focus: usize,
}

/// Every field an [`AccessibilityNode`] carries.
///
/// A partial projection is worse than no projection: an Agent that cannot read
/// `checked`, `selected`, `modal` or `invalid` has to fall back to writing Rust
/// to answer "is the switch on", which is the exact failure this session exists
/// to remove. Fields that are absent or at their default are skipped, so a
/// plain text node stays as compact on the wire as it was before.
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
    /// `/`-joined `build`/`mount` assembly keys, the Rust L3 stable handle.
    /// Distinct from `agent_id` on purpose: they are different contracts, and
    /// merging them would make a missing value ambiguous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mixed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub editable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectionDump>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub modal: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub busy: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub invalid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_step: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_value: Option<f64>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(feature = "agent")]
fn dump_accessibility_node(
    node: AccessibilityNode,
    agent_ids: &BTreeMap<u64, String>,
) -> AccessibilityDumpNode {
    dump_accessibility_node_with(node, agent_ids, &BTreeMap::new())
}

fn dump_accessibility_node_with(
    node: AccessibilityNode,
    agent_ids: &BTreeMap<u64, String>,
    agent_paths: &BTreeMap<u64, String>,
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
        agent_path: agent_paths.get(&id).cloned(),
        description: node.description.map(|value| value.to_string()),
        checked: node.checked,
        mixed: node.mixed,
        orientation: node.orientation.map(orientation_name).map(String::from),
        selected: node.selected,
        multiline: node.multiline,
        editable: node.editable,
        selection: node.selection.map(selection_dump),
        modal: node.modal,
        busy: node.busy,
        invalid: node.invalid,
        numeric_minimum: node.numeric_minimum,
        numeric_maximum: node.numeric_maximum,
        numeric_step: node.numeric_step,
        numeric_value: node.numeric_value,
    }
}

/// Map a recorded diagnostic onto the wire shape.
#[cfg(feature = "agent")]
pub(crate) fn diagnostic_dump(event: crate::DiagnosticEvent) -> protocol::DiagnosticDump {
    use crate::DiagnosticKind;
    use nana_js_engine::JsDiagnosticLevel;
    protocol::DiagnosticDump {
        sequence: event.sequence,
        elapsed_micros: event.elapsed_micros,
        kind: match event.kind {
            DiagnosticKind::JsException => "js_exception",
            DiagnosticKind::UnhandledPromiseRejection => "unhandled_promise_rejection",
            DiagnosticKind::VueWarning => "vue_warning",
            DiagnosticKind::VueError => "vue_error",
            DiagnosticKind::HostCall => "host_call",
            DiagnosticKind::ResourceLifecycle => "resource_lifecycle",
            DiagnosticKind::WindowLifecycle => "window_lifecycle",
            DiagnosticKind::Frame => "frame",
            DiagnosticKind::DeviceLost => "device_lost",
            DiagnosticKind::RenderError => "render_error",
            DiagnosticKind::Inspector => "inspector",
        }
        .into(),
        level: match event.level {
            JsDiagnosticLevel::Error => "error",
            JsDiagnosticLevel::Warning => "warn",
            JsDiagnosticLevel::Info => "info",
        }
        .into(),
        source: event.source,
        message: event.message,
        stack: event.stack,
        fields: event.fields,
    }
}

fn selection_dump(selection: TextSelection) -> SelectionDump {
    SelectionDump {
        anchor: selection.anchor,
        focus: selection.focus,
    }
}

fn orientation_name(orientation: SelectionOrientation) -> &'static str {
    match orientation {
        SelectionOrientation::Horizontal => "horizontal",
        SelectionOrientation::Vertical => "vertical",
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
        AccessibilityRole::Main => "main",
        AccessibilityRole::Navigation => "navigation",
        AccessibilityRole::Banner => "banner",
        AccessibilityRole::ContentInfo => "contentinfo",
        AccessibilityRole::Complementary => "complementary",
        AccessibilityRole::Region => "region",
        AccessibilityRole::Search => "search",
        AccessibilityRole::Form => "form",
        AccessibilityRole::Generic => "generic",
    }
}
