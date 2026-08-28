#![recursion_limit = "256"]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

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
//!            兼容绘制：nana-ui Scene host → WGPU
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
//! Runtime / UiScene → nana-ui Scene host
//! ```
//!
//! Cascade SoT for `LayoutStyle` is [`MessageBridge`] stylesheet rules.
//! `NanaTreeDocument::stylesheets` is diagnostics-only (count for host ops).
//! Retained geometry lives in UiWorld/UiScene. `LayoutBoxStore` is a
//! diagnostic layout snapshot after paint. `measure` is the pre-paint fallback +
//! `nana-css-parity` harness. There is no separate synthetic layout branch. See
//! [`docs/layout.md`](../../../docs/layout.md).
//!
//! This crate is the **L1/L2 adapter** (not the paint core):
//! - `css_map` → Layout (`LayoutStyle`) — **neutral** declaration parse
//! - `shell_contract` → documented `nana-*` / utility class → same `LayoutStyle`
//! - `css_cascade` → stylesheet match → same `LayoutStyle`
//! - `measure` → pre-paint / parity boxes (not product paint authority)
//! - `style` → L1 paint value parsing only（不拥有 layout / hit-test）
//! - `widget_map` → Semantics (`WidgetKind` + props)
//! - `layout_map` → Layout direction / Column·Row defaults
//! - Theme tiers → Tokens via `nana-ui` / core（arbitrary CSS hex ≠ token factory）
//!
//! Dependency direction:
//! ```text
//! nana-ui-core          （Style Model 合同：Tokens + Semantics + Layout 数据）
//!      ↑
//! nana-ui-vue ──► nana-js-engine ──► nana-js-v8 (JsEngine trait is the test seam)
//!      ├────────► renderer / tree     (Custom Renderer hostOps)
//!      ├────────► widget_map / layout_map / css_map / shell_contract / css_cascade / measure
//!      ├────────► MessageBridge                       ← L1+L2 同树
//!      ├────────► nana-ui Scene host  (Runtime/UiScene paint)
//!      └────────► nana-ui-web-api     ← L1 Web API 兼容（非 WebView）
//! ```
//!
//! See [`docs/how-it-works.md`](../../../docs/how-it-works.md),
//! [`docs/start.md`](../../../docs/start.md) and
//! [`docs/vue.md`](../../../docs/vue.md).
//!
//! Unique retained authority is UiWorld/UiScene. Feature `scene-view` enables the
//! nana-ui Scene-host adapter for that Scene, including Runtime Scene leaves.
//! WebView is not the product UI path. Application hosts should import [`prelude`].
//! CSS cascade / measure exports are adapter internals.
//!
//! Applications link `nana-js-v8` as the product JS engine. [`nana_js_engine::JsEngine`]
//! remains the test injection seam.
//!
//! Custom Renderer host ops attach through [`nana_js_engine::JsEngine`] only —
//! never via `v8::*`.

mod app;
mod bridge;
#[cfg(feature = "hosted")]
mod canvas_gpu;
mod css_at_rule;
mod css_cascade;
mod css_interactive;
mod css_interactive_apply;
mod css_map;
mod css_paint;
mod css_paint_transform;
#[cfg(feature = "hosted")]
mod hosted_adapter;
mod input;
mod layout_map;
mod measure;
mod multi_window;
#[cfg(feature = "scene-view")]
mod native_component;
mod renderer;
mod scroll;
mod shell_contract;
mod style;
mod svg_inline;
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
#[cfg(feature = "scene-view")]
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

/// Application-facing L1/L2 host API. CSS cascade / measure stay at crate root as
/// adapter internals; do not treat them as the product contract.
pub mod prelude {
    pub use crate::app::{
        MountOptions, NanaVueApp, mount_vue_as_nana, mount_vue_as_nana_with_engine,
        semantic_snapshot_of,
    };
    #[cfg(feature = "hosted")]
    pub use crate::hosted_adapter::{VueHostedProgram, VueHostedRuntime, VueRuntimeProgram};
    pub use crate::input::{
        CompositionEventKind, CompositionInput, HostedInputResult, InputModifiers,
        KeyboardEventKind, KeyboardInput, PointerEventKind, PointerInput, PointerType, WheelInput,
    };
    pub use crate::multi_window::{
        VueRuntime, VueWindowCommand, VueWindowGeometry, VueWindowId, VueWindowOptions,
        VueWindowRole,
    };
    #[cfg(feature = "scene-view")]
    pub use crate::{NanaTextureHandle, NativeComponentRegistry};
    pub use nana_js_engine::HostApiRegistry;
    pub use nana_ui_core::ThemeMode;
    pub use nana_ui_web_api::{compose_runtime_artifact as compose_vue_artifact, shim_artifact};
}

pub use bridge::{
    BridgeEvent, MessageBridge, SelectOptionProp, SemanticRegionViews, SemanticSnapshot,
    SemanticWidget, WidgetId, WidgetKind, WidgetProps, parse_button_kind, parse_control_size,
    resolve_kind_from_hints, widget_id,
};

/// Adapter internals: stylesheet parse / cascade. Not the L1/L2 application prelude.
pub use css_at_rule::{
    FontFaceRule, FontFaceSrc, ImportPrelude, LayerPrelude, MAX_FONT_FACE_BYTES, MAX_IMPORT_DEPTH,
    MAX_REGISTERED_FONT_BYTES, MAX_STYLESHEET_BYTES, MediaEnvironment, MediaFeature, MediaQuery,
    MediaQueryList, MediaType, MemoryStylesheetLoader, ParseStylesheetOptions, StylesheetLoader,
    evaluate_media_query, evaluate_media_query_list, evaluate_supports_condition, is_blocked_href,
    parse_import_prelude, parse_layer_prelude, parse_media_query_list,
};
pub use css_cascade::{
    AnPlusB, AttrCase, AttrOperator, AttrSelector, Combinator, CompoundSelector, DeclarationEntry,
    MatchContext, MatchNode, Selector, SimpleCompound, Specificity, StyleRule,
    StylesheetParseReport, apply_stylesheet_to_layout,
    collect_document_custom_properties_from_rules, matched_declaration_entries,
    matched_declarations, parse_stylesheet, parse_stylesheet_full,
    parse_stylesheet_full_with_options, parse_stylesheet_with_report, rebuild_layout_style,
    selector_matches,
};
pub use css_interactive::{
    GeneratedPseudo, GeneratedPseudoMatch, GeneratedPseudoRule, InteractiveMatchState,
    InteractivePseudo, InteractivePseudoFlags, InteractiveSelector, InteractiveStyleRule,
    KeyframeBlock, KeyframeSelector, KeyframesRule, MediaRule, MotionDeclarations, MotionStyleRule,
    ParsedStylesheet, keyframes_by_name, matched_generated_pseudo, matched_interactive_rules,
    matched_motion_rules, merge_parsed_stylesheet, partition_motion_entries,
};
/// Adapter internals: CSS subset → LayoutStyle. Prefer [`prelude`] for hosts.
pub use css_map::{
    AlignSpec, BoxSizing, CssLayoutParse, DirSpec, DisplaySpec, FlexDirection, FlexWrap,
    FontSizeContext, GridAutoFlow, GridTrack, GridTrackListParse, GridTrackListUnsupported,
    JustifySpec, LayoutStyle, LayoutStyleCss, LengthSpec, LineHeightSpec, OverflowSpec,
    PaddingSpec, ParentBox, PositionSpec, collect_document_css_custom_properties,
    parse_box_edge_length, parse_css_font_family, parse_css_font_size, parse_css_font_weight,
    parse_css_length_px, parse_css_letter_spacing, parse_css_line_height,
    parse_grid_template_columns, parse_grid_track_list_result, parse_inset_length,
    resolve_grid_column_widths, resolve_grid_track_sizes, resolve_paint_color,
};
#[cfg(feature = "hosted")]
pub use hosted_adapter::{VueHostedProgram, VueHostedRuntime, VueRuntimeProgram};
pub use input::{
    CompositionEventKind, CompositionInput, HostedInputResult, InputModifiers, KeyboardEventKind,
    KeyboardInput, PointerEventKind, PointerInput, PointerType, WheelInput,
};
pub use layout_map::{
    apply_direction_to_kind, apply_display_to_kind, default_layout_for_kind, layout_kind_from_tag,
};
/// Adapter internals: pre-paint / parity boxes. Product geometry is Runtime/UiScene.
pub use measure::{
    LayoutNode, MeasuredBox, measure_grid_auto_contribution, measure_layout, node_from_css,
};
pub use multi_window::{
    VueRuntime, VueWindowCommand, VueWindowGeometry, VueWindowId, VueWindowOptions, VueWindowRole,
};
pub use nana_ui_core::ThemeMode;
pub use nana_ui_web_api::{compose_runtime_artifact as compose_vue_artifact, shim_artifact};
#[cfg(feature = "scene-view")]
pub use native_component::{
    NativeComponentCommand, NativeComponentContext, NativeComponentDescriptor,
    NativeComponentFactory, NativeComponentFailure, NativeComponentRegistry, NativePropSchema,
    NativePropType,
};
#[cfg(feature = "scene-view")]
pub use renderer::register_dom_host_ops_with_components;
pub use renderer::{HostDocs, register_dom_host_ops, register_dom_host_ops_with_bridge};
pub use scroll::{
    ScrollAlign, ScrollIntoViewOptions, ScrollIntoViewResult, ScrollOffset, ScrollOffsetStore,
    is_scroll_container, reapply_scroll_translations, scroll_into_view, scrollable_widget_id,
    set_scroll_offset, shared_scroll_offset_store,
};
pub use style::{is_non_token_css_color, map_css_color_for_tokens, parse_css_color};
pub use tree::{
    BoxSnapshot, DocumentId, DomNodeKind, ElementNamespace, LayoutBox, LayoutBoxStore,
    NODE_HANDLE_DOCUMENT_STRIDE, NanaTreeDocument, NodeHandle, SharedRuntimeDocument,
    get_layout_box, get_layout_box_from,
};
#[cfg(feature = "hosted")]
pub use webgpu::JsWebGpuRuntime;

