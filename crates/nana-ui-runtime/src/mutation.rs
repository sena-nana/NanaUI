use crate::{
    AccessibilityLive, AccessibilityState, AnimationId, AnimationPlayback, AnimationSpec,
    ComponentTypeId, CustomRenderNode, DocumentId, HighlightRequest, ImeComposition,
    InteractionState, LayoutBox, NodeKind, NodeStyle, OverlayHostState, ScrollMetrics,
    ScrollOffset, StableNodeId, TextCodeFold, TextCompletion, TextContent, TextHover, TextInlay,
    TextInputState, TextSelection, TextSignatureHelp, components::TextSnippetSession,
};
use nana_ui_core::ThemeAppearance;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq)]
pub struct SharedElementClaim {
    pub key: nana_ui_core::SharedElementKey,
    pub transition: nana_ui_core::SharedElementTransition,
    pub now: Duration,
}

/// One retained-tree mutation. Mutations are validated as a batch before the
/// authoritative world changes.
#[derive(Debug, Clone, PartialEq)]
pub enum UiMutation {
    Create {
        id: StableNodeId,
        document: DocumentId,
        kind: Arc<NodeKind>,
    },
    /// Insert or reparent `child`. `before = None` appends it.
    Insert {
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
    },
    /// Put `parent`'s children in `order`, which holds each of them once;
    /// see [`MutationQueue::reorder_children`].
    ReorderChildren {
        parent: StableNodeId,
        order: Arc<[StableNodeId]>,
    },
    /// Unparent a node. It stays retained and `Mounted`, but leaves the live
    /// document until [`Self::Insert`].
    Detach {
        id: StableNodeId,
    },
    /// Unparent a subtree and keep its identities. Parked nodes stay out of the
    /// live document until inserted.
    ParkSubtree {
        root: StableNodeId,
    },
    /// Destroy a node and all descendants. Their stable IDs cannot be reused.
    DespawnSubtree {
        root: StableNodeId,
    },
    SetStyle {
        id: StableNodeId,
        style: Arc<NodeStyle>,
    },
    SetPresetTheme {
        mode: ThemeAppearance,
    },
    /// Install a validated design system.
    ///
    /// Carries the whole [`CompiledTheme`](nana_ui_core::CompiledTheme) rather
    /// than loose palette / metrics fields: those two were never the whole
    /// theme, and a mutation that can only express two categories is a
    /// mutation every later category has to route around.
    SetThemeTokens {
        theme: std::sync::Arc<nana_ui_core::CompiledTheme>,
    },
    SetText {
        id: StableNodeId,
        text: TextContent,
    },
    /// Rich text for a node: its text plus styled spans. `None` drops the
    /// spans and keeps the text. See [`MutationQueue::set_rich_text`].
    SetRichText {
        id: StableNodeId,
        rich: Option<Box<nana_ui_core::RichText>>,
    },
    SetInlineContent {
        id: StableNodeId,
        content: Box<crate::InlineContent>,
    },
    /// The language a node names for itself (`lang`). Its subtree inherits
    /// it; see [`MutationQueue::set_language`].
    SetLanguage {
        id: StableNodeId,
        language: Option<Box<nana_text::font::LanguageTag>>,
    },
    /// What a replaced resource reports of its size; see
    /// [`MutationQueue::set_replaced_metadata`].
    SetReplacedMetadata {
        resource: Box<crate::ReplacedResource>,
        metadata: Option<Box<crate::ReplacedMetadata>>,
    },
    /// The typography scale a node sets for its subtree; see
    /// [`MutationQueue::set_text_scale`].
    SetTextScale {
        id: StableNodeId,
        scale: Option<f32>,
    },
    /// Where a `display: free` parent places a node; see
    /// [`MutationQueue::set_explicit_rect`].
    SetExplicitRect {
        id: StableNodeId,
        rect: Option<Box<crate::ExplicitRect>>,
    },
    /// The locale a node's subtree reads; see [`MutationQueue::set_locale`].
    SetLocale {
        id: StableNodeId,
        locale: Option<Box<crate::Locale>>,
    },
    /// Localized text a node shows; see
    /// [`MutationQueue::set_localized_text`].
    SetLocalizedText {
        id: StableNodeId,
        text: Option<Box<crate::LocalizedText>>,
    },
    /// The responsive rule a node follows; see
    /// [`MutationQueue::set_responsive`].
    SetResponsive {
        id: StableNodeId,
        rule: Option<std::sync::Arc<crate::ResponsiveRule>>,
    },
    /// A node's per-glyph presentation. Presentation only.
    SetGlyphPresentation {
        id: StableNodeId,
        presentation: Option<Box<nana_ui_core::GlyphPresentation>>,
    },
    /// A rich text editor's caret and selection. Paint only.
    SetRichEditorMarks {
        id: StableNodeId,
        marks: Option<Box<crate::RichEditorMarks>>,
    },
    /// A document box, written by a test or a tool. Product frames must not
    /// use this to fight [`crate::RuntimeLayoutEngine`]; mixed trees flush
    /// that engine. The box is kept measured from the node's
    /// [`crate::LayoutFrame`] once the commit's tree is in place: a child
    /// not written with it moves with it.
    WriteLayout {
        id: StableNodeId,
        layout: LayoutBox,
    },
    /// Engine writeback: `layout` measured from `frame`'s origin.
    #[doc(hidden)]
    PlaceLayout {
        id: StableNodeId,
        layout: LayoutBox,
        frame: Box<crate::LayoutFrame>,
    },
    /// Move an absolute placement container. Only `offset_top` and `height`
    /// change; the rest of the authored style stays. `height: None` clears
    /// an imposed row height so a measured row can size to its content.
    PatchPlacement {
        id: StableNodeId,
        top: f32,
        height: Option<f32>,
    },
    /// Patch a node's 2D paint transform without rewriting the rest of its
    /// authored style.
    PatchTransform {
        id: StableNodeId,
        transform: Option<nana_ui_core::PaintTransform>,
    },
    SetScrollOffset {
        id: StableNodeId,
        offset: ScrollOffset,
    },
    /// A touch drag or bounce past the scrolling area: the offset is kept
    /// as given, not clamped, until the next change of the container's
    /// scrolling area. Paint and hit testing translate by it like any
    /// offset; layout boxes stay put.
    SetScrollOverscroll {
        id: StableNodeId,
        offset: ScrollOffset,
    },
    SetScrollMetrics {
        id: StableNodeId,
        metrics: Option<ScrollMetrics>,
    },
    SetInteraction {
        id: StableNodeId,
        interaction: InteractionState,
    },
    SetHitTestBehavior {
        id: StableNodeId,
        behavior: Option<nana_ui_core::HitTestBehavior>,
    },
    SetCustomRender {
        id: StableNodeId,
        content: Option<Box<CustomRenderNode>>,
    },
    /// Map the document's presentation time onto the time a host-texture
    /// node samples its resource at (Issue #303); `None` removes it. Changing
    /// it rebuilds nothing: only the time the next frame requests moves.
    SetMediaTime {
        id: StableNodeId,
        mapping: Option<Box<crate::MediaTimeMapping>>,
    },
    /// A painter for the node whatever its style says (Issue #217). It wins
    /// over [`NodeStyle::painter`] and survives a component rewriting the
    /// node's style. `None` removes it.
    SetPainter {
        id: StableNodeId,
        painter: Option<Box<crate::NodePainter>>,
    },
    SetEventListener {
        id: StableNodeId,
        event: Box<str>,
        enabled: bool,
    },
    SetComponentType {
        id: StableNodeId,
        type_id: Option<ComponentTypeId>,
    },
    SetWidgetVisual {
        id: StableNodeId,
        visual: Option<Box<crate::WidgetVisualHandle>>,
    },
    SetAccessibility {
        id: StableNodeId,
        accessibility: Box<AccessibilityState>,
    },
    /// Name `id` for assistive technology by `label`'s text when it has no
    /// label of its own (`aria-labelledby`); `None` removes the relation.
    SetLabelledBy {
        id: StableNodeId,
        label: Option<StableNodeId>,
    },
    /// Announce changes inside `id` as `live` says (`aria-live`); `None`
    /// removes the setting.
    SetLive {
        id: StableNodeId,
        live: Option<AccessibilityLive>,
    },
    /// Override `id`'s `aria-selected` state independently of its component's
    /// own [`AccessibilityState`] (for custom selectable views like grid cards).
    SetSelected {
        id: StableNodeId,
        selected: Option<bool>,
    },
    SetSurfaceOpen {
        id: StableNodeId,
        open: bool,
        menu: bool,
    },
    SetOverlayHost {
        host: StableNodeId,
        state: Box<OverlayHostState>,
    },
    /// Visual mounting scope for standalone floating overlay surfaces (`Tooltip`,
    /// `ChartTooltip`, `ContextMenu`, `AnchoredActionMenu`, etc.).
    SetOverlayMount {
        id: StableNodeId,
        mount: Option<Box<nana_ui_core::OverlayMount>>,
    },
    CapturePointer {
        pointer_id: u64,
        target: StableNodeId,
    },
    ReleasePointer {
        pointer_id: u64,
        target: StableNodeId,
    },
    StartAnimation {
        animation: Box<AnimationSpec>,
    },
    /// Replace a running track whole under its id (Issue #325).
    ReplaceAnimation {
        animation: Box<AnimationSpec>,
    },
    StopAnimation {
        id: AnimationId,
    },
    FinishAnimation {
        id: AnimationId,
    },
    ReverseAnimation {
        id: AnimationId,
    },
    PauseAnimation {
        id: AnimationId,
    },
    ResumeAnimation {
        id: AnimationId,
    },
    SetSharedElement {
        target: StableNodeId,
        key: Option<
            Box<(
                nana_ui_core::SharedElementKey,
                nana_ui_core::SharedElementTransition,
            )>,
        >,
    },
    CaptureSharedElement {
        target: StableNodeId,
        key: Option<nana_ui_core::SharedElementKey>,
    },
    ClaimSharedElement {
        target: StableNodeId,
        claim: Box<SharedElementClaim>,
    },
    RequestFocus {
        document: DocumentId,
        target: Option<StableNodeId>,
    },
    RestoreFocusWithin {
        root: StableNodeId,
    },
    SetFocusPolicy {
        id: StableNodeId,
        policy: Option<crate::FocusPolicy>,
    },
    SetFocusNeighbors {
        id: StableNodeId,
        neighbors: Box<crate::FocusNeighbors>,
    },
    SetDirectionalFocus {
        id: StableNodeId,
        scope: Option<crate::DirectionalFocusScope>,
    },
    SetIme {
        id: StableNodeId,
        composition: Option<Box<ImeComposition>>,
    },
    SetTextInput {
        id: StableNodeId,
        state: Option<Box<TextInputState>>,
    },
    SetTextSelection {
        id: StableNodeId,
        selection: TextSelection,
    },
    ReplaceTextSelection {
        id: StableNodeId,
        text: Box<str>,
    },
    SetHighlightRequest {
        id: StableNodeId,
        request: Option<Box<HighlightRequest>>,
    },
    /// Replace the collapsed code-fold set of one editor (view state; the
    /// committed value is untouched).
    SetTextInputFoldCollapsed {
        id: StableNodeId,
        folds: Arc<[TextCodeFold]>,
    },
    /// 宿主喂入或撤掉（空表）行内提示（inlay）。世界校验后按
    /// `(offset, label)` 排序去重存储；纯视图装饰，committed value 不受
    /// 影响。
    SetTextInputInlays {
        id: StableNodeId,
        inlays: Arc<[TextInlay]>,
    },
    /// Start, advance-side-effect-free replace, or end (`None`) the snippet
    /// session of one editor.
    SetTextInputSnippet {
        id: StableNodeId,
        session: Option<Box<TextSnippetSession>>,
    },
    /// 宿主喂入补全候选（过滤是宿主的职责）。空列表关闭弹层；相同列表
    /// 不重置键盘选中态。
    SetTextInputCompletions {
        id: StableNodeId,
        items: Arc<[TextCompletion]>,
    },
    /// 写回补全弹层的键盘选中与滚动位置（仅会话活跃时有意义）。
    SetTextInputCompletionView {
        id: StableNodeId,
        selected: usize,
        scroll: usize,
    },
    /// Esc 关闭补全弹层：会话数据保留，宿主重喂相同列表不复活弹层。
    SetTextInputCompletionDismissed {
        id: StableNodeId,
    },
    /// 重开补全弹层：清除关闭标记并归零选中（宿主显式重触发，如
    /// Esc 后再次 Ctrl+Space）。
    SetTextInputCompletionReopened {
        id: StableNodeId,
    },
    /// 宿主喂入或撤掉（`None`）hover 文档浮窗。
    SetTextInputHover {
        id: StableNodeId,
        hover: Option<Box<TextHover>>,
    },
    /// 诊断 span 指针命中写入的 hover。`None` 只撤诊断占用的浮窗，不碰
    /// 宿主符号 hover。
    SetTextInputDiagnosticHover {
        id: StableNodeId,
        hover: Option<Box<TextHover>>,
    },
    /// 滚动 hover 浮窗正文（逻辑行数）。
    SetTextInputHoverScroll {
        id: StableNodeId,
        scroll: usize,
    },
    /// 宿主喂入或撤掉（`None`）签名帮助浮窗。纯展示，活动参数索引由
    /// 宿主按内核 `signature_at` 写入。
    SetTextInputSignature {
        id: StableNodeId,
        help: Option<Box<TextSignatureHelp>>,
    },
}

