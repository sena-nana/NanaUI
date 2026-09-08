//! Development-only reload for the Vue tiers.
//!
//! # What a reload is here
//!
//! Browser-refresh semantics with the window kept: the artifact is re-evaluated
//! in a fresh JS context and the tree is rebuilt from scratch, while the native
//! window, its position, the wgpu `Device`/`Queue`, the `Surface` and every
//! bound host texture stay exactly as they were. Page state does not survive --
//! this is a reload, not Vite HMR, and V8 evaluates one flat classic script here
//! (see `VueRuntime::initialize`), so there is no module graph to patch.
//!
//! # Why the tree is rebuilt in place
//!
//! The obvious alternative -- build a new `RuntimeDocument` and swap it under
//! the live window -- silently breaks accessibility. `AccessibilityProjector`
//! guards its updates with a monotonic `generation` read off the world, and
//! `HostedAccessibility` is created once per window and never recreated. A fresh
//! `UiWorld` restarts that generation at 0 and restarts `StableNodeId` at 1, so
//! the projector rejects every later update and screen readers keep announcing
//! the pre-reload tree forever.
//!
//! Rebuilding inside the same `UiWorld` has neither problem: node ids are
//! permanently retired and never reused, and the generation only ever rises.
//!
//! # Why the isolate is replaced rather than re-evaluated
//!
//! Re-running the artifact in the same V8 context works on the Rust side --
//! `register_host_api` replaces the registry rather than appending, and the
//! event bridges re-resolve their globals by name. It fails on the JS side. The
//! Web API shim guards itself with an `installed` flag, so `window`/`document`
//! listener lists, the node wrapper cache and the window registry all survive;
//! every reload stacks another generation of listeners firing into dead
//! closures. Vue's own component registry survives too, so renaming a component
//! silently keeps resolving the old one. Throwing the heap away is both simpler
//! and correct.

use nana_js_engine::{HostValue, JsEngine, JsEngineError, RuntimeArtifact};

use crate::VueHost;
use crate::multi_window::{VueRuntime, VueWindowId};

/// Global the reloaded artifact reads to pick its state back up.
///
/// The framework carries the string and never interprets it: business state
/// belongs to the consuming application.
pub const RESTORE_STATE_GLOBAL: &str = "__nanaDevRestoreState";

/// Global a running artifact may define to hand state across a reload.
pub const SAVE_STATE_GLOBAL: &str = "__nanaDevSaveState";

/// What a dev harness asks a running Vue runtime to do.
///
/// The payload arrives already read from disk. File I/O belongs to whatever is
/// watching -- keeping it off this type keeps the reload itself off the
/// filesystem, and keeps `nana-ui-vue` free of any dependency on a watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevReload {
    /// Replace the artifact and remount the tree.
    Artifact { name: String, source: String },
    /// Replace one keyed stylesheet. No node is created or destroyed, so node
    /// ids, focus, scroll offsets and running animations all survive.
    Stylesheet { key: String, css: String },
}

/// Ask the running artifact for a state blob, if it published one.
///
/// Returns `None` when the application defines no [`SAVE_STATE_GLOBAL`], or when
/// the call fails -- a reload must not be blocked by an application's own save
/// handler throwing.
pub fn save_state<E: JsEngine + ?Sized>(engine: &mut E) -> Option<String> {
    let target = engine.resolve_function(SAVE_STATE_GLOBAL).ok()?;
    match engine.invoke(target, &[]).ok()? {
        HostValue::String(state) => Some(state),
        HostValue::Null | HostValue::Undefined => None,
        other => Some(other.to_json_value().to_string()),
    }
}

/// Publish the state blob the reloaded artifact will read at
/// [`RESTORE_STATE_GLOBAL`], before it is evaluated.
///
/// A failure here is not fatal: the application boots without its previous
/// state, which is strictly better than not booting.
pub fn publish_restore_state<E: JsEngine + ?Sized>(
    engine: &mut E,
    state: Option<&str>,
) -> Result<(), JsEngineError> {
    let literal = state.map_or_else(|| "null".to_owned(), json_string_literal);
    engine.initialize(RuntimeArtifact::from_source(
        "nana-dev-restore-state.js",
        format!("globalThis.{RESTORE_STATE_GLOBAL} = {literal};"),
    ))
}

