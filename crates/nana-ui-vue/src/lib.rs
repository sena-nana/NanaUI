//! Vue backend host coordination — L1/L2 bridge into Nana Style Model → Iced.
//!
//! ## Three-layer compatibility
//!
//! ```text
//! L1 CSS 子集 ──► Nana Style Model（Tokens + Semantics + Layout）
//! L2 Vue props ─► 同一套 Model
//! L3 Rust API ──► 同一套 Model（nana-ui）
//!                  ▼
//!            唯一绘制：nana-ui widgets
//! ```
//!
//! ## Internal pipeline (adapter only — not a second paint core)
//!
//! ```text
//! Style Model (nana-ui-core::box_layout / style_model / semantics)
//!      ↑ parse/cascade          ↑ measure (pre-paint / css-parity)
//! css_map + css_cascade         measure
//!      ↑ shell hints (nana-* contract; NOT business class harvest)
//! shell_contract  (apply_class_layout_hints; css_map 薄委托)
//!      ↑
//! MessageBridge / tree / renderer
//!      ↓
//! widget_map + layout_map → Semantics
//!      ↓
//! iced_app (+ svg_icon L1 SVG exception) → nana-ui → Iced
//! ```
//!
//! Cascade SoT for `LayoutStyle` is [`MessageBridge`] stylesheet rules.
//! `NanaTreeDocument::stylesheets` is diagnostics-only (count for host ops).
//! Product geometry SoT after paint is iced [`LayoutBoxStore`]; `measure` is
//! the pre-paint fallback + `nana-css-parity` harness. There is no separate
//! synthetic layout branch. See
//! [`docs/css-layout-engine-boundary.md`](../../../docs/css-layout-engine-boundary.md).
//!
//! This crate is the **L1/L2 adapter** (not the paint core):
//! - `css_map` → Layout (`LayoutStyle`) — **neutral** declaration parse
//! - `shell_contract` → documented `nana-*` / utility class → same `LayoutStyle`
//! - `css_cascade` → stylesheet match → same `LayoutStyle`
//! - `measure` → pre-paint / parity boxes (not product paint authority)
//! - `style` → L1 paint value parsing only（不拥有 layout / hit-test）
//! - `widget_map` → Semantics (`WidgetKind` + props)
//! - `layout_map` → Layout direction / Column·Row defaults
//! - `iced_app` → L3 NanaUI widgets (feature `iced-view`)
//! - Theme tiers → Tokens via `nana-ui` / core（arbitrary CSS hex ≠ token factory）
//!
//! Dependency direction:
//! ```text
//! nana-ui-core          （Style Model 合同：Tokens + Semantics + Layout 数据）
//!      ↑
//! nana-ui-vue ──► nana-js-engine ──► (app picks nana-js-quickjs XOR nana-js-v8)
//!      ├────────► renderer / tree     (Custom Renderer hostOps)
//!      ├────────► widget_map / layout_map / css_map / shell_contract / css_cascade / measure
//!      ├────────► MessageBridge                       ← L1+L2 同树
//!      ├────────► iced_app            (→ NanaUI widgets → Iced)  ← L3 绘制
//!      └────────► nana-ui-web-api     ← L1 Web API 兼容（非 WebView）
//! ```
//!
//! See [`docs/vue-nana-renderer-system.md`](../../../docs/vue-nana-renderer-system.md).
//!
//! **All visible UI draws through NanaUI foundations** (layout primitives + base
//! controls and their variants). Vue may compose custom components and drive
//! logic, but those compose Nana kinds — not a separate paint engine.
//! CustomContent / CPU raster paint has been removed. Do not restore Blitz/stylo
//! or open a WebView paint path. L1 SVG/`path` chart handling in `svg_icon` /
//! `iced_app` is a temporary adapter exception — prefer sinking to L3 widgets.
//!
//! Applications choose one JS engine:
//! - `engine-quickjs` → `nana-js-quickjs`
//! - `engine-v8` → `nana-js-v8`
//!
//! Never enable both JS engines in one artifact. Use [`refuse_dual_js_engines`]
//! (or equivalent) at the application crate.
//!
//! Custom Renderer host ops attach through [`nana_js_engine::JsEngine`] only —
//! never via `v8::*` / `rquickjs::*`.

mod app;
mod bridge;
mod capabilities;
mod css_cascade;
mod css_map;
#[cfg(feature = "iced-view")]
pub mod editor_store;
#[cfg(feature = "iced-view")]
pub mod iced_app;
mod layout_map;
mod measure;
#[cfg(feature = "iced-view")]
pub mod menu_store;
mod renderer;
mod scroll;
mod shell_contract;
mod style;
#[cfg(feature = "iced-view")]
mod svg_icon;
mod tree;
mod widget_map;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nana_js_engine::{
    HostApiRegistry, HostValue, JsEngine, JsEngineError, JsFunctionId, RuntimeArtifact,
};
use nana_ui_web_api::{
    SharedWebApiState, compose_runtime_artifact, register_web_api_host_ops, shared_web_api_state,
};

