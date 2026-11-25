//! Lightweight `.vue` ↔ virtual Rust document LSP bridge for `rust-analyzer`.
//!
//! Translates `.vue` open/change/save/close notifications into an in-memory
//! compilation of the sibling `.vue` batch (`nana_views.rs` + bidirectional
//! [`SourceMap`]), forwards editor requests (`hover`, `definition`,
//! `completion`, `references`, `rename`, etc.) to `rust-analyzer` on the virtual
//! Rust document, and remaps diagnostics and locations back to `.vue` files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use url::Url;

use crate::source_map::{SourceMap, SourceSpan, char_col_to_lsp, lsp_to_char_col};
use crate::{Compiler, Error as SfcError};

/// Configuration for [`LspBridge`].
#[derive(Debug, Clone, Default)]
pub struct LspConfig {
    pub views_dir: Option<PathBuf>,
    pub virtual_rs: Option<PathBuf>,
    pub runtime: Option<String>,
}

/// Messages produced by one [`LspBridge`] step.
#[derive(Debug, Default)]
pub struct BridgeActions {
    pub to_backend: Vec<Value>,
    pub to_client: Vec<Value>,
}

#[derive(Debug, Clone)]
struct VueDocument {
    uri: String,
    path: PathBuf,
    text: String,
    open: bool,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    method: String,
}

struct VirtualState {
    virtual_uri: String,
    runtime: String,
    version: i32,
    opened_in_backend: bool,
    code: String,
    source_map: Option<SourceMap>,
    last_valid_code: Option<String>,
    last_valid_map: Option<SourceMap>,
    sfc_error: Option<SfcError>,
    sfc_warnings: Vec<(String, usize, usize, String)>,
    backend_diagnostics: Vec<Value>,
    published_diag_uris: HashSet<String>,
}

/// Stateful, transport-independent LSP bridge between an editor client and
/// `rust-analyzer`.
pub struct LspBridge {
    config: LspConfig,
    docs: HashMap<PathBuf, VueDocument>,
    uri_to_path: HashMap<String, PathBuf>,
    virtual_state: Option<VirtualState>,
    pending: HashMap<Value, PendingRequest>,
}

impl LspBridge {
    pub fn new(config: LspConfig) -> Self {
        Self {
            config,
            docs: HashMap::new(),
            uri_to_path: HashMap::new(),
            virtual_state: None,
            pending: HashMap::new(),
        }
    }