/// Stable JS-facing descriptor for a host-owned texture. The `slot` is an
/// internal routing key accepted by `<nana-gpu :source="handle">`; callers do
/// not need to manufacture it themselves.
#[cfg(feature = "scene-view")]
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

#[cfg(feature = "scene-view")]
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
#[cfg(feature = "scene-view")]
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

/// Vue L1/L2 host: facade document, semantic props, paint-box projection, web-api.
///
/// Retained authority is the inner `UiWorld` / `UiScene`, not these adapters.
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
    /// Last IME field and leftover preedit for JS `compositionend` after Runtime
    /// has already dropped [`nana_ui_runtime::ImeComposition`].
    ime_target: Option<NodeHandle>,
    ime_preedit: String,
    /// Last focus/hover emitted to JS. Scene-host input updates Runtime first;
    /// these remember the previous JS view so blur/over events still fire.
    js_focus: Option<NodeHandle>,
    js_pointer_hover: BTreeMap<u64, Option<NodeHandle>>,
    #[cfg(feature = "scene-view")]
    components: NativeComponentRegistry,
    /// Window-local bindings for host, Canvas, and JS WebGPU textures. Views
    /// are sampled by the Scene host on the same Device/Queue.
    #[cfg(feature = "scene-view")]
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
        #[cfg_attr(not(feature = "scene-view"), allow(unused_mut))]
        let mut document =
            NanaTreeDocument::with_id(document_id, physical_width, physical_height, scale_factor);
        let mut bridge = MessageBridge::new();
        bridge.set_theme(theme);
        // body/html must exist in the semantic forest so inserts into mountRoot
        // parent correctly (otherwise every top-level node stays an orphan root).
        bridge.ensure_document_roots(document.html_root().0, document.mount_root().0);
        #[cfg(feature = "scene-view")]
        let host_textures = HostTextureRegistry::new();
        #[cfg(feature = "scene-view")]
        document.attach_host_textures(host_textures.clone());
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
            ime_target: None,
            ime_preedit: String::new(),
            js_focus: None,
            js_pointer_hover: BTreeMap::new(),
            #[cfg(feature = "scene-view")]
            components: NativeComponentRegistry::new(),
            #[cfg(feature = "scene-view")]
            host_textures,
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

    pub fn shared_runtime_document(&self) -> Arc<SharedRuntimeDocument> {
        self.document
            .lock()
            .expect("vue doc")
            .shared_runtime_document()
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
        #[cfg(feature = "scene-view")]
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

    /// Accumulated CSS skipped-content counters across stylesheet injections:
    /// malformed blocks recovered, dropped declarations, unsupported selectors,
    /// and skipped at-rules. Lets hosts surface missing styles instead of
    /// debugging silently-dropped rules.
    pub fn stylesheet_skips(&self) -> StylesheetParseReport {
        self.bridge.lock().expect("vue bridge").stylesheet_skips()
    }

    #[cfg(feature = "scene-view")]
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }

    #[cfg(feature = "scene-view")]
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
    #[cfg(feature = "scene-view")]
    pub fn invalidate_host_texture(&self, slot: &str) -> bool {
        self.host_textures.invalidate(slot).is_some()
    }

    #[cfg(feature = "scene-view")]
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
    #[cfg(feature = "scene-view")]
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

    #[cfg(feature = "scene-view")]
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

    /// Forward host-op commit rejections recorded by [`NanaTreeDocument`]
    /// to the JS diagnostics sink instead of dropping them silently.
    #[cfg(feature = "scene-view")]
    fn report_commit_rejections(&self, doc: &mut NanaTreeDocument) {
        for rejection in doc.take_commit_rejections() {
            self.report_diagnostic(
                "nana.commit",
                JsDiagnosticLevel::Error,
                format!("rejected host mutation {rejection}"),
                None,
            );
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

    /// Snapshot of the semantic widget forest for Runtime/UiScene.
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

    /// Registry shared by the Vue host and native component commands.
    #[cfg(feature = "scene-view")]
    pub fn components(&self) -> &NativeComponentRegistry {
        &self.components
    }

    #[cfg(feature = "scene-view")]
    pub(crate) fn share_components(&mut self, components: NativeComponentRegistry) {
        self.components = components;
    }

    #[cfg(feature = "scene-view")]
    pub(crate) fn share_host_textures(&mut self, textures: HostTextureRegistry) {
        self.host_textures = textures;
        if let Ok(mut document) = self.document.lock() {
            document.attach_host_textures(self.host_textures.clone());
        }
    }

    #[cfg(feature = "scene-view")]
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
    /// only in [`MessageBridge`] — never treat `NanaTreeDocument` as a second parser.
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

    /// Directory used as the jail / relative base for `@import` and `@font-face` `url()`.
    ///
    /// Typically the Vue document / SFC directory. Unset (empty / `.`) skips
    /// relative imports instead of scanning the process cwd.
    pub fn set_stylesheet_base(&self, base: PathBuf) {
        self.bridge
            .lock()
            .expect("vue bridge")
            .set_stylesheet_base(base);
    }

    /// Builds the framework-owned registry with renderer, DOM and Web APIs.
    pub fn host_api_registry(&self) -> HostApiRegistry {
        let mut api = HostApiRegistry::new();
        api.set_observer(self.diagnostics.host_calls.clone());
        #[cfg(feature = "scene-view")]
        crate::renderer::register_dom_host_ops_with_components_and_layout(
            &mut api,
            Arc::clone(&self.document),
            Arc::clone(&self.bridge),
            Arc::clone(&self.web_api),
            self.components.clone(),
            Arc::clone(&self.layout_boxes),
        );
        #[cfg(not(feature = "scene-view"))]
        crate::renderer::register_dom_host_ops_with_bridge_and_layout(
            &mut api,
            Arc::clone(&self.document),
            Arc::clone(&self.bridge),
            Arc::clone(&self.web_api),
            Arc::clone(&self.layout_boxes),
        );
        #[cfg(feature = "scene-view")]
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
        #[cfg(feature = "scene-view")]
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
        let artifact_name = artifact.name.clone();
        if artifact.is_binary_release() {
            engine.initialize(artifact)?;
            if let Some(base) = crate::renderer::stylesheet_base_from_href(&artifact_name) {
                self.set_stylesheet_base(base);
            }
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
        if let Some(base) = crate::renderer::stylesheet_base_from_href(&artifact_name) {
            self.set_stylesheet_base(base);
        }
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

    /// Commit host ops and flush Runtime layout/extract like a window Scene host.
    /// Headless sessions call this after [`Self::semantic_snapshot`].
    #[cfg(feature = "scene-view")]
    pub fn flush_scene_frame(
        &mut self,
        logical_width: f32,
        logical_height: f32,
    ) -> Result<(), nana_ui_runtime::FrameworkError> {
        {
            let mut doc = self.document.lock().expect("vue doc");
            doc.flush_host_frame();
            self.report_commit_rejections(&mut doc);
        }
        self.flush_runtime_scene(logical_width, logical_height)?;
        let shared = self.shared_runtime_document();
        let document_id = shared.get().document();
        let records: Vec<(u64, nana_ui_scene::SceneRect)> = {
            let runtime = shared.get();
            let scene = runtime.scene();
            runtime
                .context()
                .world()
                .document_order(document_id)
                .into_iter()
                .filter_map(|id| scene.node_bounds(id).map(|rect| (id.get(), rect)))
                .collect()
        };
        self.layout_boxes.begin_frame();
        for (id, rect) in records {
            self.layout_boxes
                .record(NodeHandle(id), rect.x, rect.y, rect.width, rect.height);
        }
        Ok(())
    }

    #[cfg(feature = "scene-view")]
    fn flush_runtime_scene(
        &mut self,
        logical_width: f32,
        logical_height: f32,
    ) -> Result<(), nana_ui_runtime::FrameworkError> {
        self.shared_runtime_document().get_mut().flush(
            nana_ui_runtime::LayoutViewport::new(logical_width, logical_height),
            &mut nana_ui::NanaTextShaper::default(),
        )?;
        Ok(())
    }

    pub fn resolve_layout(&mut self) {
        let painted = {
            let doc = self.document.lock().expect("vue doc");
            self.layout_boxes
                .retain(|id| doc.contains_handle(NodeHandle(id)));
            self.layout_boxes.snapshot()
        };
        if painted.is_empty() {
            // Empty paint cache: keep Runtime boxes; CSS auto-height 0 must not overwrite them.
            let mut bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            doc.flush_host_frame();
            #[cfg(feature = "scene-view")]
            self.report_commit_rejections(&mut doc);
            bridge.resolve_missing_document_layout(&mut doc);
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&painted);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }

    /// Copy Scene paint boxes into the document cache (call after a frame draws).
    ///
    /// `layoutBox` / `getBoundingClientRect` already prefer the live store; this
    /// keeps hit-tests and `snapshot_boxes` aligned with paint.
    pub fn sync_scene_layout_boxes(&mut self) {
        let painted = {
            let doc = self.document.lock().expect("vue doc");
            self.layout_boxes
                .retain(|id| doc.contains_handle(NodeHandle(id)));
            self.layout_boxes.snapshot()
        };
        if painted.is_empty() {
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&painted);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }

    /// Per-window Scene layout writeback buffer (same as probes / `layoutBox`).
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
    /// Nested drain: 0ms timeouts (ResizeObserver) still flush in-loop.
    /// rAF follows this host frame once; nested rAF (Vue `<Transition>`
    /// `nextFrame` is double-rAF, used by after-leave / Dialog/Drawer) waits
    /// for `next_wakeup` (~16ms) instead of spinning a fake 16ms deadline
    /// inside the same pump.
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
        if !fetch_completions.is_empty()
            && let Some(drain) = self.drain_fetch
        {
            let count = fetch_completions.len();
            engine.invoke(
                drain,
                &[HostValue::Array(fetch_completions.into_iter().collect())],
            )?;
            fired += count;
            engine.run_microtasks()?;
        }
        let frame_now = Instant::now();
        {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.begin_host_frame(frame_now);
        }
        // Cap nested 0ms timeouts. rAF is one host frame, not this loop.
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
        {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.end_host_frame(Instant::now());
        }
        {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            if doc.host_animation_epoch().is_none() {
                bridge.tick_css_animations(&mut doc);
            }
        }
        self.resolve_layout();
        if let Some(notify) = self.notify_layout {
            engine.invoke(notify, &[])?;
            engine.run_microtasks()?;
        }
        Ok(fired)
    }

    /// Earliest timer/rAF/fetch wake requested by the Web API state.
    /// Returns `None` when the runtime is idle.
    pub fn next_wakeup(&self) -> Option<Instant> {
        let animation_wakeup = self
            .document
            .lock()
            .ok()
            .and_then(|doc| doc.next_animation_wakeup());
        let web_wakeup = self
            .web_api
            .lock()
            .ok()
            .and_then(|guard| guard.next_wakeup(Instant::now()));
        #[cfg(feature = "hosted")]
        let gpu_wakeup = self.webgpu.as_ref().and_then(JsWebGpuRuntime::next_wakeup);
        #[cfg(not(feature = "hosted"))]
        let gpu_wakeup: Option<Instant> = None;
        animation_wakeup
            .into_iter()
            .chain(web_wakeup)
            .chain(gpu_wakeup)
            .min()
    }

    pub fn set_host_animation_epoch(&self, epoch: Instant) {
        if let Ok(mut doc) = self.document.lock() {
            doc.set_host_animation_epoch(epoch);
        }
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

    /// Resolve the topmost node under `(x, y)` for native input routing.
    ///
    /// Scene paint boxes in [`LayoutBoxStore`] win when present so file drag
    /// and early-frame probes match painted geometry. Runtime hit-test is the
    /// fallback when no paint box covers the point.
    fn hit_test_client_point(&self, x: f32, y: f32) -> Option<NodeHandle> {
        let doc = self.document.lock().expect("vue doc");
        if !self.layout_boxes.snapshot().is_empty() {
            let mut stack = vec![doc.mount_root()];
            let mut preorder = Vec::new();
            while let Some(node) = stack.pop() {
                preorder.push(node);
                for child in doc.children_of(node).into_iter().rev() {
                    stack.push(child);
                }
            }
            for handle in preorder.into_iter().rev() {
                if self.layout_boxes.contains_point(handle, x, y) {
                    return Some(handle);
                }
            }
        }
        doc.hit_test(x, y)
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
            position.and_then(|(x, y)| self.hit_test_client_point(x, y));
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

    /// Route a Runtime/bridge action into the queue and JS event listeners.
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
        let committed_input = match &event {
            BridgeEvent::Input { id, value } => Some((*id, value.as_str())),
            _ => None,
        };
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
                #[cfg(feature = "scene-view")]
                BridgeEvent::MenuSearch { .. } | BridgeEvent::MenuPath { .. } => {
                    // Host-only menu chrome; no JS listener required.
                    Vec::new()
                }
            }
        };
        #[cfg(feature = "scene-view")]
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

    fn flush_interactive_css_if_needed(&self) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        if !bridge.has_interactive_css() {
            return;
        }
        let mut doc = self.document.lock().expect("vue doc");
        bridge.reapply_interactive_cascade(&mut doc);
        bridge.sync_cascaded_layout_into_runtime(&mut doc);
        doc.flush_host_frame();
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
            WidgetKind::ListItem | WidgetKind::SidebarRow | WidgetKind::InteractiveCard => {
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
        self.dispatch_pointer_result_with(engine, input, true)
    }

    /// Fire Vue/DOM pointer events after the Scene host already applied Runtime input.
    pub fn emit_pointer_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_pointer_result_with(engine, input, false)
    }

    fn dispatch_pointer_result_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
        commit_runtime: bool,
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
            let previous = if commit_runtime {
                self.document
                    .lock()
                    .expect("vue doc")
                    .pointer_hover(input.pointer_id)
            } else {
                self.js_pointer_hover
                    .get(&input.pointer_id)
                    .copied()
                    .flatten()
            };
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
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .set_pointer_hover(input.pointer_id, physical_hit);
                    self.flush_interactive_css_if_needed();
                }
                self.js_pointer_hover.insert(input.pointer_id, physical_hit);
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
                if commit_runtime && let Some(target) = target {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .press_pointer(input.pointer_id, target);
                    self.flush_interactive_css_if_needed();
                }
                let (previous, next) = if commit_runtime {
                    self.focus_target_at(input.client_x, input.client_y)
                } else {
                    let previous = self.js_focus;
                    let next = self.document.lock().expect("vue doc").focused();
                    (previous, next)
                };
                if previous != next {
                    if let Some(previous) = previous {
                        self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                    }
                    if let Some(next) = next {
                        self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                    }
                    if commit_runtime {
                        self.flush_interactive_css_if_needed();
                    }
                }
                self.js_focus = next;
            }
            PointerEventKind::Up => {
                let pressed = if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .release_pointer_press(input.pointer_id)
                } else {
                    target.or(physical_hit)
                };
                if commit_runtime && pressed.is_some() {
                    self.flush_interactive_css_if_needed();
                }
                if !default_prevented
                    && let Some(click_target) = pressed
                    && physical_hit == Some(click_target)
                {
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
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .release_pointer_press(input.pointer_id);
                    self.flush_interactive_css_if_needed();
                }
            }
            PointerEventKind::Move => {}
        }

        self.flush_pointer_capture_events(engine)?;

        if matches!(input.kind, PointerEventKind::Up | PointerEventKind::Cancel) {
            if commit_runtime {
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
        self.dispatch_wheel_result_with(engine, input, true)
    }

    pub fn emit_wheel_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_wheel_result_with(engine, input, false)
    }

    fn dispatch_wheel_result_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
        commit_runtime: bool,
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
        let mut consumed = !allowed;
        if allowed && commit_runtime {
            let delta = crate::scroll::wheel_scroll_delta(&input);
            let scrolled = {
                let painted = self.layout_boxes.snapshot();
                let mut document = self.document.lock().expect("vue doc");
                // `pump_frame` / engine flush rewrites fixture chrome and
                // shrinks overflow content. Restore the host paint boxes
                // before committing ScrollOffset so chrome stays put.
                if !painted.is_empty() {
                    document.inject_layout_boxes(&painted);
                }
                let bridge = self.bridge.lock().expect("vue bridge");
                crate::scroll::apply_runtime_wheel_from(
                    &mut document,
                    &bridge,
                    &self.layout_boxes,
                    Some(target),
                    delta,
                )
                .is_some()
            };
            // Consume only after catalog qualification, when Scene owns the
            // offset/clip.
            consumed |= scrolled && {
                #[cfg(feature = "scene-view")]
                {
                    nana_ui::component_uses_runtime(nana_ui::component_ids::SIDEBAR_FRAME)
                }
                #[cfg(not(feature = "scene-view"))]
                {
                    true
                }
            };
        } else if allowed {
            consumed = true;
        }
        Ok(HostedInputResult {
            targeted: true,
            default_prevented: !allowed,
            consumed,
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
        self.dispatch_keyboard_with(engine, input, target, true)
    }

    pub fn emit_keyboard_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_keyboard_with(engine, input, target, false)
    }

    fn dispatch_keyboard_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
        commit_runtime: bool,
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
                    | WidgetKind::SidebarRow
                    | WidgetKind::InteractiveCard => {
                        !repeated && matches!(key.as_str(), "enter" | " " | "space" | "spacebar")
                    }
                    WidgetKind::Switch | WidgetKind::Checkbox => {
                        !repeated && matches!(key.as_str(), " " | "space" | "spacebar")
                    }
                    WidgetKind::Range => commit_runtime && requested_value.is_some(),
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
            let (previous, next) = if commit_runtime {
                self.advance_tab_focus(input.modifiers.shift)
            } else {
                let previous = self.js_focus;
                let next = self.document.lock().expect("vue doc").focused();
                (previous, next)
            };
            if previous != next {
                if let Some(previous) = previous {
                    self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                }
                if let Some(next) = next {
                    self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                }
            }
            self.js_focus = next;
            if commit_runtime {
                self.flush_interactive_css_if_needed();
            }
        } else if !commit_runtime {
            self.js_focus = self.document.lock().expect("vue doc").focused();
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(allowed)
    }

    #[cfg(any(test, feature = "hosted"))]
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
        self.js_focus = Some(target);
        self.flush_interactive_css_if_needed();
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    #[cfg(feature = "hosted")]
    pub(crate) fn accessibility_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let result = self.semantic_default_action(engine, target, None, Some(BTreeMap::new()))?;
        Ok(result.handled && !result.default_prevented)
    }

    #[cfg(any(test, feature = "hosted"))]
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

    #[cfg(any(test, feature = "hosted"))]
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
        self.commit_text_on(engine, target, text, input_type)
    }

    /// Commit text into a specific field. Used so leftover IME after blur
    /// cannot retarget the newly focused node.
    fn commit_text_on<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        text: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        if self.text_commit_blocked(target) {
            return Ok(false);
        }
        let (widget_editable, existing, fallback_value, tag, contenteditable) = {
            let widget_editable = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .is_some_and(|widget| {
                    widget.kind.is_choice_field()
                        || matches!(
                            widget.kind,
                            WidgetKind::Input | WidgetKind::Textarea | WidgetKind::ContextMenu
                        )
                });
            let document = self.document.lock().expect("vue doc");
            (
                widget_editable,
                document.text_input_state(target),
                document.get_attribute(target, "value").unwrap_or_default(),
                document.element_tag(target),
                document.get_attribute(target, "contenteditable"),
            )
        };
        let editable = widget_editable
            || matches!(
                tag.as_deref(),
                Some(
                    "input"
                        | "textarea"
                        | "nana-input"
                        | "nana-textarea"
                        | "nana-context-menu"
                        | "nana-search"
                        | "nana-dropdown"
                )
            )
            || contenteditable.is_some_and(|value| value != "false");
        let Some(mut state) =
            existing.or_else(|| editable.then(|| TextInputState::new(fallback_value)))
        else {
            return Ok(false);
        };
        if !state.replace_selection(text) {
            return Ok(false);
        }
        let next = state;
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
        #[cfg(feature = "scene-view")]
        {
            let is_menu = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .is_some_and(|widget| widget.kind == WidgetKind::ContextMenu);
            if is_menu {}
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    fn text_commit_blocked(&self, target: NodeHandle) -> bool {
        if self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .is_some_and(|widget| widget.props.disabled || widget.props.read_only)
        {
            return true;
        }
        let document = self.document.lock().expect("vue doc");
        document.element_tag(target).is_none()
            || document.get_attribute(target, "disabled").is_some()
            || document.get_attribute(target, "readonly").is_some()
    }

    fn remember_ime_target(&mut self, target: NodeHandle, preedit: String) {
        self.ime_target = Some(target);
        self.ime_preedit = preedit;
    }

    fn clear_ime_target(&mut self) {
        self.ime_target = None;
        self.ime_preedit.clear();
    }

    fn ime_composition_target(document: &NanaTreeDocument) -> Option<NodeHandle> {
        document
            .collect_element_preorder(document.html_root())
            .into_iter()
            .map(NodeHandle)
            .find(|&node| document.ime_composition(node).is_some())
    }

    /// Composition target and leftover preedit. Runtime drops `ImeComposition`
    /// on blur, so the remembered field wins over current focus.
    fn take_ime_leftover(&mut self) -> Option<(NodeHandle, String)> {
        let remembered = self.ime_target.take();
        let remembered_text = std::mem::take(&mut self.ime_preedit);
        let mut document = self.document.lock().expect("vue doc");
        let target = Self::ime_composition_target(&document).or(remembered)?;
        let data = document
            .ime_composition(target)
            .map(|ime| ime.text)
            .unwrap_or(remembered_text);
        document.set_ime_composition(target, None);
        Some((target, data))
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
        match input.kind {
            CompositionEventKind::Start | CompositionEventKind::Update => {
                self.remember_ime_target(target, input.data.clone());
            }
            CompositionEventKind::End => {
                self.clear_ime_target();
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
        self.dispatch_composition_event_with(engine, target, input, true)
    }

    fn dispatch_composition_event_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        input: &CompositionInput,
        commit_runtime: bool,
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
            return if commit_runtime {
                self.commit_text_on(engine, target, &input.data, "insertCompositionText")
            } else {
                self.emit_text_events_from_runtime(
                    engine,
                    target,
                    &input.data,
                    "insertCompositionText",
                )
            };
        }
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    pub(crate) fn emit_text_events_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        data: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        let value = {
            let document = self.document.lock().expect("vue doc");
            document
                .text_input_state(target)
                .map(|state| state.value)
                .or_else(|| document.get_attribute(target, "value"))
                .unwrap_or_default()
        };
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(data));
        detail.insert("inputType".into(), HostValue::string(input_type));
        detail.insert("value".into(), HostValue::string(&value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        self.document
            .lock()
            .expect("vue doc")
            .set_attribute(target, "value", &value);
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }

    /// Forwards desktop winit IME lifecycle into Vue composition events.
    ///
    /// Preedit stays on Runtime [`nana_ui_runtime::ImeComposition`]. Commit and
    /// leftover Disabled preedit update Runtime [`TextInputState`] on the
    /// original composition field through [`Self::commit_text_on`], matching
    /// [`Self::dispatch_composition`] End even if focus has moved. This path
    /// does not write a second editor buffer.
    pub fn dispatch_native_ime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_native_ime_with(engine, event, true)
    }

    /// Emit JS composition/`input` after the Scene host already applied Runtime IME.
    pub fn emit_native_ime_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_native_ime_with(engine, event, false)
    }

    fn dispatch_native_ime_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
        commit_runtime: bool,
    ) -> Result<bool, JsEngineError> {
        match event {
            ImeEvent::Enabled => Ok(self.focused().is_some()),
            ImeEvent::Preedit { text, selection } => {
                let Some(target) = self.focused() else {
                    return Ok(false);
                };
                let started = if commit_runtime {
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
                } else {
                    self.ime_target.is_none()
                };
                self.remember_ime_target(target, text.clone());
                if started {
                    self.dispatch_composition_event_with(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::Start, ""),
                        commit_runtime,
                    )?;
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::Update, text),
                    commit_runtime,
                )
            }
            ImeEvent::Commit(text) => {
                let Some(target) = self.ime_target.or_else(|| self.focused()) else {
                    return Ok(false);
                };
                self.clear_ime_target();
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .set_ime_composition(target, None);
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, text),
                    commit_runtime,
                )
            }
            ImeEvent::Disabled => {
                if commit_runtime {
                    let leftover = self.take_ime_leftover();
                    let Some((target, data)) = leftover else {
                        return Ok(self.focused().is_some());
                    };
                    if self.text_commit_blocked(target) {
                        return Ok(true);
                    }
                    return self.dispatch_composition_event_with(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::End, data),
                        true,
                    );
                }
                let leftover = self.ime_target.take().map(|target| {
                    let data = std::mem::take(&mut self.ime_preedit);
                    (target, data)
                });
                let Some((target, data)) = leftover else {
                    return Ok(self.focused().is_some());
                };
                if data.is_empty() {
                    return Ok(true);
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, data),
                    false,
                )
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

    /// Platform IME request for a focused Runtime Input/Textarea.
    ///
    /// When this is enabled, the hosted window must not also feed winit IME
    /// into a second editor.
    pub fn text_input_request(&self) -> Option<nana_ui_platform::TextInputRequest> {
        let target = self.focused()?;
        let document = self.document.lock().ok()?;
        let _ = document.text_input_state(target)?;
        let widget = self.bridge.lock().ok()?.get(target.0).cloned();
        let (disabled, read_only, secure) = widget
            .as_ref()
            .map(|widget| {
                (
                    widget.props.disabled,
                    widget.props.read_only,
                    widget.props.secure,
                )
            })
            .unwrap_or((false, false, false));
        if disabled || read_only {
            return Some(nana_ui_platform::TextInputRequest {
                enabled: false,
                cursor_area: None,
                purpose: nana_ui_platform::TextInputPurpose::Normal,
            });
        }
        let cursor_area =
            crate::get_layout_box_from(&self.layout_boxes, &document, target).map(|layout| {
                nana_ui_core::LogicalRect::new(layout.x, layout.y, layout.width, layout.height)
            });
        Some(nana_ui_platform::TextInputRequest {
            enabled: true,
            cursor_area,
            purpose: if secure {
                nana_ui_platform::TextInputPurpose::Password
            } else {
                nana_ui_platform::TextInputPurpose::Normal
            },
        })
    }

    #[cfg(feature = "scene-view")]
    fn native_component_name(&self, id: WidgetId) -> Option<String> {
        self.bridge
            .lock()
            .ok()?
            .get(id)
            .and_then(|widget| widget.props.native_component.clone())
    }

    #[cfg(not(feature = "scene-view"))]
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
        #[cfg(feature = "scene-view")]
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
        host.sync_scene_layout_boxes();
        (first, second)
    }

    fn install_focused_native_input(
        host: &mut VueHost,
        value: &str,
    ) -> (NodeHandle, nana_ui_runtime::DocumentId) {
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(host);
        host.bridge().lock().expect("bridge").register(
            input.0,
            WidgetKind::Input,
            WidgetProps {
                value: value.into(),
                ..WidgetProps::default()
            },
        );
        let document_id = {
            let snapshot = host.bridge().lock().expect("bridge").snapshot();
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.sync_semantic_styles(&snapshot);
            doc.set_attribute(input, "value", value);
            doc.set_focus(input);
            doc.runtime_document().document()
        };
        (input, document_id)
    }

    fn install_textarea_node(host: &mut VueHost, value: &str) -> NodeHandle {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let root = doc.mount_root();
        let area = doc.create_element("textarea");
        doc.set_attribute(area, "value", value);
        assert!(doc.set_text_input_state(area, TextInputState::new(value)));
        doc.insert(area, root, None);
        doc.set_focus(area);
        drop(doc);

        let store = host.layout_box_store();
        store.begin_frame();
        store.record(area, 0.0, 0.0, 160.0, 80.0);
        host.sync_scene_layout_boxes();
        area
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

    fn install_sidebar_frame(
        host: &mut VueHost,
    ) -> (NodeHandle, NodeHandle, NodeHandle, NodeHandle) {
        let (frame, top, body, footer, content) = {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            let root = doc.mount_root();
            let frame = doc.create_element("nana-sidebar-frame");
            let top = doc.create_element("nana-column");
            let body = doc.create_element("nana-column");
            let footer = doc.create_element("nana-column");
            let content = doc.create_element("nana-sidebar-row");
            doc.set_attribute(body, "class", "nana-sidebar-frame__body");
            doc.set_attribute(body, "data-slot", "sidebar-body");
            doc.insert(frame, root, None);
            doc.insert(top, frame, None);
            doc.insert(body, frame, None);
            doc.insert(footer, frame, None);
            doc.insert(content, body, None);
            (frame, top, body, footer, content)
        };

        {
            let mut bridge = host.bridge.lock().expect("bridge");
            let mut frame_props = WidgetProps::default();
            frame_props.class_names = vec!["nana-sidebar-frame".into()];
            frame_props
                .layout
                .apply_class_layout_hints(&frame_props.class_names);
            bridge.register(frame.0, WidgetKind::SidebarFrame, frame_props);

            let mut top_props = WidgetProps::default();
            top_props.class_names = vec!["nana-sidebar-frame__top".into()];
            top_props
                .attrs
                .insert("data-slot".into(), "sidebar-top".into());
            top_props
                .layout
                .apply_class_layout_hints(&top_props.class_names);
            bridge.register(top.0, WidgetKind::Column, top_props);

            let mut body_props = WidgetProps::default();
            body_props.class_names = vec!["nana-sidebar-frame__body".into()];
            body_props
                .attrs
                .insert("data-slot".into(), "sidebar-body".into());
            body_props
                .layout
                .apply_class_layout_hints(&body_props.class_names);
            bridge.register(body.0, WidgetKind::Column, body_props);

            let mut footer_props = WidgetProps::default();
            footer_props.class_names = vec!["nana-sidebar-frame__footer".into()];
            footer_props
                .attrs
                .insert("data-slot".into(), "sidebar-footer".into());
            footer_props
                .layout
                .apply_class_layout_hints(&footer_props.class_names);
            bridge.register(footer.0, WidgetKind::Column, footer_props);

            let mut content_props = WidgetProps::default();
            content_props.label = "工作区".into();
            bridge.register(content.0, WidgetKind::SidebarRow, content_props);

            bridge.insert_child(top.0, frame.0, None);
            bridge.insert_child(body.0, frame.0, None);
            bridge.insert_child(footer.0, frame.0, None);
            bridge.insert_child(content.0, body.0, None);
        }

        let snapshot = host.bridge.lock().expect("bridge").snapshot();
        let store = host.layout_box_store();
        store.begin_frame();
        store.record(frame, 0.0, 0.0, 220.0, 320.0);
        store.record(top, 0.0, 0.0, 220.0, 40.0);
        store.record(body, 0.0, 40.0, 220.0, 200.0);
        store.record(content, 0.0, 40.0, 220.0, 400.0);
        store.record(footer, 0.0, 250.0, 220.0, 40.0);
        {
            let mut doc = host.document.lock().expect("document");
            doc.sync_semantic_styles(&snapshot);
            doc.apply_layout_boxes(&store.snapshot());
        }
        (frame, top, body, footer)
    }

    #[test]
    fn sidebar_frame_wheel_updates_runtime_body_without_moving_chrome() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (_frame, top, body, footer) = install_sidebar_frame(&mut host);
        let mut engine = RecordingEngine::default();

        let top_before = host
            .document
            .lock()
            .expect("document")
            .layout_box(top)
            .expect("top box");
        let footer_before = host
            .document
            .lock()
            .expect("document")
            .layout_box(footer)
            .expect("footer box");
        assert_eq!(
            host.document
                .lock()
                .expect("document")
                .scroll_offset(body)
                .y,
            0.0
        );
        assert_eq!(
            host.document.lock().expect("document").scroll_offset(top).y,
            0.0
        );
        assert_eq!(
            host.document
                .lock()
                .expect("document")
                .scroll_offset(footer)
                .y,
            0.0
        );

        let result = host
            .dispatch_wheel_result(&mut engine, WheelInput::pixels(20.0, 80.0, 0.0, -48.0))
            .expect("wheel");
        assert!(result.targeted);
        assert!(!result.default_prevented);
        assert_eq!(
            result.consumed,
            {
                #[cfg(feature = "scene-view")]
                {
                    nana_ui::component_uses_runtime(nana_ui::component_ids::SIDEBAR_FRAME)
                }
                #[cfg(not(feature = "scene-view"))]
                {
                    true
                }
            },
            "consume hosted wheel only when Scene owns SidebarFrame paint"
        );

        let document = host.document.lock().expect("document");
        assert!(
            document.scroll_offset(body).y > 0.0,
            "body Runtime scroll_offset must move"
        );
        assert_eq!(document.scroll_offset(top).y, 0.0);
        assert_eq!(document.scroll_offset(footer).y, 0.0);
        assert_eq!(document.layout_box(top).expect("top after"), top_before);
        assert_eq!(
            document.layout_box(footer).expect("footer after"),
            footer_before
        );
        drop(document);
        assert!(
            crate::scroll::shared_scroll_offset_store()
                .take_pending()
                .is_empty(),
            "sidebar body must not depend on pending scroll tasks"
        );
    }

    #[test]
    fn sidebar_frame_wheel_prevent_default_does_not_scroll_runtime() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (_frame, top, body, footer) = install_sidebar_frame(&mut host);
        let mut engine = RecordingEngine {
            prevent_event: Some("wheel".into()),
            ..Default::default()
        };

        let result = host
            .dispatch_wheel_result(&mut engine, WheelInput::pixels(20.0, 80.0, 0.0, -48.0))
            .expect("wheel");
        assert!(result.targeted);
        assert!(result.default_prevented);
        assert!(result.consumed);

        let document = host.document.lock().expect("document");
        assert_eq!(document.scroll_offset(body).y, 0.0);
        assert_eq!(document.scroll_offset(top).y, 0.0);
        assert_eq!(document.scroll_offset(footer).y, 0.0);
    }

    #[test]
    fn scene_scroll_event_updates_runtime_without_firing_vue_event() {
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

        let layout_x = |host: &VueHost| {
            let api = host.host_api_registry();
            match api
                .call("layoutBox", &[HostValue::Number(node.0 as f64)])
                .expect("layoutBox")
            {
                HostValue::Object(map) => map.get("x").and_then(HostValue::as_f64).unwrap(),
                other => panic!("expected object, got {other:?}"),
            }
        };
        assert_eq!(layout_x(&first), 10.0);
        assert_eq!(layout_x(&second), 100.0);
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
    fn native_ime_commit_updates_runtime_value_and_emits_input() {
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
        {
            let document = host.document();
            let document = document.lock().expect("document");
            let composition = document
                .ime_composition(input)
                .expect("runtime preedit stays on ImeComposition");
            assert_eq!(composition.text, "世");
            assert_eq!(composition.selection, Some((3, 3)));
            assert_eq!(
                document.get_attribute(input, "value").as_deref(),
                Some("Nana"),
                "preedit must not mutate committed Runtime value"
            );
        }
        host.dispatch_native_ime(&mut engine, &ImeEvent::Commit("世界".into()))
            .expect("commit lifecycle");
        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        let state = document
            .text_input_state(input)
            .expect("runtime text input state");
        assert_eq!(state.value, "Nana世界");
        assert_eq!(
            state.selection,
            nana_ui_runtime::TextSelection::caret("Nana世界".len())
        );
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana世界")
        );
        drop(document);

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
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
            Some("Nana世界")
        );
        assert_eq!(
            input_event.2.get("inputType").and_then(HostValue::as_str),
            Some("insertCompositionText")
        );
        assert_eq!(
            events.iter().filter(|(_, name, _)| name == "input").count(),
            1,
            "native IME commit must not double-insert"
        );
    }

    #[test]
    fn scene_host_ime_path_commits_once_into_runtime_then_emits_js() {
        let mut host = VueHost::new();
        let (input, document_id) = install_focused_native_input(&mut host, "Nana");
        let mut engine = RecordingEngine::default();

        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            assert!(
                doc.context_mut()
                    .set_ime_preedit(document_id, "世".into(), Some((0, "世".len())))
                    .expect("runtime preedit")
            );
        }
        host.emit_native_ime_from_runtime(
            &mut engine,
            &ImeEvent::Preedit {
                text: "世".into(),
                selection: Some((0, "世".len())),
            },
        )
        .expect("emit preedit");
        {
            let document = host.document();
            let document = document.lock().expect("document");
            assert_eq!(
                document.ime_composition(input).map(|ime| ime.text),
                Some("世".into())
            );
            assert_eq!(
                document.text_input_state(input).map(|state| state.value),
                Some("Nana".into()),
                "emit must not write a second preedit buffer"
            );
        }

        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            assert!(
                doc.context_mut()
                    .commit_ime(document_id, "世界")
                    .expect("runtime commit")
            );
        }
        host.emit_native_ime_from_runtime(&mut engine, &ImeEvent::Commit("世界".into()))
            .expect("emit commit");

        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        let state = document
            .text_input_state(input)
            .expect("runtime committed once");
        assert_eq!(state.value, "Nana世界");
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana世界")
        );
        let field = document
            .accessibility_snapshot()
            .into_iter()
            .find(|node| node.id.get() == input.0)
            .expect("committed value stays on the AccessKit TextInput");
        assert_eq!(field.value.as_deref(), Some("Nana世界"));
        drop(document);

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "compositionstart",
                "compositionupdate",
                "compositionend",
                "beforeinput",
                "input"
            ]
        );
        assert_eq!(
            events.iter().filter(|(_, name, _)| name == "input").count(),
            1,
            "scene-host IME must not double-insert on emit"
        );
    }

    #[test]
    fn scene_host_ime_disabled_commits_leftover_once() {
        let mut host = VueHost::new();
        let (input, document_id) = install_focused_native_input(&mut host, "Nana");
        let mut engine = RecordingEngine::default();

        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            assert!(
                doc.context_mut()
                    .set_ime_preedit(document_id, "世".into(), Some((0, "世".len())))
                    .expect("runtime preedit")
            );
        }
        host.emit_native_ime_from_runtime(
            &mut engine,
            &ImeEvent::Preedit {
                text: "世".into(),
                selection: Some((0, "世".len())),
            },
        )
        .expect("emit preedit");
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            let leftover = doc.ime_composition(input).expect("preedit").text.clone();
            assert!(
                doc.context_mut()
                    .commit_ime(document_id, &leftover)
                    .expect("runtime leftover commit")
            );
        }
        host.emit_native_ime_from_runtime(&mut engine, &ImeEvent::Disabled)
            .expect("emit disabled");

        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        assert_eq!(
            document
                .text_input_state(input)
                .expect("leftover committed once")
                .value,
            "Nana世"
        );
        drop(document);
        assert_eq!(
            fired_events(&engine)
                .iter()
                .filter(|(_, name, _)| name == "input")
                .count(),
            1
        );
    }

    #[test]
    fn window_blur_keeps_runtime_text_field_focus() {
        let mut host = VueHost::new();
        let (input, _) = install_focused_native_input(&mut host, "NanaUI");
        let mut engine = RecordingEngine::default();
        host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Blur)
            .expect("window blur");
        let document = host.document();
        let doc = document.lock().expect("document");
        assert_eq!(doc.focused(), Some(input));
        let field = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|node| node.id.get() == input.0)
            .expect("TextField remains in the tree");
        assert!(field.focused);
        assert!(
            !fired_events(&engine)
                .iter()
                .any(|(_, name, _)| name == "blur")
        );
    }

    #[test]
    fn native_ime_commit_updates_runtime_textarea_multiline_state() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let area = install_textarea_node(&mut host, "第一行\n");
        let mut engine = RecordingEngine::default();

        host.dispatch_native_ime(
            &mut engine,
            &ImeEvent::Preedit {
                text: "第二".into(),
                selection: Some((0, "第".len())),
            },
        )
        .expect("textarea preedit");
        {
            let document = host.document();
            let document = document.lock().expect("document");
            let composition = document
                .ime_composition(area)
                .expect("textarea keeps CJK preedit selection");
            assert_eq!(composition.text, "第二");
            assert_eq!(composition.selection, Some((0, "第".len())));
            assert_eq!(
                document
                    .text_input_state(area)
                    .expect("textarea state")
                    .value,
                "第一行\n"
            );
        }

        host.dispatch_native_ime(&mut engine, &ImeEvent::Commit("第二行".into()))
            .expect("textarea commit");
        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(area).is_none());
        let state = document
            .text_input_state(area)
            .expect("textarea runtime state");
        assert_eq!(state.value, "第一行\n第二行");
        assert_eq!(
            state.selection,
            nana_ui_runtime::TextSelection::caret("第一行\n第二行".len())
        );
        assert_eq!(
            document.get_attribute(area, "value").as_deref(),
            Some("第一行\n第二行")
        );
        drop(document);

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "compositionstart",
                "compositionupdate",
                "compositionend",
                "beforeinput",
                "input"
            ]
        );
        let input_event = events.last().expect("textarea input event");
        assert_eq!(input_event.0, area.0);
        assert_eq!(
            input_event.2.get("value").and_then(HostValue::as_str),
            Some("第一行\n第二行")
        );
        assert_eq!(
            input_event.2.get("inputType").and_then(HostValue::as_str),
            Some("insertCompositionText")
        );
        assert_eq!(
            events.iter().filter(|(_, name, _)| name == "input").count(),
            1,
            "textarea IME commit must not double-insert"
        );
    }

    #[test]
    fn focused_runtime_textarea_advertises_hosted_ime_request() {
        let mut host = VueHost::new();
        let area = install_textarea_node(&mut host, "第一行\n");
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_focus(area);
        }
        let request = host
            .text_input_request()
            .expect("focused textarea owns IME");
        assert!(request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
    }

    #[test]
    fn native_ime_disabled_commits_leftover_runtime_preedit() {
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
                selection: Some((0, "世".len())),
            },
        )
        .expect("preedit");
        host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
            .expect("disabled leftover");

        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        let state = document
            .text_input_state(input)
            .expect("runtime text input state");
        assert_eq!(state.value, "Nana世");
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana世")
        );
        drop(document);

        let events = fired_events(&engine);
        assert_eq!(
            events
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "compositionstart",
                "compositionupdate",
                "compositionend",
                "beforeinput",
                "input"
            ]
        );
        assert_eq!(
            events
                .last()
                .and_then(|(_, _, detail)| detail.get("inputType"))
                .and_then(HostValue::as_str),
            Some("insertCompositionText")
        );
    }

    #[test]
    fn commit_text_ignores_disabled_and_read_only_input() {
        for (disabled, read_only) in [(true, false), (false, true)] {
            let mut host = VueHost::new();
            host.fire_event = Some(JsFunctionId(1));
            let (input, next) = install_input_nodes(&mut host);
            host.bridge().lock().expect("bridge").register(
                input.0,
                WidgetKind::Input,
                WidgetProps {
                    value: "Nana".into(),
                    disabled,
                    read_only,
                    ..WidgetProps::default()
                },
            );
            {
                let document = host.document();
                let mut doc = document.lock().expect("document");
                doc.set_attribute(input, "value", "Nana");
                doc.set_focus(input);
                assert!(doc.set_text_input_state(input, TextInputState::new("Nana")));
            }
            let mut engine = RecordingEngine::default();
            assert!(
                !host.commit_text(&mut engine, "界", "insertText").unwrap(),
                "disabled={disabled} read_only={read_only}"
            );
            assert!(
                host.dispatch_key(&mut engine, "a", "KeyA", Some(input))
                    .unwrap()
            );
            {
                let document = host.document();
                let doc = document.lock().expect("document");
                assert_eq!(
                    doc.text_input_state(input).expect("text input state").value,
                    "Nana"
                );
                assert_eq!(doc.get_attribute(input, "value").as_deref(), Some("Nana"));
            }
            assert!(
                fired_events(&engine)
                    .iter()
                    .all(|(_, name, _)| name != "beforeinput" && name != "input"),
                "disabled/read-only commit must not fire input events"
            );

            host.document().lock().expect("document").set_focus(next);
            assert!(
                !host.commit_text(&mut engine, "x", "insertText").unwrap(),
                "non-editable focus must not invent text input state"
            );
            assert!(
                host.document()
                    .lock()
                    .expect("document")
                    .text_input_state(next)
                    .is_none()
            );
        }

        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, _) = install_input_nodes(&mut host);
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_attribute(input, "value", "Nana");
            doc.set_attribute(input, "readonly", "");
            doc.set_focus(input);
            assert!(doc.set_text_input_state(input, TextInputState::new("Nana")));
        }
        let mut engine = RecordingEngine::default();
        assert!(!host.commit_text(&mut engine, "界", "insertText").unwrap());
        assert_eq!(
            host.document()
                .lock()
                .expect("document")
                .get_attribute(input, "value")
                .as_deref(),
            Some("Nana")
        );
        assert!(
            fired_events(&engine)
                .iter()
                .all(|(_, name, _)| name != "beforeinput" && name != "input")
        );
    }

    #[test]
    fn native_ime_disabled_after_blur_commits_original_field() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, next) = install_input_nodes(&mut host);
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
                selection: Some((0, "世".len())),
            },
        )
        .expect("preedit");
        host.document().lock().expect("document").set_focus(next);
        host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
            .expect("disabled leftover after blur");

        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        let state = document
            .text_input_state(input)
            .expect("original IME field");
        assert_eq!(state.value, "Nana世");
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana世")
        );
        assert!(document.text_input_state(next).is_none());
        assert!(document.get_attribute(next, "value").is_none());
        drop(document);

        let events = fired_events(&engine);
        assert!(
            events
                .iter()
                .any(|(target, name, _)| *target == input.0 && name == "compositionend")
        );
        assert!(
            events
                .iter()
                .any(|(target, name, _)| *target == input.0 && name == "beforeinput")
        );
        assert!(
            events
                .iter()
                .any(|(target, name, _)| *target == input.0 && name == "input")
        );
        assert!(
            !events.iter().any(|(target, name, _)| {
                *target == next.0
                    && matches!(name.as_str(), "compositionend" | "beforeinput" | "input")
            }),
            "leftover preedit must not insert into the new focus"
        );
        assert_eq!(
            events
                .iter()
                .rev()
                .find(|(_, name, _)| name == "input")
                .and_then(|(_, _, detail)| detail.get("inputType"))
                .and_then(HostValue::as_str),
            Some("insertCompositionText")
        );
    }

    #[test]
    fn native_ime_disabled_clears_blocked_original_without_commit() {
        let mut host = VueHost::new();
        host.fire_event = Some(JsFunctionId(1));
        let (input, next) = install_input_nodes(&mut host);
        host.bridge().lock().expect("bridge").register(
            input.0,
            WidgetKind::Input,
            WidgetProps {
                value: "Nana".into(),
                ..WidgetProps::default()
            },
        );
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
                selection: Some((0, "世".len())),
            },
        )
        .expect("preedit");
        host.bridge()
            .lock()
            .expect("bridge")
            .get_mut(input.0)
            .expect("registered input")
            .props
            .disabled = true;
        host.document().lock().expect("document").set_focus(next);
        host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
            .expect("disabled leftover on blocked field");

        let document = host.document();
        let document = document.lock().expect("document");
        assert!(document.ime_composition(input).is_none());
        assert_eq!(
            document
                .text_input_state(input)
                .expect("original field")
                .value,
            "Nana"
        );
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana")
        );
        drop(document);
        assert!(
            !fired_events(&engine)
                .iter()
                .any(|(_, name, _)| name == "beforeinput" || name == "input")
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

    #[test]
    fn next_wakeup_is_none_when_idle() {
        let host = VueHost::new();
        assert!(host.next_wakeup().is_none());
    }

    #[test]
    fn pump_frame_fires_pending_raf_on_host_frame() {
        let mut host = VueHost::new();
        let mut engine = RecordingEngine::default();
        host.bind_event_bridge(&mut engine).unwrap();
        host.web_api().lock().expect("web-api").schedule_raf(1);
        assert!(
            host.next_wakeup().is_some(),
            "pending rAF must request a host wake"
        );

        let fired = host.pump_frame(&mut engine).unwrap();
        assert!(
            fired >= 1,
            "host frame must drain rAF without waiting a fake 16ms"
        );
        assert!(
            host.next_wakeup().is_none(),
            "idle after drain must return None"
        );
    }

    #[test]
    fn pump_frame_nested_raf_follows_next_wakeup_not_busy_loop() {
        struct RescheduleEngine {
            web_api: SharedWebApiState,
            drain_count: usize,
        }
        impl JsEngine for RescheduleEngine {
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
                _target: JsFunctionId,
                args: &[HostValue],
            ) -> Result<HostValue, JsEngineError> {
                if let Some(HostValue::Object(payload)) = args.first()
                    && let Some(HostValue::Array(raf)) = payload.get("raf")
                    && !raf.is_empty()
                {
                    self.drain_count += 1;
                    if self.drain_count == 1
                        && let Ok(mut web) = self.web_api.lock()
                    {
                        web.schedule_raf(2);
                    }
                }
                Ok(HostValue::Null)
            }
            fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
                Ok(())
            }
            fn interrupt(&mut self) {}
            fn request_gc(&mut self) {}
            fn shutdown(&mut self) {}
        }

        let mut host = VueHost::new();
        let mut engine = RescheduleEngine {
            web_api: host.web_api(),
            drain_count: 0,
        };
        host.bind_event_bridge(&mut engine).unwrap();
        host.web_api().lock().expect("web-api").schedule_raf(1);

        let before = Instant::now();
        let fired = host.pump_frame(&mut engine).unwrap();
        assert_eq!(
            engine.drain_count, 1,
            "nested rAF must not drain in the same host frame"
        );
        assert!(fired >= 1);
        let wakeup = host
            .next_wakeup()
            .expect("nested rAF must schedule the next host frame");
        assert!(
            wakeup >= before + std::time::Duration::from_millis(8),
            "nested rAF must wait for next_wakeup (~16ms), not spin"
        );
        assert!(wakeup <= before + std::time::Duration::from_millis(50));
    }
}