pub use app::{
    MountOptions, NanaVueApp, mount_vue_as_nana, mount_vue_as_nana_with_engine,
    semantic_snapshot_of,
};
pub use bridge::{
    BridgeEvent, MessageBridge, SelectOptionProp, SemanticRegionViews, SemanticSnapshot,
    SemanticWidget, WidgetId, WidgetKind, WidgetProps, parse_button_kind, parse_control_size,
    resolve_kind_from_hints, widget_id,
};
pub use capabilities::{
    Capability, PermissionPolicy, SharedPermissionPolicy, WorkspaceBootstrap, WorkspaceRecord,
    register_capability_host_ops, shared_permission_policy,
};
pub use css_cascade::{
    AnPlusB, AttrCase, AttrOperator, AttrSelector, Combinator, CompoundSelector, DeclarationEntry,
    MatchContext, MatchNode, Selector, SimpleCompound, Specificity, StyleRule,
    apply_stylesheet_to_layout, collect_document_custom_properties_from_rules,
    matched_declaration_entries, matched_declarations, parse_stylesheet, rebuild_layout_style,
};
pub use css_map::{
    AlignSpec, BoxSizing, CssLayoutParse, DisplaySpec, FlexDirection, FlexWrap, FontSizeContext,
    GridAutoFlow, GridTrack, GridTrackListParse, GridTrackListUnsupported, JustifySpec,
    LayoutStyle, LayoutStyleCss, LengthSpec, LineHeightSpec, OverflowSpec, PaddingSpec, ParentBox,
    PositionSpec, collect_document_css_custom_properties, parse_box_edge_length,
    parse_css_font_family, parse_css_font_size, parse_css_font_weight, parse_css_length_px,
    parse_css_letter_spacing, parse_css_line_height, parse_grid_template_columns,
    parse_grid_track_list_result, parse_inset_length, resolve_grid_column_widths,
    resolve_grid_track_sizes, resolve_paint_color,
};
#[cfg(feature = "iced-view")]
pub use editor_store::EditorStore;
#[cfg(feature = "iced-view")]
pub use iced_app::{
    view_semantic_tree, view_semantic_tree_static, view_semantic_tree_static_with_editors,
    view_semantic_tree_static_with_viewport, view_semantic_tree_with_editors,
    view_semantic_tree_with_viewport, writeback_containing_blocks, writeback_iced_layout_boxes,
    writeback_iced_layout_boxes_with_scroll,
};
pub use layout_map::{
    apply_direction_to_kind, apply_display_to_kind, default_layout_for_kind, layout_kind_from_tag,
};
pub use measure::{
    LayoutNode, MeasuredBox, measure_grid_auto_contribution, measure_layout, node_from_css,
};
#[cfg(feature = "iced-view")]
pub use menu_store::MenuStore;
pub use nana_ui_core::ThemeMode;
pub use nana_ui_web_api::{compose_runtime_artifact as compose_vue_artifact, shim_artifact};
pub use renderer::{HostDocs, register_dom_host_ops, register_dom_host_ops_with_bridge};
#[cfg(feature = "iced-view")]
pub use scroll::drain_pending_scroll_tasks;
pub use scroll::{
    ScrollAlign, ScrollIntoViewOptions, ScrollIntoViewResult, ScrollOffset, ScrollOffsetStore,
    is_scroll_container, reapply_scroll_translations, scroll_into_view, scrollable_widget_id,
    set_scroll_offset, shared_scroll_offset_store,
};
pub use style::{is_non_token_css_color, map_css_color_for_tokens, parse_css_color};
pub use tree::{
    BoxSnapshot, DocumentId, DomNodeKind, ElementNamespace, LayoutBox, LayoutBoxStore,
    NanaTreeDocument, NodeHandle, get_layout_box, get_layout_box_from, shared_layout_box_store,
};

/// Build L3 [`nana_ui::ThemeTokens`] from a semantic snapshot + native material flag.
///
/// Applies Appearance `backdrop_*` / `titlebar_follows_sidebar` into region alphas.
#[cfg(feature = "iced-view")]
pub fn theme_tokens_from_snapshot(
    snap: &SemanticSnapshot,
    native_material: bool,
) -> nana_ui::ThemeTokens {
    use nana_ui::ThemeModeExt;
    nana_ui::ThemeTokens::new(snap.theme.colors(), snap.appearance.metrics())
        .with_workspace_corners(snap.appearance.workspace_corners_enabled())
        .with_backdrop(
            native_material,
            snap.appearance.backdrop_target(),
            snap.appearance.backdrop_opacity(),
            snap.appearance.titlebar_follows_sidebar(),
        )
}

/// Host → JS window/document lifecycle events (shim EventTarget).
///
/// Satisfies Lilia `lifecycle.ts` focus refresh (`focus`/`blur`) and
/// `repoRefreshEvents` / launch controllers (`visibilitychange`), plus
/// `window.resize` listeners. Not CSS `position:fixed` / Page Visibility polyfill
/// beyond document.hidden / visibilityState.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowLifecycleEvent {
    Resize { width: f64, height: f64 },
    Focus,
    Blur,
    VisibilityChange { hidden: bool },
}