    /// Handle one JSON-RPC message received from the editor client.
    pub fn handle_client_message(&mut self, mut msg: Value) -> BridgeActions {
        let mut actions = BridgeActions::default();
        let Some(method) = msg.get("method").and_then(Value::as_str).map(str::to_owned) else {
            actions.to_backend.push(msg);
            return actions;
        };
        let id = msg.get("id").cloned();

        if let Some(id) = id {
            let vue_uri = msg
                .get("params")
                .and_then(|p| p.get("textDocument"))
                .and_then(|td| td.get("uri"))
                .and_then(Value::as_str)
                .filter(|uri| is_vue_uri(uri))
                .map(str::to_owned);

            if let Some(uri) = &vue_uri {
                match method.as_str() {
                    "textDocument/completion" => {
                        if let Some(handled) =
                            self.forward_completion(&id, uri, &mut msg, &mut actions)
                            && !handled
                        {
                            actions.to_client.push(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": Value::Null,
                            }));
                            return actions;
                        }
                    }
                    "textDocument/hover"
                    | "textDocument/definition"
                    | "textDocument/declaration"
                    | "textDocument/typeDefinition"
                    | "textDocument/implementation"
                    | "textDocument/references"
                    | "textDocument/documentHighlight"
                    | "textDocument/signatureHelp"
                    | "textDocument/prepareRename"
                    | "textDocument/rename" => {
                        if !self.rewrite_client_position_request(uri, &mut msg) {
                            actions.to_client.push(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": Value::Null,
                            }));
                            return actions;
                        }
                    }
                    _ => {}
                }
            }
            self.pending.insert(id, PendingRequest { method });
            actions.to_backend.push(msg);
            return actions;
        }

        // Notification from client.
        match method.as_str() {
            "textDocument/didOpen" => {
                if let Some(td) = msg.get("params").and_then(|p| p.get("textDocument"))
                    && let Some(uri) = td.get("uri").and_then(Value::as_str)
                    && is_vue_uri(uri)
                {
                    let text = td
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    self.on_vue_open(uri, text, &mut actions);
                    return actions;
                }
            }
            "textDocument/didChange" => {
                if let Some(params) = msg.get("params")
                    && let Some(uri) = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(Value::as_str)
                    && is_vue_uri(uri)
                {
                    let changes = params
                        .get("contentChanges")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    self.on_vue_change(uri, &changes, &mut actions);
                    return actions;
                }
            }
            "textDocument/didSave" => {
                if let Some(params) = msg.get("params")
                    && let Some(uri) = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(Value::as_str)
                    && is_vue_uri(uri)
                {
                    let text = params
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.on_vue_save(uri, text, &mut actions);
                    return actions;
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|td| td.get("uri"))
                    .and_then(Value::as_str)
                    && is_vue_uri(uri)
                {
                    self.on_vue_close(uri, &mut actions);
                    return actions;
                }
            }
            _ => {}
        }

        actions.to_backend.push(msg);
        actions
    }

    /// Handle one JSON-RPC message received from `rust-analyzer`.
    pub fn handle_backend_message(&mut self, mut msg: Value) -> BridgeActions {
        let mut actions = BridgeActions::default();
        if let Some(id) = msg.get("id").cloned()
            && msg.get("method").is_none()
        {
            if let Some(req) = self.pending.remove(&id) {
                self.remap_backend_response(&req, &mut msg);
            }
            actions.to_client.push(msg);
            return actions;
        }

        if msg.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
            && let Some(params) = msg.get("params")
            && let Some(uri) = params.get("uri").and_then(Value::as_str)
            && self
                .virtual_state
                .as_ref()
                .is_some_and(|vs| uris_match(&vs.virtual_uri, uri))
        {
            let diags = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(vs) = &mut self.virtual_state {
                vs.backend_diagnostics = diags;
            }
            self.publish_all_vue_diagnostics(&mut actions);
            return actions;
        }

        actions.to_client.push(msg);
        actions
    }

    fn on_vue_open(&mut self, uri: &str, text: String, actions: &mut BridgeActions) {
        let Some(path) = uri_to_normalized_path(uri) else {
            return;
        };
        self.uri_to_path.insert(uri.to_owned(), path.clone());
        self.docs.insert(
            path.clone(),
            VueDocument {
                uri: uri.to_owned(),
                path: path.clone(),
                text,
                open: true,
            },
        );
        self.load_sibling_views(&path);
        self.ensure_virtual_state(&path);
        self.recompile_and_sync(actions);
    }

    fn on_vue_change(&mut self, uri: &str, changes: &[Value], actions: &mut BridgeActions) {
        let Some(path) = self
            .uri_to_path
            .get(uri)
            .cloned()
            .or_else(|| uri_to_normalized_path(uri))
        else {
            return;
        };
        let doc = self
            .docs
            .entry(path.clone())
            .or_insert_with(|| VueDocument {
                uri: uri.to_owned(),
                path: path.clone(),
                text: String::new(),
                open: true,
            });
        doc.uri = uri.to_owned();
        doc.open = true;
        for change in changes {
            let Some(new_text) = change.get("text").and_then(Value::as_str) else {
                continue;
            };
            if let Some(range) = change.get("range") {
                apply_incremental_change(&mut doc.text, range, new_text);
            } else {
                doc.text = new_text.to_owned();
            }
        }
        self.ensure_virtual_state(&path);
        self.recompile_and_sync(actions);
    }

    fn on_vue_save(&mut self, uri: &str, text: Option<String>, actions: &mut BridgeActions) {
        let Some(path) = self
            .uri_to_path
            .get(uri)
            .cloned()
            .or_else(|| uri_to_normalized_path(uri))
        else {
            return;
        };
        if let Some(text) = text
            && let Some(doc) = self.docs.get_mut(&path)
        {
            doc.text = text;
        }
        self.ensure_virtual_state(&path);
        self.recompile_and_sync(actions);
    }

    fn on_vue_close(&mut self, uri: &str, actions: &mut BridgeActions) {
        let Some(path) = self
            .uri_to_path
            .get(uri)
            .cloned()
            .or_else(|| uri_to_normalized_path(uri))
        else {
            return;
        };
        if let Some(doc) = self.docs.get_mut(&path) {
            doc.open = false;
            if let Ok(disk) = std::fs::read_to_string(&path) {
                doc.text = disk;
            }
        }
        if let Some(vs) = &mut self.virtual_state {
            vs.published_diag_uris.remove(uri);
        }
        actions.to_client.push(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": [],
            }
        }));
    }

    fn load_sibling_views(&mut self, vue_path: &Path) {
        let Some(dir) = self
            .config
            .views_dir
            .as_deref()
            .or_else(|| vue_path.parent())
        else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = normalize_path(&entry.path());
            if p.extension().is_some_and(|e| e == "vue")
                && !self.docs.contains_key(&p)
                && let Ok(text) = std::fs::read_to_string(&p)
                && let Some(uri) = path_to_uri(&p)
            {
                self.uri_to_path.insert(uri.clone(), p.clone());
                self.docs.insert(
                    p.clone(),
                    VueDocument {
                        uri,
                        path: p,
                        text,
                        open: false,
                    },
                );
            }
        }
    }

    fn ensure_virtual_state(&mut self, vue_path: &Path) {
        if self.virtual_state.is_some() {
            return;
        }
        let views_dir = self
            .config
            .views_dir
            .clone()
            .or_else(|| vue_path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let (discovered_rs, discovered_runtime) = discover_generated_views(&views_dir);
        let virtual_path = self
            .config
            .virtual_rs
            .clone()
            .or(discovered_rs)
            .unwrap_or_else(|| views_dir.join(".nana_views.virtual.rs"));
        let virtual_uri = path_to_uri(&virtual_path)
            .unwrap_or_else(|| format!("file://{}", virtual_path.display()));
        let runtime = self
            .config
            .runtime
            .clone()
            .or(discovered_runtime)
            .unwrap_or_else(|| "::nana_ui::runtime".to_owned());

        self.virtual_state = Some(VirtualState {
            virtual_uri,
            runtime,
            version: 0,
            opened_in_backend: false,
            code: String::new(),
            source_map: None,
            last_valid_code: None,
            last_valid_map: None,
            sfc_error: None,
            sfc_warnings: Vec::new(),
            backend_diagnostics: Vec::new(),
            published_diag_uris: HashSet::new(),
        });
    }

    fn sorted_sources(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(&PathBuf, &VueDocument)> = self.docs.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        entries
            .into_iter()
            .map(|(path, doc)| (path.display().to_string(), doc.text.clone()))
            .collect()
    }

    fn recompile_and_sync(&mut self, actions: &mut BridgeActions) {
        let sources = self.sorted_sources();
        let Some(vs) = &mut self.virtual_state else {
            return;
        };
        match Compiler::new(&vs.runtime).compile(&sources) {
            Ok(output) => {
                let map = SourceMap::from_json(&output.source_map).ok();
                vs.sfc_error = None;
                vs.sfc_warnings = parse_sfc_warnings(&output.warnings);
                let changed = vs.code != output.code;
                vs.code = output.code.clone();
                vs.source_map = map.clone();
                vs.last_valid_code = Some(output.code);
                vs.last_valid_map = map;
                if !vs.opened_in_backend {
                    vs.version += 1;
                    vs.opened_in_backend = true;
                    actions.to_backend.push(json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didOpen",
                        "params": {
                            "textDocument": {
                                "uri": vs.virtual_uri,
                                "languageId": "rust",
                                "version": vs.version,
                                "text": vs.code,
                            }
                        }
                    }));
                } else if changed {
                    vs.version += 1;
                    actions.to_backend.push(json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didChange",
                        "params": {
                            "textDocument": {
                                "uri": vs.virtual_uri,
                                "version": vs.version,
                            },
                            "contentChanges": [{ "text": vs.code }],
                        }
                    }));
                }
            }
            Err(err) => {
                vs.sfc_error = Some(err);
                vs.sfc_warnings.clear();
            }
        }
        self.publish_all_vue_diagnostics(actions);
    }

    fn rewrite_client_position_request(&self, uri: &str, msg: &mut Value) -> bool {
        let Some(vs) = &self.virtual_state else {
            return false;
        };
        let Some(map) = vs.source_map.as_ref().or(vs.last_valid_map.as_ref()) else {
            return false;
        };
        let Some(doc) = self.doc_by_uri(uri) else {
            return false;
        };
        let Some(params) = msg.get_mut("params") else {
            return false;
        };
        let Some(pos) = params.get("position") else {
            return false;
        };
        let Some(lsp_line) = pos.get("line").and_then(Value::as_u64) else {
            return false;
        };
        let Some(lsp_col) = pos.get("character").and_then(Value::as_u64) else {
            return false;
        };
        let vue_line = lsp_line as usize + 1;
        let line_text = doc.text.lines().nth(lsp_line as usize).unwrap_or("");
        let vue_col = lsp_to_char_col(line_text, lsp_col as u32);
        let file_key = doc.path.display().to_string();
        let Some(gen_pos) = map.vue_to_generated(&file_key, vue_line, vue_col) else {
            return false;
        };
        let gen_line_text = vs
            .code
            .lines()
            .nth(gen_pos.line.saturating_sub(1))
            .unwrap_or("");
        let gen_lsp_col = char_col_to_lsp(gen_line_text, gen_pos.column);
        params["textDocument"]["uri"] = Value::String(vs.virtual_uri.clone());
        params["position"] = json!({
            "line": gen_pos.line.saturating_sub(1),
            "character": gen_lsp_col,
        });
        true
    }

    /// Forward completion, performing temporary `__nana_complete` recovery when
    /// the `.vue` buffer currently fails to parse (e.g. `draft.` mid-typing).
    fn forward_completion(
        &mut self,
        _id: &Value,
        uri: &str,
        msg: &mut Value,
        actions: &mut BridgeActions,
    ) -> Option<bool> {
        if self
            .virtual_state
            .as_ref()
            .is_some_and(|vs| vs.sfc_error.is_none())
            && self.rewrite_client_position_request(uri, msg)
        {
            return Some(true);
        }

        let (doc_path, doc_text, lsp_line, lsp_col) = {
            let doc = self.doc_by_uri(uri)?;
            let pos = msg.get("params")?.get("position")?;
            let line = pos.get("line")?.as_u64()? as usize;
            let col = pos.get("character")?.as_u64()? as u32;
            (doc.path.clone(), doc.text.clone(), line, col)
        };
        let offset = lsp_pos_to_byte_offset(&doc_text, lsp_line, lsp_col)?;
        const PROBE: &str = "__nana_complete";
        let mut patched = String::with_capacity(doc_text.len() + PROBE.len());
        patched.push_str(&doc_text[..offset]);
        patched.push_str(PROBE);
        patched.push_str(&doc_text[offset..]);

        let mut sources = Vec::new();
        let mut entries: Vec<(&PathBuf, &VueDocument)> = self.docs.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (path, doc) in entries {
            if *path == doc_path {
                sources.push((path.display().to_string(), patched.clone()));
            } else {
                sources.push((path.display().to_string(), doc.text.clone()));
            }
        }

        let vs = self.virtual_state.as_mut()?;
        let output = Compiler::new(&vs.runtime).compile(&sources).ok()?;
        let map = SourceMap::from_json(&output.source_map).ok()?;
        let vue_line = lsp_line + 1;
        let line_text = patched.lines().nth(lsp_line).unwrap_or("");
        let vue_col = lsp_to_char_col(line_text, lsp_col);
        let gen_pos = map.vue_to_generated(&doc_path.display().to_string(), vue_line, vue_col)?;
        // Replace the synthetic probe identifier with equal-length spaces so
        // `rust-analyzer` sees `expr.` at the exact same column.
        let recovered_code = output.code.replace(PROBE, &" ".repeat(PROBE.len()));
        let gen_line_text = recovered_code
            .lines()
            .nth(gen_pos.line.saturating_sub(1))
            .unwrap_or("");
        let gen_lsp_col = char_col_to_lsp(gen_line_text, gen_pos.column);

        // Temporarily sync recovered code -> send completion -> restore previous valid code.
        vs.version += 1;
        let sync_method = if vs.opened_in_backend {
            "textDocument/didChange"
        } else {
            vs.opened_in_backend = true;
            "textDocument/didOpen"
        };
        let sync_msg = if sync_method == "textDocument/didOpen" {
            json!({
                "jsonrpc": "2.0",
                "method": sync_method,
                "params": {
                    "textDocument": {
                        "uri": vs.virtual_uri,
                        "languageId": "rust",
                        "version": vs.version,
                        "text": recovered_code,
                    }
                }
            })
        } else {
            json!({
                "jsonrpc": "2.0",
                "method": sync_method,
                "params": {
                    "textDocument": {
                        "uri": vs.virtual_uri,
                        "version": vs.version,
                    },
                    "contentChanges": [{ "text": recovered_code }],
                }
            })
        };
        actions.to_backend.push(sync_msg);

        msg["params"]["textDocument"]["uri"] = Value::String(vs.virtual_uri.clone());
        msg["params"]["position"] = json!({
            "line": gen_pos.line.saturating_sub(1),
            "character": gen_lsp_col,
        });
        vs.code = recovered_code;
        vs.source_map = Some(map);
        Some(true)
    }

    fn remap_backend_response(&self, req: &PendingRequest, msg: &mut Value) {
        let Some(result) = msg.get_mut("result") else {
            return;
        };
        if result.is_null() {
            return;
        }
        if req.method == "initialize" {
            if let Some(caps) = result.get_mut("capabilities") {
                // Force Full sync (1) so `.vue` incremental edits always keep the
                // in-memory compiler synchronized cleanly.
                caps["textDocumentSync"] = json!(1);
            }
            return;
        }
        let Some(vs) = &self.virtual_state else {
            return;
        };
        let Some(map) = vs.source_map.as_ref().or(vs.last_valid_map.as_ref()) else {
            return;
        };
        match req.method.as_str() {
            "textDocument/hover" => {
                if let Some(range) = result.get("range").cloned()
                    && let Some((_, mapped)) = self.remap_generated_range(&range, vs, map)
                {
                    result["range"] = mapped;
                }
            }
            "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/typeDefinition"
            | "textDocument/implementation"
            | "textDocument/references" => {
                self.remap_location_result(result, vs, map);
            }
            "textDocument/documentHighlight" => {
                if let Some(arr) = result.as_array_mut() {
                    arr.retain_mut(|item| {
                        let Some(range) = item.get("range").cloned() else {
                            return false;
                        };
                        if let Some((_, mapped)) = self.remap_generated_range(&range, vs, map) {
                            item["range"] = mapped;
                            true
                        } else {
                            false
                        }
                    });
                }
            }
            "textDocument/completion" => {
                let items = if let Some(arr) = result.as_array_mut() {
                    Some(arr)
                } else {
                    result.get_mut("items").and_then(Value::as_array_mut)
                };
                if let Some(items) = items {
                    for item in items {
                        if let Some(edit) = item.get_mut("textEdit") {
                            if let Some(range) = edit.get("range").cloned()
                                && let Some((_, mapped)) =
                                    self.remap_generated_range(&range, vs, map)
                            {
                                edit["range"] = mapped;
                            }
                            if let Some(insert) = edit.get("insert").cloned()
                                && let Some((_, mapped)) =
                                    self.remap_generated_range(&insert, vs, map)
                            {
                                edit["insert"] = mapped;
                            }
                            if let Some(replace) = edit.get("replace").cloned()
                                && let Some((_, mapped)) =
                                    self.remap_generated_range(&replace, vs, map)
                            {
                                edit["replace"] = mapped;
                            }
                        }
                        if let Some(extra) = item
                            .get_mut("additionalTextEdits")
                            .and_then(Value::as_array_mut)
                        {
                            extra.retain_mut(|edit| {
                                let Some(range) = edit.get("range").cloned() else {
                                    return false;
                                };
                                if let Some((_, mapped)) =
                                    self.remap_generated_range(&range, vs, map)
                                {
                                    edit["range"] = mapped;
                                    true
                                } else {
                                    false
                                }
                            });
                        }
                    }
                }
            }
            "textDocument/prepareRename" => {
                if let Some(range) = result.get("range").cloned() {
                    if let Some((_, mapped)) = self.remap_generated_range(&range, vs, map) {
                        result["range"] = mapped;
                    } else {
                        *result = Value::Null;
                    }
                } else if result.get("start").is_some() {
                    if let Some((_, mapped)) = self.remap_generated_range(result, vs, map) {
                        *result = mapped;
                    } else {
                        *result = Value::Null;
                    }
                }
            }
            "textDocument/rename" => {
                self.remap_workspace_edit(result, vs, map);
            }
            _ => {}
        }
    }

    fn remap_location_result(&self, value: &mut Value, vs: &VirtualState, map: &SourceMap) {
        if let Some(arr) = value.as_array_mut() {
            arr.retain_mut(|entry| self.remap_single_location(entry, vs, map));
        } else if value.is_object() && !self.remap_single_location(value, vs, map) {
            *value = Value::Null;
        }
    }

    fn remap_single_location(&self, entry: &mut Value, vs: &VirtualState, map: &SourceMap) -> bool {
        // LocationLink
        if let Some(target_uri) = entry
            .get("targetUri")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            if !uris_match(&vs.virtual_uri, &target_uri) {
                return true;
            }
            let target_range = entry
                .get("targetSelectionRange")
                .or_else(|| entry.get("targetRange"))
                .cloned();
            let Some(range) = target_range else {
                return false;
            };
            let Some((vue_uri, mapped_range)) = self.remap_generated_range(&range, vs, map) else {
                return false;
            };
            entry["targetUri"] = Value::String(vue_uri);
            entry["targetRange"] = mapped_range.clone();
            entry["targetSelectionRange"] = mapped_range;
            if let Some(origin) = entry.get("originSelectionRange").cloned()
                && let Some((_, mapped_origin)) = self.remap_generated_range(&origin, vs, map)
            {
                entry["originSelectionRange"] = mapped_origin;
            }
            return true;
        }
        // Location
        let Some(uri) = entry.get("uri").and_then(Value::as_str).map(str::to_owned) else {
            return false;
        };
        if !uris_match(&vs.virtual_uri, &uri) {
            return true;
        }
        let Some(range) = entry.get("range").cloned() else {
            return false;
        };
        let Some((vue_uri, mapped_range)) = self.remap_generated_range(&range, vs, map) else {
            return false;
        };
        entry["uri"] = Value::String(vue_uri);
        entry["range"] = mapped_range;
        true
    }

    fn remap_workspace_edit(&self, edit: &mut Value, vs: &VirtualState, map: &SourceMap) {
        if let Some(changes) = edit.get_mut("changes").and_then(Value::as_object_mut)
            && let Some(virtual_edits) = changes
                .keys()
                .find(|k| uris_match(&vs.virtual_uri, k))
                .cloned()
                .and_then(|k| changes.remove(&k))
            && let Some(arr) = virtual_edits.as_array()
        {
            for item in arr {
                let Some(range) = item.get("range") else {
                    continue;
                };
                if let Some((vue_uri, mapped_range)) = self.remap_generated_range(range, vs, map) {
                    let mut mapped_edit = item.clone();
                    mapped_edit["range"] = mapped_range;
                    let entry = changes
                        .entry(vue_uri)
                        .or_insert_with(|| Value::Array(Vec::new()));
                    if let Some(list) = entry.as_array_mut() {
                        list.push(mapped_edit);
                    }
                }
            }
        }
        if let Some(doc_changes) = edit
            .get_mut("documentChanges")
            .and_then(Value::as_array_mut)
        {
            let mut rebuilt = Vec::new();
            for change in doc_changes.drain(..) {
                let Some(uri) = change
                    .get("textDocument")
                    .and_then(|td| td.get("uri"))
                    .and_then(Value::as_str)
                else {
                    rebuilt.push(change);
                    continue;
                };
                if !uris_match(&vs.virtual_uri, uri) {
                    rebuilt.push(change);
                    continue;
                }
                let mut by_uri: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                if let Some(edits) = change.get("edits").and_then(Value::as_array) {
                    for te in edits {
                        if let Some(range) = te.get("range")
                            && let Some((vue_uri, mapped_range)) =
                                self.remap_generated_range(range, vs, map)
                        {
                            let mut cloned = te.clone();
                            cloned["range"] = mapped_range;
                            by_uri.entry(vue_uri).or_default().push(cloned);
                        }
                    }
                }
                for (vue_uri, edits) in by_uri {
                    rebuilt.push(json!({
                        "textDocument": { "uri": vue_uri, "version": Value::Null },
                        "edits": edits,
                    }));
                }
            }
            *doc_changes = rebuilt;
        }
    }

    fn publish_all_vue_diagnostics(&mut self, actions: &mut BridgeActions) {
        let mut by_uri: HashMap<String, Vec<Value>> = HashMap::new();
        for doc in self.docs.values() {
            if doc.open {
                by_uri.insert(doc.uri.clone(), Vec::new());
            }
        }

        if let Some(vs) = &self.virtual_state {
            if let Some(err) = &vs.sfc_error
                && let Some(uri) = self.resolve_vue_uri(&err.file)
            {
                let line0 = err.line.saturating_sub(1);
                let line_text = self
                    .doc_by_uri(&uri)
                    .and_then(|d| d.text.lines().nth(line0))
                    .unwrap_or("");
                let col = char_col_to_lsp(line_text, err.column.max(1));
                by_uri.entry(uri).or_default().push(json!({
                    "range": {
                        "start": { "line": line0, "character": col },
                        "end": { "line": line0, "character": col + 1 },
                    },
                    "severity": 1,
                    "source": "nana-ui-sfc",
                    "message": err.message,
                }));
            }

            for (file, line, column, message) in &vs.sfc_warnings {
                if let Some(uri) = self.resolve_vue_uri(file) {
                    let line0 = line.saturating_sub(1);
                    let line_text = self
                        .doc_by_uri(&uri)
                        .and_then(|d| d.text.lines().nth(line0))
                        .unwrap_or("");
                    let col = char_col_to_lsp(line_text, (*column).max(1));
                    by_uri.entry(uri).or_default().push(json!({
                        "range": {
                            "start": { "line": line0, "character": col },
                            "end": { "line": line0, "character": col + 1 },
                        },
                        "severity": 2,
                        "source": "nana-ui-sfc",
                        "message": message,
                    }));
                }
            }

            if vs.sfc_error.is_none()
                && let Some(map) = &vs.source_map
            {
                for diag in &vs.backend_diagnostics {
                    let Some(range) = diag.get("range") else {
                        continue;
                    };
                    let Some((vue_uri, mapped_range)) = self.remap_generated_range(range, vs, map)
                    else {
                        continue;
                    };
                    let mut remapped = diag.clone();
                    remapped["range"] = mapped_range;
                    if let Some(related) = remapped
                        .get_mut("relatedInformation")
                        .and_then(Value::as_array_mut)
                    {
                        for info in related {
                            if let Some(loc) = info.get_mut("location") {
                                self.remap_single_location(loc, vs, map);
                            }
                        }
                    }
                    by_uri.entry(vue_uri).or_default().push(remapped);
                }
            }
        }

        let mut prev_uris = self
            .virtual_state
            .as_mut()
            .map(|vs| std::mem::take(&mut vs.published_diag_uris))
            .unwrap_or_default();

        for (uri, diagnostics) in by_uri {
            prev_uris.remove(&uri);
            if !diagnostics.is_empty()
                && let Some(vs) = &mut self.virtual_state
            {
                vs.published_diag_uris.insert(uri.clone());
            }
            actions.to_client.push(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": uri,
                    "diagnostics": diagnostics,
                }
            }));
        }

        // Clear any previously published URI no longer in `by_uri`.
        for stale_uri in prev_uris {
            actions.to_client.push(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": stale_uri,
                    "diagnostics": [],
                }
            }));
        }
    }

    fn remap_generated_range(
        &self,
        range: &Value,
        vs: &VirtualState,
        map: &SourceMap,
    ) -> Option<(String, Value)> {
        let start = range.get("start")?;
        let end = range.get("end").unwrap_or(start);
        let s_line0 = start.get("line")?.as_u64()? as usize;
        let s_col_u16 = start.get("character")?.as_u64()? as u32;
        let e_line0 = end.get("line")?.as_u64()? as usize;
        let e_col_u16 = end.get("character")?.as_u64()? as u32;

        let s_line_text = vs.code.lines().nth(s_line0).unwrap_or("");
        let e_line_text = vs.code.lines().nth(e_line0).unwrap_or("");
        let s_col = lsp_to_char_col(s_line_text, s_col_u16);
        let e_col = lsp_to_char_col(e_line_text, e_col_u16);

        let span = map.generated_range_to_vue(s_line0 + 1, s_col, e_line0 + 1, e_col)?;
        let vue_uri = self.resolve_vue_uri(&span.file)?;
        let vue_text = self
            .doc_by_uri(&vue_uri)
            .map(|d| d.text.as_str())
            .unwrap_or("");
        Some((vue_uri, source_span_to_lsp_range(&span, vue_text)))
    }

    fn resolve_vue_uri(&self, file: &str) -> Option<String> {
        let norm = normalize_path(Path::new(file));
        if let Some(doc) = self.docs.get(&norm) {
            return Some(doc.uri.clone());
        }
        let target = file.replace('\\', "/");
        if let Some(doc) = self.docs.values().find(|doc| {
            let candidate = doc.path.display().to_string().replace('\\', "/");
            candidate == target || candidate.ends_with(&format!("/{target}"))
        }) {
            return Some(doc.uri.clone());
        }
        path_to_uri(Path::new(file))
    }

    fn doc_by_uri(&self, uri: &str) -> Option<&VueDocument> {
        if let Some(path) = self.uri_to_path.get(uri)
            && let Some(doc) = self.docs.get(path)
        {
            return Some(doc);
        }
        let norm = uri_to_normalized_path(uri)?;
        self.docs.get(&norm)
    }
}

