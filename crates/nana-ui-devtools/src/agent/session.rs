//! The capability trait every headless session implements, plus the one copy of
//! command dispatch and the stdio loop.
//!
//! Before this existed, the JSON protocol lived inside the Vue session, so half
//! the capabilities the Runtime session already had were unreachable over stdio
//! and the Vue session had none of them at all. The trait is object-safe on
//! purpose: the CLI drives a `&mut dyn AgentSession`, and a consuming product's
//! own session can join without naming a Vue type.

use std::io::{BufRead, Write};
use std::path::Path;

use super::AccessibilityDumpNode;
use super::pixels;
use super::protocol::{
    A11yFilter, AgentCommand, AgentReply, DiagnosticDump, GpuDump, HitDump, KeyStroke, PixelStats,
    PointerGesture, SceneProbeDump, SemanticDumpWidget, SessionInfo, Target, ThemeName,
};
use crate::agent::AgentError;
use crate::offscreen::{self, Size};

/// Roles a caller can act on. `a11y` with `interactive_only` keeps these and
/// drops the structural nodes that otherwise dominate a large dump.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "combo-box",
    "list-item",
    "menu-item",
    "radio",
    "row",
    "slider",
    "switch",
    "tab",
    "text-input",
];

pub trait AgentSession {
    // ---- required: the capability primitives ----
    fn describe(&self) -> SessionInfo;
    fn flush(&mut self) -> Result<(), AgentError>;
    fn accessibility_nodes(&self) -> Vec<AccessibilityDumpNode>;
    fn set_viewport(&mut self, width: u32, height: u32, scale: f32) -> Result<(), AgentError>;
    fn set_theme(&mut self, mode: ThemeName) -> Result<(), AgentError>;
    /// `None` restores "follow the active theme background".
    fn set_clear(&mut self, clear: Option<[f32; 4]>);
    fn pointer(&mut self, gesture: PointerGesture) -> Result<bool, AgentError>;
    /// Activate a node the way this backend's own click path does. Kept
    /// separate from [`Self::pointer`] because the two tiers legitimately
    /// differ: the Runtime applies an accessibility action, the Vue session
    /// falls back to a bridge event when a node has no painted geometry.
    fn activate(&mut self, node: u64) -> Result<bool, AgentError>;
    fn keyboard(&mut self, stroke: KeyStroke) -> Result<(), AgentError>;
    fn type_text(&mut self, text: &str) -> Result<(), AgentError>;
    fn set_value(&mut self, node: u64, value: &str) -> Result<bool, AgentError>;
    fn hit_test(&self, x: f32, y: f32) -> Vec<HitDump>;
    fn scene_probe(&self, node: u64) -> Option<SceneProbeDump>;
    fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError>;

    /// Vue overrides this; a Runtime document has no semantic layer, and
    /// synthesizing one would be a second facade.
    fn semantic_widgets(&self) -> Option<Vec<SemanticDumpWidget>> {
        None
    }

    /// Re-evaluate the application from `js` and rebuild the tree, keeping the
    /// window and GPU context. Only a tier with a JS engine can do this; the
    /// Vue-free Runtime tier has no artifact to re-evaluate, because its
    /// application is Rust code that a running process cannot replace.
    fn reload_artifact(&mut self, _path: &Path) -> Result<(), AgentError> {
        Err(AgentError(
            "this session cannot reload an artifact: its application is compiled in".into(),
        ))
    }

    /// Replace one keyed stylesheet. Creates and destroys no node.
    fn reload_stylesheet(&mut self, _key: &str, _path: &Path) -> Result<(), AgentError> {
        Err(AgentError(
            "this session has no author stylesheets to replace".into(),
        ))
    }

    /// Recorded JS exceptions, Vue errors, render errors and device loss.
    /// Empty for a session that records none.
    fn diagnostics(&self) -> Vec<DiagnosticDump> {
        Vec::new()
    }

    /// Vue overrides this to consult the semantic snapshot, which can carry a
    /// widget that never reaches the accessibility projection.
    fn resolve_agent_id(&self, agent_id: &str) -> Option<u64> {
        self.accessibility_nodes()
            .into_iter()
            .find(|node| node.agent_id.as_deref() == Some(agent_id))
            .map(|node| node.id)
    }

    // ---- provided ----

    fn screenshot_png(&mut self, path: &Path) -> Result<PixelStats, AgentError> {
        let clear = self.describe().clear;
        let (size, rgba) = self.screenshot_rgba()?;
        offscreen::write_png(path, size, &rgba).map_err(|error| AgentError(error.to_string()))?;
        Ok(pixels::pixel_stats(size, &rgba, clear))
    }