const _: () = assert!(std::mem::size_of::<UiMutation>() <= 40);

/// One box [`crate::RuntimeLayoutEngine`] computed, measured from `frame`'s
/// origin: what [`UiMutation::PlaceLayout`] carries, 40 bytes of it.
///
/// A layout pass writes back one of these per box it moved, a whole page of
/// them on a first layout. They are committed as a batch of their own
/// ([`crate::AppContext::commit_layout_placements`]) rather than queued as
/// mutations, each of which takes the room the largest mutation needs (240
/// bytes): the queue of a 21 001-box page was 7.9 MB allocated and freed
/// every cold pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayoutPlacement {
    pub id: StableNodeId,
    pub layout: LayoutBox,
    pub frame: crate::LayoutFrame,
}

/// Bitmask summarizing which mutation categories a [`MutationQueue`] batch
/// contains, allowing validation and commit passes to skip unrelated phases in O(1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BatchCapabilities(u16);

impl BatchCapabilities {
    pub const EMPTY: Self = Self(0);
    pub const WRITES_TEXT: Self = Self(1 << 0);
    pub const TOPOLOGY: Self = Self(1 << 1);
    pub const STYLE: Self = Self(1 << 2);
    pub const TEXT: Self = Self(1 << 3);
    pub const LAYOUT_OR_SCROLL: Self = Self(1 << 4);
    pub const VISUAL_OR_INTERACTION: Self = Self(1 << 5);
    pub const ANIMATION: Self = Self(1 << 6);
    pub const FOCUS: Self = Self(1 << 7);
    pub const EDITOR: Self = Self(1 << 8);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn for_mutation(mutation: &UiMutation) -> Self {
        match mutation {
            UiMutation::Create { .. }
            | UiMutation::Insert { .. }
            | UiMutation::ReorderChildren { .. }
            | UiMutation::Detach { .. }
            | UiMutation::ParkSubtree { .. }
            | UiMutation::DespawnSubtree { .. } => Self::TOPOLOGY,
            UiMutation::SetStyle { .. }
            | UiMutation::SetPresetTheme { .. }
            | UiMutation::SetThemeTokens { .. }
            | UiMutation::SetResponsive { .. }
            | UiMutation::SetLanguage { .. }
            | UiMutation::SetTextScale { .. }
            | UiMutation::SetLocale { .. }
            | UiMutation::PatchTransform { .. } => Self::STYLE,
            UiMutation::SetText { .. }
            | UiMutation::SetInlineContent { .. }
            | UiMutation::SetRichText { .. }
            | UiMutation::SetLocalizedText { .. }
            | UiMutation::SetGlyphPresentation { .. }
            | UiMutation::SetRichEditorMarks { .. }
            | UiMutation::SetReplacedMetadata { .. } => Self::TEXT,
            UiMutation::SetExplicitRect { .. }
            | UiMutation::WriteLayout { .. }
            | UiMutation::PlaceLayout { .. }
            | UiMutation::PatchPlacement { .. }
            | UiMutation::SetScrollOffset { .. }
            | UiMutation::SetScrollOverscroll { .. }
            | UiMutation::SetScrollMetrics { .. } => Self::LAYOUT_OR_SCROLL,
            UiMutation::SetInteraction { .. }
            | UiMutation::SetHitTestBehavior { .. }
            | UiMutation::SetCustomRender { .. }
            | UiMutation::SetMediaTime { .. }
            | UiMutation::SetPainter { .. }
            | UiMutation::SetEventListener { .. }
            | UiMutation::SetComponentType { .. }
            | UiMutation::SetWidgetVisual { .. }
            | UiMutation::SetAccessibility { .. }
            | UiMutation::SetLabelledBy { .. }
            | UiMutation::SetLive { .. }
            | UiMutation::SetSelected { .. }
            | UiMutation::SetSurfaceOpen { .. }
            | UiMutation::SetOverlayHost { .. }
            | UiMutation::SetOverlayMount { .. }
            | UiMutation::CapturePointer { .. }
            | UiMutation::ReleasePointer { .. } => Self::VISUAL_OR_INTERACTION,
            UiMutation::StartAnimation { .. }
            | UiMutation::ReplaceAnimation { .. }
            | UiMutation::StopAnimation { .. }
            | UiMutation::FinishAnimation { .. }
            | UiMutation::ReverseAnimation { .. }
            | UiMutation::PauseAnimation { .. }
            | UiMutation::ResumeAnimation { .. }
            | UiMutation::SetSharedElement { .. }
            | UiMutation::CaptureSharedElement { .. }
            | UiMutation::ClaimSharedElement { .. } => Self::ANIMATION,
            UiMutation::RequestFocus { .. }
            | UiMutation::RestoreFocusWithin { .. }
            | UiMutation::SetFocusPolicy { .. }
            | UiMutation::SetFocusNeighbors { .. }
            | UiMutation::SetDirectionalFocus { .. } => Self::FOCUS,
            UiMutation::SetTextInput { .. } | UiMutation::ReplaceTextSelection { .. } => {
                Self::EDITOR.union(Self::WRITES_TEXT)
            }
            UiMutation::SetIme { .. }
            | UiMutation::SetTextSelection { .. }
            | UiMutation::SetHighlightRequest { .. }
            | UiMutation::SetTextInputFoldCollapsed { .. }
            | UiMutation::SetTextInputInlays { .. }
            | UiMutation::SetTextInputSnippet { .. }
            | UiMutation::SetTextInputCompletions { .. }
            | UiMutation::SetTextInputCompletionView { .. }
            | UiMutation::SetTextInputCompletionDismissed { .. }
            | UiMutation::SetTextInputCompletionReopened { .. }
            | UiMutation::SetTextInputHover { .. }
            | UiMutation::SetTextInputDiagnosticHover { .. }
            | UiMutation::SetTextInputHoverScroll { .. }
            | UiMutation::SetTextInputSignature { .. } => Self::EDITOR,
        }
    }
}