impl WindowLifecycleEvent {
    fn to_host_value(self) -> HostValue {
        let mut map = BTreeMap::new();
        match self {
            Self::Resize { width, height } => {
                map.insert("type".into(), HostValue::string("resize"));
                map.insert("width".into(), HostValue::Number(width));
                map.insert("height".into(), HostValue::Number(height));
            }
            Self::Focus => {
                map.insert("type".into(), HostValue::string("focus"));
            }
            Self::Blur => {
                map.insert("type".into(), HostValue::string("blur"));
            }
            Self::VisibilityChange { hidden } => {
                map.insert("type".into(), HostValue::string("visibilitychange"));
                map.insert("hidden".into(), HostValue::Bool(hidden));
            }
        }
        HostValue::Object(map)
    }
}

/// Vue host shell: owns the tree document, message bridge, web-api state, and renderer host ops.
#[derive(Debug)]
pub struct VueHost {
    pub theme: ThemeMode,
    document: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    web_api: SharedWebApiState,
    permissions: SharedPermissionPolicy,
    workspace: Arc<Mutex<WorkspaceBootstrap>>,
    fire_event: Option<JsFunctionId>,
    drain_timers: Option<JsFunctionId>,
    apply_theme: Option<JsFunctionId>,
    /// Optional web-api ResizeObserver flush after layout (`__nanaNotifyLayout`).
    notify_layout: Option<JsFunctionId>,
    /// Optional window/document lifecycle pump (`__nanaPumpLifecycle`).
    lifecycle_pump: Option<JsFunctionId>,
    /// Host-owned multi-line editor buffers (L2 Textarea → text_editor::Content).
    #[cfg(feature = "iced-view")]
    editors: EditorStore,
    #[cfg(feature = "iced-view")]
    menus: MenuStore,
}

impl Default for VueHost {
    fn default() -> Self {
        Self::new()
    }
}

impl VueHost {
    pub fn new() -> Self {
        Self::with_viewport(800, 600, 1.0)
    }

    pub fn with_viewport(physical_width: u32, physical_height: u32, scale_factor: f32) -> Self {
        let theme = ThemeMode::Light;
        let document = NanaTreeDocument::new(physical_width, physical_height, scale_factor);
        let mut bridge = MessageBridge::new();
        bridge.set_theme(theme);
        // body/html must exist in the semantic forest so inserts into mountRoot
        // parent correctly (otherwise every top-level node stays an orphan root).
        bridge.ensure_document_roots(document.html_root().0, document.mount_root().0);
        Self {
            theme,
            document: Arc::new(Mutex::new(document)),
            bridge: Arc::new(Mutex::new(bridge)),
            web_api: shared_web_api_state(),
            // Default: workspace read for demos; privileged ops stay denied until granted.
            permissions: shared_permission_policy(PermissionPolicy::with_workspace_read()),
            workspace: Arc::new(Mutex::new(WorkspaceBootstrap::default())),
            fire_event: None,
            drain_timers: None,
            apply_theme: None,
            notify_layout: None,
            lifecycle_pump: None,
            #[cfg(feature = "iced-view")]
            editors: EditorStore::new(),
            #[cfg(feature = "iced-view")]
            menus: MenuStore::new(),
        }
    }

    pub fn permissions(&self) -> SharedPermissionPolicy {
        Arc::clone(&self.permissions)
    }

    pub fn workspace_state(&self) -> Arc<Mutex<WorkspaceBootstrap>> {
        Arc::clone(&self.workspace)
    }

    /// Replace the permission policy (e.g. grant `workspace.switch` for an authorized session).
    pub fn set_permission_policy(&mut self, policy: PermissionPolicy) {
        if let Ok(mut guard) = self.permissions.lock() {
            *guard = policy;
        }
    }

    pub fn document_id(&self) -> DocumentId {
        self.document.lock().expect("vue doc").id()
    }

    pub fn document(&self) -> Arc<Mutex<NanaTreeDocument>> {
        Arc::clone(&self.document)
    }

    pub fn bridge(&self) -> Arc<Mutex<MessageBridge>> {
        Arc::clone(&self.bridge)
    }

    pub fn web_api(&self) -> SharedWebApiState {
        Arc::clone(&self.web_api)
    }

    pub fn mount_root(&self) -> NodeHandle {
        self.document.lock().expect("vue doc").mount_root()
    }

    /// Snapshot of the semantic widget forest for Iced `view`.
    ///
    /// Syncs Appearance backdrop fields from the L1 web-api document state so
    /// ThemeTokens can honor `backdrop_*` / `titlebar_follows_sidebar`.
    /// Also writebacks layout containing blocks from the document viewport so
    /// Fill parent chains feed `style` `%` on the next patch.
    pub fn semantic_snapshot(&self) -> SemanticSnapshot {
        self.sync_appearance_from_document();
        let (logical_w, logical_h) = self.document.lock().expect("vue doc").logical_size();
        let mut bridge = self.bridge.lock().expect("vue bridge");
        bridge.reparent_orphans();
        {
            let mut doc = self.document.lock().expect("vue doc");
            bridge.sync_sidebar_footer_into_document(&mut doc);
        }
        bridge.sync_layout_containing_blocks(ParentBox::from_viewport(logical_w, logical_h));
        bridge.snapshot()
    }

    /// Ensure host-owned [`EditorStore`] buffers exist for every Textarea node.
    #[cfg(feature = "iced-view")]
    pub fn prepare_editors(&mut self) {
        let snap = self.semantic_snapshot();
        for widget in &snap.widgets {
            if widget.kind == WidgetKind::Textarea {
                self.editors.sync_text(widget.id, &widget.props.value);
            }
        }
    }

