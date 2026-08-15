#![recursion_limit = "256"]

//! Vue backend host coordination — first-class L1/L2 consumer of Runtime/UiScene.
//!
//! ## Three-layer compatibility
//!
//! ```text
//! L1 CSS 子集 ──► Nana Style Model（Tokens + Semantics + Layout）
//! L2 Vue props ─► 同一套 Model
//! L3 Rust API ──► 同一套 Model（nana-ui 适配器）
//!                  ▼
//!            保留权威：UiWorld / UiScene
//!            兼容绘制：iced_app → nana-ui → Iced
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
//! Retained geometry lives in UiWorld/UiScene; iced [`LayoutBoxStore`] is the
//! compatibility view after paint. `measure` is the pre-paint fallback +
//! `nana-css-parity` harness. There is no separate synthetic layout branch. See
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
//! - `iced_app` → Iced compatibility view of Runtime/Scene (feature `iced-view`)
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
//!      ├────────► iced_app            (Iced compatibility view of Runtime/Scene)
//!      └────────► nana-ui-web-api     ← L1 Web API 兼容（非 WebView）
//! ```
//!
//! See [`docs/vue-nana-renderer-system.md`](../../../docs/vue-nana-renderer-system.md).
//!
//! Unique retained authority is UiWorld/UiScene. `iced_app` (feature `iced-view`)
//! is the Iced compatibility view of that Scene, including Runtime Scene leaves.
//! WebView is not the product UI path. L1 SVG/`path` handling in `svg_icon` /
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
#[cfg(feature = "hosted")]
mod canvas_gpu;
mod css_cascade;
mod css_map;
#[cfg(feature = "iced-view")]
pub mod editor_store;
#[cfg(feature = "hosted")]
mod hosted_adapter;
#[cfg(feature = "iced-view")]
pub mod iced_app;
mod input;
mod layout_map;
mod measure;
#[cfg(feature = "iced-view")]
pub mod menu_store;
mod multi_window;
#[cfg(feature = "iced-view")]
mod native_component;
mod renderer;
#[cfg(feature = "iced-view")]
mod runtime_text;
mod scroll;
mod shell_contract;
mod style;
#[cfg(feature = "iced-view")]
mod svg_icon;
mod tree;
#[cfg(feature = "hosted")]
mod webgpu;
mod widget_map;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nana_js_engine::{
    HostApiRegistry, HostCallObserver, HostValue, JsDiagnosticEvent, JsDiagnosticLevel,
    JsDiagnosticSink, JsEngine, JsEngineError, JsFunctionId, RuntimeArtifact,
};
#[cfg(feature = "iced-view")]
use nana_ui::{HostTexture, HostTextureAlphaMode, HostTextureRegistry};
pub use nana_ui_platform::ImeEvent;
use nana_ui_runtime::TextInputState;
use nana_ui_web_api::{
    SharedCanvasRuntime, SharedWebApiState, compose_runtime_artifact, default_shared_clipboard,
    register_web_api_host_ops_with_resources, shared_canvas_runtime, shared_web_api_state,
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
#[cfg(feature = "hosted")]
pub use hosted_adapter::{VueHostedProgram, VueHostedRuntime};
#[cfg(feature = "iced-view")]
pub use iced_app::{
    view_semantic_tree, view_semantic_tree_static, view_semantic_tree_static_with_editors,
    view_semantic_tree_static_with_native_components, view_semantic_tree_static_with_resources,
    view_semantic_tree_static_with_viewport, view_semantic_tree_with_editors,
    view_semantic_tree_with_viewport, writeback_containing_blocks, writeback_iced_layout_boxes,
    writeback_iced_layout_boxes_with_scroll,
};
pub use input::{
    CompositionEventKind, CompositionInput, HostedInputResult, InputModifiers, KeyboardEventKind,
    KeyboardInput, PointerEventKind, PointerInput, PointerType, WheelInput,
};
pub use layout_map::{
    apply_direction_to_kind, apply_display_to_kind, default_layout_for_kind, layout_kind_from_tag,
};
pub use measure::{
    LayoutNode, MeasuredBox, measure_grid_auto_contribution, measure_layout, node_from_css,
};
#[cfg(feature = "iced-view")]
pub use menu_store::MenuStore;
pub use multi_window::{
    VueRuntime, VueWindowCommand, VueWindowGeometry, VueWindowId, VueWindowOptions, VueWindowRole,
};
pub use nana_ui_core::ThemeMode;
pub use nana_ui_web_api::{compose_runtime_artifact as compose_vue_artifact, shim_artifact};
#[cfg(feature = "iced-view")]
pub use native_component::{
    NativeComponentCommand, NativeComponentContext, NativeComponentDescriptor,
    NativeComponentFactory, NativeComponentFailure, NativeComponentRegistry, NativePropSchema,
    NativePropType,
};
#[cfg(feature = "iced-view")]
pub use renderer::register_dom_host_ops_with_components;
pub use renderer::{HostDocs, register_dom_host_ops, register_dom_host_ops_with_bridge};
#[cfg(feature = "iced-view")]
pub use runtime_text::IcedTextShaper;
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
    NODE_HANDLE_DOCUMENT_STRIDE, NanaTreeDocument, NodeHandle, get_layout_box, get_layout_box_from,
    shared_layout_box_store,
};
#[cfg(feature = "hosted")]
pub use webgpu::JsWebGpuRuntime;

/// Stable JS-facing descriptor for a host-owned texture. The `slot` is an
/// internal routing key accepted by `<nana-gpu :source="handle">`; callers do
/// not need to manufacture it themselves.
#[cfg(feature = "iced-view")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NanaTextureHandle {
    pub slot: String,
    pub id: u64,
    pub generation: u64,
    pub version: u64,
    pub width: u32,
    pub height: u32,
    pub alpha_mode: HostTextureAlphaMode,
}