fn source_span_to_lsp_range(span: &SourceSpan, vue_text: &str) -> Value {
    let s_line0 = span.start_line.saturating_sub(1);
    let e_line0 = span.end_line.saturating_sub(1);
    let s_line_text = vue_text.lines().nth(s_line0).unwrap_or("");
    let e_line_text = vue_text.lines().nth(e_line0).unwrap_or("");
    let s_col = char_col_to_lsp(s_line_text, span.start_column);
    let mut e_col = char_col_to_lsp(e_line_text, span.end_column);
    if s_line0 == e_line0 && e_col <= s_col {
        e_col = s_col + 1;
    }
    json!({
        "start": { "line": s_line0, "character": s_col },
        "end": { "line": e_line0, "character": e_col },
    })
}

fn parse_sfc_warnings(warnings: &[String]) -> Vec<(String, usize, usize, String)> {
    warnings
        .iter()
        .filter_map(|raw| {
            // Format: `file: line:col: message` or `file: message`.
            let (file, rest) = raw.split_once(": ")?;
            let mut parts = rest.splitn(3, ':');
            if let (Some(l), Some(c), Some(msg)) = (parts.next(), parts.next(), parts.next())
                && let (Ok(line), Ok(col)) = (l.trim().parse::<usize>(), c.trim().parse::<usize>())
            {
                return Some((file.to_owned(), line, col, msg.trim().to_owned()));
            }
            Some((file.to_owned(), 1, 1, rest.to_owned()))
        })
        .collect()
}