impl VueRuntime {
    /// Tear down everything the previous artifact owned, leaving the document
    /// scaffold, the `UiWorld` and every GPU binding in place.
    ///
    /// Ordered deliberately: the tree goes first, so widget unregistration runs
    /// against a live cascade, then the state that outlives individual nodes.
    pub fn dev_teardown(&self) -> Result<(), JsEngineError> {
        for id in self.window_ids() {
            let Some(host) = self.host(id) else {
                continue;
            };
            host.lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                .dev_teardown()?;
        }
        // Process-global, so the per-window loop above does not reach it.
        crate::scroll::shared_scroll_offset_store().clear();
        Ok(())
    }

    /// Close every window except the primary one.
    ///
    /// Reload is whole-runtime, not per-window: one isolate and one module graph
    /// are shared by every Vue window, so a per-window reload is not
    /// expressible. Auxiliary windows come back when the reloaded artifact opens
    /// them again.
    pub fn dev_close_auxiliary_windows(&self) -> Vec<nana_ui_platform::WindowCommand> {
        for id in self.window_ids() {
            if id != VueWindowId::PRIMARY {
                let _ = self.request_close(id);
            }
        }
        self.drain_runtime_window_commands()
    }
}

impl VueHost {
    /// Tear one window down to its bare scaffold.
    ///
    /// Leaves the `html`/`body` nodes, the `UiWorld`, the layout viewport and
    /// every GPU binding in place; removes everything the previous artifact put
    /// there. Ordered so the tree goes first, while the cascade that widget
    /// unregistration consults is still populated.
    pub fn dev_teardown(&self) -> Result<(), JsEngineError> {
        // `clearMount` is the production teardown op: per child it unmounts the
        // native subtree, removes it from the document (which preserves the
        // scaffold), drops its layout boxes and unregisters it from the bridge.
        // `UiWorld` clears focus, IME, pointer capture and running animations as
        // part of the same despawn, so none of that needs repeating here.
        self.host_api_registry()
            .call("clearMount", &[])
            .map_err(|error| {
                JsEngineError::new(format!("dev reload could not clear the mount: {error}"))
            })?;

        // Everything below outlives a node, and would otherwise accumulate one
        // full copy per reload -- invisible for the first few saves, obvious
        // after twenty.
        //
        // Author sheets: the artifact re-injects its own when it mounts.
        self.clear_stylesheets();
        // Events queued for JS that is about to be replaced.
        self.bridge
            .lock()
            .map_err(|_| JsEngineError::new("Vue bridge poisoned"))?
            .drain_events();
        // Timers, animation frames, in-flight fetch, open sockets.
        self.web_api()
            .lock()
            .map_err(|_| JsEngineError::new("Web API state poisoned"))?
            .reset_pending();
        Ok(())
    }
}

/// Minimal JSON string escaping.
///
/// The blob is opaque application data spliced into a source line, so it has to
/// survive quotes, backslashes, control characters and the two Unicode line
/// terminators that are legal in JSON but end a line in JS source. Pulling a
/// JSON serializer into a crate that has none, for one string, is not worth it.
fn json_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            LINE_SEPARATOR => out.push_str("\\u2028"),
            PARAGRAPH_SEPARATOR => out.push_str("\\u2029"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// U+2028: valid inside a JSON string, ends a line in JS source.
const LINE_SEPARATOR: char = '\u{2028}';
/// U+2029: same problem.
const PARAGRAPH_SEPARATOR: char = '\u{2029}';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_blob_survives_quotes_backslashes_and_newlines() {
        assert_eq!(json_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string_literal("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string_literal("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string_literal("a\tb"), "\"a\\tb\"");
    }

    #[test]
    fn js_line_terminators_and_control_characters_are_escaped() {
        // Emitted verbatim these would split the assignment statement in two,
        // and the reloaded artifact would fail to parse for reasons that point
        // at nothing the developer wrote.
        assert_eq!(
            json_string_literal(&format!("a{LINE_SEPARATOR}b")),
            "\"a\\u2028b\""
        );
        assert_eq!(
            json_string_literal(&format!("a{PARAGRAPH_SEPARATOR}b")),
            "\"a\\u2029b\""
        );
        assert_eq!(json_string_literal("a\u{0}b"), "\"a\\u0000b\"");
        assert_eq!(json_string_literal("a\u{1f}b"), "\"a\\u001fb\"");
    }
}