#[cfg(feature = "iced-view")]
impl NanaTextureHandle {
    pub fn to_host_value(&self) -> HostValue {
        HostValue::Object(
            [
                ("__nanaTexture".into(), HostValue::Bool(true)),
                ("slot".into(), HostValue::String(self.slot.clone())),
                ("id".into(), HostValue::BigInt(self.id)),
                (
                    "generation".into(),
                    HostValue::Number(self.generation as f64),
                ),
                ("version".into(), HostValue::Number(self.version as f64)),
                ("width".into(), HostValue::Number(self.width as f64)),
                ("height".into(), HostValue::Number(self.height as f64)),
                (
                    "alphaMode".into(),
                    HostValue::String(self.alpha_mode.as_str().into()),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowLifecycleEvent {
    Resize {
        width: f64,
        height: f64,
    },
    ResizeWithScale {
        width: f64,
        height: f64,
        scale_factor: f64,
    },
    Focus,
    Blur,
    VisibilityChange {
        hidden: bool,
    },
}

/// Native file drag lifecycle translated to Vue DOM-style drag events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDragEventKind {
    Hover,
    Drop,
    Cancel,
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
            Self::ResizeWithScale {
                width,
                height,
                scale_factor,
            } => {
                map.insert("type".into(), HostValue::string("resize"));
                map.insert("width".into(), HostValue::Number(width));
                map.insert("height".into(), HostValue::Number(height));
                map.insert("scaleFactor".into(), HostValue::Number(scale_factor));
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
#[derive(Clone, Default)]
struct DiagnosticBindings {
    sink: Option<JsDiagnosticSink>,
    host_calls: Option<HostCallObserver>,
}

impl std::fmt::Debug for DiagnosticBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiagnosticBindings")
            .field("sink", &self.sink.is_some())
            .field("host_calls", &self.host_calls.is_some())
            .finish()
    }
}

#[derive(Debug)]
pub struct VueHost {
    pub theme: ThemeMode,
    document: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    layout_boxes: Arc<LayoutBoxStore>,
    web_api: SharedWebApiState,
    canvas: SharedCanvasRuntime,
    diagnostics: DiagnosticBindings,
    fire_event: Option<JsFunctionId>,
    drain_timers: Option<JsFunctionId>,
    drain_fetch: Option<JsFunctionId>,
    apply_theme: Option<JsFunctionId>,
    /// Optional web-api ResizeObserver flush after layout (`__nanaNotifyLayout`).
    notify_layout: Option<JsFunctionId>,
    /// Optional window/document lifecycle pump (`__nanaPumpLifecycle`).
    lifecycle_pump: Option<JsFunctionId>,
    /// Auxiliary-window identity. `None` preserves the original primary-window
    /// three-argument event bridge.
    event_window_id: Option<u64>,
    input: Arc<Mutex<input::InputState>>,
    file_drag_target: Option<NodeHandle>,
    /// Host-owned multi-line editor buffers (L2 Textarea → text_editor::Content).
    #[cfg(feature = "iced-view")]
    editors: EditorStore,
    #[cfg(feature = "iced-view")]
    menus: MenuStore,
    #[cfg(feature = "iced-view")]
    components: NativeComponentRegistry,
    /// Window-local bindings for host, Canvas, and JS WebGPU textures. Views
    /// are sampled by Iced on the same renderer Device/Queue.
    #[cfg(feature = "iced-view")]
    host_textures: HostTextureRegistry,
    #[cfg(feature = "hosted")]
    webgpu: Option<JsWebGpuRuntime>,
    #[cfg(feature = "hosted")]
    canvas_gpu: Option<canvas_gpu::CanvasGpuBridge>,
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
        Self::with_document_id_and_web_api_state(
            DocumentId(1),
            physical_width,
            physical_height,
            scale_factor,
            shared_web_api_state(),
            shared_canvas_runtime(),
        )
    }

    /// Creates an independent window document inside the same JS runtime.
    pub fn with_document_id(
        id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Self {
        Self::with_document_id_and_web_api_state(
            id,
            physical_width,
            physical_height,
            scale_factor,
            shared_web_api_state(),
            shared_canvas_runtime(),
        )
    }

    fn with_web_api_state(
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        web_api: SharedWebApiState,
    ) -> Self {
        Self::with_document_id_and_web_api_state(
            DocumentId(1),
            physical_width,
            physical_height,
            scale_factor,
            web_api,
            shared_canvas_runtime(),
        )
    }

    pub(crate) fn with_document_id_and_shared_resources(
        document_id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        canvas: SharedCanvasRuntime,
        local_storage: nana_ui_web_api::SharedStorage,
    ) -> Self {
        Self::with_document_id_and_web_api_state(
            document_id,
            physical_width,
            physical_height,
            scale_factor,
            nana_ui_web_api::shared_web_api_state_with_local_storage(local_storage),
            canvas,
        )
    }

    fn with_document_id_and_web_api_state(
        document_id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        web_api: SharedWebApiState,
        canvas: SharedCanvasRuntime,
    ) -> Self {
        let theme = ThemeMode::Light;
        let document =
            NanaTreeDocument::with_id(document_id, physical_width, physical_height, scale_factor);
        let mut bridge = MessageBridge::new();
        bridge.set_theme(theme);
        // body/html must exist in the semantic forest so inserts into mountRoot
        // parent correctly (otherwise every top-level node stays an orphan root).
        bridge.ensure_document_roots(document.html_root().0, document.mount_root().0);
        Self {
            theme,
            document: Arc::new(Mutex::new(document)),
            bridge: Arc::new(Mutex::new(bridge)),
            layout_boxes: Arc::new(LayoutBoxStore::new()),
            web_api,
            canvas,
            diagnostics: DiagnosticBindings::default(),
            fire_event: None,
            drain_timers: None,
            drain_fetch: None,
            apply_theme: None,
            notify_layout: None,
            lifecycle_pump: None,
            event_window_id: None,
            input: Arc::new(Mutex::new(input::InputState::default())),
            file_drag_target: None,
            #[cfg(feature = "iced-view")]
            editors: EditorStore::new(),
            #[cfg(feature = "iced-view")]
            menus: MenuStore::new(),
            #[cfg(feature = "iced-view")]
            components: NativeComponentRegistry::new(),
            #[cfg(feature = "iced-view")]
            host_textures: HostTextureRegistry::new(),
            #[cfg(feature = "hosted")]
            webgpu: None,
            #[cfg(feature = "hosted")]
            canvas_gpu: None,
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

    pub fn canvas_runtime(&self) -> SharedCanvasRuntime {
        Arc::clone(&self.canvas)
    }

    pub fn canvas_runtime_ref(&self) -> &SharedCanvasRuntime {
        &self.canvas
    }

    /// Current compatibility-layer resource counts for development snapshots.
    pub fn resource_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        counts.insert(
            "canvas".into(),
            self.canvas
                .lock()
                .map(|runtime| runtime.active_resource_count())
                .unwrap_or_default(),
        );
        #[cfg(feature = "iced-view")]
        counts.insert("hostTexture".into(), self.host_textures.len());
        #[cfg(feature = "hosted")]
        if let Some(webgpu) = &self.webgpu {
            counts.extend(
                webgpu
                    .resource_counts()
                    .into_iter()
                    .map(|(kind, count)| (format!("webgpu.{kind}"), count)),
            );
        }
        counts
    }

    /// Connect development diagnostics. The sink receives Vue warnings/errors;
    /// the observer receives privacy-preserving Host API timing records.
    pub fn set_diagnostics(
        &mut self,
        sink: Option<JsDiagnosticSink>,
        host_calls: Option<HostCallObserver>,
    ) {
        self.diagnostics = DiagnosticBindings { sink, host_calls };
    }

    #[cfg(feature = "iced-view")]
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }

    #[cfg(feature = "iced-view")]
    pub fn register_host_texture(
        &self,
        slot: impl Into<String>,
        texture: HostTexture,
        width: u32,
        height: u32,
        alpha_mode: HostTextureAlphaMode,
    ) -> NanaTextureHandle {
        let binding = self
            .host_textures
            .register(slot, texture, width, height, alpha_mode);
        self.report_diagnostic(
            "nana.resource",
            JsDiagnosticLevel::Info,
            format!("host texture registered: {}", binding.slot),
            None,
        );
        NanaTextureHandle {
            slot: binding.slot,
            id: binding.texture.id(),
            generation: binding.texture.generation(),
            version: binding.texture.version(),
            width: binding.width,
            height: binding.height,
            alpha_mode: binding.alpha_mode,
        }
    }

    /// Content update boundary. A true result means callers should request a
    /// redraw from the hosted runtime.
    #[cfg(feature = "iced-view")]
    pub fn invalidate_host_texture(&self, slot: &str) -> bool {
        self.host_textures.invalidate(slot).is_some()
    }

    #[cfg(feature = "iced-view")]
    pub fn remove_host_texture(&self, slot: &str) -> bool {
        let removed = self.host_textures.remove(slot).is_some();
        if removed {
            self.report_diagnostic(
                "nana.resource",
                JsDiagnosticLevel::Info,
                format!("host texture released: {slot}"),
                None,
            );
        }
        removed
    }

    /// Device-loss boundary: all prior texture descriptors become invalid.
    #[cfg(feature = "iced-view")]
    pub fn invalidate_host_textures(&self) -> usize {
        self.host_textures.invalidate_all()
    }

    /// Attach the hosted renderer's existing adapter/device/queue to the JS
    /// WebGPU facade. Call again after device recovery, then re-register the
    /// host API on the engine so existing JS wrappers observe the generation.
    #[cfg(feature = "hosted")]
    pub fn bind_host_gpu(&mut self, resources: nana_ui::HostedGpuResources) -> u64 {
        match &self.canvas_gpu {
            Some(canvas_gpu) => canvas_gpu.replace_device(resources.clone()),
            None => {
                self.canvas_gpu = Some(canvas_gpu::CanvasGpuBridge::new(
                    resources.clone(),
                    Arc::clone(&self.canvas),
                    self.host_textures.clone(),
                ));
            }
        }
        match &self.webgpu {
            Some(runtime) => runtime.replace_device(resources),
            None => {
                let runtime = JsWebGpuRuntime::new(resources, self.host_textures.clone());
                let generation = runtime.generation();
                self.webgpu = Some(runtime);
                generation
            }
        }
    }

    #[cfg(feature = "hosted")]
    pub(crate) fn share_canvas_gpu(&mut self, canvas_gpu: canvas_gpu::CanvasGpuBridge) {
        self.canvas_gpu = Some(canvas_gpu);
    }

    #[cfg(feature = "hosted")]
    pub(crate) fn canvas_gpu(&self) -> Option<&canvas_gpu::CanvasGpuBridge> {
        self.canvas_gpu.as_ref()
    }

    #[cfg(feature = "hosted")]
    pub(crate) fn prepare_canvas_gpu(&self) {
        let Some(canvas_gpu) = &self.canvas_gpu else {
            return;
        };
        let ids = self
            .bridge
            .lock()
            .map(|bridge| {
                bridge
                    .snapshot()
                    .widgets
                    .iter()
                    .filter_map(|widget| {
                        widget
                            .props
                            .attrs
                            .get("data-nana-canvas")
                            .or_else(|| widget.props.attrs.get("data-nana-image"))
                            .and_then(|id| id.parse::<u64>().ok())
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            if let Err(error) = canvas_gpu.sync(nana_ui_web_api::CanvasId(id)) {
                self.report_diagnostic("canvas.gpu", JsDiagnosticLevel::Error, error, None);
            }
        }
    }

    /// Rebind the JS WebGPU facade after host device recovery and notify the
    /// existing JavaScript device before replacing its native resources.
    #[cfg(feature = "hosted")]
    pub fn replace_host_gpu<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        resources: nana_ui::HostedGpuResources,
        message: &str,
    ) -> Result<u64, JsEngineError> {
        self.report_diagnostic(
            "wgpu.device",
            JsDiagnosticLevel::Error,
            message.to_owned(),
            None,
        );
        if let Ok(notify) = engine.resolve_function("__nanaWebGpuDeviceLost") {
            engine.invoke(notify, &[HostValue::String(message.to_owned())])?;
            engine.run_microtasks()?;
        }
        let generation = self.bind_host_gpu(resources);
        engine.register_host_api(&self.host_api_registry())?;
        Ok(generation)
    }

    #[cfg(feature = "iced-view")]
    fn report_diagnostic(
        &self,
        source: &str,
        level: JsDiagnosticLevel,
        message: String,
        stack: Option<String>,
    ) {
        if let Some(sink) = &self.diagnostics.sink {
            sink(JsDiagnosticEvent {
                source: source.to_owned(),
                level,
                message,
                stack,
            });
        }
    }

    #[cfg(feature = "hosted")]
    pub fn webgpu_runtime(&self) -> Option<&JsWebGpuRuntime> {
        self.webgpu.as_ref()
    }

    #[cfg(feature = "hosted")]
    pub(crate) fn share_webgpu_runtime(&mut self, runtime: JsWebGpuRuntime) {
        self.webgpu = Some(runtime);
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
        let mut snapshot = bridge.snapshot();
        let mut document = self.document.lock().expect("vue doc");
        document.apply_runtime_hierarchy(&mut snapshot);
        document.sync_semantic_styles(&snapshot);
        snapshot
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

    /// Registry shared by the Vue host and its Iced semantic renderer.
    #[cfg(feature = "iced-view")]
    pub fn components(&self) -> &NativeComponentRegistry {
        &self.components
    }

    #[cfg(feature = "iced-view")]
    pub(crate) fn share_components(&mut self, components: NativeComponentRegistry) {
        self.components = components;
    }

    #[cfg(feature = "iced-view")]
    pub(crate) fn share_host_textures(&mut self, textures: HostTextureRegistry) {
        self.host_textures = textures;
    }

    #[cfg(feature = "iced-view")]
    fn unmount_all_native_components(&self) {
        let mounted = self
            .bridge
            .lock()
            .map(|bridge| {
                bridge
                    .snapshot()
                    .widgets
                    .into_iter()
                    .filter_map(|widget| {
                        widget.props.native_component.map(|name| (name, widget.id))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (component, id) in mounted {
            self.components.unmount(&component, id);
        }
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

    /// Builds the framework-owned registry with renderer, DOM and Web APIs.
    pub fn host_api_registry(&self) -> HostApiRegistry {
        let mut api = HostApiRegistry::new();
        api.set_observer(self.diagnostics.host_calls.clone());
        #[cfg(feature = "iced-view")]
        crate::renderer::register_dom_host_ops_with_components_and_layout(
            &mut api,
            Arc::clone(&self.document),
            Arc::clone(&self.bridge),
            Arc::clone(&self.web_api),
            self.components.clone(),
            Arc::clone(&self.layout_boxes),
        );
        #[cfg(not(feature = "iced-view"))]
        crate::renderer::register_dom_host_ops_with_bridge_and_layout(
            &mut api,
            Arc::clone(&self.document),
            Arc::clone(&self.bridge),
            Arc::clone(&self.web_api),
            Arc::clone(&self.layout_boxes),
        );
        #[cfg(feature = "iced-view")]
        {
            let components = self.components.clone();
            api.register("componentList", move |_| {
                Ok(HostValue::Array(
                    components
                        .names()
                        .into_iter()
                        .map(HostValue::string)
                        .collect(),
                ))
            });
        }
        #[cfg(feature = "iced-view")]
        {
            let components = self.components.clone();
            let bridge = Arc::clone(&self.bridge);
            api.register_async("componentCall", move |args, context| {
                let id = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| id as WidgetId)
                    .ok_or_else(|| {
                        nana_js_engine::JsException::new("native component node is required")
                            .with_name("NativeComponentCommandError")
                    })?;
                let command = args.get(1).and_then(HostValue::as_str).ok_or_else(|| {
                    nana_js_engine::JsException::new("native component command is required")
                        .with_name("NativeComponentCommandError")
                })?;
                let payload = args.get(2).cloned().unwrap_or(HostValue::Null);
                let component = bridge
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue bridge poisoned"))?
                    .get(id)
                    .and_then(|widget| widget.props.native_component.clone())
                    .ok_or_else(|| {
                        nana_js_engine::JsException::new(format!(
                            "node `{id}` is not a registered native component"
                        ))
                        .with_name("NativeComponentCommandError")
                    })?;
                let result = components.command(&component, id, command, payload);
                let (completion, pending) = context.pending();
                completion.complete(result);
                Ok(pending)
            });
        }
        register_web_api_host_ops_with_resources(
            &mut api,
            Arc::clone(&self.web_api),
            default_shared_clipboard(),
            Arc::clone(&self.canvas),
        );
        #[cfg(feature = "hosted")]
        if let Some(webgpu) = &self.webgpu {
            webgpu.register_host_ops(&mut api);
        }
        {
            let sink = self.diagnostics.sink.clone();
            api.register("diagnosticReport", move |args| {
                let Some(event) = args.first().and_then(HostValue::as_object) else {
                    return Err(nana_js_engine::JsException::new(
                        "diagnostic report must be an object",
                    ));
                };
                if let Some(sink) = &sink {
                    let source = event
                        .get("source")
                        .and_then(HostValue::as_str)
                        .unwrap_or("vue.error");
                    let level = match event.get("level").and_then(HostValue::as_str) {
                        Some("info") => JsDiagnosticLevel::Info,
                        Some("warning") | Some("warn") => JsDiagnosticLevel::Warning,
                        _ => JsDiagnosticLevel::Error,
                    };
                    sink(JsDiagnosticEvent {
                        source: source.to_owned(),
                        level,
                        message: event
                            .get("message")
                            .and_then(HostValue::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        stack: event
                            .get("stack")
                            .and_then(HostValue::as_str)
                            .map(str::to_owned),
                    });
                }
                Ok(HostValue::Null)
            });
        }
        self.register_input_host_ops(&mut api);
        // JS `documentElement.dataset.theme` must not wait for semantic_snapshot.
        self.wrap_document_element_set_for_appearance_sync(&mut api);
        api
    }

    fn register_input_host_ops(&self, api: &mut HostApiRegistry) {
        {
            let document = Arc::clone(&self.document);
            api.register("setPointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64))
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer node"))?;
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let mut document = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?;
                if document.element_tag(node).is_none() {
                    return Err(nana_js_engine::JsException::new(
                        "pointer node is not mounted",
                    ));
                }
                if !document.capture_pointer(pointer_id, node) {
                    return Err(nana_js_engine::JsException::new(
                        "pointer capture could not be committed",
                    ));
                }
                Ok(HostValue::Null)
            });
        }
        {
            let document = Arc::clone(&self.document);
            api.register("releasePointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64));
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let mut document = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?;
                let released = node.is_some_and(|node| document.release_pointer(pointer_id, node));
                Ok(HostValue::Bool(released))
            });
        }
        {
            let document = Arc::clone(&self.document);
            api.register("hasPointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64));
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let captured = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?
                    .pointer_capture(pointer_id);
                Ok(HostValue::Bool(captured == node && node.is_some()))
            });
        }
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
        self.initialize_with_web_api_and_host_api(engine, artifact, &HostApiRegistry::new())
    }

    /// Initialize the runtime with framework defaults and application APIs.
    pub fn initialize_with_web_api_and_host_api<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        artifact: RuntimeArtifact,
        application_api: &HostApiRegistry,
    ) -> Result<(), JsEngineError> {
        let mut api = self.host_api_registry();
        api.try_extend(application_api)?;
        engine.register_host_api(&api)?;
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

    /// Resolve renderer and Web API completion hooks after initialization.
    pub fn bind_event_bridge<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        self.event_window_id = None;
        self.fire_event = Some(engine.resolve_function("__nanaFireEvent")?);
        // Drain helper is optional (counter fixture / shim may still install it).
        self.drain_timers = engine.resolve_function("__nanaDrainTimers").ok();
        self.drain_fetch = engine.resolve_function("__nanaDrainFetch").ok();
        self.apply_theme = engine.resolve_function("__nanaApplyTheme").ok();
        self.notify_layout = engine.resolve_function("__nanaNotifyLayout").ok();
        self.lifecycle_pump = engine.resolve_function("__nanaPumpLifecycle").ok();
        Ok(())
    }

    /// Resolve event functions for an auxiliary Vue window while retaining the
    /// same engine context and function table.
    pub fn bind_event_bridge_for_window<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        window_id: u64,
    ) -> Result<(), JsEngineError> {
        self.event_window_id = Some(window_id);
        self.fire_event = Some(engine.resolve_function("__nanaFireWindowEvent")?);
        self.drain_timers = engine.resolve_function("__nanaDrainTimers").ok();
        self.drain_fetch = engine.resolve_function("__nanaDrainFetch").ok();
        self.apply_theme = engine.resolve_function("__nanaApplyWindowTheme").ok();
        self.notify_layout = engine.resolve_function("__nanaNotifyLayout").ok();
        self.lifecycle_pump = engine.resolve_function("__nanaPumpWindowLifecycle").ok();
        Ok(())
    }

    pub fn set_viewport(&mut self, physical_width: u32, physical_height: u32, scale_factor: f32) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.set_viewport(physical_width, physical_height, scale_factor);
        bridge.resolve_document_layout(&mut doc);
    }

    pub fn resolve_layout(&mut self) {
        let iced = self.layout_boxes.snapshot();
        if iced.is_empty() {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            bridge.resolve_document_layout(&mut doc);
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&iced);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }

    /// Copy iced paint boxes into the document cache (call after a frame draws).
    ///
    /// `layoutBox` / `getBoundingClientRect` already prefer the live store; this
    /// keeps hit-tests and `snapshot_boxes` aligned with paint.
    pub fn sync_iced_layout_boxes(&mut self) {
        let iced = self.layout_boxes.snapshot();
        if iced.is_empty() {
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&iced);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }

    /// Shared iced layout writeback buffer (same as probes / `layoutBox`).
    pub fn layout_box_store(&self) -> Arc<LayoutBoxStore> {
        Arc::clone(&self.layout_boxes)
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
            let args = match self.event_window_id {
                Some(window_id) => vec![
                    HostValue::Number(window_id as f64),
                    HostValue::string(label),
                ],
                None => vec![HostValue::string(label)],
            };
            engine.invoke(apply, &args)?;
            engine.run_microtasks()?;
        }
        Ok(())
    }

    /// Settle completed fetches, drain timers, then run microtasks and layout.
    ///
    /// After layout resolves, invokes optional `__nanaNotifyLayout` so
    /// `ResizeObserver` callbacks see fresh `layoutBox` geometry.
    ///
    /// Nested drain: Vue runtime-dom `<Transition>` `nextFrame` is double-rAF
    /// (leave/enter → `whenTransitionEnds` → `@after-leave`). One shot would
    /// leave Transition-driven overlay presence hung.
    pub fn pump_frame<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<usize, JsEngineError> {
        let mut fired = 0usize;
        #[cfg(feature = "hosted")]
        if let Some(webgpu) = &self.webgpu {
            let completions = webgpu.poll();
            if completions > 0 {
                fired += completions;
                engine.run_microtasks()?;
            }
        }
        let fetch_completions = {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.drain_fetch_completions()
        };
        if !fetch_completions.is_empty() {
            if let Some(drain) = self.drain_fetch {
                let count = fetch_completions.len();
                engine.invoke(
                    drain,
                    &[HostValue::Array(fetch_completions.into_iter().collect())],
                )?;
                fired += count;
                engine.run_microtasks()?;
            }
        }
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

    /// Earliest timer/fetch wake requested by the Web API state.
    /// Returns `None` when the runtime is idle.
    pub fn next_wakeup(&self) -> Option<Instant> {
        let web_wakeup = self
            .web_api
            .lock()
            .ok()
            .and_then(|guard| guard.next_wakeup(Instant::now()));
        #[cfg(feature = "hosted")]
        let gpu_wakeup = self.webgpu.as_ref().and_then(JsWebGpuRuntime::next_wakeup);
        #[cfg(not(feature = "hosted"))]
        let gpu_wakeup: Option<Instant> = None;
        web_wakeup.into_iter().chain(gpu_wakeup).min()
    }

    /// Pump a host window lifecycle event into the shim EventTarget surface.
    ///
    /// No-op (returns `Ok(false)`) when `__nanaPumpLifecycle` is absent (e.g. counter
    /// fixture without web-api shim). After dispatch, runs microtasks so listeners
    /// scheduled via `queueMicrotask` / promises settle.
    pub fn pump_lifecycle<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: WindowLifecycleEvent,
    ) -> Result<bool, JsEngineError> {
        let Some(pump) = self.lifecycle_pump else {
            return Ok(false);
        };
        if event == WindowLifecycleEvent::Blur {
            if let Some(target) = self.file_drag_target.take() {
                self.fire_dom_event(engine, target, "dragleave", file_drag_detail(&[], None))?;
            }
            let focused = {
                let mut document = self.document.lock().expect("vue doc");
                let focused = document.focused();
                document.clear_focus();
                focused
            };
            if let Some(focused) = focused {
                self.fire_dom_event(engine, focused, "blur", BTreeMap::new())?;
            }
            self.input.lock().expect("input state").clear();
            {
                let mut document = self.document.lock().expect("vue doc");
                document.clear_pointer_interactions();
                // A pending acquisition was never observable before blur. Match
                // the previous DOM-compatible behavior by publishing only the
                // release of captures that actually remained authoritative.
                let _ = document.take_pointer_capture_changes();
                document.clear_pointer_captures();
            }
            self.flush_pointer_capture_events(engine)?;
        }
        let args = match self.event_window_id {
            Some(window_id) => vec![HostValue::Number(window_id as f64), event.to_host_value()],
            None => vec![event.to_host_value()],
        };
        engine.invoke(pump, &args)?;
        engine.run_microtasks()?;
        Ok(true)
    }

    /// Dispatch a native file hover/drop lifecycle through the same Vue event
    /// tree as pointer input. Dropped files are descriptors with an absolute
    /// path; reading their contents remains an application Host API decision.
    pub fn dispatch_file_drag<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        kind: FileDragEventKind,
        paths: &[PathBuf],
        position: Option<(f32, f32)>,
    ) -> Result<bool, JsEngineError> {
        let target_at_position =
            position.and_then(|(x, y)| self.document.lock().expect("vue doc").hit_test(x, y));
        let mount_root = self.document.lock().expect("vue doc").mount_root();
        let target = target_at_position
            .or(self.file_drag_target)
            .unwrap_or(mount_root);
        let detail = file_drag_detail(paths, position);
        let mut allowed = true;

        match kind {
            FileDragEventKind::Hover => {
                if self.file_drag_target != Some(target) {
                    if let Some(previous) = self.file_drag_target {
                        allowed &=
                            self.fire_dom_event(engine, previous, "dragleave", detail.clone())?;
                    }
                    allowed &= self.fire_dom_event(engine, target, "dragenter", detail.clone())?;
                    self.file_drag_target = Some(target);
                }
                allowed &= self.fire_dom_event(engine, target, "dragover", detail)?;
            }
            FileDragEventKind::Drop => {
                allowed &= self.fire_dom_event(engine, target, "drop", detail)?;
                self.file_drag_target = None;
            }
            FileDragEventKind::Cancel => {
                if let Some(previous) = self.file_drag_target.take() {
                    allowed &= self.fire_dom_event(engine, previous, "dragleave", detail)?;
                }
            }
        }
        engine.run_microtasks()?;
        Ok(allowed)
    }

    /// Route an Iced widget action into the bridge queue and JS event listeners.
    pub fn dispatch_bridge_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: BridgeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_bridge_event_inner(engine, event, true)
    }

    fn dispatch_bridge_event_inner<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: BridgeEvent,
        emit_compatibility_click: bool,
    ) -> Result<bool, JsEngineError> {
        let id = event.widget_id();
        if let BridgeEvent::Scroll {
            id,
            offset,
            metrics,
        } = event
        {
            let mut document = self.document.lock().expect("vue doc");
            let changed = crate::scroll::sync_host_scroll_offset(
                &mut document,
                &self.layout_boxes,
                id,
                offset,
                metrics,
            );
            return Ok(changed);
        }
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
        let committed_input = match &event {
            BridgeEvent::Input { id, value } => Some((*id, value.as_str())),
            _ => None,
        };
        #[cfg(feature = "iced-view")]
        let committed_input = committed_input.or_else(|| match (&event, editor_text.as_deref()) {
            (BridgeEvent::Editor { id, .. }, Some(value)) => Some((*id, value)),
            _ => None,
        });
        if let Some((id, value)) = committed_input {
            let target = NodeHandle(id);
            let mut document = self.document.lock().expect("vue doc");
            let Some(mut state) = document.text_input_state(target) else {
                return Err(JsEngineError::new(
                    "native input target has no retained text input state",
                ));
            };
            state.synchronize_editor_value(value);
            document.set_text_input_state(target, state);
            document.set_attribute(target, "value", value);
        }
        if let BridgeEvent::Native { name, payload, .. } = &event {
            let detail = match payload {
                HostValue::Object(detail) => detail.clone(),
                value => BTreeMap::from([("value".into(), value.clone())]),
            };
            self.fire_dom_event(engine, NodeHandle(id), name, detail)?;
            engine.run_microtasks()?;
            let _ = self.pump_frame(engine)?;
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
                BridgeEvent::Scroll { .. } | BridgeEvent::Native { .. } => Vec::new(),
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
        for name in js_events {
            if name == "click" && !emit_compatibility_click {
                continue;
            }
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
            self.fire_dom_event(engine, NodeHandle(id), name, detail)?;
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        // Drop drained duplicates from note_* (host already consumed the intent).
        let _ = self.bridge.lock().expect("vue bridge").drain_events();
        Ok(true)
    }

    fn fire_dom_event<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        target: NodeHandle,
        name: &str,
        detail: BTreeMap<String, HostValue>,
    ) -> Result<bool, JsEngineError> {
        let fire = self.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        let args = match self.event_window_id {
            Some(window_id) => vec![
                HostValue::Number(window_id as f64),
                HostValue::Number(target.0 as f64),
                HostValue::string(name),
                HostValue::Object(detail),
            ],
            None => vec![
                HostValue::Number(target.0 as f64),
                HostValue::string(name),
                HostValue::Object(detail),
            ],
        };
        let result = engine.invoke(fire, &args)?;
        Ok(result.as_bool().unwrap_or(true))
    }

    fn focus_target_at(&self, x: f32, y: f32) -> (Option<NodeHandle>, Option<NodeHandle>) {
        let mut doc = self.document.lock().expect("vue doc");
        let previous = doc.focused();
        let next = doc.hit_test(x, y).and_then(|hit| {
            let route = doc.event_route(hit)?;
            for id in std::iter::once(route.target).chain(route.bubble) {
                let node = NodeHandle::from(id);
                let tag = doc.element_tag(node).unwrap_or_default();
                if is_focusable_tag(&tag) || self.native_component_name(node.0).is_some() {
                    return Some(node);
                }
            }
            None
        });
        if previous != next {
            if let Some(next) = next {
                doc.set_focus(next);
            } else {
                doc.clear_focus();
            }
        }
        (previous, next)
    }

    fn pointer_detail(
        &self,
        input: PointerInput,
        target: NodeHandle,
    ) -> BTreeMap<String, HostValue> {
        let mut detail = input.detail();
        let Some(bounds) = self
            .document
            .lock()
            .ok()
            .and_then(|doc| get_layout_box_from(&self.layout_boxes, &doc, target))
        else {
            return detail;
        };
        let (local_x, local_y) = self
            .layout_boxes
            .local_point(target, input.client_x, input.client_y)
            .unwrap_or((input.client_x - bounds.x, input.client_y - bounds.y));
        detail.insert("offsetX".into(), HostValue::Number(local_x as f64));
        detail.insert("offsetY".into(), HostValue::Number(local_y as f64));
        detail
    }

    fn pointer_transition_paths(
        &self,
        previous: Option<NodeHandle>,
        next: Option<NodeHandle>,
    ) -> (Vec<NodeHandle>, Vec<NodeHandle>) {
        let doc = self.document.lock().expect("vue doc");
        let path = |start: Option<NodeHandle>| {
            start
                .and_then(|target| doc.event_route(target))
                .map(|route| {
                    std::iter::once(route.target)
                        .chain(route.bubble)
                        .map(NodeHandle::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let previous_path = path(previous);
        let next_path = path(next);
        let common = previous_path
            .iter()
            .find(|node| next_path.contains(node))
            .copied();
        let leaving = previous_path
            .into_iter()
            .take_while(|node| Some(*node) != common)
            .collect();
        let mut entering: Vec<_> = next_path
            .into_iter()
            .take_while(|node| Some(*node) != common)
            .collect();
        entering.reverse();
        (leaving, entering)
    }

    fn flush_pointer_capture_events<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let changes = self
            .document
            .lock()
            .expect("vue doc")
            .take_pointer_capture_changes();
        for change in changes {
            let mut detail = BTreeMap::new();
            detail.insert(
                "pointerId".into(),
                HostValue::Number(change.pointer_id as f64),
            );
            self.fire_dom_event(
                engine,
                NodeHandle::from(change.target),
                if change.captured {
                    "gotpointercapture"
                } else {
                    "lostpointercapture"
                },
                detail,
            )?;
        }
        Ok(())
    }

    fn semantic_default_action<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        requested_value: Option<f64>,
        click_detail: Option<BTreeMap<String, HostValue>>,
    ) -> Result<SemanticActionResult, JsEngineError> {
        let widget = self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .cloned();
        let Some(widget) = widget else {
            return Ok(SemanticActionResult::default());
        };
        if widget.props.disabled || widget.props.loading {
            return Ok(SemanticActionResult {
                handled: true,
                default_prevented: false,
            });
        }
        if let Some(click_detail) = click_detail
            && !self.fire_dom_event(engine, target, "click", click_detail)?
        {
            return Ok(SemanticActionResult {
                handled: true,
                default_prevented: true,
            });
        }
        let event = match widget.kind {
            WidgetKind::Switch | WidgetKind::Checkbox => Some(BridgeEvent::Toggle {
                id: target.0,
                value: !widget.props.toggled,
            }),
            WidgetKind::Range => requested_value.map(|value| BridgeEvent::Change {
                id: target.0,
                value: quantize_range_value(&widget.props, value),
            }),
            WidgetKind::ListItem | WidgetKind::SidebarRow => {
                Some(BridgeEvent::Select { id: target.0 })
            }
            WidgetKind::Button | WidgetKind::Chip => Some(BridgeEvent::Press { id: target.0 }),
            _ => None,
        };
        if let Some(event) = event {
            self.dispatch_bridge_event_inner(engine, event, false)?;
        }
        Ok(SemanticActionResult {
            handled: true,
            default_prevented: false,
        })
    }

    /// Dispatch one browser-style pointer event with hit-testing and capture.
    pub fn dispatch_pointer_result<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        let physical_hit = {
            let doc = self.document.lock().expect("vue doc");
            doc.hit_test(input.client_x, input.client_y)
        };
        let mut captured = self
            .document
            .lock()
            .expect("vue doc")
            .pointer_capture(input.pointer_id);
        if captured.is_some_and(|target| {
            self.document
                .lock()
                .expect("vue doc")
                .element_tag(target)
                .is_none()
        }) {
            if let Some(captured) = captured {
                self.document
                    .lock()
                    .expect("vue doc")
                    .release_pointer(input.pointer_id, captured);
            }
            self.flush_pointer_capture_events(engine)?;
            captured = None;
        }
        let target = captured.or_else(|| {
            if input.kind == PointerEventKind::Cancel {
                return self
                    .document
                    .lock()
                    .expect("vue doc")
                    .pointer_hover(input.pointer_id);
            }
            let doc = self.document.lock().expect("vue doc");
            doc.hit_event_target(input.client_x, input.client_y, input.kind.pointer_name())
                .or(physical_hit)
        });
        let fallback = self.document.lock().expect("vue doc").mount_root();
        let event_target = target.unwrap_or(fallback);
        let detail = self.pointer_detail(input, event_target);

        if matches!(
            input.kind,
            PointerEventKind::Move | PointerEventKind::Cancel
        ) && captured.is_none()
        {
            let previous = self
                .document
                .lock()
                .expect("vue doc")
                .pointer_hover(input.pointer_id);
            if previous != physical_hit {
                if let Some(previous) = previous {
                    let mut transition = detail.clone();
                    transition.insert(
                        "relatedTarget".into(),
                        physical_hit
                            .map(|node| HostValue::Number(node.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, previous, "pointerout", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, previous, "mouseout", transition.clone())?;
                    }
                }
                if let Some(next) = physical_hit {
                    let mut transition = self.pointer_detail(input, next);
                    transition.insert(
                        "relatedTarget".into(),
                        previous
                            .map(|node| HostValue::Number(node.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, next, "pointerover", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, next, "mouseover", transition.clone())?;
                    }
                }
                let (leaving, entering) = self.pointer_transition_paths(previous, physical_hit);
                for node in leaving {
                    let mut transition = self.pointer_detail(input, node);
                    transition.insert(
                        "relatedTarget".into(),
                        physical_hit
                            .map(|n| HostValue::Number(n.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, node, "pointerleave", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, node, "mouseleave", transition)?;
                    }
                }
                for node in entering {
                    let mut transition = self.pointer_detail(input, node);
                    transition.insert(
                        "relatedTarget".into(),
                        previous
                            .map(|n| HostValue::Number(n.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, node, "pointerenter", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, node, "mouseenter", transition)?;
                    }
                }
                self.document
                    .lock()
                    .expect("vue doc")
                    .set_pointer_hover(input.pointer_id, physical_hit);
            }
        }

        let mut default_prevented = !self.fire_dom_event(
            engine,
            event_target,
            input.kind.pointer_name(),
            detail.clone(),
        )?;
        self.flush_pointer_capture_events(engine)?;
        if input.pointer_type == PointerType::Mouse
            && let Some(mouse_name) = input.kind.mouse_name()
        {
            default_prevented |=
                !self.fire_dom_event(engine, event_target, mouse_name, detail.clone())?;
        }

        let mut consumed = false;
        match input.kind {
            PointerEventKind::Down => {
                if let Some(target) = target {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .press_pointer(input.pointer_id, target);
                }
                let (previous, next) = self.focus_target_at(input.client_x, input.client_y);
                if previous != next {
                    if let Some(previous) = previous {
                        self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                    }
                    if let Some(next) = next {
                        self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                    }
                }
            }
            PointerEventKind::Up => {
                let pressed = self
                    .document
                    .lock()
                    .expect("vue doc")
                    .release_pointer_press(input.pointer_id);
                if !default_prevented && pressed.is_some() && pressed == physical_hit {
                    let click_target = pressed.expect("checked above");
                    let is_semantic = self
                        .bridge
                        .lock()
                        .expect("vue bridge")
                        .contains(click_target.0);
                    if is_semantic {
                        let requested_value =
                            self.pointer_range_value(click_target, input.client_x);
                        let result = self.semantic_default_action(
                            engine,
                            click_target,
                            requested_value,
                            Some(detail.clone()),
                        )?;
                        default_prevented |= result.default_prevented;
                        consumed = result.handled;
                    } else {
                        default_prevented |=
                            !self.fire_dom_event(engine, click_target, "click", detail.clone())?;
                        consumed = true;
                    }
                }
            }
            PointerEventKind::Cancel => {
                self.document
                    .lock()
                    .expect("vue doc")
                    .release_pointer_press(input.pointer_id);
            }
            PointerEventKind::Move => {}
        }

        self.flush_pointer_capture_events(engine)?;

        if matches!(input.kind, PointerEventKind::Up | PointerEventKind::Cancel) {
            let captured = self
                .document
                .lock()
                .expect("vue doc")
                .pointer_capture(input.pointer_id);
            if let Some(captured) = captured {
                self.document
                    .lock()
                    .expect("vue doc")
                    .release_pointer(input.pointer_id, captured);
            }
            self.flush_pointer_capture_events(engine)?;
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(HostedInputResult {
            targeted: target.is_some(),
            default_prevented,
            consumed,
        })
    }

    fn pointer_range_value(&self, target: NodeHandle, x: f32) -> Option<f64> {
        let widget = self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .cloned()?;
        if widget.kind != WidgetKind::Range {
            return None;
        }
        let bounds = self.document.lock().expect("vue doc").layout_box(target)?;
        let ratio = if bounds.width > 0.0 {
            ((x - bounds.x) / bounds.width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(
            f64::from(widget.props.min)
                + f64::from(ratio) * f64::from(widget.props.max - widget.props.min),
        )
    }

    pub fn dispatch_pointer<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_pointer_result(engine, input)
            .map(|result| result.targeted)
    }

    /// Compatibility helper for callers that only expose an atomic click.
    pub fn pointer_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
    ) -> Result<bool, JsEngineError> {
        let down =
            self.dispatch_pointer(engine, PointerInput::mouse(PointerEventKind::Down, x, y))?;
        let up = self.dispatch_pointer(engine, PointerInput::mouse(PointerEventKind::Up, x, y))?;
        Ok(down || up)
    }

    pub fn dispatch_wheel_result<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            doc.hit_event_target(input.client_x, input.client_y, "wheel")
                .or_else(|| doc.hit_test(input.client_x, input.client_y))
        };
        let Some(target) = target else {
            return Ok(HostedInputResult::default());
        };
        let allowed = self.fire_dom_event(engine, target, "wheel", input.detail())?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(HostedInputResult {
            targeted: true,
            default_prevented: !allowed,
            consumed: !allowed,
        })
    }

    pub fn dispatch_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_wheel_result(engine, input)
            .map(|result| result.targeted)
    }

    pub fn pointer_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_wheel(engine, WheelInput::pixels(x, y, delta_x, delta_y))
    }

    pub fn dispatch_keyboard<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            target
                .or_else(|| doc.focused())
                .unwrap_or_else(|| doc.mount_root())
        };
        let repeated = self
            .input
            .lock()
            .expect("input state")
            .note_key(&input.code, input.kind == KeyboardEventKind::Down);
        let mut detail = input.detail();
        if repeated {
            detail.insert("repeat".into(), HostValue::Bool(true));
        }
        let mut allowed = self.fire_dom_event(engine, target, input.kind.as_str(), detail)?;
        if allowed && input.kind == KeyboardEventKind::Down {
            let widget = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .cloned();
            if let Some(widget) = widget {
                let key = input.key.to_ascii_lowercase();
                let requested_value = match widget.kind {
                    WidgetKind::Range => match key.as_str() {
                        "arrowleft" | "arrowdown" => {
                            Some(f64::from(widget.props.number - widget.props.step))
                        }
                        "arrowright" | "arrowup" => {
                            Some(f64::from(widget.props.number + widget.props.step))
                        }
                        "pagedown" => {
                            Some(f64::from(widget.props.number - widget.props.step * 10.0))
                        }
                        "pageup" => Some(f64::from(widget.props.number + widget.props.step * 10.0)),
                        "home" => Some(f64::from(widget.props.min)),
                        "end" => Some(f64::from(widget.props.max)),
                        _ => None,
                    },
                    _ => None,
                };
                let activates = match widget.kind {
                    WidgetKind::Button
                    | WidgetKind::Chip
                    | WidgetKind::ListItem
                    | WidgetKind::SidebarRow => {
                        !repeated && matches!(key.as_str(), "enter" | " " | "space" | "spacebar")
                    }
                    WidgetKind::Switch | WidgetKind::Checkbox => {
                        !repeated && matches!(key.as_str(), " " | "space" | "spacebar")
                    }
                    WidgetKind::Range => requested_value.is_some(),
                    _ => false,
                };
                if activates {
                    let result = self.semantic_default_action(
                        engine,
                        target,
                        requested_value,
                        Some(BTreeMap::new()),
                    )?;
                    if result.handled {
                        allowed = false;
                    }
                }
            }
        }
        if allowed && input.kind == KeyboardEventKind::Down && input.key.eq_ignore_ascii_case("tab")
        {
            let (previous, next) = self.advance_tab_focus(input.modifiers.shift);
            if previous != next {
                if let Some(previous) = previous {
                    self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                }
                if let Some(next) = next {
                    self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                }
            }
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(allowed)
    }

    pub(crate) fn accessibility_focus<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let previous = {
            let mut document = self.document.lock().expect("vue doc");
            if document.element_tag(target).is_none() {
                return Ok(false);
            }
            let previous = document.focused();
            if previous == Some(target) {
                return Ok(false);
            }
            document.set_focus(target);
            previous
        };
        if let Some(previous) = previous {
            self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
        }
        self.fire_dom_event(engine, target, "focus", BTreeMap::new())?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    pub(crate) fn accessibility_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let result = self.semantic_default_action(engine, target, None, Some(BTreeMap::new()))?;
        Ok(result.handled && !result.default_prevented)
    }

    pub(crate) fn accessibility_set_value<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        value: &str,
    ) -> Result<bool, JsEngineError> {
        let range = {
            let bridge = self.bridge.lock().expect("vue bridge");
            bridge
                .get(target.0)
                .filter(|widget| widget.kind == WidgetKind::Range)
                .cloned()
        };
        if let Some(range) = range {
            if range.props.disabled || range.props.loading {
                return Ok(false);
            }
            let Ok(value) = value.parse::<f64>() else {
                return Ok(false);
            };
            let result = self.semantic_default_action(engine, target, Some(value), None)?;
            return Ok(result.handled && !result.default_prevented);
        }
        let supported = {
            let document = self.document.lock().expect("vue doc");
            document.text_input_state(target).is_some()
                && document.get_attribute(target, "disabled").is_none()
                && document.get_attribute(target, "readonly").is_none()
        };
        if !supported {
            return Ok(false);
        }

        let next = TextInputState::new(value);
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(value));
        detail.insert(
            "inputType".into(),
            HostValue::string("insertReplacementText"),
        );
        detail.insert("value".into(), HostValue::string(value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        {
            let mut document = self.document.lock().expect("vue doc");
            if !document.set_text_input_state(target, next) {
                return Ok(false);
            }
            document.set_attribute(target, "value", value);
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    pub(crate) fn accessibility_set_selection<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        selection: nana_ui_runtime::TextSelection,
    ) -> Result<bool, JsEngineError> {
        {
            let mut document = self.document.lock().expect("vue doc");
            if document.get_attribute(target, "disabled").is_some() {
                return Ok(false);
            }
            let Some(mut state) = document.text_input_state(target) else {
                return Ok(false);
            };
            if !selection.is_valid_for(&state.value) || state.selection == selection {
                return Ok(false);
            }
            state.selection = selection;
            if !document.set_text_input_state(target, state) {
                return Ok(false);
            }
        }
        self.fire_dom_event(engine, target, "select", BTreeMap::new())?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    fn advance_tab_focus(&self, reverse: bool) -> (Option<NodeHandle>, Option<NodeHandle>) {
        let mut document = self.document.lock().expect("vue doc");
        let previous = document.focused();
        let root = document.mount_root();
        let mut order = document
            .collect_element_preorder(root)
            .into_iter()
            .map(NodeHandle)
            .filter_map(|node| {
                let tag = document.element_tag(node)?;
                if document.get_attribute(node, "disabled").is_some() {
                    return None;
                }
                let tabindex = document
                    .get_attribute(node, "tabindex")
                    .and_then(|value| value.parse::<i32>().ok());
                if tabindex.is_some_and(|value| value < 0) {
                    return None;
                }
                let naturally_focusable = is_focusable_tag(&tag)
                    || self.native_component_name(node.0).is_some()
                    || document
                        .get_attribute(node, "contenteditable")
                        .is_some_and(|value| value != "false");
                (naturally_focusable || tabindex.is_some()).then_some((tabindex.unwrap_or(0), node))
            })
            .collect::<Vec<_>>();
        order.sort_by_key(|(tabindex, _)| {
            if *tabindex > 0 {
                (0, *tabindex)
            } else {
                (1, 0)
            }
        });
        if order.is_empty() {
            document.clear_focus();
            return (previous, None);
        }
        let current = previous.and_then(|focused| {
            order
                .iter()
                .position(|(_, candidate)| *candidate == focused)
        });
        let next_index = if reverse {
            current.map_or(order.len() - 1, |index| {
                if index == 0 {
                    order.len() - 1
                } else {
                    index - 1
                }
            })
        } else {
            current.map_or(0, |index| (index + 1) % order.len())
        };
        let next = order.get(next_index).map(|(_, node)| *node);
        if let Some(next) = next {
            document.set_focus(next);
        }
        (previous, next)
    }

    /// Commit text from a keyboard or IME into the focused Vue control.
    pub fn commit_text<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        text: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.document.lock().expect("vue doc").focused() else {
            return Ok(false);
        };
        let next = {
            let doc = self.document.lock().expect("vue doc");
            let mut state = doc.text_input_state(target).unwrap_or_else(|| {
                TextInputState::new(doc.get_attribute(target, "value").unwrap_or_default())
            });
            if !state.replace_selection(text) {
                return Ok(false);
            }
            state
        };
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(text));
        detail.insert("inputType".into(), HostValue::string(input_type));
        detail.insert("value".into(), HostValue::string(&next.value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        {
            let mut document = self.document.lock().expect("vue doc");
            if !document.set_text_input_state(target, next.clone()) {
                return Ok(false);
            }
            document.set_attribute(target, "value", &next.value);
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    pub fn dispatch_composition<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.document.lock().expect("vue doc").focused() else {
            return Ok(false);
        };
        {
            let mut document = self.document.lock().expect("vue doc");
            if document.text_input_state(target).is_none() {
                let value = document.get_attribute(target, "value").unwrap_or_default();
                if !document.set_text_input_state(target, TextInputState::new(value)) {
                    return Err(JsEngineError::new(
                        "composition target has no retained text input state",
                    ));
                }
            }
            let composition = match input.kind {
                CompositionEventKind::Start | CompositionEventKind::Update => {
                    Some(nana_ui_runtime::ImeComposition {
                        text: input.data.clone(),
                        selection: None,
                    })
                }
                CompositionEventKind::End => None,
            };
            if !document.set_ime_composition(target, composition) {
                return Err(JsEngineError::new("invalid composition state"));
            }
        }
        self.dispatch_composition_event(engine, target, input)
    }

    fn dispatch_composition_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(&input.data));
        detail.insert(
            "isComposing".into(),
            HostValue::Bool(input.kind != CompositionEventKind::End),
        );
        self.fire_dom_event(engine, target, input.kind.as_str(), detail)?;
        engine.run_microtasks()?;
        if input.kind == CompositionEventKind::End && !input.data.is_empty() {
            return self.commit_text(engine, &input.data, "insertCompositionText");
        }
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    /// Forwards desktop winit IME lifecycle into Vue composition events.
    ///
    /// Commit text itself continues through the retained Iced input/editor and
    /// [`BridgeEvent::Input`], avoiding a duplicate insertion while still giving
    /// Vue the browser composition lifecycle.
    pub fn dispatch_native_ime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.focused() else {
            return Ok(false);
        };
        match event {
            ImeEvent::Enabled => Ok(true),
            ImeEvent::Preedit { text, selection } => {
                let started = {
                    let mut document = self.document.lock().expect("vue doc");
                    if document.text_input_state(target).is_none() {
                        let value = document.get_attribute(target, "value").unwrap_or_default();
                        if !document.set_text_input_state(target, TextInputState::new(value)) {
                            return Err(JsEngineError::new(
                                "native IME target has no retained text input state",
                            ));
                        }
                    }
                    let started = document.ime_composition(target).is_none();
                    if !document.set_ime_composition(
                        target,
                        Some(nana_ui_runtime::ImeComposition {
                            text: text.clone(),
                            selection: *selection,
                        }),
                    ) {
                        return Err(JsEngineError::new("invalid native IME preedit state"));
                    }
                    started
                };
                if started {
                    self.dispatch_composition_event(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::Start, ""),
                    )?;
                }
                self.dispatch_composition_event(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::Update, text),
                )
            }
            ImeEvent::Commit(text) => {
                self.document
                    .lock()
                    .expect("vue doc")
                    .set_ime_composition(target, None);
                // Browser compositionend carries committed text. The Iced
                // editor's ensuing Input message owns the value mutation.
                let mut detail = BTreeMap::new();
                detail.insert("data".into(), HostValue::string(text));
                detail.insert("isComposing".into(), HostValue::Bool(false));
                self.fire_dom_event(engine, target, "compositionend", detail)?;
                engine.run_microtasks()?;
                let _ = self.pump_frame(engine)?;
                Ok(true)
            }
            ImeEvent::Disabled => {
                let data = {
                    let mut document = self.document.lock().expect("vue doc");
                    let data = document.ime_composition(target).map(|ime| ime.text);
                    document.set_ime_composition(target, None);
                    data
                };
                let Some(data) = data else {
                    return Ok(true);
                };
                let target = self.document.lock().expect("vue doc").focused();
                let Some(target) = target else {
                    return Ok(false);
                };
                let mut detail = BTreeMap::new();
                detail.insert("data".into(), HostValue::string(data));
                detail.insert("isComposing".into(), HostValue::Bool(false));
                self.fire_dom_event(engine, target, "compositionend", detail)?;
                engine.run_microtasks()?;
                let _ = self.pump_frame(engine)?;
                Ok(true)
            }
        }
    }

    /// Legacy keydown helper; printable text is committed separately for compatibility.
    pub fn dispatch_key<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        key: &str,
        code: &str,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let input = KeyboardInput::key_down(key, code);
        self.dispatch_keyboard(engine, &input, target)?;
        if target.or_else(|| self.focused()).is_some()
            && key.chars().count() == 1
            && !key
                .chars()
                .next()
                .is_some_and(|character| character.is_control())
        {
            self.commit_text(engine, key, "insertText")?;
        }
        Ok(true)
    }

    pub fn focused(&self) -> Option<NodeHandle> {
        self.document.lock().expect("vue doc").focused()
    }

    #[cfg(feature = "iced-view")]
    fn native_component_name(&self, id: WidgetId) -> Option<String> {
        self.bridge
            .lock()
            .ok()?
            .get(id)
            .and_then(|widget| widget.props.native_component.clone())
    }

    #[cfg(not(feature = "iced-view"))]
    fn native_component_name(&self, _id: u64) -> Option<String> {
        None
    }
}

fn file_drag_detail(
    paths: &[PathBuf],
    position: Option<(f32, f32)>,
) -> BTreeMap<String, HostValue> {
    let mut detail = BTreeMap::new();
    if let Some((x, y)) = position {
        detail.insert("clientX".into(), HostValue::Number(f64::from(x)));
        detail.insert("clientY".into(), HostValue::Number(f64::from(y)));
    }
    let files = paths
        .iter()
        .map(|path| {
            let mut file = BTreeMap::new();
            file.insert(
                "name".into(),
                HostValue::string(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
            file.insert(
                "path".into(),
                HostValue::string(path.to_string_lossy().into_owned()),
            );
            file.insert("type".into(), HostValue::string(""));
            if let Ok(metadata) = path.metadata() {
                file.insert("size".into(), HostValue::Number(metadata.len() as f64));
                if let Ok(modified) = metadata.modified()
                    && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
                {
                    file.insert(
                        "lastModified".into(),
                        HostValue::Number(duration.as_secs_f64() * 1000.0),
                    );
                }
            }
            HostValue::Object(file)
        })
        .collect();
    detail.insert("files".into(), HostValue::Array(files));
    detail
}

impl Drop for VueHost {
    fn drop(&mut self) {
        #[cfg(feature = "iced-view")]
        self.unmount_all_native_components();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SemanticActionResult {
    handled: bool,
    default_prevented: bool,
}

fn quantize_range_value(props: &WidgetProps, value: f64) -> f64 {
    let minimum = f64::from(props.min);
    let maximum = f64::from(props.max);
    let step = f64::from(props.step);
    if !minimum.is_finite()
        || !maximum.is_finite()
        || maximum <= minimum
        || !step.is_finite()
        || step <= 0.0
    {
        return minimum;
    }
    let steps = ((value.clamp(minimum, maximum) - minimum) / step).round();
    (minimum + steps * step).clamp(minimum, maximum)
}

fn is_focusable_tag(tag: &str) -> bool {
    matches!(
        tag,
        "input"
            | "textarea"
            | "button"
            | "select"
            | "a"
            | "nana-button"
            | "nana-switch"
            | "nana-sidebar-row"
            | "nana-input"
            | "nana-textarea"
            | "nana-checkbox"
            | "nana-select"
            | "nana-range"
    )
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

    #[derive(Default)]
    struct RecordingEngine {
        invocations: Vec<(JsFunctionId, Vec<HostValue>)>,
        prevent_event: Option<String>,
    }

    impl JsEngine for RecordingEngine {
        fn initialize(&mut self, _artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
            Ok(())
        }

        fn register_host_api(&mut self, _api: &HostApiRegistry) -> Result<(), JsEngineError> {
            Ok(())
        }

        fn resolve_function(&mut self, _name: &str) -> Result<JsFunctionId, JsEngineError> {
            Ok(JsFunctionId(1))
        }

        fn invoke(
            &mut self,
            target: JsFunctionId,
            args: &[HostValue],
        ) -> Result<HostValue, JsEngineError> {
            self.invocations.push((target, args.to_vec()));
            let allowed = args
                .get(1)
                .and_then(HostValue::as_str)
                .is_none_or(|name| self.prevent_event.as_deref() != Some(name));
            Ok(HostValue::Bool(allowed))
        }

        fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
            Ok(())
        }

        fn interrupt(&mut self) {}
        fn request_gc(&mut self) {}
        fn shutdown(&mut self) {}
    }

    fn fired_events(engine: &RecordingEngine) -> Vec<(u64, String, BTreeMap<String, HostValue>)> {
        engine
            .invocations
            .iter()
            .filter_map(|(_, args)| {
                let target = args.first()?.as_f64()? as u64;
                let name = args.get(1)?.as_str()?.to_string();
                let detail = args.get(2)?.as_object()?.clone();
                Some((target, name, detail))
            })
            .collect()
    }

    fn install_input_nodes(host: &mut VueHost) -> (NodeHandle, NodeHandle) {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let root = doc.mount_root();
        let first = doc.create_element("input");
        let second = doc.create_element("button");
        doc.insert(first, root, None);
        doc.insert(second, root, None);
        drop(doc);

        let store = host.layout_box_store();
        store.begin_frame();
        store.record(first, 0.0, 0.0, 80.0, 40.0);
        store.record(second, 100.0, 0.0, 80.0, 40.0);
        host.sync_iced_layout_boxes();
        (first, second)
    }

    fn install_semantic_switch(host: &mut VueHost) -> NodeHandle {
        let node = {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            let node = doc.create_element("nana-switch");
            let root = doc.mount_root();
            doc.insert(node, root, None);
            node
        };
        host.bridge.lock().expect("bridge").register(
            node.0,
            WidgetKind::Switch,
            WidgetProps {
                label: "Preview".into(),
                toggled: false,
                ..Default::default()
            },
        );
        let snapshot = host.bridge.lock().expect("bridge").snapshot();
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.sync_semantic_styles(&snapshot);
        doc.apply_layout_boxes(&[(
            node,
            LayoutBox {
                handle: node,
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 32.0,
            },
        )]);
        node
    }

    #[test]
    fn semantic_switch_pointer_default_action_updates_once_and_honors_prevent_default() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let node = install_semantic_switch(&mut host);
        let mut engine = RecordingEngine::default();

        host.pointer_click(&mut engine, 10.0, 10.0).unwrap();
        let events = fired_events(&engine);
        for name in ["click", "change", "update:modelValue"] {
            assert_eq!(
                events.iter().filter(|(_, event, _)| event == name).count(),
                1,
                "{name} must be emitted once"
            );
        }
        assert!(
            host.bridge
                .lock()
                .expect("bridge")
                .get(node.0)
                .unwrap()
                .props
                .toggled
        );

        let mut prevented = RecordingEngine {
            prevent_event: Some("click".into()),
            ..Default::default()
        };
        host.pointer_click(&mut prevented, 10.0, 10.0).unwrap();
        assert!(
            host.bridge
                .lock()
                .expect("bridge")
                .get(node.0)
                .unwrap()
                .props
                .toggled,
            "prevented click must not apply the toggle default action"
        );
        let prevented_events = fired_events(&prevented);
        assert_eq!(
            prevented_events
                .iter()
                .filter(|(_, event, _)| event == "click")
                .count(),
            1
        );
        assert!(
            prevented_events
                .iter()
                .all(|(_, event, _)| event != "change" && event != "update:modelValue")
        );
    }

    #[test]
    fn range_keyboard_and_accessibility_share_quantized_change_action() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let node = {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            let node = doc.create_element("nana-range");
            let root = doc.mount_root();
            doc.insert(node, root, None);
            node
        };
        host.bridge.lock().expect("bridge").register(
            node.0,
            WidgetKind::Range,
            WidgetProps {
                min: 0.0,
                max: 1.0,
                step: 0.25,
                number: 0.5,
                ..Default::default()
            },
        );
        let snapshot = host.bridge.lock().expect("bridge").snapshot();
        host.document()
            .lock()
            .expect("document")
            .sync_semantic_styles(&snapshot);
        let mut engine = RecordingEngine::default();

        assert!(
            !host
                .dispatch_keyboard(
                    &mut engine,
                    &KeyboardInput::key_down("ArrowRight", "ArrowRight"),
                    Some(node),
                )
                .unwrap()
        );
        assert_eq!(
            host.bridge
                .lock()
                .expect("bridge")
                .get(node.0)
                .unwrap()
                .props
                .number,
            0.75
        );
        assert!(
            host.accessibility_set_value(&mut engine, node, "0.88")
                .unwrap()
        );
        assert_eq!(
            host.bridge
                .lock()
                .expect("bridge")
                .get(node.0)
                .unwrap()
                .props
                .number,
            1.0
        );
        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .filter(|(_, event, _)| event == "change")
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|(_, event, _)| event == "update:modelValue")
                .count(),
            2
        );
    }