    fn resolve(&self, target: &Target) -> Result<u64, AgentError> {
        if let Some(node) = target.node {
            return Ok(node);
        }
        if let Some(agent_id) = &target.agent_id {
            return self
                .resolve_agent_id(agent_id)
                .ok_or_else(|| AgentError(format!("unknown agent_id {agent_id}")));
        }
        if let Some(path) = &target.agent_path {
            return self
                .accessibility_nodes()
                .into_iter()
                .find(|node| node.agent_path.as_deref() == Some(path.as_str()))
                .map(|node| node.id)
                .ok_or_else(|| AgentError(format!("unknown agent_path {path}")));
        }
        if target.role.is_none() && target.label.is_none() {
            return Err(AgentError(
                "target needs one of node, agent_id, role/label, or x+y".into(),
            ));
        }
        let matches: Vec<_> = self
            .accessibility_nodes()
            .into_iter()
            .filter(|node| {
                target.role.as_ref().is_none_or(|role| &node.role == role)
                    && target
                        .label
                        .as_ref()
                        .is_none_or(|label| node.label.as_deref() == Some(label.as_str()))
            })
            .collect();
        match (matches.len(), target.nth) {
            (0, _) => Err(AgentError(format!(
                "no node matches role={:?} label={:?}",
                target.role, target.label
            ))),
            (_, Some(nth)) => matches.get(nth).map(|node| node.id).ok_or_else(|| {
                AgentError(format!(
                    "nth={nth} is out of range, {} matched",
                    matches.len()
                ))
            }),
            (1, None) => Ok(matches[0].id),
            // An arbitrary pick would silently test the wrong widget.
            (count, None) => Err(AgentError(format!(
                "{count} nodes match role={:?} label={:?}; pass nth. First: {:?}",
                target.role,
                target.label,
                matches
                    .iter()
                    .take(3)
                    .map(|node| (node.id, node.label.clone()))
                    .collect::<Vec<_>>()
            ))),
        }
    }

    /// Centre of a node's accessibility box, for synthesizing a pointer.
    fn node_center(&self, node: u64) -> Option<(f32, f32)> {
        self.accessibility_nodes()
            .into_iter()
            .find(|candidate| candidate.id == node)
            .filter(|candidate| candidate.bounds.width > 0.0 || candidate.bounds.height > 0.0)
            .map(|candidate| {
                (
                    candidate.bounds.x + candidate.bounds.width * 0.5,
                    candidate.bounds.y + candidate.bounds.height * 0.5,
                )
            })
    }