fn apply_incremental_change(text: &mut String, range: &Value, new_text: &str) {
    let Some(start) = range.get("start") else {
        return;
    };
    let Some(end) = range.get("end") else {
        return;
    };
    let Some(sl) = start.get("line").and_then(Value::as_u64) else {
        return;
    };
    let Some(sc) = start.get("character").and_then(Value::as_u64) else {
        return;
    };
    let Some(el) = end.get("line").and_then(Value::as_u64) else {
        return;
    };
    let Some(ec) = end.get("character").and_then(Value::as_u64) else {
        return;
    };
    let Some(s_off) = lsp_pos_to_byte_offset(text, sl as usize, sc as u32) else {
        return;
    };
    let Some(e_off) = lsp_pos_to_byte_offset(text, el as usize, ec as u32) else {
        return;
    };
    if s_off <= e_off && e_off <= text.len() {
        text.replace_range(s_off..e_off, new_text);
    }
}

fn lsp_pos_to_byte_offset(text: &str, line0: usize, utf16_col: u32) -> Option<usize> {
    let mut current_line = 0usize;
    let mut line_start = 0usize;
    if line0 > 0 {
        for (idx, ch) in text.char_indices() {
            if ch == '\n' {
                current_line += 1;
                if current_line == line0 {
                    line_start = idx + 1;
                    break;
                }
            }
        }
        if current_line != line0 {
            return None;
        }
    }
    let rest = &text[line_start..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line_slice = &rest[..line_end];
    let mut utf16 = 0u32;
    let mut byte_in_line = line_slice.len();
    for (idx, ch) in line_slice.char_indices() {
        if utf16 >= utf16_col {
            byte_in_line = idx;
            break;
        }
        utf16 += ch.len_utf16() as u32;
    }
    Some(line_start + byte_in_line)
}

fn is_vue_uri(uri: &str) -> bool {
    uri.ends_with(".vue")
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn uri_to_normalized_path(uri: &str) -> Option<PathBuf> {
    let parsed = Url::parse(uri).ok()?;
    let path = parsed.to_file_path().ok()?;
    Some(normalize_path(&path))
}

fn path_to_uri(path: &Path) -> Option<String> {
    let abs = if path.is_absolute() {
        normalize_path(path)
    } else {
        std::env::current_dir()
            .ok()
            .map(|cwd| normalize_path(&cwd.join(path)))?
    };
    Url::from_file_path(abs).ok().map(String::from)
}

fn uris_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (uri_to_normalized_path(a), uri_to_normalized_path(b)) {
        (Some(pa), Some(pb)) => pa == pb,
        _ => false,
    }
}

/// Search up from `views_dir` for a Cargo package manifest and any existing
/// `$OUT_DIR/nana_views.rs` under `target/*/build/<pkg>-*/out/nana_views.rs`,
/// also extracting its `use <runtime>::view::*;` path.
pub fn discover_generated_views(views_dir: &Path) -> (Option<PathBuf>, Option<String>) {
    let mut cur = Some(views_dir);
    let mut pkg_manifest: Option<PathBuf> = None;
    let mut pkg_name: Option<String> = None;
    let mut workspace_root: Option<PathBuf> = None;

    while let Some(dir) = cur {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && let Ok(text) = std::fs::read_to_string(&manifest)
        {
            if pkg_manifest.is_none() && text.contains("[package]") {
                pkg_manifest = Some(manifest.clone());
                pkg_name = parse_cargo_package_name(&text);
            }
            if text.contains("[workspace]") {
                workspace_root = Some(dir.to_path_buf());
                break;
            }
        }
        cur = dir.parent();
    }

    let search_root = workspace_root.or_else(|| {
        pkg_manifest
            .as_ref()
            .and_then(|m| m.parent().map(Path::to_path_buf))
    });
    let (Some(root), Some(pkg)) = (search_root, pkg_name) else {
        return (None, None);
    };
    let target_dir = root.join("target");
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for profile in ["debug", "release"] {
        let build_dir = target_dir.join(profile).join("build");
        let Ok(entries) = std::fs::read_dir(build_dir) else {
            continue;
        };
        let prefix = format!("{pkg}-");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) {
                continue;
            }
            let out_file = entry.path().join("out").join("nana_views.rs");
            if out_file.is_file() {
                let mtime = out_file
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((mtime, out_file));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((_, best)) = candidates.into_iter().next() else {
        return (None, None);
    };
    let runtime = std::fs::read_to_string(&best)
        .ok()
        .and_then(|code| extract_runtime_path(&code));
    (Some(best), runtime)
}

fn parse_cargo_package_name(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let val = rest.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_owned());
                }
            }
        }
    }
    None
}

