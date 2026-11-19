//! Wire types shared by every headless Agent session.
//!
//! The trait that consumes these types lives in [`crate::agent::session`].

use serde::{Deserialize, Serialize};

use super::AccessibilityDumpNode;

/// How a command addresses a node.
///
/// `agent_id` is an explicit id the application attached; a Rust L3 tree
/// usually has none, which is why `role` + `label` exists — it reads the
/// projection the Runtime already computes, so it needs nothing from the
/// application author.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Rust L3 assembly key path, e.g. `"root/increment"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Which match to take when `role`/`label` are ambiguous. Without it an
    /// ambiguous target is an error rather than an arbitrary pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
}

impl Target {
    pub fn is_empty(&self) -> bool {
        self.node.is_none()
            && self.agent_id.is_none()
            && self.agent_path.is_none()
            && self.role.is_none()
            && self.label.is_none()
            && self.x.is_none()
            && self.y.is_none()
    }

    pub fn point(&self) -> Option<(f32, f32)> {
        match (self.x, self.y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
    }
}

/// Narrows an accessibility dump. A real application projects hundreds of
/// nodes; an unfiltered dump costs the caller more context than it returns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct A11yFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id_prefix: Option<String>,
    /// Keep only nodes that can be acted on (not `text`, `generic`, containers).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interactive_only: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    Light,
    Dark,
}

/// Pointer input, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerGesture {
    /// `button` follows the platform contract: 0 primary, 2 secondary.
    Click {
        x: f32,
        y: f32,
        button: i16,
    },
    Hover {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
}

/// One named key press and release. `text` is not committed — that is what
/// separates navigation from typing.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyStroke {
    pub key: String,
    pub code: String,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

/// One hit-test candidate, topmost first. Answers "I clicked and nothing
/// happened" by naming what is actually under the point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HitDump {
    pub node: u64,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RectDump {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Why a node is or is not on screen.
///
/// Every field comes from state the Scene already holds. Without this the only
/// answer to "why is this invisible" is to look at the PNG and guess between
/// clipped, occluded, transparent, zero-sized and off-viewport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SceneProbeDump {
    pub node: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_bounds: Option<RectDump>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw_bounds: Option<RectDump>,
    pub in_viewport: bool,
    pub primitive_count: usize,
    /// Node opacity multiplied through every isolating ancestor group.
    pub effective_opacity: f32,
    pub clips: Vec<RectDump>,
    /// Topmost hit at the centre of `draw_bounds` when it is not this node.
    /// Hit-test order, *not* painted alpha: a `pointer-events: none` overlay
    /// covers the node visually without appearing here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occluded_by: Option<HitDump>,
    /// One-word verdict derived from the fields above.
    pub verdict: String,
}

/// Whether a frame painted anything, as a number rather than a judgement call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PixelStats {
    pub width: u32,
    pub height: u32,
    /// Distinct RGB values. `1` means nothing but the clear colour was drawn.
    /// A high count is necessary, not sufficient: a UI painting entirely the
    /// wrong thing also has many colours.
    pub unique_colors: usize,
    pub nonclear_ratio: f32,
    pub mean_luma: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PixelDiff {
    pub changed_pixels: u64,
    pub changed_ratio: f32,
    pub max_channel_delta: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<RectDump>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuDump {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One recorded diagnostic.
///
/// A node as the declarative view layer sees it (`inspect`): the built-in
/// control and each bindable field, where its bindings were declared, and,
/// with `reactive-trace`, the signal writes that last patched it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InspectDump {
    /// The control table's tag, `"Button"`; absent for other components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    /// `file:line:column` of the element, for a node with bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub fields: Vec<FieldDump>,
    /// Why the node last changed, newest cause first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causes: Vec<CauseDump>,
    /// Why layout last reached the node, or why it waits for the next pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutCauseDump>,
    /// The elasticity the node's parent last read and what its line last
    /// assigned it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<DynamicDump>,
    /// The node's layout identity, context and participation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_node: Option<LayoutNodeDump>,
}

/// Dynamic Layout facts for a node. Lengths are in 1/64 px.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DynamicDump {
    pub generation: u64,
    pub segments: Vec<SegmentDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<AppliedDump>,
}