    fn filtered_accessibility(&self, filter: &A11yFilter) -> (Vec<AccessibilityDumpNode>, bool) {
        let all = self.accessibility_nodes();
        if filter == &A11yFilter::default() {
            return (all, false);
        }
        let total = all.len();
        let depths = node_depths(&all);
        let subtree = filter.root.map(|root| subtree_of(&all, root));
        let kept: Vec<_> = all
            .into_iter()
            .filter(|node| {
                subtree
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(&node.id))
                    && filter.role.as_ref().is_none_or(|role| &node.role == role)
                    && filter.label.as_ref().is_none_or(|label| {
                        node.label
                            .as_deref()
                            .is_some_and(|value| value.contains(label.as_str()))
                    })
                    && filter.agent_id_prefix.as_ref().is_none_or(|prefix| {
                        node.agent_id
                            .as_deref()
                            .or(node.agent_path.as_deref())
                            .is_some_and(|value| value.starts_with(prefix.as_str()))
                    })
                    && (!filter.interactive_only || INTERACTIVE_ROLES.contains(&node.role.as_str()))
                    && filter
                        .depth
                        .is_none_or(|depth| depths.get(&node.id).copied().unwrap_or(0) <= depth)
            })
            .collect();
        let truncated = kept.len() < total;
        (kept, truncated)
    }

    fn execute(&mut self, command: AgentCommand) -> AgentReply {
        let name = command.name();
        let reply = self.dispatch(command);
        reply.echo(None, name)
    }

    #[doc(hidden)]
    fn dispatch(&mut self, command: AgentCommand) -> AgentReply {
        match command {
            AgentCommand::Info => AgentReply::ok().with_info(self.describe()),
            AgentCommand::Diagnostics => AgentReply::ok().with_diagnostics(self.diagnostics()),
            AgentCommand::Gpu => {
                let probe = offscreen::gpu_probe();
                AgentReply::ok().with_gpu(GpuDump {
                    available: probe.available,
                    adapter: probe.adapter,
                    backend: probe.backend,
                    reason: probe.reason,
                })
            }
            AgentCommand::Pump => match self.flush() {
                Ok(()) => AgentReply::ok(),
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::A11y { filter } => {
                let (nodes, truncated) = self.filtered_accessibility(&filter);
                let mut reply = AgentReply::ok().with_nodes(nodes);
                if truncated {
                    reply.truncated = Some(true);
                }
                reply
            }
            AgentCommand::Semantic => match self.semantic_widgets() {
                Some(widgets) => AgentReply::ok().with_widgets(widgets),
                None => {
                    AgentReply::err("semantic dump is Vue-only; use a11y for a Runtime document")
                }
            },
            AgentCommand::Screenshot { path } => {
                let path = path.unwrap_or_else(|| "target/agent-session/screenshot.png".into());
                match self.screenshot_png(Path::new(&path)) {
                    Ok(stats) => AgentReply::ok().with_path(path).with_pixels(stats),
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::HitTest { x, y } => AgentReply::ok().with_hits(self.hit_test(x, y)),
            AgentCommand::Probe { target } => match self.resolve(&target) {
                Ok(node) => match self.scene_probe(node) {
                    Some(probe) => AgentReply::ok().with_target(node).with_probe(probe),
                    None => AgentReply::err(format!("node {node} is not in the scene")),
                },
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::Diff {
                baseline,
                candidate,
            } => self.dispatch_diff(&baseline, candidate.as_deref()),
            AgentCommand::Click { target, button } => {
                let button = button.unwrap_or(0);
                if let Some((x, y)) = target.point() {
                    return match self.pointer(PointerGesture::Click { x, y, button }) {
                        Ok(handled) => AgentReply::ok().with_handled(handled),
                        Err(error) => AgentReply::err(error.0),
                    };
                }
                let node = match self.resolve(&target) {
                    Ok(node) => node,
                    Err(error) => return AgentReply::err(error.0),
                };
                // A secondary press has no accessibility action, so it can only
                // be delivered as a real pointer at the node's centre.
                let result = if button == 0 {
                    self.activate(node)
                } else {
                    match self.node_center(node) {
                        Some((x, y)) => self.pointer(PointerGesture::Click { x, y, button }),
                        None => Err(AgentError(format!("node {node} has no painted geometry"))),
                    }
                };
                match result {
                    Ok(handled) => AgentReply::ok().with_target(node).with_handled(handled),
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::Hover { target } => {
                let point = match target.point() {
                    Some(point) => Some(point),
                    None => match self.resolve(&target) {
                        Ok(node) => self.node_center(node),
                        Err(error) => return AgentReply::err(error.0),
                    },
                };
                match point {
                    Some((x, y)) => match self.pointer(PointerGesture::Hover { x, y }) {
                        Ok(_) => AgentReply::ok(),
                        Err(error) => AgentReply::err(error.0),
                    },
                    None => AgentReply::err("hover target has no painted geometry"),
                }
            }
            AgentCommand::Scroll { x, y, dx, dy } => {
                let info = self.describe();
                let x = x.unwrap_or(info.width as f32 * 0.5);
                let y = y.unwrap_or(info.height as f32 * 0.5);
                match self.pointer(PointerGesture::Scroll {
                    x,
                    y,
                    delta_x: dx,
                    delta_y: dy,
                }) {
                    Ok(_) => AgentReply::ok(),
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::Key {
                key,
                code,
                alt,
                ctrl,
                meta,
                shift,
            } => {
                let code = code.unwrap_or_else(|| "Unidentified".into());
                match self.keyboard(KeyStroke {
                    key,
                    code,
                    alt,
                    ctrl,
                    meta,
                    shift,
                }) {
                    Ok(()) => AgentReply::ok(),
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::Type { text } => match self.type_text(&text) {
                Ok(()) => AgentReply::ok(),
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::SetValue { target, value } => match self.resolve(&target) {
                Ok(node) => match self.set_value(node, &value) {
                    Ok(handled) => AgentReply::ok().with_target(node).with_handled(handled),
                    Err(error) => AgentReply::err(error.0),
                },
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::Viewport {
                width,
                height,
                scale,
            } => {
                let info = self.describe();
                match self.set_viewport(
                    width.unwrap_or(info.width),
                    height.unwrap_or(info.height),
                    scale.unwrap_or(info.scale),
                ) {
                    Ok(()) => AgentReply::ok().with_info(self.describe()),
                    Err(error) => AgentReply::err(error.0),
                }
            }
            AgentCommand::Theme { mode } => match self.set_theme(mode) {
                Ok(()) => AgentReply::ok().with_info(self.describe()),
                Err(error) => AgentReply::err(error.0),
            },
            AgentCommand::Clear { color } => {
                self.set_clear(color);
                AgentReply::ok().with_info(self.describe())
            }
            AgentCommand::Reload { js, css, css_key } => {
                self.dispatch_reload(js.as_deref(), css.as_deref(), css_key.as_deref())
            }
        }
    }

    /// Apply a reload, then flush so the reply describes the tree that resulted
    /// rather than the one that was there when the command arrived.
    ///
    /// The stylesheet goes first: it is the cheap path, and when a command
    /// carries both, applying CSS before the artifact means the artifact's own
    /// mount-time injection has the last word, matching what a real save does.
    #[doc(hidden)]
    fn dispatch_reload(
        &mut self,
        js: Option<&str>,
        css: Option<&str>,
        css_key: Option<&str>,
    ) -> AgentReply {
        if js.is_none() && css.is_none() {
            return AgentReply::err("reload needs js, css, or both");
        }
        if let Some(css) = css {
            let key = css_key.unwrap_or(css);
            if let Err(error) = self.reload_stylesheet(key, Path::new(css)) {
                return AgentReply::err(error.0);
            }
        }
        if let Some(js) = js
            && let Err(error) = self.reload_artifact(Path::new(js))
        {
            return AgentReply::err(error.0);
        }
        match self.flush() {
            Ok(()) => AgentReply::ok().with_info(self.describe()),
            Err(error) => AgentReply::err(error.0),
        }
    }

    #[doc(hidden)]
    fn dispatch_diff(&mut self, baseline: &str, candidate: Option<&str>) -> AgentReply {
        let Some((base_size, base)) = offscreen::read_png(Path::new(baseline)) else {
            return AgentReply::err(format!("cannot read baseline {baseline}"));
        };
        let (size, current) = match candidate {
            Some(path) => match offscreen::read_png(Path::new(path)) {
                Some(loaded) => loaded,
                None => return AgentReply::err(format!("cannot read candidate {path}")),
            },
            None => match self.screenshot_rgba() {
                Ok(frame) => frame,
                Err(error) => return AgentReply::err(error.0),
            },
        };
        if size != base_size {
            return AgentReply::err(format!(
                "frame sizes differ: baseline {}x{}, candidate {}x{}",
                base_size.width, base_size.height, size.width, size.height
            ));
        }
        match pixels::pixel_diff(size, &base, &current, 0) {
            Ok(diff) => AgentReply::ok().with_diff(diff),
            Err(error) => AgentReply::err(error),
        }
    }
}

fn node_depths(nodes: &[AccessibilityDumpNode]) -> std::collections::HashMap<u64, usize> {
    let parents: std::collections::HashMap<u64, Option<u64>> =
        nodes.iter().map(|node| (node.id, node.parent)).collect();
    let mut depths = std::collections::HashMap::with_capacity(nodes.len());
    for node in nodes {
        let mut depth = 0usize;
        let mut cursor = node.parent;
        // The projection is a tree, but a malformed one must not hang a dump.
        while let Some(parent) = cursor {
            depth += 1;
            if depth > nodes.len() {
                break;
            }
            cursor = parents.get(&parent).copied().flatten();
        }
        depths.insert(node.id, depth);
    }
    depths
}

fn subtree_of(nodes: &[AccessibilityDumpNode], root: u64) -> std::collections::HashSet<u64> {
    let children: std::collections::HashMap<u64, &Vec<u64>> =
        nodes.iter().map(|node| (node.id, &node.children)).collect();
    let mut kept = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !kept.insert(id) {
            continue;
        }
        if let Some(next) = children.get(&id) {
            stack.extend(next.iter().copied());
        }
    }
    kept
}

#[derive(serde::Deserialize)]
struct CommandEnvelope {
    #[serde(default)]
    id: Option<u64>,
}

/// JSON lines in, JSON lines out.
///
/// A malformed line is answered and the session continues: an Agent piping a
/// heredoc of commands would otherwise lose every later step to one typo. Each
/// reply is flushed, so a driver that writes one command and waits for its
/// answer cannot deadlock on a buffered writer.
pub fn run_stdio(
    session: &mut dyn AgentSession,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = serde_json::from_str::<CommandEnvelope>(trimmed)
            .ok()
            .and_then(|envelope| envelope.id);
        let reply = match serde_json::from_str::<AgentCommand>(trimmed) {
            Ok(command) => {
                let name = command.name();
                session.dispatch(command).echo(id, name)
            }
            Err(error) => AgentReply {
                id,
                ..AgentReply::err(format!("cannot parse command: {error}"))
            },
        };
        writeln!(output, "{}", serde_json::to_string(&reply)?)?;
        output.flush()?;
    }
    Ok(())
}
