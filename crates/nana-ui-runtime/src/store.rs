//! Dense node table plus sparse side maps. Replaces `bevy_ecs::World` inside
//! [`crate::UiWorld`]. Public identity stays [`crate::StableNodeId`].

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::ComputedStyle;
use crate::component_registry::ComponentTypeId;
use crate::components::{
    AccessibilityState, CustomRenderNode, EmptyStateTextPresentation, EventListeners,
    ImeComposition, InteractionState, LayoutBox, ModalTextPresentation, MountState, NodeStyle,
    OverlayHostState, ScrollMetrics, ScrollOffset, StandardVisual, TextCodeFold, TextContent,
    TextInputPresentation, TextInputState, TextMetrics, TextSnippetSession,
};
use crate::presentation::{HighlightRequest, TextPresentation};
use crate::schedule::DirtyMask;
use crate::world::{DocumentId, NodeKind, StableNodeId};

static EMPTY_CHILDREN: LazyLock<Arc<Vec<StableNodeId>>> = LazyLock::new(|| Arc::new(Vec::new()));
static INTERNED_KIND_DOCUMENT: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Document));
static INTERNED_KIND_TEXT: LazyLock<Arc<NodeKind>> = LazyLock::new(|| Arc::new(NodeKind::Text));
static INTERNED_KIND_COMMENT: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Comment));
static INTERNED_KIND_DIV: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Element { tag: "div".into() }));
static INTERNED_DEFAULT_STYLE: LazyLock<Arc<ComputedStyle>> =
    LazyLock::new(|| Arc::new(ComputedStyle::default()));

fn intern_kind(kind: &NodeKind) -> Arc<NodeKind> {
    match kind {
        NodeKind::Document => Arc::clone(&INTERNED_KIND_DOCUMENT),
        NodeKind::Text => Arc::clone(&INTERNED_KIND_TEXT),
        NodeKind::Comment => Arc::clone(&INTERNED_KIND_COMMENT),
        NodeKind::Element { tag } if tag == "div" => Arc::clone(&INTERNED_KIND_DIV),
        _ => Arc::new(kind.clone()),
    }
}

pub(crate) fn intern_empty_children(children: &mut Arc<Vec<StableNodeId>>) {
    if children.is_empty() {
        *children = Arc::clone(&EMPTY_CHILDREN);
    }
}

#[derive(Clone)]
pub(crate) struct Hierarchy {
    pub parent: Option<StableNodeId>,
    pub children: Arc<Vec<StableNodeId>>,
}

