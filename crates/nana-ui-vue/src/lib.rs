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
mod css_font_face;
mod css_interactive;
mod css_interactive_apply;
mod css_map;
mod css_paint;
mod css_paint_transform;
mod host;
#[cfg(feature = "hosted")]
mod hosted_adapter;
mod input;
mod layout_map;
mod measure;
#[cfg(feature = "hosted")]
mod media_gpu;
mod multi_window;
#[cfg(feature = "scene-view")]
mod native_component;
mod renderer;
mod scroll;
mod shared_document;
mod shell_contract;
mod style;
#[cfg(feature = "hosted")]
mod svg_gpu;
mod svg_inline;
mod svg_raster;
mod tree;
pub use shared_document::SharedRuntimeDocument;
mod video;
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
    SharedCanvasRuntime, SharedMediaRuntime, SharedWebApiState, compose_runtime_artifact,
    default_shared_clipboard, register_media_host_ops, register_web_api_host_ops_with_resources,
    shared_canvas_runtime, shared_media_runtime, shared_web_api_state,
};

pub use app::{
    MountOptions, NanaVueApp, mount_vue_as_nana, mount_vue_as_nana_with_engine,
    semantic_snapshot_of,
};
pub use video::{SharedVideoRuntime, VideoId, VideoRect, VideoRuntime, shared_video_runtime};

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
    MatchContext, MatchNode, MatchSubtree, MediaEnv, OwnedMatchTree, Selector, SimpleCompound,
    Specificity, StyleRule, StylesheetParseReport, apply_stylesheet_to_layout,
    collect_document_custom_properties_from_rules, matched_declaration_entries,
    matched_declarations, parse_stylesheet, parse_stylesheet_full,
    parse_stylesheet_full_with_layers, parse_stylesheet_full_with_options,
    parse_stylesheet_with_report, rebuild_layout_style, selector_matches,
    stylesheet_needs_relative,
};
pub use css_font_face::{
    FontFaceSrcKind, FontFaceStyle, parse_font_face_at_rule, parse_font_face_rules,
};

pub use css_interactive::{
    GeneratedPseudo, GeneratedPseudoMatch, GeneratedPseudoRule, InteractiveMatchState,
    InteractivePseudo, InteractivePseudoFlags, InteractiveSelector, InteractiveStyleRule,
    KeyframeBlock, KeyframeSelector, KeyframesRule, MediaRule, MotionDeclarations, MotionStyleRule,
    ParsedStylesheet, ScrollbarPseudo, ScrollbarPseudoRule, keyframes_by_name,
    matched_generated_pseudo, matched_interactive_rules, matched_motion_rules,
    matched_scrollbar_pseudo, merge_parsed_stylesheet, partition_motion_entries,
};
/// Adapter internals: CSS subset → LayoutStyle. Prefer [`prelude`] for hosts.
pub use css_map::{
    AlignSpec, BoxSizing, CssLayoutParse, DirSpec, DisplaySpec, FlexDirection, FlexWrap,
    FontSizeContext, GridAutoFlow, GridTrack, GridTrackListParse, GridTrackListUnsupported,
    JustifySpec, LayoutStyle, LayoutStyleCss, LengthSpec, LineHeightSpec, OverflowSpec,
    PaddingSpec, ParentBox, PositionSpec, collect_document_css_custom_properties,
    parse_box_edge_length, parse_css_font_family, parse_css_font_feature_settings,
    parse_css_font_kerning, parse_css_font_size, parse_css_font_variation_settings,
    parse_css_font_weight, parse_css_length_px, parse_css_letter_spacing, parse_css_line_break,
    parse_css_line_height, parse_css_word_break, parse_grid_template_columns,
    parse_grid_track_list_result, parse_inset_length, resolve_grid_column_widths,
    resolve_grid_track_sizes, resolve_paint_color,
};
#[cfg(feature = "hosted")]
pub use hosted_adapter::{VueHostedProgram, VueHostedRuntime, VueRuntimeProgram};
pub use input::{
    CompositionEventKind, CompositionInput, HostedInputResult, InputModifiers, KeyboardEventKind,
    KeyboardInput, PointerEventKind, PointerInput, PointerType, WheelInput,
};
pub use layout_map::{default_layout_for_kind, layout_kind_from_tag};
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
    NODE_HANDLE_DOCUMENT_STRIDE, NanaTreeDocument, NodeHandle, get_layout_box, get_layout_box_from,
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

fn hosted_material_support_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "none"
    } else if cfg!(target_os = "windows") {
        "mica-acrylic"
    } else {
        "none"
    }
}

/// Build L3 [`nana_ui::ThemeTokens`] from Appearance + transparent-surface flag.
#[cfg(feature = "scene-view")]
pub fn theme_tokens_from_appearance(
    theme: nana_ui::ThemeMode,
    appearance: &nana_ui::AppearanceSettings,
    transparent_surface: bool,
) -> nana_ui::ThemeTokens {
    use nana_ui::ThemeModeExt;
    nana_ui::ThemeTokens::new(theme.colors(), appearance.metrics())
        .with_workspace_corners(appearance.workspace_corners_enabled())
        .with_backdrop(
            transparent_surface,
            appearance.backdrop_target(),
            appearance.backdrop_opacity(),
            appearance.titlebar_follows_sidebar(),
        )
}