    #[cfg(feature = "iced-view")]
    pub fn editors(&self) -> &EditorStore {
        &self.editors
    }

    #[cfg(feature = "iced-view")]
    pub fn editors_mut(&mut self) -> &mut EditorStore {
        &mut self.editors
    }

    /// Sync host-owned [`MenuStore`] trees for every ContextMenu node.
    #[cfg(feature = "iced-view")]
    pub fn prepare_menus(&mut self) {
        let snap = self.semantic_snapshot();
        self.menus.sync_from_snapshot(&snap);
    }

    #[cfg(feature = "iced-view")]
    pub fn menus(&self) -> &MenuStore {
        &self.menus
    }

    #[cfg(feature = "iced-view")]
    pub fn menus_mut(&mut self) -> &mut MenuStore {
        &mut self.menus
    }

    /// Latest Appearance settings mirrored from L1 document dataset/style.
    pub fn appearance(&self) -> nana_ui_core::AppearanceSettings {
        self.sync_appearance_from_document();
        self.bridge.lock().expect("vue bridge").appearance()
    }

    fn sync_appearance_from_document(&self) {
        Self::sync_appearance_shared(&self.web_api, &self.document, &self.bridge);
    }

    /// Apply L1 `documentElement` dataset/style into document + bridge.
    ///
    /// Must run on the JS write path (`documentElementSet`), not only on
    /// [`Self::semantic_snapshot`], so theme-conditional `var(--*)` resolve
    /// before the host's next cached snapshot.
    fn sync_appearance_shared(
        web_api: &SharedWebApiState,
        document: &Arc<Mutex<NanaTreeDocument>>,
        bridge: &Arc<Mutex<MessageBridge>>,
    ) {
        let (dataset, style) = {
            let web = web_api.lock().expect("web-api");
            (web.document_dataset().clone(), web.document_style().clone())
        };
        if let Some(theme) = dataset.get("theme")
            && let Ok(mut doc) = document.lock()
        {
            doc.set_document_theme(theme);
        }
        let mut bridge = bridge.lock().expect("vue bridge");
        bridge.apply_document_appearance(&dataset, &style);
    }

    /// Wrap web-api `documentElementSet` so theme/appearance writes immediately
    /// rebuild bridge stylesheet vars (JS Appearance uses `dataset.theme`).
    fn wrap_document_element_set_for_appearance_sync(&self, api: &mut HostApiRegistry) {
        let Some(inner) = api.get("documentElementSet").cloned() else {
            return;
        };
        let web_api = Arc::clone(&self.web_api);
        let document = Arc::clone(&self.document);
        let bridge = Arc::clone(&self.bridge);
        api.register("documentElementSet", move |args| {
            let result = inner(args)?;
            // Appearance / `__nanaApplyTheme` write via shim → documentElementSet.
            // Sync set_theme → rebuild_stylesheet_vars → reapply cascade now.
            Self::sync_appearance_shared(&web_api, &document, &bridge);
            Ok(result)
        });
    }

    /// Inject author CSS onto the cascade SoT ([`MessageBridge`]).
    ///
    /// Also mirrors raw source onto the document for diagnostics
    /// (`stylesheet_count` host op). Cascade / `LayoutStyle` rebuild happens
    /// only in the bridge — never treat `NanaTreeDocument` as a second parser.
    pub fn inject_stylesheet(&self, css: &str) {
        self.document
            .lock()
            .expect("vue doc")
            .inject_stylesheet(css);
        self.bridge
            .lock()
            .expect("vue bridge")
            .inject_stylesheet(css);
    }