/// Frame-local command boundary for reconcilers and adapters.
#[derive(Debug, Default)]
pub struct MutationQueue {
    mutations: Vec<UiMutation>,
    capabilities: BatchCapabilities,
}

impl MutationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty queue with room for `capacity` mutations, for a batch whose
    /// size is known up front (a component's creation): it does not regrow.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            mutations: Vec::with_capacity(capacity),
            capabilities: BatchCapabilities::EMPTY,
        }
    }

    #[inline]
    fn record(&mut self, caps: BatchCapabilities, mutation: UiMutation) {
        self.capabilities = self.capabilities.union(caps);
        self.mutations.push(mutation);
    }

    pub(crate) fn capabilities(&self) -> BatchCapabilities {
        self.capabilities
    }

    pub fn create(&mut self, id: StableNodeId, document: DocumentId, kind: NodeKind) {
        self.record(
            BatchCapabilities::TOPOLOGY,
            UiMutation::Create {
                id,
                document,
                kind: Arc::new(kind),
            },
        );
    }

    pub fn insert(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
    ) {
        self.record(
            BatchCapabilities::TOPOLOGY,
            UiMutation::Insert {
                parent,
                child,
                before,
            },
        );
    }

    /// Put the children `parent` has into `order` in one edit, whatever the
    /// length of the list: reversing or sorting a long list this way costs
    /// its length, where an [`Self::insert`] per moved child costs the
    /// length each. `order` must hold exactly the children `parent` has when
    /// the batch reaches this mutation, each once; it brings in no child and
    /// leaves none out, so insert the new ones and remove the old ones
    /// before it. Children that keep their order among themselves are not
    /// touched.
    pub fn reorder_children(
        &mut self,
        parent: StableNodeId,
        order: impl Into<Arc<[StableNodeId]>>,
    ) {
        self.record(
            BatchCapabilities::TOPOLOGY,
            UiMutation::ReorderChildren {
                parent,
                order: order.into(),
            },
        );
    }

    pub fn detach(&mut self, id: StableNodeId) {
        self.record(BatchCapabilities::TOPOLOGY, UiMutation::Detach { id });
    }

    pub fn park_subtree(&mut self, root: StableNodeId) {
        self.record(
            BatchCapabilities::TOPOLOGY,
            UiMutation::ParkSubtree { root },
        );
    }

    pub fn despawn_subtree(&mut self, root: StableNodeId) {
        self.record(
            BatchCapabilities::TOPOLOGY,
            UiMutation::DespawnSubtree { root },
        );
    }

    pub fn set_style(&mut self, id: StableNodeId, style: NodeStyle) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetStyle {
                id,
                style: Arc::new(style),
            },
        );
    }

    pub fn set_preset_theme(&mut self, mode: ThemeAppearance) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetPresetTheme { mode },
        );
    }

    pub fn set_theme_tokens(&mut self, theme: std::sync::Arc<nana_ui_core::CompiledTheme>) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetThemeTokens { theme },
        );
    }

    pub fn set_text(&mut self, id: StableNodeId, text: TextContent) {
        self.record(BatchCapabilities::TEXT, UiMutation::SetText { id, text });
    }

    /// Sets canonical inline content. Atomic ids must be direct retained
    /// children after this batch. This low-level mutation does not create or
    /// destroy components; AppContext's content reconciliation owns lifetime.
    pub fn set_inline_content(
        &mut self,
        id: StableNodeId,
        content: impl Into<crate::InlineContent>,
    ) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetInlineContent {
                id,
                content: Box::new(content.into().normalized()),
            },
        );
    }

    /// Give `id` rich text: the node's text becomes `rich`'s, and its spans
    /// style it over the node's computed style.
    ///
    /// A change is priced by the tier it touches: a shaping field reshapes
    /// and relays the node out; a paint field (colour, decoration, stroke,
    /// shadow) only repaints it; an effect index costs no text work. The
    /// value is the application's — keep it, and hand the next one in when it
    /// changes. A later [`Self::set_text`] with different text drops the
    /// spans.
    pub fn set_rich_text(&mut self, id: StableNodeId, rich: nana_ui_core::RichText) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetRichText {
                id,
                rich: Some(Box::new(rich)),
            },
        );
    }

    /// Name the language `id` and the subtree under it are written in, as
    /// HTML's `lang` does; `None` inherits the parent's again. Text shapes in
    /// its nearest named language, then the application's default, then the
    /// text engine's fallback. A change reaches only the text that inherits
    /// it: the subtree's styles resolve again, and text whose language moved
    /// is shaped again and lays out again only if its metrics moved.
    pub fn set_language(
        &mut self,
        id: StableNodeId,
        language: Option<nana_text::font::LanguageTag>,
    ) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetLanguage {
                id,
                language: language.map(Box::new),
            },
        );
    }

    /// Set the typography scale text under `id` is set at, as an
    /// accessibility or content-size scope does; `None` inherits the
    /// parent's again. A scale multiplies the font size and an absolute line
    /// height; box lengths, `em` ones among them, resolve against the size
    /// before it. Text takes its nearest scope's scale, then its window's
    /// ([`crate::UiWorld::set_document_text_scale`]), then the application's
    /// ([`crate::UiWorld::set_default_text_scale`]). A change reaches only
    /// the scope: nodes under it that set their own scale are not visited,
    /// its text is laid out again at the new size, and layout hears of it
    /// only from metrics that moved. The scale must be a positive finite
    /// number.
    pub fn set_text_scale(&mut self, id: StableNodeId, scale: Option<f32>) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetTextScale { id, scale },
        );
    }

    /// Place `id` by `rect` in its `display: free` parent: logical pixels of
    /// the parent's padding box, origin at the top left, y down. `None`
    /// returns it to the padding box's origin at its shrink-to-fit size.
    ///
    /// The rect is not a layout style: `left`, `top`, `width` and `height`
    /// stay as authored. It depends on the parent alone. Moving it places the
    /// node again and measures nothing; resizing it also lays out what is
    /// inside the node. Neither measures a sibling or anything outside the
    /// free container. A rect on a node whose parent is not a free container
    /// is kept and takes effect once it is. Every value must be finite and
    /// the size not negative.
    pub fn set_explicit_rect(&mut self, id: StableNodeId, rect: Option<crate::ExplicitRect>) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::SetExplicitRect {
                id,
                rect: rect.map(Box::new),
            },
        );
    }

    /// Make `id` a locale scope: the localized text under it, itself included,
    /// resolves in `locale`, shapes in its language, and the subtree lays out
    /// in its direction. `None` returns the subtree to the scope above. Only
    /// the localized text the scope holds is visited; literal text is not.
    pub fn set_locale(&mut self, id: StableNodeId, locale: Option<crate::Locale>) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetLocale {
                id,
                locale: locale.map(Box::new),
            },
        );
    }

    /// Show `text` on `id`, resolved in the locale of the scope it is in and
    /// again whenever that locale changes. `None` keeps what it shows and
    /// makes it literal again.
    pub fn set_localized_text(&mut self, id: StableNodeId, text: Option<crate::LocalizedText>) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetLocalizedText {
                id,
                text: text.map(Box::new),
            },
        );
    }

    /// Make `id` follow `rule`: it takes the layout variant of the bucket its
    /// container's size is in, over its own layout. `None` returns it to its
    /// own layout. A container that already has a box is read now; one that
    /// does not is read after its first layout. A named container must exist
    /// when the rule is set. Breakpoints must be finite and ascend, at most
    /// [`crate::MAX_RESPONSIVE_BREAKPOINTS`] of them.
    pub fn set_responsive(
        &mut self,
        id: StableNodeId,
        rule: Option<std::sync::Arc<crate::ResponsiveRule>>,
    ) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::SetResponsive { id, rule },
        );
    }

    /// Report what `resource` knows of its size: an image's natural size
    /// once it decoded, a texture's or a video's resolution. `None` forgets
    /// it. The nodes that show the resource paint again; of them only the
    /// ones whose box reads a natural size -- an auto width or height --
    /// lay out again, found through an index, not a scan. A new frame or
    /// texture generation of the same resource is not this: it is
    /// [`Self::set_custom_render`], and paint.
    pub fn set_replaced_metadata(
        &mut self,
        resource: crate::ReplacedResource,
        metadata: Option<crate::ReplacedMetadata>,
    ) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetReplacedMetadata {
                resource: Box::new(resource),
                metadata: metadata.map(Box::new),
            },
        );
    }

    /// Present `id`'s glyphs with `presentation`: the effects its rich spans'
    /// `effect` indices name, and a reveal. No shaping, layout, glyph
    /// rasterization or instance rebuild: the text vertex shader samples it on
    /// the motion clock. `None` stops presenting.
    pub fn set_glyph_presentation(
        &mut self,
        id: StableNodeId,
        presentation: Option<nana_ui_core::GlyphPresentation>,
    ) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetGlyphPresentation {
                id,
                presentation: presentation.map(Box::new),
            },
        );
    }

    /// A rich text editor's caret and selection marks. Paint only.
    pub fn set_rich_editor_marks(
        &mut self,
        id: StableNodeId,
        marks: Option<crate::RichEditorMarks>,
    ) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetRichEditorMarks {
                id,
                marks: marks.map(Box::new),
            },
        );
    }

    /// Drop `id`'s spans, keeping its text.
    pub fn clear_rich_text(&mut self, id: StableNodeId) {
        self.record(
            BatchCapabilities::TEXT,
            UiMutation::SetRichText { id, rich: None },
        );
    }

    /// Publish a document box for `id`, as a test or a tool lays it out.
    pub fn write_layout(&mut self, id: StableNodeId, layout: LayoutBox) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::WriteLayout { id, layout },
        );
    }

    /// Queue a box [`crate::RuntimeLayoutEngine`] computed, measured from
    /// `frame`'s origin, among other mutations. A layout pass's writeback
    /// commits [`LayoutPlacement`]s instead; tests queue one to place a box
    /// beside the mutations they are about.
    #[cfg(test)]
    pub(crate) fn place_layout(
        &mut self,
        id: StableNodeId,
        layout: LayoutBox,
        frame: crate::LayoutFrame,
    ) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::PlaceLayout {
                id,
                layout,
                frame: Box::new(frame),
            },
        );
    }

    /// Queue [`UiMutation::PatchPlacement`].
    pub fn patch_placement(&mut self, id: StableNodeId, top: f32, height: Option<f32>) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::PatchPlacement { id, top, height },
        );
    }

    /// Queue [`UiMutation::PatchTransform`].
    pub fn patch_transform(
        &mut self,
        id: StableNodeId,
        transform: Option<nana_ui_core::PaintTransform>,
    ) {
        self.record(
            BatchCapabilities::STYLE,
            UiMutation::PatchTransform { id, transform },
        );
    }

    pub fn set_scroll_offset(&mut self, id: StableNodeId, offset: ScrollOffset) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::SetScrollOffset { id, offset },
        );
    }

    /// Queue [`UiMutation::SetScrollOverscroll`]; the touch scroll path's.
    pub(crate) fn set_scroll_overscroll(&mut self, id: StableNodeId, offset: ScrollOffset) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::SetScrollOverscroll { id, offset },
        );
    }

    pub fn set_scroll_metrics(&mut self, id: StableNodeId, metrics: Option<ScrollMetrics>) {
        self.record(
            BatchCapabilities::LAYOUT_OR_SCROLL,
            UiMutation::SetScrollMetrics { id, metrics },
        );
    }

    pub fn set_interaction(&mut self, id: StableNodeId, interaction: InteractionState) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetInteraction { id, interaction },
        );
    }

    pub fn set_hit_test_behavior(
        &mut self,
        id: StableNodeId,
        behavior: Option<nana_ui_core::HitTestBehavior>,
    ) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetHitTestBehavior { id, behavior },
        );
    }

    pub fn set_custom_render(&mut self, id: StableNodeId, content: Option<CustomRenderNode>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetCustomRender {
                id,
                content: content.map(Box::new),
            },
        );
    }

    /// See [`UiMutation::SetMediaTime`].
    pub fn set_media_time(&mut self, id: StableNodeId, mapping: Option<crate::MediaTimeMapping>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetMediaTime {
                id,
                mapping: mapping.map(Box::new),
            },
        );
    }

    pub fn set_widget_visual<V: crate::WidgetVisual>(
        &mut self,
        id: StableNodeId,
        visual: Option<V>,
    ) {
        self.set_widget_visual_handle(id, visual.map(crate::WidgetVisualHandle::new));
    }

    pub fn set_widget_visual_handle(
        &mut self,
        id: StableNodeId,
        visual: Option<crate::WidgetVisualHandle>,
    ) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetWidgetVisual {
                id,
                visual: visual.map(Box::new),
            },
        );
    }

    /// Paint `id` with `painter`, whatever its style says. See
    /// [`UiMutation::SetPainter`].
    pub fn set_painter(&mut self, id: StableNodeId, painter: Option<crate::NodePainter>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetPainter {
                id,
                painter: painter.map(Box::new),
            },
        );
    }

    pub fn set_event_listener(
        &mut self,
        id: StableNodeId,
        event: impl Into<String>,
        enabled: bool,
    ) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetEventListener {
                id,
                event: event.into().into_boxed_str(),
                enabled,
            },
        );
    }

    pub fn set_component_type(&mut self, id: StableNodeId, type_id: Option<ComponentTypeId>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetComponentType { id, type_id },
        );
    }

    pub fn append(&mut self, mut other: Self) {
        self.capabilities = self.capabilities.union(other.capabilities);
        self.mutations.append(&mut other.mutations);
    }

    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn set_accessibility(&mut self, id: StableNodeId, accessibility: AccessibilityState) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetAccessibility {
                id,
                accessibility: Box::new(accessibility),
            },
        );
    }

    /// Let `label`'s text name `id` when `id` has no label of its own: a
    /// composite whose label sits beside the control it holds (a settings
    /// row's switch). The relation belongs to the composite, not to the
    /// control's own [`AccessibilityState`], so the control re-projecting
    /// its state keeps it.
    pub fn set_labelled_by(&mut self, id: StableNodeId, label: Option<StableNodeId>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetLabelledBy { id, label },
        );
    }

    /// Make `id` a live region: assistive technology announces what changes
    /// inside it as `live` says, `None` removes the setting. A composite
    /// whose notices come and go (a stack of toasts) sets it on the node
    /// that holds them. Like [`Self::set_labelled_by`] it belongs to the
    /// composite, not to the node's own [`AccessibilityState`], so the node
    /// re-projecting its state keeps it.
    pub fn set_live(&mut self, id: StableNodeId, live: Option<AccessibilityLive>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetLive { id, live },
        );
    }

    /// Override `id`'s `aria-selected` state independently of its component's
    /// own [`AccessibilityState`]; `None` removes the override.
    pub fn set_selected(&mut self, id: StableNodeId, selected: Option<bool>) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetSelected { id, selected },
        );
    }

    pub fn set_overlay_host(&mut self, host: StableNodeId, state: OverlayHostState) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetOverlayHost {
                host,
                state: Box::new(state),
            },
        );
    }

    pub fn set_overlay_mount(
        &mut self,
        id: StableNodeId,
        mount: Option<nana_ui_core::OverlayMount>,
    ) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetOverlayMount {
                id,
                mount: mount.map(Box::new),
            },
        );
    }

    pub fn capture_pointer(&mut self, pointer_id: u64, target: StableNodeId) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::CapturePointer { pointer_id, target },
        );
    }

    /// Release capture only when `target` still owns `pointer_id`.
    pub fn release_pointer(&mut self, pointer_id: u64, target: StableNodeId) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::ReleasePointer { pointer_id, target },
        );
    }

    pub(crate) fn set_surface_open(&mut self, id: StableNodeId, open: bool, menu: bool) {
        self.record(
            BatchCapabilities::VISUAL_OR_INTERACTION,
            UiMutation::SetSurfaceOpen { id, open, menu },
        );
    }

    pub fn start_animation(&mut self, animation: AnimationSpec) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::StartAnimation {
                animation: Box::new(animation),
            },
        );
    }

    pub fn start_animation_with_playback(
        &mut self,
        animation: AnimationSpec,
        playback: AnimationPlayback,
    ) {
        self.start_animation(animation.with_playback(playback));
    }

    /// FLIP: layout is already `last`; compositor transform plays invert → identity.
    pub fn start_layout_flip(
        &mut self,
        target: StableNodeId,
        first: nana_ui_core::FlipRect,
        last: nana_ui_core::FlipRect,
        now: Duration,
        duration: Duration,
        easing: crate::Easing,
    ) {
        if let Some(spec) = crate::layout_flip_spec(target, first, last, now, duration, easing) {
            self.start_animation(spec);
        }
    }

    pub fn hold_layout_flip(
        &mut self,
        target: StableNodeId,
        invert: nana_ui_core::PaintTransform,
        now: Duration,
    ) {
        if let Some(spec) = crate::layout_flip_hold_spec(target, invert, now) {
            self.start_animation(spec);
        }
    }

    pub fn play_layout_flip(
        &mut self,
        target: StableNodeId,
        invert: nana_ui_core::PaintTransform,
        now: Duration,
        duration: Duration,
        easing: crate::Easing,
    ) {
        if let Some(spec) = crate::layout_flip_play_spec(target, invert, now, duration, easing) {
            self.start_animation(spec);
        }
    }

    /// Replace the running track `animation.id` whole, as written: no
    /// retarget from the old run, no `Cancelled`, no new id. The document
    /// samples it at its current presentation time, so a paused document
    /// shows the edited curve at the frame it holds instead of replaying it
    /// from 0. Read the current tracks with `UiWorld::animation_tracks`.
    /// Fails with `MissingAnimation` when no run has that id.
    pub fn replace_animation(&mut self, animation: AnimationSpec) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::ReplaceAnimation {
                animation: Box::new(animation),
            },
        );
    }

    pub fn stop_animation(&mut self, id: AnimationId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::StopAnimation { id },
        );
    }

    pub fn finish_animation(&mut self, id: AnimationId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::FinishAnimation { id },
        );
    }

    pub fn reverse_animation(&mut self, id: AnimationId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::ReverseAnimation { id },
        );
    }

    pub fn pause_animation(&mut self, id: AnimationId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::PauseAnimation { id },
        );
    }

    pub fn resume_animation(&mut self, id: AnimationId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::ResumeAnimation { id },
        );
    }

    pub fn register_shared_element(
        &mut self,
        target: StableNodeId,
        key: impl Into<nana_ui_core::SharedElementKey>,
        transition: nana_ui_core::SharedElementTransition,
    ) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::SetSharedElement {
                target,
                key: Some(Box::new((key.into(), transition))),
            },
        );
    }

    pub fn clear_shared_element(&mut self, target: StableNodeId) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::SetSharedElement { target, key: None },
        );
    }

    pub fn capture_shared_element(
        &mut self,
        target: StableNodeId,
        key: Option<nana_ui_core::SharedElementKey>,
    ) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::CaptureSharedElement { target, key },
        );
    }

    pub fn claim_shared_element(
        &mut self,
        target: StableNodeId,
        key: impl Into<nana_ui_core::SharedElementKey>,
        transition: nana_ui_core::SharedElementTransition,
        now: Duration,
    ) {
        self.record(
            BatchCapabilities::ANIMATION,
            UiMutation::ClaimSharedElement {
                target,
                claim: Box::new(SharedElementClaim {
                    key: key.into(),
                    transition,
                    now,
                }),
            },
        );
    }

    pub fn request_focus(&mut self, document: DocumentId, target: Option<StableNodeId>) {
        self.record(
            BatchCapabilities::FOCUS,
            UiMutation::RequestFocus { document, target },
        );
    }

    /// Restore the last valid focus in a registered scope after it is mounted.
    /// Missing, hidden, disabled or reparented targets leave focus unchanged.
    pub fn restore_focus_within(&mut self, root: StableNodeId) {
        self.record(
            BatchCapabilities::FOCUS,
            UiMutation::RestoreFocusWithin { root },
        );
    }

    pub fn set_focus_policy(&mut self, id: StableNodeId, policy: Option<crate::FocusPolicy>) {
        self.record(
            BatchCapabilities::FOCUS,
            UiMutation::SetFocusPolicy { id, policy },
        );
    }

    pub fn set_focus_neighbors(&mut self, id: StableNodeId, neighbors: crate::FocusNeighbors) {
        self.record(
            BatchCapabilities::FOCUS,
            UiMutation::SetFocusNeighbors {
                id,
                neighbors: Box::new(neighbors),
            },
        );
    }

    pub fn set_directional_focus(
        &mut self,
        id: StableNodeId,
        scope: Option<crate::DirectionalFocusScope>,
    ) {
        self.record(
            BatchCapabilities::FOCUS,
            UiMutation::SetDirectionalFocus { id, scope },
        );
    }

    pub fn set_ime(&mut self, id: StableNodeId, composition: Option<ImeComposition>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetIme {
                id,
                composition: composition.map(Box::new),
            },
        );
    }

    pub fn set_text_input(&mut self, id: StableNodeId, state: Option<TextInputState>) {
        self.record(
            BatchCapabilities::EDITOR.union(BatchCapabilities::WRITES_TEXT),
            UiMutation::SetTextInput {
                id,
                state: state.map(Box::new),
            },
        );
    }

    pub fn set_text_selection(&mut self, id: StableNodeId, selection: TextSelection) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextSelection { id, selection },
        );
    }

    pub fn replace_text_selection(&mut self, id: StableNodeId, text: impl Into<String>) {
        self.record(
            BatchCapabilities::EDITOR.union(BatchCapabilities::WRITES_TEXT),
            UiMutation::ReplaceTextSelection {
                id,
                text: text.into().into_boxed_str(),
            },
        );
    }

    pub fn set_highlight_request(&mut self, id: StableNodeId, request: Option<HighlightRequest>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetHighlightRequest {
                id,
                request: request.map(Box::new),
            },
        );
    }

    pub fn set_text_input_fold_collapsed(&mut self, id: StableNodeId, folds: Arc<[TextCodeFold]>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputFoldCollapsed { id, folds },
        );
    }

    /// 喂入或撤掉（空表）行内提示（见 [`TextInlay`]）。
    pub fn set_text_input_inlays(&mut self, id: StableNodeId, inlays: Arc<[TextInlay]>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputInlays { id, inlays },
        );
    }

    pub fn set_text_input_snippet(
        &mut self,
        id: StableNodeId,
        session: Option<TextSnippetSession>,
    ) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputSnippet {
                id,
                session: session.map(Box::new),
            },
        );
    }

    pub fn set_text_input_completions(&mut self, id: StableNodeId, items: Arc<[TextCompletion]>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputCompletions { id, items },
        );
    }

    pub fn set_text_input_completion_view(
        &mut self,
        id: StableNodeId,
        selected: usize,
        scroll: usize,
    ) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputCompletionView {
                id,
                selected,
                scroll,
            },
        );
    }

    pub fn set_text_input_completion_dismissed(&mut self, id: StableNodeId) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputCompletionDismissed { id },
        );
    }

    pub fn set_text_input_completion_reopened(&mut self, id: StableNodeId) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputCompletionReopened { id },
        );
    }

    pub fn set_text_input_hover(&mut self, id: StableNodeId, hover: Option<TextHover>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputHover {
                id,
                hover: hover.map(Box::new),
            },
        );
    }

    pub fn set_text_input_diagnostic_hover(&mut self, id: StableNodeId, hover: Option<TextHover>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputDiagnosticHover {
                id,
                hover: hover.map(Box::new),
            },
        );
    }

    pub fn set_text_input_hover_scroll(&mut self, id: StableNodeId, scroll: usize) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputHoverScroll { id, scroll },
        );
    }

    /// 喂入或撤掉（`None`）签名帮助浮窗。
    pub fn set_text_input_signature(&mut self, id: StableNodeId, help: Option<TextSignatureHelp>) {
        self.record(
            BatchCapabilities::EDITOR,
            UiMutation::SetTextInputSignature {
                id,
                help: help.map(Box::new),
            },
        );
    }

    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    pub fn clear(&mut self) {
        self.mutations.clear();
        self.capabilities = BatchCapabilities::EMPTY;
    }

    pub fn as_slice(&self) -> &[UiMutation] {
        &self.mutations
    }

    /// Append one pre-built mutation. Used by adapters that replay a queue
    /// one mutation at a time after a batch was rejected.
    pub fn push(&mut self, mutation: UiMutation) {
        self.record(BatchCapabilities::for_mutation(&mutation), mutation);
    }

    /// Whether any mutation in the queue writes an editor's text
    /// (`SetTextInput` or `ReplaceTextSelection`).
    pub fn writes_text(&self) -> bool {
        self.capabilities.contains(BatchCapabilities::WRITES_TEXT)
    }
}