/// One piece of capacity: how much, what each unit costs (absent when
/// forbidden), what gives it up and what work taking it needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentDump {
    pub capacity: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<u32>,
    pub kind: String,
    pub class: String,
    pub sources: u16,
}

/// What a line assigned the node, and the part its padding closed up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedDump {
    pub axis: String,
    pub amount: i32,
    pub padding: i32,
}

/// A node as layout sees it (Issue #197). Kinds are lowercase variant
/// names: `flex`, `griditem`, `nativetext`, …
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayoutNodeDump {
    pub node: u64,
    /// The resolved layout intent that selects the context:
    /// `display=grid direction=auto position=static`.
    pub intent: String,
    /// `children`, `text`, `replaced` or `empty`.
    pub content: String,
    /// The context the node lays its own content out in; absent for an
    /// empty leaf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub established: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_context: Option<String>,
    /// Absent until the parent has been laid out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_generation: Option<u64>,
    pub context_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_generation: Option<u64>,
}

/// A typed layout cause, by flag name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayoutCauseDump {
    /// Queued by a mutation and not laid out yet.
    pub pending: bool,
    /// Queued for this node itself; `false` when a dependency edge of
    /// another node reached it.
    pub seed: bool,
    /// The authority that emitted it: `author`, `text`, `structure`, …
    pub source: String,
    /// Why: `style`, `text`, `topology`, …
    pub reasons: Vec<String>,
    /// Stages it runs: `measure`, `placement`, `context_reflow`, …
    pub stages: Vec<String>,
    /// Layout input groups that changed: `sizing`, `spacing`, …
    pub changed: Vec<String>,
    /// Dependencies the frontier follows from it:
    /// `exports_intrinsic_block`, `depends_on_sibling_prefix`, …
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldDump {
    pub name: String,
    /// `Debug` of the value.
    pub value: String,
    /// Where the binding driving the field was declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_at: Option<String>,
}

/// One resolved override entry from a `ThemePatch` or `ThemePackage` layer (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeOverrideDump {
    pub path: String,
    pub value: String,
}

/// One effective `ColorOverrides` entry active on a node (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeColorOverrideDump {
    pub role: String,
    pub rgba: [f32; 4],
}

/// One `ThemeScope` layer along a node's ancestor-to-leaf scope chain (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeScopeLayerDump {
    pub scope_id: u32,
    pub owner: u64,
    pub generation: u32,
    pub invalid_patch: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ThemeOverrideDump>,
}

/// One package patch layer recorded in `ThemePackageProvenance` (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePackageLayerDump {
    pub theme_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ThemeOverrideDump>,
}

/// Source provenance chain for an installed `ThemePackage` (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePackageProvenanceDump {
    pub base_chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<ThemePackageLayerDump>,
}

/// Count of live nodes subscribing to one `ThemeDependencyMask` category (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeCategoryCountDump {
    pub category: String,
    pub count: usize,
}

/// Aggregate dependency category counts across a document's retained world (#109).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeDependencySummaryDump {
    pub total_tracked_nodes: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category_counts: Vec<ThemeCategoryCountDump>,
}

/// Summary of the most recent theme or scope invalidation pass (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeInvalidationDump {
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_scope_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_scope_id: Option<u32>,
    pub changed_categories: Vec<String>,
    pub changed_mask_bits: u32,
    pub candidates_invalidated: usize,
    pub nodes_skipped_by_dependency: usize,
    pub scope_nodes_invalidated: usize,
    pub recipe_nodes_invalidated: usize,
    pub layout_seeds: usize,
    pub motion_policy_only: bool,
}