    /// Builds a HostApiRegistry with DOM + bridge + web-api + capability-gated host ops.
    pub fn host_api_registry(&self) -> HostApiRegistry {
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&self.document),
            Arc::clone(&self.bridge),
            Arc::clone(&self.web_api),
        );
        register_web_api_host_ops(&mut api, Arc::clone(&self.web_api));
        // JS `documentElement.dataset.theme` must not wait for semantic_snapshot.
        self.wrap_document_element_set_for_appearance_sync(&mut api);
        register_capability_host_ops(
            &mut api,
            Arc::clone(&self.permissions),
            Arc::clone(&self.workspace),
        );
        api
    }

    /// Binds an engine-agnostic JS runtime and installs renderer + web-api host ops.
    pub fn attach_engine<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let api = self.host_api_registry();
        engine.register_host_api(&api)?;
        Ok(())
    }

    /// Initialize engine with web-api shim prepended to `artifact`.
    ///
    /// Binary Release artifacts ([`RuntimeArtifact::is_binary_release`]) must already
    /// include the shim (compile after `compose_runtime_artifact`) and are loaded as-is.
    pub fn initialize_with_web_api<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        artifact: RuntimeArtifact,
    ) -> Result<(), JsEngineError> {
        self.attach_engine(engine)?;
        if artifact.is_binary_release() {
            engine.initialize(artifact)?;
            return Ok(());
        }
        let source = artifact.source_utf8()?;
        let composed = if source.contains("__nanaWebApi") {
            // Already composed / shim already present.
            artifact
        } else {
            compose_runtime_artifact(artifact.name.clone(), source)
        };
        engine.initialize(composed)?;
        Ok(())
    }

    /// Resolve `__nanaFireEvent` / `__nanaDrainTimers` / optional theme + layout + lifecycle hooks after init.
    pub fn bind_event_bridge<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        self.fire_event = Some(engine.resolve_function("__nanaFireEvent")?);
        // Drain helper is optional for Phase 3 Counter (shim may still install it).
        self.drain_timers = engine.resolve_function("__nanaDrainTimers").ok();
        self.apply_theme = engine.resolve_function("__nanaApplyTheme").ok();
        self.notify_layout = engine.resolve_function("__nanaNotifyLayout").ok();
        self.lifecycle_pump = engine.resolve_function("__nanaPumpLifecycle").ok();
        Ok(())
    }

    pub fn set_viewport(&mut self, physical_width: u32, physical_height: u32, scale_factor: f32) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.set_viewport(physical_width, physical_height, scale_factor);
        bridge.resolve_document_layout(&mut doc);
    }

    pub fn resolve_layout(&mut self) {
        // After iced has painted, writeback is authoritative (chrome/scroll offsets).
        let iced = shared_layout_box_store().snapshot();
        if !iced.is_empty() {
            let bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            doc.apply_layout_boxes(&iced);
            reapply_scroll_translations(
                &mut doc,
                &bridge,
                &shared_layout_box_store(),
                &shared_scroll_offset_store(),
            );
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        bridge.resolve_document_layout(&mut doc);
    }

    /// Copy iced paint boxes into the document cache (call after a frame draws).
    ///
    /// `layoutBox` / `getBoundingClientRect` already prefer the live store; this
    /// keeps hit-tests and `snapshot_boxes` aligned with paint.
    pub fn sync_iced_layout_boxes(&mut self) {
        let iced = shared_layout_box_store().snapshot();
        if iced.is_empty() {
            return;
        }
        let bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&iced);
        reapply_scroll_translations(
            &mut doc,
            &bridge,
            &shared_layout_box_store(),
            &shared_scroll_offset_store(),
        );
    }

    /// Shared iced layout writeback buffer (same as probes / `layoutBox`).
    pub fn layout_box_store(&self) -> Arc<LayoutBoxStore> {
        shared_layout_box_store()
    }

    /// Rust → Vue theme inject (bridge + document + web-api + optional `__nanaApplyTheme`).
    ///
    /// Reverse path: JS `dataset.theme` / `setDocumentTheme` →
    /// [`MessageBridge::apply_document_appearance`] immediately
    /// (`documentElementSet` wrap / `setDocumentTheme` host op).
    pub fn inject_theme<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        theme: ThemeMode,
    ) -> Result<(), JsEngineError> {
        self.theme = theme;
        let label = match theme {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        // Same store `sync_appearance_shared` reads — must not lag behind bridge.
        if let Ok(mut web) = self.web_api.lock() {
            web.set_document_dataset("theme", label);
        }
        {
            let mut doc = self.document.lock().expect("vue doc");
            doc.set_document_theme(label);
        }
        {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            bridge.set_theme(theme);
        }
        if let Some(apply) = self.apply_theme {
            engine.invoke(apply, &[HostValue::string(label)])?;
            engine.run_microtasks()?;
        }
        Ok(())
    }

    /// Drain due rAF/timeouts into JS, then microtasks + Style-Model layout.
    ///
    /// After layout resolves, invokes optional `__nanaNotifyLayout` so
    /// `ResizeObserver` callbacks see fresh `layoutBox` geometry.
    ///
    /// Nested drain: Vue runtime-dom `<Transition>` `nextFrame` is double-rAF
    /// (leave/enter → `whenTransitionEnds` → `@after-leave`). One shot would
    /// leave LiliaUI Dialog/Drawer/Dropdown overlay presence hung.
    pub fn pump_frame<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<usize, JsEngineError> {
        let mut fired = 0usize;
        // Cap nested callbacks (Transition nextFrame + ResizeObserver rAF).
        const MAX_TIMER_PASSES: usize = 16;
        for _ in 0..MAX_TIMER_PASSES {
            let due = {
                let mut guard = self
                    .web_api
                    .lock()
                    .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
                guard.due_timers(Instant::now())
            };
            if due.is_empty() {
                break;
            }
            if let Some(drain) = self.drain_timers {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                engine.invoke(drain, &[due.to_host_value(now_ms)])?;
                fired += due.raf.len() + due.timeouts.len() + due.intervals.len();
            }
            engine.run_microtasks()?;
        }
        self.resolve_layout();
        if let Some(notify) = self.notify_layout {
            engine.invoke(notify, &[])?;
            engine.run_microtasks()?;
        }
        Ok(fired)
    }

    /// Pump a host window lifecycle event into the shim EventTarget surface.
    ///
    /// No-op (returns `Ok(false)`) when `__nanaPumpLifecycle` is absent (e.g. Phase 3
    /// counter without web-api shim). After dispatch, runs microtasks so listeners
    /// scheduled via `queueMicrotask` / promises settle.
    pub fn pump_lifecycle<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: WindowLifecycleEvent,
    ) -> Result<bool, JsEngineError> {
        let Some(pump) = self.lifecycle_pump else {
            return Ok(false);
        };
        engine.invoke(pump, &[event.to_host_value()])?;
        engine.run_microtasks()?;
        Ok(true)
    }

    /// Route an Iced widget action into the bridge queue and JS event listeners.
    pub fn dispatch_bridge_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: BridgeEvent,
    ) -> Result<bool, JsEngineError> {
        let id = event.widget_id();
        #[cfg(feature = "iced-view")]
        let editor_text = if let BridgeEvent::Editor { id, action } = &event {
            self.editors.perform(*id, action.clone());
            // Do not acknowledge_bridge here: JS v-model may lag behind host
            // Content. prepare_editors/sync_text clears dirty only when bridge
            // value catches up to host text.
            Some(self.editors.text(*id))
        } else {
            None
        };
        #[cfg(feature = "iced-view")]
        if let BridgeEvent::MenuSearch { id, query } = &event {
            self.menus.set_query(*id, query.clone());
        }
        #[cfg(feature = "iced-view")]
        if let BridgeEvent::MenuPath { id, path } = &event {
            self.menus.set_active_path(*id, path.clone());
        }
        #[cfg(feature = "iced-view")]
        let mut menu_confirm_armed = false;
        #[cfg(feature = "iced-view")]
        if let BridgeEvent::SelectValue { id, value } = &event {
            let is_menu = {
                let bridge = self.bridge.lock().expect("vue bridge");
                bridge
                    .get(*id)
                    .is_some_and(|w| w.kind == WidgetKind::ContextMenu)
            };
            if is_menu && self.menus.arm_danger_confirm(*id, value) {
                menu_confirm_armed = true;
            }
        }
        #[cfg(feature = "iced-view")]
        if let BridgeEvent::Toggle { id, value: false } = &event {
            self.menus.set_pending(*id, None);
        }
        #[cfg(feature = "iced-view")]
        if menu_confirm_armed {
            return Ok(true);
        }
        let js_events = {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            match &event {
                BridgeEvent::Press { id } => bridge.note_press(*id),
                BridgeEvent::Toggle { id, value } => bridge.note_toggle(*id, *value),
                BridgeEvent::Select { id } => bridge.note_select(*id),
                BridgeEvent::SelectValue { id, value } => {
                    bridge.note_select_value(*id, value.clone())
                }
                BridgeEvent::Input { id, value } => bridge.note_input(*id, value.clone()),
                BridgeEvent::Change { id, value } => bridge.note_change(*id, *value),
                #[cfg(feature = "iced-view")]
                BridgeEvent::Editor { id, .. } => {
                    let text = editor_text.clone().unwrap_or_default();
                    bridge.note_input(*id, text)
                }
                #[cfg(feature = "iced-view")]
                BridgeEvent::MenuSearch { .. } | BridgeEvent::MenuPath { .. } => {
                    // Host-only menu chrome; no JS listener required.
                    Vec::new()
                }
            }
        };
        #[cfg(feature = "iced-view")]
        if matches!(
            &event,
            BridgeEvent::MenuSearch { .. } | BridgeEvent::MenuPath { .. }
        ) {
            return Ok(true);
        }
        if js_events.is_empty() {
            return Ok(false);
        }
        let fire = self.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        for name in js_events {
            let mut detail = BTreeMap::new();
            match &event {
                BridgeEvent::Toggle { value, .. } => {
                    detail.insert("value".into(), HostValue::Bool(*value));
                    detail.insert("checked".into(), HostValue::Bool(*value));
                }
                BridgeEvent::SelectValue { value, .. } | BridgeEvent::Input { value, .. } => {
                    detail.insert("value".into(), HostValue::string(value));
                }
                BridgeEvent::Change { value, .. } => {
                    detail.insert("value".into(), HostValue::Number(*value));
                }
                #[cfg(feature = "iced-view")]
                BridgeEvent::Editor { .. } => {
                    if let Some(text) = &editor_text {
                        detail.insert("value".into(), HostValue::string(text));
                    }
                }
                _ => {}
            }
            engine.invoke(
                fire,
                &[
                    HostValue::Number(id as f64),
                    HostValue::string(name),
                    HostValue::Object(detail),
                ],
            )?;
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        // Drop drained duplicates from note_* (host already consumed the intent).
        let _ = self.bridge.lock().expect("vue bridge").drain_events();
        Ok(true)
    }

    /// Hit-test a click in logical CSS pixels and invoke the JS event bridge.
    ///
    /// Always fans out `pointerdown` (hit or miss) so Lilia `useDismissableLayer` /
    /// ContextMenu `window` capture listeners can close overlays on outside click.
    pub fn pointer_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
    ) -> Result<bool, JsEngineError> {
        let (click_target, focus_target, fan_target) = {
            let mut doc = self.document.lock().expect("vue doc");
            let hit = doc.hit_test(x, y);
            let click_target = doc.hit_event_target(x, y, "click");
            let focus_target = hit.and_then(|h| {
                let mut walk = Some(h);
                while let Some(cur) = walk {
                    let tag = doc.element_tag(cur).unwrap_or_default();
                    if matches!(
                        tag.as_str(),
                        "input"
                            | "textarea"
                            | "button"
                            | "select"
                            | "a"
                            | "nana-button"
                            | "nana-switch"
                            | "nana-sidebar-row"
                    ) {
                        doc.set_focus(cur);
                        return Some(cur);
                    }
                    walk = doc.parent_node(cur);
                }
                None
            });
            let fan_target = click_target
                .or(focus_target)
                .or(hit)
                .unwrap_or_else(|| doc.mount_root());
            (click_target, focus_target, fan_target)
        };
        let fire = self.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        // Outside-click / ContextMenu capture path (document+window listeners).
        engine.invoke(
            fire,
            &[
                HostValue::Number(fan_target.0 as f64),
                HostValue::string("pointerdown"),
                HostValue::Object(Default::default()),
            ],
        )?;
        if click_target.is_none() && focus_target.is_none() {
            engine.run_microtasks()?;
            let _ = self.pump_frame(engine)?;
            return Ok(true);
        }
        if let Some(handle) = focus_target {
            engine.invoke(
                fire,
                &[
                    HostValue::Number(handle.0 as f64),
                    HostValue::string("focus"),
                    HostValue::Object(Default::default()),
                ],
            )?;
        }
        if let Some(handle) = click_target {
            // Prefer semantic press when the target is a registered widget.
            let is_semantic = self.bridge.lock().expect("vue bridge").contains(handle.0);
            if is_semantic {
                let _ = self.dispatch_bridge_event(engine, BridgeEvent::Press { id: handle.0 })?;
            } else {
                engine.invoke(
                    fire,
                    &[
                        HostValue::Number(handle.0 as f64),
                        HostValue::string("click"),
                        HostValue::Object(Default::default()),
                    ],
                )?;
                engine.run_microtasks()?;
                let _ = self.pump_frame(engine)?;
            }
        }
        Ok(true)
    }

    /// Hit-test a wheel gesture in logical CSS pixels and fire `wheel` on the target.
    pub fn pointer_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            doc.hit_event_target(x, y, "wheel")
                .or_else(|| doc.hit_test(x, y))
        };
        let Some(handle) = target else {
            return Ok(false);
        };
        let fire = self.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        let mut detail = BTreeMap::new();
        detail.insert("deltaX".into(), HostValue::Number(delta_x as f64));
        detail.insert("deltaY".into(), HostValue::Number(delta_y as f64));
        engine.invoke(
            fire,
            &[
                HostValue::Number(handle.0 as f64),
                HostValue::string("wheel"),
                HostValue::Object(detail),
            ],
        )?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    /// Dispatch a keyboard event to the focused element (or `target` override).
    ///
    /// Falls back to `body` (`mount_root`) so Escape reaches document/window
    /// fan-out even when nothing is focused (Lilia ContextMenu / dismiss layers).
    pub fn dispatch_key<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        key: &str,
        code: &str,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let handle = {
            let doc = self.document.lock().expect("vue doc");
            target
                .or_else(|| doc.focused())
                .unwrap_or_else(|| doc.mount_root())
        };
        let fire = self.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        let mut detail = BTreeMap::new();
        detail.insert("key".into(), HostValue::string(key));
        detail.insert("code".into(), HostValue::string(code));
        engine.invoke(
            fire,
            &[
                HostValue::Number(handle.0 as f64),
                HostValue::string("keydown"),
                HostValue::Object(detail.clone()),
            ],
        )?;
        // Printable input → also fire `input` for v-model (only when a real focus target).
        let focused_for_input = {
            let doc = self.document.lock().expect("vue doc");
            target.or_else(|| doc.focused())
        };
        if let Some(handle) = focused_for_input
            && key.chars().count() == 1
            && !key.chars().next().is_some_and(|c| c.is_control())
        {
            let mut input_detail = BTreeMap::new();
            input_detail.insert("data".into(), HostValue::string(key));
            input_detail.insert("value".into(), HostValue::string(key));
            // Append into the attribute value when possible.
            {
                let mut doc = self.document.lock().expect("vue doc");
                let prev = doc.get_attribute(handle, "value").unwrap_or_default();
                let next = format!("{prev}{key}");
                doc.set_attribute(handle, "value", &next);
                input_detail.insert("value".into(), HostValue::string(next));
            }
            engine.invoke(
                fire,
                &[
                    HostValue::Number(handle.0 as f64),
                    HostValue::string("input"),
                    HostValue::Object(input_detail),
                ],
            )?;
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    pub fn focused(&self) -> Option<NodeHandle> {
        self.document.lock().expect("vue doc").focused()
    }
}