impl Default for Hierarchy {
    fn default() -> Self {
        Self {
            parent: None,
            children: Arc::clone(&EMPTY_CHILDREN),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ResolvedStyle(pub Arc<ComputedStyle>, pub u64);

impl ResolvedStyle {
    fn interned_default() -> Self {
        Self(Arc::clone(&INTERNED_DEFAULT_STYLE), 0)
    }
}

pub(crate) struct NodeRecord {
    pub document: DocumentId,
    pub kind: Arc<NodeKind>,
    pub hierarchy: Hierarchy,
    pub mount: MountState,
    pub style: NodeStyle,
    pub resolved: ResolvedStyle,
    pub text: TextContent,
    pub text_metrics: TextMetrics,
    pub layout: LayoutBox,
    pub layout_padding: Option<nana_ui_core::PaddingSpec>,
    pub scroll_offset: ScrollOffset,
    pub interaction: InteractionState,
    pub accessibility: AccessibilityState,
    pub dirty: DirtyMask,
}

impl NodeRecord {
    pub fn new(document: DocumentId, kind: &NodeKind, interaction: InteractionState) -> Self {
        Self {
            document,
            kind: intern_kind(kind),
            hierarchy: Hierarchy::default(),
            mount: MountState::default(),
            style: NodeStyle::default(),
            resolved: ResolvedStyle::interned_default(),
            text: TextContent::default(),
            text_metrics: TextMetrics::default(),
            layout: LayoutBox::default(),
            layout_padding: None,
            scroll_offset: ScrollOffset::default(),
            interaction,
            accessibility: AccessibilityState::default(),
            dirty: DirtyMask::all(),
        }
    }
}

macro_rules! sparse {
    ($field:ident, $ty:ty, $get:ident, $set:ident) => {
        pub fn $get(&self, id: StableNodeId) -> Option<&$ty> {
            self.$field.get(&id)
        }

        pub fn $set(&mut self, id: StableNodeId, value: Option<$ty>) {
            if let Some(value) = value {
                self.$field.insert(id, value);
            } else {
                self.$field.remove(&id);
            }
        }
    };
}

/// 代码折叠视图状态（仅喂过折叠区间的编辑器节点持有条目）。
#[derive(Clone, Debug, Default)]
pub(crate) struct TextFoldViewState {
    /// 宿主上一次喂入的折叠区间（用于重喂时的漂移匹配）。
    pub offered: Arc<[TextCodeFold]>,
    /// 当前折叠态的区间（`offered` 的子集，按 `start` 排序）。
    pub collapsed: Vec<TextCodeFold>,
}

/// 补全弹层会话状态（仅候选会话活跃的编辑器节点持有条目）。
///
/// 候选列表由宿主喂入（过滤是宿主的职责），组件只维护键盘选中与滚动：
/// 喂入相同列表不重置选中态（包括 Esc 关闭态）；喂入不同列表时视为新
/// 会话（选中归零、重新打开）；喂入空列表整个条目移除（零分配待机）。
#[derive(Clone, Debug)]
pub(crate) struct TextCompletionViewState {
    pub items: Arc<[crate::TextCompletion]>,
    pub selected: usize,
    /// 第一条可见候选的绝对下标。
    pub scroll: usize,
    /// Esc 关闭标记：会话数据保留（宿主重喂相同列表不复活弹层），
    /// 换新列表或宿主撤空重喂后清除。
    pub dismissed: bool,
}

/// hover 文档浮窗状态（仅宿主喂入 hover 的编辑器节点持有条目）。
#[derive(Clone, Debug)]
pub(crate) struct TextHoverViewState {
    pub doc: crate::TextHover,
    /// 正文滚过的逻辑行数。
    pub scroll: usize,
}

#[derive(Default)]
pub(crate) struct NodeStore {
    nodes: HashMap<StableNodeId, NodeRecord>,
    visuals: HashMap<StableNodeId, StandardVisual>,
    custom_render: HashMap<StableNodeId, CustomRenderNode>,
    event_listeners: HashMap<StableNodeId, EventListeners>,
    component_types: HashMap<StableNodeId, ComponentTypeId>,
    overlay_hosts: HashMap<StableNodeId, OverlayHostState>,
    scroll_metrics: HashMap<StableNodeId, ScrollMetrics>,
    ime: HashMap<StableNodeId, ImeComposition>,
    text_inputs: HashMap<StableNodeId, TextInputState>,
    highlights: HashMap<StableNodeId, HighlightRequest>,
    text_presentations: HashMap<StableNodeId, TextPresentation>,
    text_input_presentations: HashMap<StableNodeId, TextInputPresentation>,
    empty_state_text: HashMap<StableNodeId, EmptyStateTextPresentation>,
    modal_text: HashMap<StableNodeId, ModalTextPresentation>,
    /// 代码折叠视图状态（仅喂过折叠区间的编辑器节点持有条目）。
    text_fold_views: HashMap<StableNodeId, TextFoldViewState>,
    /// 活跃 snippet 会话（仅会话进行中的编辑器持有条目）。
    text_snippets: HashMap<StableNodeId, TextSnippetSession>,
    /// 补全弹层会话（仅候选会话活跃的编辑器持有条目）。
    text_completions: HashMap<StableNodeId, TextCompletionViewState>,
    /// hover 文档浮窗（仅宿主喂入 hover 的编辑器持有条目）。
    text_hovers: HashMap<StableNodeId, TextHoverViewState>,
    /// minimap 视口钉住（仅显式导航过的多行编辑器持有条目）。
    text_viewport_pins: HashMap<StableNodeId, ScrollOffset>,
    /// 拖拽移动选中文本的落点指示线（仅拖拽态编辑器持有条目；文本空间
    /// 矩形）。框架侧拖拽状态机写入，提取层翻译为节点空间图元。
    text_drop_indicators: HashMap<StableNodeId, LayoutBox>,
}

/// minimap 视口钉住：显式视口导航（minimap 点击/拖动）写入的滚动偏移。
/// 几何层在「钉住值 == 请求滚动」期间跳过光标 reveal（视口停在用户导航
/// 到的位置）；光标移动后由 shape 趟清除，reveal 恢复权威。
pub(crate) type TextViewportPin = ScrollOffset;

impl NodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn get(&self, id: StableNodeId) -> Option<&NodeRecord> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        self.nodes.get_mut(&id)
    }

    pub fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.nodes.keys().copied()
    }