/// Build L3 [`nana_ui::ThemeTokens`] from a semantic snapshot + transparent-surface flag.
///
/// Applies Appearance `backdrop_*` / `titlebar_follows_sidebar` into region alphas.
#[cfg(feature = "scene-view")]
pub fn theme_tokens_from_snapshot(
    snap: &SemanticSnapshot,
    transparent_surface: bool,
) -> nana_ui::ThemeTokens {
    theme_tokens_from_appearance(snap.theme, &snap.appearance, transparent_surface)
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
    input_projection: host::input_projection::State,
    callbacks: host::callbacks::State,
    pub theme: ThemeMode,
    document: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    layout_boxes: Arc<LayoutBoxStore>,
    web_api: SharedWebApiState,
    canvas: SharedCanvasRuntime,
    video: video::SharedVideoRuntime,
    media: SharedMediaRuntime,
    diagnostics: DiagnosticBindings,
    input: Arc<Mutex<input::InputState>>,
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
    #[cfg(feature = "hosted")]
    video_gpu: Option<video::VideoGpuBridge>,
    #[cfg(feature = "hosted")]
    svg_gpu: Option<svg_gpu::SvgGpuBridge>,
    #[cfg(feature = "hosted")]
    media_gpu: Option<media_gpu::MediaGpuBridge>,
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
            shared_media_runtime(),
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
            shared_media_runtime(),
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
            shared_media_runtime(),
        )
    }

    pub(crate) fn with_document_id_and_shared_resources(
        document_id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        canvas: SharedCanvasRuntime,
        media: SharedMediaRuntime,
        local_storage: nana_ui_web_api::SharedStorage,
    ) -> Self {
        Self::with_document_id_and_web_api_state(
            document_id,
            physical_width,
            physical_height,
            scale_factor,
            nana_ui_web_api::shared_web_api_state_with_local_storage(local_storage),
            canvas,
            media,
        )
    }

    fn with_document_id_and_web_api_state(
        document_id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        web_api: SharedWebApiState,
        canvas: SharedCanvasRuntime,
        media: SharedMediaRuntime,
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
            video: video::shared_video_runtime(),
            media,
            diagnostics: DiagnosticBindings::default(),
            input_projection: host::input_projection::State::default(),
            callbacks: host::callbacks::State::default(),
            input: Arc::new(Mutex::new(input::InputState::default())),
            #[cfg(feature = "scene-view")]
            components: NativeComponentRegistry::new(),
            #[cfg(feature = "scene-view")]
            host_textures,
            #[cfg(feature = "hosted")]
            webgpu: None,
            #[cfg(feature = "hosted")]
            canvas_gpu: None,
            #[cfg(feature = "hosted")]
            video_gpu: None,
            #[cfg(feature = "hosted")]
            svg_gpu: None,
            #[cfg(feature = "hosted")]
            media_gpu: None,
        }
    }

    pub fn document_id(&self) -> DocumentId {
        self.document.lock().expect("vue doc").id()
    }

    pub fn document(&self) -> Arc<Mutex<NanaTreeDocument>> {
        Arc::clone(&self.document)
    }

    pub fn shared_runtime_document(&self) -> Arc<SharedRuntimeDocument> {
        Arc::new(SharedRuntimeDocument::new(Arc::clone(&self.document)))
    }

    pub fn bridge(&self) -> Arc<Mutex<MessageBridge>> {
        Arc::clone(&self.bridge)
    }

    pub fn web_api(&self) -> SharedWebApiState {
        Arc::clone(&self.web_api)
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
        register_media_host_ops(&mut api, Arc::clone(&self.media));
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

fn is_submit_control(widget: &SemanticWidget) -> bool {
    let ty = crate::widget_map::attr_value(&widget.props, &["type"]).unwrap_or("");
    match widget.props.element_tag.as_str() {
        tag if tag.eq_ignore_ascii_case("button") => {
            ty.is_empty() || ty.eq_ignore_ascii_case("submit")
        }
        tag if tag.eq_ignore_ascii_case("input") => ty.eq_ignore_ascii_case("submit"),
        _ => false,
    }
}

fn ancestor_form(bridge: &Mutex<MessageBridge>, from: WidgetId) -> Option<WidgetId> {
    let bridge = bridge.lock().expect("vue bridge");
    let mut cursor = bridge.get(from).and_then(|widget| widget.parent);
    while let Some(id) = cursor {
        let widget = bridge.get(id)?;
        if widget.props.element_tag.eq_ignore_ascii_case("form") {
            return Some(id);
        }
        cursor = widget.parent;
    }
    None
}

fn exclusive_check_radios(bridge: &Mutex<MessageBridge>, selected: WidgetId) {
    let mut bridge = bridge.lock().expect("vue bridge");
    let Some(name) = bridge
        .get(selected)
        .and_then(|widget| crate::widget_map::attr_value(&widget.props, &["name"]))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    let peers: Vec<WidgetId> = bridge
        .widgets()
        .filter(|widget| {
            widget.id != selected
                && widget.kind == WidgetKind::Radio
                && crate::widget_map::attr_value(&widget.props, &["name"]) == Some(name.as_str())
        })
        .map(|widget| widget.id)
        .collect();
    for id in peers {
        bridge.patch_prop(id, "checked", &HostValue::Bool(false));
        bridge.patch_prop(id, "toggled", &HostValue::Bool(false));
    }
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
            | "checkbox"
            | "dialog"
            | "progress"
            | "nana-switch"
            | "nana-sidebar-row"
            | "nana-number-input"
            | "nana-icon-button"
            | "range-field"
            | "nana-list-item"
            | "nana-scroll-view"
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
mod tests;