/// On-demand theme inspection payload for one node (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeInspectDump {
    pub node: u64,
    pub document: u64,
    pub effective_theme_id: String,
    pub effective_theme_generation: u32,
    pub appearance: String,
    pub active_scope: u32,
    pub scope_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_theme_override: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_chain: Vec<ThemeScopeLayerDump>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub color_overrides: Vec<ThemeColorOverrideDump>,
    pub dependency_categories: Vec<String>,
    pub dependency_mask_bits: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_provenance: Option<ThemePackageProvenanceDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_invalidation: Option<ThemeInvalidationDump>,
    pub dependency_summary: ThemeDependencySummaryDump,
}

/// Result of parsing, compiling, and optionally applying a `ThemePackage` (#109).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePackageLoadDump {
    pub theme_id: String,
    pub generation: u32,
    pub appearance: String,
    pub display_name: String,
    pub applied: bool,
    pub provenance: ThemePackageProvenanceDump,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CauseDump {
    /// Where the signal was written.
    pub written_at: String,
    /// Where the signal was created, while it exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticDump {
    pub sequence: u64,
    pub elapsed_micros: u64,
    /// `render_error`, `device_lost`, …
    pub kind: String,
    /// `error`, `warn` or `info`.
    pub level: String,
    pub source: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fields: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    /// `"runtime"`, or the name a product's own session reports.
    pub kind: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub clear: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum AgentCommand {
    // ---- observe ----
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    A11y {
        #[serde(flatten)]
        filter: A11yFilter,
    },
    Probe {
        #[serde(flatten)]
        target: Target,
    },
    HitTest {
        x: f32,
        y: f32,
    },
    Diff {
        baseline: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        candidate: Option<String>,
    },
    Gpu,
    Info,
    /// A node's control fields, binding sites and, with `reactive-trace`,
    /// why it last changed.
    Inspect {
        #[serde(flatten)]
        target: Target,
    },
    /// Errors and device loss a session recorded since it started. The Runtime
    /// session records none; a product's own session may.
    Diagnostics,

    // ---- act ----
    Click {
        #[serde(flatten)]
        target: Target,
        /// 0 primary (default), 2 secondary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<i16>,
    },
    Hover {
        #[serde(flatten)]
        target: Target,
    },
    Scroll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f32>,
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
    },
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        alt: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ctrl: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        meta: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        shift: bool,
    },
    Type {
        text: String,
    },
    SetValue {
        #[serde(flatten)]
        target: Target,
        value: String,
    },
    /// Write one field of a built-in control as its binding would.
    SetField {
        #[serde(flatten)]
        target: Target,
        field: String,
        value: String,
    },

    // ---- environment ----
    Viewport {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f32>,
    },
    Theme {
        mode: ThemeName,
    },
    Clear {
        /// `null` restores "follow the active theme background".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<[f32; 4]>,
    },
    Pump,
    /// Swap one keyed author stylesheet in the session without recreating its
    /// window or GPU context. `css` is a path on the host's filesystem, read by
    /// the session.
    ///
    /// Serde-only here: the implementation belongs to whichever session can
    /// actually reload stylesheets. The Runtime session has no author
    /// stylesheets to replace.
    Reload {
        css: String,
        /// Stylesheet key `css` replaces. Defaults to the path itself, which is
        /// what an app that injects with an `href` will have used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        css_key: Option<String>,
    },
    /// On-demand theme resolution trace, scope chain, color overrides, and
    /// dependency mask for `target` (#109).
    ThemeInspect {
        #[serde(flatten)]
        target: Target,
    },
    /// Most recent theme or scope invalidation summary recorded by this session (#109).
    ThemeInvalidation,
    /// Parse, validate, and register a JSON `ThemePackage` fail-closed; apply it
    /// to the session when `apply` is true (#109).
    LoadThemePackage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default)]
        apply: bool,
    },
}

