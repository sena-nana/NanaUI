//! Wire types shared by every headless Agent session.
//!
//! This module deliberately names no Vue type and no JS engine: it sits in the
//! `runtime-agent` tier so a plain Rust product can speak the same protocol
//! without pulling the Vue renderer or V8 into its dependency graph. The trait
//! that consumes these types lives in [`crate::agent::session`].

use serde::{Deserialize, Serialize};

use super::AccessibilityDumpNode;

/// How a command addresses a node.
///
/// One grammar for both tiers. `agent_id` is the Vue `data-agent-id`; a Rust L3
/// tree usually has none, which is why `role` + `label` exists — it reads the
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticDumpWidget {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub agent_id: String,
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
/// `nana-js-engine` carries no serde derives, so the wire shape is defined here
/// rather than by adding a serialization surface to an engine crate for a
/// dev-only consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticDump {
    pub sequence: u64,
    pub elapsed_micros: u64,
    /// `js_exception`, `vue_error`, `render_error`, `device_lost`, …
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
    /// `"runtime"` or `"vue"`.
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
    Semantic,
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
    /// JS exceptions, Vue errors, render errors and device loss recorded since
    /// the session started. A blank screenshot is most often an exception, and
    /// without this the Agent cannot see one.
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
    /// Reload the session's application without recreating its window or GPU
    /// context. `js` re-evaluates the artifact and rebuilds the tree; `css`
    /// swaps one keyed stylesheet and touches no node. Both are paths on the
    /// host's filesystem, read by the session.
    ///
    /// Serde-only here: the implementation belongs to whichever session tier can
    /// actually reload, so the Vue-free tier still names no JS engine.
    Reload {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        js: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        css: Option<String>,
        /// Stylesheet key `css` replaces. Defaults to the path itself, which is
        /// what an app that injects with an `href` will have used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        css_key: Option<String>,
    },
}

impl AgentCommand {
    /// Wire name, echoed back so a batch driver can key replies by command
    /// rather than by array position.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Screenshot { .. } => "screenshot",
            Self::A11y { .. } => "a11y",
            Self::Semantic => "semantic",
            Self::Probe { .. } => "probe",
            Self::HitTest { .. } => "hit_test",
            Self::Diff { .. } => "diff",
            Self::Gpu => "gpu",
            Self::Info => "info",
            Self::Diagnostics => "diagnostics",
            Self::Click { .. } => "click",
            Self::Hover { .. } => "hover",
            Self::Scroll { .. } => "scroll",
            Self::Key { .. } => "key",
            Self::Type { .. } => "type",
            Self::SetValue { .. } => "set_value",
            Self::Viewport { .. } => "viewport",
            Self::Theme { .. } => "theme",
            Self::Clear { .. } => "clear",
            Self::Pump => "pump",
            Self::Reload { .. } => "reload",
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
    pub widgets: Option<Vec<SemanticDumpWidget>>,
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

    pub fn with_widgets(mut self, widgets: Vec<SemanticDumpWidget>) -> Self {
        self.widgets = Some(widgets);
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

    pub(crate) fn echo(mut self, id: Option<u64>, cmd: &str) -> Self {
        self.id = id;
        self.cmd = Some(cmd.to_owned());
        self
    }
}