/// Measure the semantic forest with the Style-Model layout subset and map boxes
/// onto tree node handles (`WidgetId` ≡ `NodeHandle`).
fn measure_bridge_layout_boxes(
    bridge: &MessageBridge,
    viewport_w: f32,
    viewport_h: f32,
) -> Vec<(NodeHandle, LayoutBox)> {
    fn to_node(bridge: &MessageBridge, id: WidgetId) -> Option<LayoutNode> {
        let widget = bridge.get(id)?;
        let children = widget
            .children
            .iter()
            .filter_map(|&child| to_node(bridge, child))
            .collect::<Vec<_>>();
        Some(LayoutNode::with_children(
            id.to_string(),
            widget.props.layout.clone(),
            children,
        ))
    }

    let Some(root_id) = bridge.root_ids().first().copied() else {
        return Vec::new();
    };
    let Some(root) = to_node(bridge, root_id) else {
        return Vec::new();
    };
    measure_layout(&root, viewport_w, viewport_h)
        .into_iter()
        .filter_map(|(id, measured)| {
            let id: u64 = id.parse().ok()?;
            let handle = NodeHandle(id);
            Some((
                handle,
                LayoutBox {
                    handle,
                    x: measured.x,
                    y: measured.y,
                    width: measured.width,
                    height: measured.height,
                },
            ))
        })
        .collect()
}