impl AgentCommand {
    /// Wire name, echoed back so a batch driver can key replies by command
    /// rather than by array position.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Screenshot { .. } => "screenshot",
            Self::A11y { .. } => "a11y",
            Self::Probe { .. } => "probe",
            Self::HitTest { .. } => "hit_test",
            Self::Diff { .. } => "diff",
            Self::Gpu => "gpu",
            Self::Info => "info",
            Self::Inspect { .. } => "inspect",
            Self::Diagnostics => "diagnostics",
            Self::Click { .. } => "click",
            Self::Hover { .. } => "hover",
            Self::Scroll { .. } => "scroll",
            Self::Key { .. } => "key",
            Self::Type { .. } => "type",
            Self::SetValue { .. } => "set_value",
            Self::SetField { .. } => "set_field",
            Self::Viewport { .. } => "viewport",
            Self::Theme { .. } => "theme",
            Self::Clear { .. } => "clear",
            Self::Pump => "pump",
            Self::Reload { .. } => "reload",
            Self::ThemeInspect { .. } => "theme_inspect",
            Self::ThemeInvalidation => "theme_invalidation",
            Self::LoadThemePackage { .. } => "load_theme_package",
        }
    }
}

/// A flat reply with optional payloads.
///
/// Deliberately not a tagged payload enum: the consumer is an Agent that writes
/// one JSON line and then reads one field. A fixed path (`reply["nodes"]`)
/// stays readable without knowing which variant came back.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentReply {
    pub ok: bool,
    /// Echo of the request's `id`, when it carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// Echo of the command name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<AccessibilityDumpNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits: Option<Vec<HitDump>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<SceneProbeDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<PixelStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<PixelDiff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<SessionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<DiagnosticDump>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<InspectDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_inspect: Option<ThemeInspectDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_invalidation: Option<ThemeInvalidationDump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_package: Option<ThemePackageLoadDump>,
    /// Set when a filter truncated the dump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl AgentReply {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }

    pub fn with_nodes(mut self, nodes: Vec<AccessibilityDumpNode>) -> Self {
        self.nodes = Some(nodes);
        self
    }

    pub fn with_handled(mut self, handled: bool) -> Self {
        self.handled = Some(handled);
        self
    }

    pub fn with_target(mut self, target: u64) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_pixels(mut self, pixels: PixelStats) -> Self {
        self.pixels = Some(pixels);
        self
    }

    pub fn with_hits(mut self, hits: Vec<HitDump>) -> Self {
        self.hits = Some(hits);
        self
    }

    pub fn with_probe(mut self, probe: SceneProbeDump) -> Self {
        self.probe = Some(probe);
        self
    }

    pub fn with_diff(mut self, diff: PixelDiff) -> Self {
        self.diff = Some(diff);
        self
    }

    pub fn with_gpu(mut self, gpu: GpuDump) -> Self {
        self.gpu = Some(gpu);
        self
    }

    pub fn with_info(mut self, info: SessionInfo) -> Self {
        self.info = Some(info);
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<DiagnosticDump>) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    pub fn with_inspect(mut self, inspect: InspectDump) -> Self {
        self.inspect = Some(inspect);
        self
    }

    pub fn with_theme_inspect(mut self, theme_inspect: ThemeInspectDump) -> Self {
        self.theme_inspect = Some(theme_inspect);
        self
    }

    pub fn with_theme_invalidation(mut self, theme_invalidation: ThemeInvalidationDump) -> Self {
        self.theme_invalidation = Some(theme_invalidation);
        self
    }

    pub fn with_theme_package(mut self, theme_package: ThemePackageLoadDump) -> Self {
        self.theme_package = Some(theme_package);
        self
    }

    pub(crate) fn echo(mut self, id: Option<u64>, cmd: &str) -> Self {
        self.id = id;
        self.cmd = Some(cmd.to_owned());
        self
    }
}