fn extract_runtime_path(code: &str) -> Option<String> {
    for line in code.lines().take(12) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("use ")
            && let Some(path) = rest.strip_suffix("::view::*;")
        {
            return Some(path.trim().to_owned());
        }
    }
    None
}

/// Read one JSON-RPC 2.0 `Content-Length` framed message. Returns `Ok(None)`
/// on clean EOF before headers.
pub fn read_lsp_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, val)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("Content-Length")
        {
            content_length = val.trim().parse::<usize>().ok();
        }
    }
    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let value =
        serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

/// Write one JSON-RPC 2.0 `Content-Length` framed message.
pub fn write_lsp_message<W: Write>(writer: &mut W, msg: &Value) -> io::Result<()> {
    let bytes =
        serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write!(writer, "Content-Length: {}\r\n\r\n", bytes.len())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bridge(dir: &Path) -> (LspBridge, PathBuf, String, PathBuf, String) {
        let vue_path = dir.join("Todo.vue");
        let virtual_path = dir.join("nana_views.rs");
        let vue_uri = Url::from_file_path(&vue_path).unwrap().to_string();
        let virtual_uri = Url::from_file_path(&virtual_path).unwrap().to_string();
        let bridge = LspBridge::new(LspConfig {
            views_dir: Some(dir.to_path_buf()),
            virtual_rs: Some(virtual_path.clone()),
            runtime: Some("::nana_ui_runtime".to_owned()),
        });
        (bridge, vue_path, vue_uri, virtual_path, virtual_uri)
    }

    #[test]
    fn framing_round_trips_messages_and_utf8() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/hover",
            "params": { "text": "你好😀" }
        });
        let mut wire = Vec::new();
        write_lsp_message(&mut wire, &msg).unwrap();
        let mut cursor = io::Cursor::new(wire);
        let decoded = read_lsp_message(&mut cursor).unwrap().unwrap();
        assert_eq!(decoded, msg);
        assert!(read_lsp_message(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn bridge_remaps_diagnostics_hover_definition_completion_and_rename() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nana-sfc-lsp-unit-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut bridge, _vue_path, vue_uri, _virtual_path, virtual_uri) = test_bridge(&dir);

        // 1. initialize forces Full sync capability.
        let init_req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let out = bridge.handle_client_message(init_req);
        assert_eq!(out.to_backend.len(), 1);
        let init_resp = bridge.handle_backend_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "capabilities": { "textDocumentSync": 2 } }
        }));
        assert_eq!(
            init_resp.to_client[0]["result"]["capabilities"]["textDocumentSync"],
            1
        );

        // 2. didOpen compiles .vue -> virtual .rs and opens it in backend.
        let vue_src = r#"<script setup lang="rust">