/// Expand in application crates that expose both `engine-quickjs` and `engine-v8` features.
#[macro_export]
macro_rules! refuse_dual_js_engines {
    () => {
        #[cfg(all(feature = "engine-quickjs", feature = "engine-v8"))]
        compile_error!(
            "nana-js-quickjs and nana-js-v8 are mutually exclusive; enable only one JS engine"
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_element_set_theme_rebuilds_bg_before_snapshot() {
        // Bugbot: JS Appearance writes dataset.theme via documentElementSet;
        // bridge stylesheet vars must rebuild immediately, not wait for
        // semantic_snapshot (hosts may cache the last snap).
        let host = VueHost::new();
        {
            let bridge_arc = host.bridge();
            let mut bridge = bridge_arc.lock().expect("bridge");
            bridge.register(
                1,
                WidgetKind::Column,
                WidgetProps {
                    class_names: vec!["surface".into()],
                    ..WidgetProps::default()
                },
            );
            bridge.inject_stylesheet(
                r#"
                :root { --bg: #181818; }
                :root[data-theme="light"] { --bg: #ffffff; }
                .surface { background: var(--bg); width: 100px; height: 40px; }
                "#,
            );
            let light_bg = bridge.get(1).expect("widget").props.layout.background;
            assert_eq!(
                light_bg,
                Some([1.0, 1.0, 1.0, 1.0]),
                "default ThemeMode::Light must resolve light --bg"
            );
        }

        let api = host.host_api_registry();
        api.call(
            "documentElementSet",
            &[
                HostValue::string("dataset"),
                HostValue::string("theme"),
                HostValue::string("dark"),
            ],
        )
        .expect("documentElementSet theme");

        // Assert *before* semantic_snapshot — the whole point of the fix.
        {
            let bridge_arc = host.bridge();
            let bridge = bridge_arc.lock().expect("bridge");
            assert_eq!(bridge.theme(), ThemeMode::Dark);
            let dark_bg = bridge.get(1).expect("widget").props.layout.background;
            assert_eq!(
                dark_bg,
                Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
                "JS dataset.theme must rebuild var(--bg) before next semantic_snapshot"
            );
        }

        let snap = host.semantic_snapshot();
        assert_eq!(snap.theme, ThemeMode::Dark);
    }

    #[test]
    fn set_document_theme_survives_appearance_sync() {
        // Bugbot: setDocumentTheme must write web-api dataset.theme; otherwise
        // semantic_snapshot / appearance → sync_appearance_shared re-applies a
        // stale theme and reverts var(--*).
        let host = VueHost::new();
        {
            let web_api = host.web_api();
            let mut web = web_api.lock().expect("web-api");
            web.set_document_dataset("theme", "light");
        }
        {
            let bridge_arc = host.bridge();
            let mut bridge = bridge_arc.lock().expect("bridge");
            bridge.register(
                1,
                WidgetKind::Column,
                WidgetProps {
                    class_names: vec!["surface".into()],
                    ..WidgetProps::default()
                },
            );
            bridge.inject_stylesheet(
                r#"
                :root { --bg: #181818; }
                :root[data-theme="light"] { --bg: #ffffff; }
                .surface { background: var(--bg); width: 100px; height: 40px; }
                "#,
            );
        }

        let api = host.host_api_registry();
        api.call("setDocumentTheme", &[HostValue::string("dark")])
            .expect("setDocumentTheme");

        {
            let web_api = host.web_api();
            let web = web_api.lock().expect("web-api");
            assert_eq!(
                web.document_dataset().get("theme").map(String::as_str),
                Some("dark"),
                "setDocumentTheme must mirror into web-api dataset.theme"
            );
        }
        {
            let bridge_arc = host.bridge();
            let bridge = bridge_arc.lock().expect("bridge");
            assert_eq!(bridge.theme(), ThemeMode::Dark);
            let dark_bg = bridge.get(1).expect("widget").props.layout.background;
            assert_eq!(
                dark_bg,
                Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
                "setDocumentTheme must rebuild var(--bg) immediately"
            );
        }

        // Snapshot / appearance sync must not revert to the prior web-api theme.
        let snap = host.semantic_snapshot();
        assert_eq!(snap.theme, ThemeMode::Dark);
        let _ = host.appearance();
        {
            let bridge_arc = host.bridge();
            let bridge = bridge_arc.lock().expect("bridge");
            assert_eq!(bridge.theme(), ThemeMode::Dark);
            let dark_bg = bridge.get(1).expect("widget").props.layout.background;
            assert_eq!(
                dark_bg,
                Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
                "sync_appearance_shared must not revert theme after setDocumentTheme"
            );
        }
    }
}
