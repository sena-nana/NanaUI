use crate::{
    AccessibilityState, AnimationId, AnimationPlayback, AnimationSpec, ComponentTypeId,
    CustomRenderNode, DocumentId, HighlightRequest, ImeComposition, InteractionState, LayoutBox,
    NodeKind, NodeStyle, OverlayHostState, ScrollMetrics, ScrollOffset, StableNodeId,
    StandardVisual, TextContent, TextInputState, TextSelection,
};
use nana_ui_core::ThemeMode;

/// One retained-tree mutation. Mutations are validated as a batch before the
/// authoritative world changes.
#[derive(Debug, Clone, PartialEq)]
pub enum UiMutation {
    Create {
        id: StableNodeId,
        document: DocumentId,
        kind: NodeKind,
    },
    /// Insert or reparent `child`. `before = None` appends it.
    Insert {
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
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
        style: NodeStyle,
    },
    SetTheme {
        mode: ThemeMode,
    },
    SetStyleTokens {
        mode: ThemeMode,
        metrics: nana_ui_core::ThemeMetrics,
        palette: Box<nana_ui_core::SemanticPalette>,
        titlebar: nana_ui_core::SemanticColor,
    },
    SetText {
        id: StableNodeId,
        text: TextContent,
    },
    /// Engine writeback (and tests). Product Vue frames must not use this to
    /// fight [`crate::RuntimeLayoutEngine`]; mixed trees flush that engine.
    WriteLayout {
        id: StableNodeId,
        layout: LayoutBox,
    },
    SetScrollOffset {
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
    SetCustomRender {
        id: StableNodeId,
        content: Option<CustomRenderNode>,
    },
    SetEventListener {
        id: StableNodeId,
        event: String,
        enabled: bool,
    },
    SetComponentType {
        id: StableNodeId,
        type_id: Option<ComponentTypeId>,
    },
    SetStandardVisual {
        id: StableNodeId,
        visual: Option<StandardVisual>,
    },
    SetAccessibility {
        id: StableNodeId,
        accessibility: AccessibilityState,
    },
    SetOverlayHost {
        host: StableNodeId,
        state: OverlayHostState,
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
        animation: AnimationSpec,
    },
    StopAnimation {
        id: AnimationId,
    },
    RequestFocus {
        document: DocumentId,
        target: Option<StableNodeId>,
    },
    SetIme {
        id: StableNodeId,
        composition: Option<ImeComposition>,
    },
    SetTextInput {
        id: StableNodeId,
        state: Option<TextInputState>,
    },
    SetTextSelection {
        id: StableNodeId,
        selection: TextSelection,
    },
    ReplaceTextSelection {
        id: StableNodeId,
        text: String,
    },
    SetHighlightRequest {
        id: StableNodeId,
        request: Option<HighlightRequest>,
    },
}

/// Frame-local command boundary for reconcilers and adapters.
#[derive(Debug, Default)]
pub struct MutationQueue {
    mutations: Vec<UiMutation>,
}

impl MutationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self, id: StableNodeId, document: DocumentId, kind: NodeKind) {
        self.mutations
            .push(UiMutation::Create { id, document, kind });
    }

    pub fn insert(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
    ) {
        self.mutations.push(UiMutation::Insert {
            parent,
            child,
            before,
        });
    }

    pub fn detach(&mut self, id: StableNodeId) {
        self.mutations.push(UiMutation::Detach { id });
    }

    pub fn park_subtree(&mut self, root: StableNodeId) {
        self.mutations.push(UiMutation::ParkSubtree { root });
    }

    pub fn despawn_subtree(&mut self, root: StableNodeId) {
        self.mutations.push(UiMutation::DespawnSubtree { root });
    }

    pub fn set_style(&mut self, id: StableNodeId, style: NodeStyle) {
        self.mutations.push(UiMutation::SetStyle { id, style });
    }

    pub fn set_theme(&mut self, mode: ThemeMode) {
        self.mutations.push(UiMutation::SetTheme { mode });
    }

    pub fn set_style_tokens(
        &mut self,
        mode: ThemeMode,
        metrics: nana_ui_core::ThemeMetrics,
        palette: nana_ui_core::SemanticPalette,
        titlebar: nana_ui_core::SemanticColor,
    ) {
        self.mutations.push(UiMutation::SetStyleTokens {
            mode,
            metrics,
            palette: Box::new(palette),
            titlebar,
        });
    }

    pub fn set_text(&mut self, id: StableNodeId, text: TextContent) {
        self.mutations.push(UiMutation::SetText { id, text });
    }

    /// Publish a box computed by [`crate::RuntimeLayoutEngine`], or by a test.
    /// Vue CSS measure is not a second product layout writer.
    pub fn write_layout(&mut self, id: StableNodeId, layout: LayoutBox) {
        self.mutations.push(UiMutation::WriteLayout { id, layout });
    }

    pub fn set_scroll_offset(&mut self, id: StableNodeId, offset: ScrollOffset) {
        self.mutations
            .push(UiMutation::SetScrollOffset { id, offset });
    }

    pub fn set_scroll_metrics(&mut self, id: StableNodeId, metrics: Option<ScrollMetrics>) {
        self.mutations
            .push(UiMutation::SetScrollMetrics { id, metrics });
    }

    pub fn set_interaction(&mut self, id: StableNodeId, interaction: InteractionState) {
        self.mutations
            .push(UiMutation::SetInteraction { id, interaction });
    }

    pub fn set_custom_render(&mut self, id: StableNodeId, content: Option<CustomRenderNode>) {
        self.mutations
            .push(UiMutation::SetCustomRender { id, content });
    }

    pub fn set_event_listener(
        &mut self,
        id: StableNodeId,
        event: impl Into<String>,
        enabled: bool,
    ) {
        self.mutations.push(UiMutation::SetEventListener {
            id,
            event: event.into(),
            enabled,
        });
    }

    pub fn set_component_type(&mut self, id: StableNodeId, type_id: Option<ComponentTypeId>) {
        self.mutations
            .push(UiMutation::SetComponentType { id, type_id });
    }

    pub fn append(&mut self, mut other: Self) {
        self.mutations.append(&mut other.mutations);
    }

    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn set_standard_visual(&mut self, id: StableNodeId, visual: Option<StandardVisual>) {
        self.mutations
            .push(UiMutation::SetStandardVisual { id, visual });
    }

    pub fn set_accessibility(&mut self, id: StableNodeId, accessibility: AccessibilityState) {
        self.mutations
            .push(UiMutation::SetAccessibility { id, accessibility });
    }

    pub fn set_overlay_host(&mut self, host: StableNodeId, state: OverlayHostState) {
        self.mutations
            .push(UiMutation::SetOverlayHost { host, state });
    }

    pub fn capture_pointer(&mut self, pointer_id: u64, target: StableNodeId) {
        self.mutations
            .push(UiMutation::CapturePointer { pointer_id, target });
    }

    /// Release capture only when `target` still owns `pointer_id`.
    pub fn release_pointer(&mut self, pointer_id: u64, target: StableNodeId) {
        self.mutations
            .push(UiMutation::ReleasePointer { pointer_id, target });
    }

    pub fn start_animation(&mut self, animation: AnimationSpec) {
        self.mutations
            .push(UiMutation::StartAnimation { animation });
    }

    pub fn start_animation_with_playback(
        &mut self,
        animation: AnimationSpec,
        playback: AnimationPlayback,
    ) {
        self.start_animation(animation.with_playback(playback));
    }

    pub fn stop_animation(&mut self, id: AnimationId) {
        self.mutations.push(UiMutation::StopAnimation { id });
    }

    pub fn request_focus(&mut self, document: DocumentId, target: Option<StableNodeId>) {
        self.mutations
            .push(UiMutation::RequestFocus { document, target });
    }

    pub fn set_ime(&mut self, id: StableNodeId, composition: Option<ImeComposition>) {
        self.mutations.push(UiMutation::SetIme { id, composition });
    }

    pub fn set_text_input(&mut self, id: StableNodeId, state: Option<TextInputState>) {
        self.mutations.push(UiMutation::SetTextInput { id, state });
    }

    pub fn set_text_selection(&mut self, id: StableNodeId, selection: TextSelection) {
        self.mutations
            .push(UiMutation::SetTextSelection { id, selection });
    }

    pub fn replace_text_selection(&mut self, id: StableNodeId, text: impl Into<String>) {
        self.mutations.push(UiMutation::ReplaceTextSelection {
            id,
            text: text.into(),
        });
    }

    pub fn set_highlight_request(&mut self, id: StableNodeId, request: Option<HighlightRequest>) {
        self.mutations
            .push(UiMutation::SetHighlightRequest { id, request });
    }

    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    pub fn as_slice(&self) -> &[UiMutation] {
        &self.mutations
    }

    /// Append one pre-built mutation. Used by adapters that replay a queue
    /// one mutation at a time after a batch was rejected.
    pub fn push(&mut self, mutation: UiMutation) {
        self.mutations.push(mutation);
    }
}