    #[test]
    fn iced_scroll_event_updates_runtime_without_firing_vue_event() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let document = host.document();
        let scrollport = {
            let mut doc = document.lock().expect("document");
            let root = doc.mount_root();
            let scrollport = doc.create_element("div");
            doc.insert(scrollport, root, None);
            scrollport
        };
        let mut engine = RecordingEngine::default();

        assert!(
            host.dispatch_bridge_event(
                &mut engine,
                BridgeEvent::Scroll {
                    id: scrollport.0,
                    offset: ScrollOffset { x: 0.0, y: 48.0 },
                    metrics: nana_ui_runtime::ScrollMetrics {
                        viewport_width: 100.0,
                        viewport_height: 100.0,
                        content_width: 100.0,
                        content_height: 300.0,
                    },
                },
            )
            .expect("dispatch scroll")
        );
        assert_eq!(
            document.lock().expect("document").scroll_offset(scrollport),
            ScrollOffset { x: 0.0, y: 48.0 }
        );
        assert!(engine.invocations.is_empty());

        assert!(
            !host
                .dispatch_bridge_event(
                    &mut engine,
                    BridgeEvent::Scroll {
                        id: scrollport.0,
                        offset: ScrollOffset { x: 0.0, y: 48.0 },
                        metrics: nana_ui_runtime::ScrollMetrics {
                            viewport_width: 100.0,
                            viewport_height: 100.0,
                            content_width: 100.0,
                            content_height: 300.0,
                        },
                    },
                )
                .expect("repeat scroll")
        );
    }

    #[test]
    fn vue_hosts_isolate_paint_geometry_for_equal_node_handles() {
        let first = VueHost::new();
        let second = VueHost::new();
        let node = NodeHandle(2);
        first
            .layout_box_store()
            .record(node, 10.0, 20.0, 30.0, 40.0);
        second
            .layout_box_store()
            .record(node, 100.0, 200.0, 300.0, 400.0);

        assert_eq!(first.layout_box_store().get(node).unwrap().x, 10.0);
        assert_eq!(second.layout_box_store().get(node).unwrap().x, 100.0);
    }

    #[test]
    fn pointer_capture_keeps_target_and_blur_releases_it() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        host.lifecycle_pump = Some(JsFunctionId(2));
        let (first, second) = install_input_nodes(&mut host);
        let mut engine = RecordingEngine::default();

        let mut down = PointerInput::mouse(PointerEventKind::Down, 10.0, 12.0);
        down.pointer_id = 7;
        down.screen_x = 210.0;
        down.screen_y = 312.0;
        down.buttons = 1;
        down.pressure = 0.5;
        down.modifiers.shift = true;
        assert!(
            host.dispatch_pointer(&mut engine, down)
                .expect("pointer down")
        );

        let api = host.host_api_registry();
        api.call(
            "setPointerCapture",
            &[HostValue::Number(first.0 as f64), HostValue::Number(7.0)],
        )
        .expect("capture pointer");
        assert_eq!(
            api.call(
                "hasPointerCapture",
                &[HostValue::Number(first.0 as f64), HostValue::Number(7.0),],
            )
            .expect("query capture"),
            HostValue::Bool(true)
        );

        let mut movement = PointerInput::mouse(PointerEventKind::Move, 120.0, 12.0);
        movement.pointer_id = 7;
        movement.buttons = 1;
        movement.pressure = 0.25;
        movement.tangential_pressure = -0.2;
        movement.tilt_x = 25;
        movement.tilt_y = -12;
        movement.twist = 180;
        movement.modifiers.alt = true;
        assert!(
            host.dispatch_pointer(&mut engine, movement)
                .expect("captured move")
        );

        let events = fired_events(&engine);
        let captured_move = events
            .iter()
            .find(|(_, name, detail)| {
                name == "pointermove"
                    && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
            })
            .expect("pointermove event");
        assert_eq!(captured_move.0, first.0);
        assert_ne!(captured_move.0, second.0);
        assert_eq!(
            captured_move.2.get("clientX").and_then(HostValue::as_f64),
            Some(120.0)
        );
        assert_eq!(
            captured_move.2.get("altKey").and_then(HostValue::as_bool),
            Some(true)
        );
        let tangential_pressure = captured_move
            .2
            .get("tangentialPressure")
            .and_then(HostValue::as_f64)
            .expect("tangential pressure");
        assert!((tangential_pressure + 0.2).abs() < 1e-6);
        assert_eq!(
            captured_move.2.get("tiltX").and_then(HostValue::as_f64),
            Some(25.0)
        );
        assert_eq!(
            captured_move.2.get("tiltY").and_then(HostValue::as_f64),
            Some(-12.0)
        );
        assert_eq!(
            captured_move.2.get("twist").and_then(HostValue::as_f64),
            Some(180.0)
        );
        assert!(events.iter().any(|(target, name, detail)| {
            *target == first.0
                && name == "gotpointercapture"
                && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
        }));

        host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Blur)
            .expect("window blur");
        assert_eq!(
            api.call(
                "hasPointerCapture",
                &[HostValue::Number(first.0 as f64), HostValue::Number(7.0),],
            )
            .expect("capture released"),
            HostValue::Bool(false)
        );
        assert!(fired_events(&engine).iter().any(|(target, name, detail)| {
            *target == first.0
                && name == "lostpointercapture"
                && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
        }));
    }

    #[test]
    fn file_drag_tracks_hit_target_and_exposes_file_descriptors() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (first, second) = install_input_nodes(&mut host);
        let mut engine = RecordingEngine::default();
        let paths = vec![
            PathBuf::from("C:/drop/avatar.png"),
            PathBuf::from("C:/drop/background.jpg"),
        ];

        host.dispatch_file_drag(
            &mut engine,
            FileDragEventKind::Hover,
            &paths,
            Some((10.0, 12.0)),
        )
        .expect("hover first target");
        host.dispatch_file_drag(
            &mut engine,
            FileDragEventKind::Hover,
            &paths,
            Some((120.0, 12.0)),
        )
        .expect("hover second target");
        host.dispatch_file_drag(
            &mut engine,
            FileDragEventKind::Drop,
            &paths,
            Some((120.0, 12.0)),
        )
        .expect("drop second target");

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(target, name, _)| (*target, name.as_str()))
                .collect::<Vec<_>>(),
            [
                (first.0, "dragenter"),
                (first.0, "dragover"),
                (first.0, "dragleave"),
                (second.0, "dragenter"),
                (second.0, "dragover"),
                (second.0, "drop"),
            ]
        );
        let files = events
            .last()
            .and_then(|(_, _, detail)| detail.get("files"))
            .and_then(HostValue::as_array)
            .expect("drop files");
        assert_eq!(files.len(), 2);
        let file = files[0].as_object().expect("file descriptor");
        assert_eq!(
            file.get("name").and_then(HostValue::as_str),
            Some("avatar.png")
        );
        assert_eq!(
            file.get("path").and_then(HostValue::as_str),
            Some("C:/drop/avatar.png")
        );
        assert_eq!(
            files[1]
                .as_object()
                .and_then(|file| file.get("name"))
                .and_then(HostValue::as_str),
            Some("background.jpg")
        );
    }

    #[test]
    fn composition_end_commits_through_beforeinput_and_input() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_attribute(input, "value", "Nana");
            doc.set_focus(input);
        }
        let mut engine = RecordingEngine::default();

        host.dispatch_composition(
            &mut engine,
            &CompositionInput::new(CompositionEventKind::Start, ""),
        )
        .expect("composition start");
        host.dispatch_composition(
            &mut engine,
            &CompositionInput::new(CompositionEventKind::Update, "界"),
        )
        .expect("composition update");
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .ime_composition(input)
                .expect("runtime composition")
                .text,
            "界"
        );
        host.dispatch_composition(
            &mut engine,
            &CompositionInput::new(CompositionEventKind::End, "界"),
        )
        .expect("composition end");
        assert!(
            host.document()
                .lock()
                .expect("document")
                .ime_composition(input)
                .is_none()
        );

        let events = fired_events(&engine);
        let names = events
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "compositionstart",
                "compositionupdate",
                "compositionend",
                "beforeinput",
                "input"
            ]
        );
        let input_event = events.last().expect("input event");
        assert_eq!(input_event.0, input.0);
        assert_eq!(
            input_event.2.get("value").and_then(HostValue::as_str),
            Some("Nana界")
        );
        assert_eq!(
            input_event.2.get("inputType").and_then(HostValue::as_str),
            Some("insertCompositionText")
        );
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .get_attribute(input, "value")
                .as_deref(),
            Some("Nana界")
        );
    }

    #[test]
    fn committed_text_replaces_runtime_owned_unicode_selection() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_attribute(input, "value", "你好ab");
            doc.set_focus(input);
            assert!(doc.set_text_input_state(
                input,
                TextInputState {
                    value: "你好ab".into(),
                    selection: nana_ui_runtime::TextSelection {
                        anchor: 0,
                        focus: "你".len(),
                    },
                }
            ));
        }
        let mut engine = RecordingEngine::default();

        assert!(host.commit_text(&mut engine, "娜", "insertText").unwrap());
        let document = host.document();
        let doc = document.lock().expect("document");
        let state = doc.text_input_state(input).expect("text input state");
        assert_eq!(state.value, "娜好ab");
        assert_eq!(
            state.selection,
            nana_ui_runtime::TextSelection::caret("娜".len())
        );
        assert_eq!(doc.get_attribute(input, "value").as_deref(), Some("娜好ab"));
    }

    #[test]
    fn native_ime_emits_composition_without_double_committing_iced_value() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_attribute(input, "value", "Nana");
            doc.set_focus(input);
        }
        let mut engine = RecordingEngine::default();

        host.dispatch_native_ime(
            &mut engine,
            &ImeEvent::Preedit {
                text: "世".into(),
                selection: Some((3, 3)),
            },
        )
        .expect("preedit");
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .ime_composition(input)
                .expect("runtime preedit")
                .selection,
            Some((3, 3))
        );
        host.dispatch_native_ime(&mut engine, &ImeEvent::Commit("世界".into()))
            .expect("commit lifecycle");
        assert!(
            host.document()
                .lock()
                .expect("document")
                .ime_composition(input)
                .is_none()
        );

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["compositionstart", "compositionupdate", "compositionend"]
        );
        assert_eq!(
            events
                .last()
                .and_then(|(_, _, detail)| detail.get("data"))
                .and_then(HostValue::as_str),
            Some("世界")
        );
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .get_attribute(input, "value")
                .as_deref(),
            Some("Nana"),
            "Iced BridgeEvent::Input remains the single value-commit path"
        );
    }

    #[test]
    fn native_input_commits_runtime_value_before_firing_vue_events() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        host.bridge().lock().expect("bridge").register(
            input.0,
            WidgetKind::Input,
            WidgetProps {
                value: "你好ab".into(),
                ..WidgetProps::default()
            },
        );
        {
            let document = host.document();
            let mut document = document.lock().expect("document");
            document.set_attribute(input, "value", "你好ab");
            assert!(document.set_text_input_state(input, TextInputState::new("你好ab")));
        }
        let mut engine = RecordingEngine::default();

        assert!(
            host.dispatch_bridge_event(
                &mut engine,
                BridgeEvent::Input {
                    id: input.0,
                    value: "你娜好ab".into(),
                },
            )
            .expect("native input")
        );

        let document = host.document();
        let document = document.lock().expect("document");
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("你娜好ab")
        );
        assert_eq!(
            document.text_input_state(input),
            Some(TextInputState {
                value: "你娜好ab".into(),
                selection: nana_ui_runtime::TextSelection::caret("你娜".len()),
            })
        );
        drop(document);
        assert_eq!(
            fired_events(&engine)
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["input", "update:modelValue"]
        );
    }

    #[test]
    fn tab_and_shift_tab_move_focus_in_document_order() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (first, second) = install_input_nodes(&mut host);
        host.document().lock().expect("document").set_focus(first);
        let mut engine = RecordingEngine::default();

        host.dispatch_keyboard(&mut engine, &KeyboardInput::key_down("Tab", "Tab"), None)
            .expect("tab");
        assert_eq!(host.focused(), Some(second));

        let mut key_up = KeyboardInput::key_down("Tab", "Tab");
        key_up.kind = KeyboardEventKind::Up;
        host.dispatch_keyboard(&mut engine, &key_up, None)
            .expect("tab keyup");
        let mut reverse = KeyboardInput::key_down("Tab", "Tab");
        reverse.modifiers.shift = true;
        host.dispatch_keyboard(&mut engine, &reverse, None)
            .expect("shift tab");
        assert_eq!(host.focused(), Some(first));

        let events = fired_events(&engine);
        assert!(
            events
                .iter()
                .any(|(target, name, _)| { *target == first.0 && name == "blur" })
        );
        assert!(
            events
                .iter()
                .any(|(target, name, _)| { *target == second.0 && name == "focus" })
        );
    }

    #[test]
    fn accessibility_focus_uses_retained_focus_and_dom_lifecycle() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (first, second) = install_input_nodes(&mut host);
        host.document().lock().expect("document").set_focus(first);
        let mut engine = RecordingEngine::default();

        assert!(host.accessibility_focus(&mut engine, second).unwrap());
        assert_eq!(host.focused(), Some(second));
        assert_eq!(
            fired_events(&engine)
                .iter()
                .map(|(target, name, _)| (*target, name.as_str()))
                .collect::<Vec<_>>(),
            [(first.0, "blur"), (second.0, "focus")]
        );
    }

    #[test]
    fn accessibility_set_value_uses_the_committed_text_event_path() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut document = document.lock().expect("document");
            document.set_attribute(input, "value", "旧值");
            assert!(document.set_text_input_state(input, TextInputState::new("旧值")));
        }
        let _ = host.semantic_snapshot();
        let mut engine = RecordingEngine::default();

        assert!(
            host.accessibility_set_value(&mut engine, input, "新的值")
                .unwrap()
        );
        let document = host.document();
        let document = document.lock().expect("document");
        let state = document.text_input_state(input).expect("text input state");
        assert_eq!(state.value, "新的值");
        assert_eq!(
            state.selection,
            nana_ui_runtime::TextSelection::caret("新的值".len())
        );
        assert_eq!(
            fired_events(&engine)
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["beforeinput", "input"]
        );
        assert_eq!(
            fired_events(&engine)
                .last()
                .and_then(|(_, _, detail)| detail.get("inputType"))
                .and_then(HostValue::as_str),
            Some("insertReplacementText")
        );
        drop(document);

        host.document()
            .lock()
            .expect("document")
            .set_attribute(input, "readonly", "");
        let event_count = fired_events(&engine).len();
        assert!(
            !host
                .accessibility_set_value(&mut engine, input, "禁止写入")
                .unwrap()
        );
        assert_eq!(fired_events(&engine).len(), event_count);
    }

    #[test]
    fn accessibility_selection_updates_runtime_and_allows_read_only_text() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut document = document.lock().expect("document");
            document.set_attribute(input, "value", "你a");
            document.set_attribute(input, "readonly", "");
            assert!(document.set_text_input_state(input, TextInputState::new("你a")));
        }
        let mut engine = RecordingEngine::default();
        let selection = nana_ui_runtime::TextSelection {
            anchor: "你".len(),
            focus: "你a".len(),
        };

        assert!(
            host.accessibility_set_selection(&mut engine, input, selection)
                .unwrap()
        );
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .text_input_state(input)
                .unwrap()
                .selection,
            selection
        );
        assert_eq!(
            fired_events(&engine)
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["select"]
        );

        host.document()
            .lock()
            .expect("document")
            .set_attribute(input, "disabled", "");
        assert!(
            !host
                .accessibility_set_selection(
                    &mut engine,
                    input,
                    nana_ui_runtime::TextSelection::caret(0),
                )
                .unwrap()
        );
    }

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