    /// 持有 hover 浮窗状态的节点（稀疏表直接迭代，命中测试路由用）。
    pub fn text_hover_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.text_hovers.keys().copied()
    }

    pub fn insert(&mut self, id: StableNodeId, record: NodeRecord) {
        self.nodes.insert(id, record);
    }

    /// Drop the dense record and every sparse entry. The only despawn path.
    pub fn remove(&mut self, id: StableNodeId) -> Option<NodeRecord> {
        let record = self.nodes.remove(&id)?;
        self.visuals.remove(&id);
        self.custom_render.remove(&id);
        self.event_listeners.remove(&id);
        self.component_types.remove(&id);
        self.overlay_hosts.remove(&id);
        self.scroll_metrics.remove(&id);
        self.ime.remove(&id);
        self.text_inputs.remove(&id);
        self.highlights.remove(&id);
        self.text_presentations.remove(&id);
        self.text_input_presentations.remove(&id);
        self.empty_state_text.remove(&id);
        self.modal_text.remove(&id);
        self.text_fold_views.remove(&id);
        self.text_snippets.remove(&id);
        self.text_completions.remove(&id);
        self.text_hovers.remove(&id);
        self.text_viewport_pins.remove(&id);
        self.text_drop_indicators.remove(&id);
        Some(record)
    }

    sparse!(visuals, StandardVisual, visual, set_visual);
    sparse!(
        custom_render,
        CustomRenderNode,
        custom_render,
        set_custom_render
    );
    sparse!(
        event_listeners,
        EventListeners,
        event_listeners,
        set_event_listeners
    );
    sparse!(
        component_types,
        ComponentTypeId,
        component_type,
        set_component_type
    );
    sparse!(
        overlay_hosts,
        OverlayHostState,
        overlay_host,
        set_overlay_host
    );
    sparse!(
        scroll_metrics,
        ScrollMetrics,
        scroll_metrics,
        set_scroll_metrics
    );
    sparse!(ime, ImeComposition, ime, set_ime);
    sparse!(text_inputs, TextInputState, text_input, set_text_input);
    sparse!(highlights, HighlightRequest, highlight, set_highlight);
    sparse!(
        text_presentations,
        TextPresentation,
        text_presentation,
        set_text_presentation
    );
    sparse!(
        text_input_presentations,
        TextInputPresentation,
        text_input_presentation,
        set_text_input_presentation
    );
    sparse!(
        empty_state_text,
        EmptyStateTextPresentation,
        empty_state_text,
        set_empty_state_text
    );
    sparse!(
        modal_text,
        ModalTextPresentation,
        modal_text,
        set_modal_text
    );
    sparse!(
        text_fold_views,
        TextFoldViewState,
        text_fold_view,
        set_text_fold_view
    );
    sparse!(
        text_snippets,
        TextSnippetSession,
        text_snippet_session,
        set_text_snippet_session
    );
    sparse!(
        text_completions,
        TextCompletionViewState,
        text_completion_view,
        set_text_completion_view
    );
    sparse!(
        text_hovers,
        TextHoverViewState,
        text_hover_view,
        set_text_hover_view
    );
    sparse!(
        text_viewport_pins,
        ScrollOffset,
        text_viewport_pin,
        set_text_viewport_pin
    );
    sparse!(
        text_drop_indicators,
        LayoutBox,
        text_drop_indicator,
        set_text_drop_indicator
    );

    pub fn text_input_mut(&mut self, id: StableNodeId) -> Option<&mut TextInputState> {
        self.text_inputs.get_mut(&id)
    }

    pub(crate) fn text_completion_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextCompletionViewState> {
        self.text_completions.get_mut(&id)
    }

    pub(crate) fn text_hover_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextHoverViewState> {
        self.text_hovers.get_mut(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InteractionState;
    use crate::world::{DocumentId, NodeKind, StableNodeId};

    fn node(id: u64) -> StableNodeId {
        StableNodeId::new(id).unwrap()
    }

    fn document(id: u64) -> DocumentId {
        DocumentId::new(id).unwrap()
    }

    #[test]
    fn remove_clears_sparse_side_maps() {
        let mut store = NodeStore::new();
        let id = node(2);
        store.insert(
            id,
            NodeRecord::new(
                document(1),
                &NodeKind::Text,
                InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
            ),
        );
        store.set_scroll_metrics(
            id,
            Some(ScrollMetrics {
                viewport_width: 10.0,
                viewport_height: 10.0,
                content_width: 20.0,
                content_height: 20.0,
            }),
        );
        store.set_component_type(id, Some(ComponentTypeId::new("nana.button").unwrap()));
        store.set_ime(
            id,
            Some(ImeComposition {
                text: "a".into(),
                selection: None,
            }),
        );
        assert!(store.scroll_metrics(id).is_some());
        assert!(store.remove(id).is_some());
        assert!(!store.contains(id));
        assert!(store.scroll_metrics(id).is_none());
        assert!(store.component_type(id).is_none());
        assert!(store.ime(id).is_none());
    }

    #[test]
    fn sparse_set_none_removes_entry() {
        let mut store = NodeStore::new();
        let id = node(3);
        store.insert(
            id,
            NodeRecord::new(
                document(1),
                &NodeKind::Element { tag: "div".into() },
                InteractionState::default(),
            ),
        );
        store.set_overlay_host(id, Some(OverlayHostState::default()));
        assert!(store.overlay_host(id).is_some());
        store.set_overlay_host(id, None);
        assert!(store.overlay_host(id).is_none());
        assert!(store.contains(id));
    }
}
