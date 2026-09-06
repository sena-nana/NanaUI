//! Headless Agent session: offscreen pixels plus in-process pointer/keyboard.
//!
//! CPU readback stays here. Product `run_runtime` present never uses this path.
//!
//! [`runtime::RuntimeAgentSession`] drives a plain L3 `RuntimeDocument` and is
//! available under `runtime-agent` alone: a Rust product must not pull the Vue
//! renderer and a JS engine into its dependency graph just to click a button
//! without a window. [`vue::VueAgentSession`] needs `agent`.

#[cfg(feature = "agent")]
pub mod vue;

pub mod runtime;

pub use runtime::RuntimeAgentSession;
#[cfg(feature = "agent")]
pub use vue::{
    AgentCommand, AgentReply, SemanticDumpWidget, VueAgentSession, semantic_counter_artifact,
    semantic_counter_source,
};

use std::collections::BTreeMap;

use nana_ui::runtime::{AccessibilityNode, AccessibilityRole, StableNodeId};
use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_CLEAR: [f32; 4] = [0.96, 0.96, 0.96, 1.0];

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