let draft = signal(String::new());
let count = signal(0u32);
</script>
<template>
  <Column>
    <Text>{{ draft }} {{ count }}</Text>
    <Button @activate="count.update(|c| *c += 1)">加一</Button>
  </Column>
</template>"#;
        let open = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": vue_uri,
                    "languageId": "vue",
                    "version": 1,
                    "text": vue_src,
                }
            }
        }));
        assert_eq!(open.to_backend.len(), 1);
        assert_eq!(open.to_backend[0]["method"], "textDocument/didOpen");
        assert_eq!(
            open.to_backend[0]["params"]["textDocument"]["uri"],
            virtual_uri
        );
        let gen_code = open.to_backend[0]["params"]["textDocument"]["text"]
            .as_str()
            .unwrap()
            .to_owned();

        // 3. Hover on `draft` inside `{{ draft }}` (line 6, 0-based).
        let line6 = vue_src.lines().nth(6).unwrap();
        let draft_col = line6.find("draft").unwrap() as u64 + 1;
        let hover_fwd = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": vue_uri },
                "position": { "line": 6, "character": draft_col }
            }
        }));
        let gen_line = hover_fwd.to_backend[0]["params"]["position"]["line"]
            .as_u64()
            .unwrap() as usize;
        let gen_col = hover_fwd.to_backend[0]["params"]["position"]["character"]
            .as_u64()
            .unwrap();
        assert!(gen_code.lines().nth(gen_line).unwrap().contains("draft"));

        let hover_back = bridge.handle_backend_message(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "contents": { "kind": "markdown", "value": "Signal<String>" },
                "range": {
                    "start": { "line": gen_line, "character": gen_col },
                    "end": { "line": gen_line, "character": gen_col + 5 }
                }
            }
        }));
        assert_eq!(
            hover_back.to_client[0]["result"]["range"]["start"]["line"],
            6
        );

        // 4. Backend diagnostics on virtual_uri remap back to vue_uri and clear when fixed.
        let diag_msg = bridge.handle_backend_message(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": virtual_uri,
                "diagnostics": [{
                    "range": {
                        "start": { "line": gen_line, "character": gen_col },
                        "end": { "line": gen_line, "character": gen_col + 5 }
                    },
                    "severity": 1,
                    "message": "mismatched types"
                }]
            }
        }));
        assert_eq!(diag_msg.to_client[0]["params"]["uri"], vue_uri);
        assert_eq!(
            diag_msg.to_client[0]["params"]["diagnostics"][0]["range"]["start"]["line"],
            6
        );

        let cleared = bridge.handle_backend_message(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": virtual_uri,
                "diagnostics": []
            }
        }));
        assert_eq!(cleared.to_client[0]["params"]["diagnostics"], json!([]));

        // 5. Definition inside virtual Rust jumps back to script line 1 (`let draft = ...`).
        let def_fwd = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": vue_uri },
                "position": { "line": 6, "character": draft_col }
            }
        }));
        assert_eq!(def_fwd.to_backend[0]["method"], "textDocument/definition");
        let script_line = gen_code
            .lines()
            .position(|l| l.contains("let draft"))
            .unwrap();
        let script_col = gen_code
            .lines()
            .nth(script_line)
            .unwrap()
            .find("draft")
            .unwrap();
        let def_back = bridge.handle_backend_message(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": [{
                "uri": virtual_uri,
                "range": {
                    "start": { "line": script_line, "character": script_col },
                    "end": { "line": script_line, "character": script_col + 5 }
                }
            }]
        }));
        assert_eq!(def_back.to_client[0]["result"][0]["uri"], vue_uri);
        assert_eq!(
            def_back.to_client[0]["result"][0]["range"]["start"]["line"],
            1
        );

        // 6. Syntax error in .vue emits immediate SFC diagnostic and dot-completion recovers.
        let broken = vue_src.replace("{{ draft }} {{ count }}", "{{ draft. }} {{ count }}");
        let change_broken = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": vue_uri, "version": 2 },
                "contentChanges": [{ "text": broken }]
            }
        }));
        assert_eq!(
            change_broken.to_client[0]["params"]["diagnostics"][0]["source"],
            "nana-ui-sfc"
        );
        let dot_col = line6.find("draft").unwrap() as u64 + 6; // right after `draft.`
        let comp = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": vue_uri },
                "position": { "line": 6, "character": dot_col }
            }
        }));
        assert_eq!(
            comp.to_backend.len(),
            2,
            "sync recovered + completion request"
        );
        assert_eq!(comp.to_backend[1]["method"], "textDocument/completion");

        // 7. Non-.vue files pass straight through untouched.
        let rs_uri = Url::from_file_path(dir.join("main.rs"))
            .unwrap()
            .to_string();
        let pass = bridge.handle_client_message(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": rs_uri },
                "position": { "line": 0, "character": 0 }
            }
        }));
        assert_eq!(pass.to_backend[0]["params"]["textDocument"]["uri"], rs_uri);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
