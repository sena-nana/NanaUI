//! Dense node table plus sparse side maps. Replaces `bevy_ecs::World` inside
//! [`crate::UiWorld`]. Public identity stays [`crate::StableNodeId`].

mod paged;

use hashbrown::HashMap;
use std::sync::{Arc, LazyLock};

use crate::ComputedStyle;
use crate::component_registry::ComponentTypeId;
use crate::components::{
    AccessibilityState, CustomRenderNode, EmptyStateTextPresentation, EventListeners, ImeView,
    InteractionState, LayoutBox, ModalTextPresentation, MountState, NodeStyle, OverlayHostState,
    ScrollMetrics, ScrollOffset, TextCodeFold, TextContent, TextInputPresentation, TextInputView,
    TextMetrics, TextSnippetSession,
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
pub(crate) static EMPTY_LIST_ITEM_VISUAL: LazyLock<crate::WidgetVisualHandle> =
    LazyLock::new(|| crate::WidgetVisualHandle::new(crate::ListItemVisual::default()));

/// How many element tags a store shares kinds for ([`NodeStore::intern_kind`]).
const KINDS: usize = 256;

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
    /// This node's index in its parent's `children`, counted from the
    /// parent's [`Self::children_base`] and wrapping: finding a sibling
    /// reads it instead of scanning the list
    /// (`UiWorld::renumber_children_between` keeps it).
    pub index_in_parent: u32,
    /// What this node's first child records as its `index_in_parent`. An
    /// edit that shifts a block of the children moves this instead of
    /// writing each of them a new number.
    pub children_base: u32,
}

impl Default for Hierarchy {
    fn default() -> Self {
        Self {
            parent: None,
            children: Arc::clone(&EMPTY_CHILDREN),
            index_in_parent: 0,
            children_base: 0,
        }
    }
}

/// `(own, palette_epoch, inherited_unpainted)`: `.0` holds the node's own
/// resolved style (with any local background/border/ring paint); `.2` strips
/// non-inherited local box paint so descendants inside painted containers can
/// share it directly via `Arc::clone`.
#[derive(Clone)]
pub(crate) struct ResolvedStyle(
    pub Arc<ComputedStyle>,
    pub u32,
    pub u32,
    pub Arc<ComputedStyle>,
);

const _: () = assert!(std::mem::size_of::<ResolvedStyle>() == 24);
const _: () = assert!(std::mem::size_of::<Hierarchy>() == 24);

impl ResolvedStyle {
    fn interned_default() -> Self {
        Self(
            Arc::clone(&INTERNED_DEFAULT_STYLE),
            0,
            0,
            Arc::clone(&INTERNED_DEFAULT_STYLE),
        )
    }
}

/// The process-wide default a root node inherits from. Sharing this avoids
/// building a fresh `ComputedStyle` per root during style resolution.
pub(crate) fn interned_default_style() -> Arc<ComputedStyle> {
    Arc::clone(&INTERNED_DEFAULT_STYLE)
}

/// Compact inline state of a container's far-start axes (`Option<[bool; 2]>`).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum LayoutFarStart {
    #[default]
    Unset = 0,
    FalseFalse = 1,
    TrueFalse = 2,
    FalseTrue = 3,
    TrueTrue = 4,
}

impl LayoutFarStart {
    #[inline]
    fn from_pair(far: [bool; 2]) -> Self {
        match far {
            [false, false] => Self::FalseFalse,
            [true, false] => Self::TrueFalse,
            [false, true] => Self::FalseTrue,
            [true, true] => Self::TrueTrue,
        }
    }

    #[inline]
    fn to_option(self) -> Option<[bool; 2]> {
        match self {
            Self::Unset => None,
            Self::FalseFalse => Some([false, false]),
            Self::TrueFalse => Some([true, false]),
            Self::FalseTrue => Some([false, true]),
            Self::TrueTrue => Some([true, true]),
        }
    }
}

/// Bitmask of sparse attachments present for a [`NodeRecord`] inside [`NodeStore`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NodePresenceBits(u32);

impl NodePresenceBits {
    pub(crate) const EMPTY: Self = Self(0);
    pub(crate) const GEOMETRY: Self = Self(1 << 0);
    pub(crate) const WIDGET_VISUAL: Self = Self(1 << 1);
    pub(crate) const CUSTOM_RENDER: Self = Self(1 << 2);
    pub(crate) const EVENT_LISTENERS: Self = Self(1 << 3);
    pub(crate) const COMPONENT_TYPE: Self = Self(1 << 4);
    pub(crate) const OVERLAY_HOST: Self = Self(1 << 5);
    pub(crate) const OVERLAY_MOUNT: Self = Self(1 << 6);
    pub(crate) const SCROLL: Self = Self(1 << 7);
    pub(crate) const EDITOR: Self = Self(1 << 8);
    pub(crate) const EMPTY_STATE_TEXT: Self = Self(1 << 9);
    pub(crate) const MODAL_TEXT: Self = Self(1 << 10);
    pub(crate) const TEXT_DROP_INDICATOR: Self = Self(1 << 11);
    pub(crate) const EXPLICIT_RECT: Self = Self(1 << 12);
    pub(crate) const TEXT_NATURAL_WIDTH: Self = Self(1 << 13);
    pub(crate) const BLOCK_STRUT: Self = Self(1 << 14);
    pub(crate) const TEXT_WRAP_LIMIT: Self = Self(1 << 15);
    pub(crate) const LANGUAGE: Self = Self(1 << 16);
    pub(crate) const TEXT_SCALE: Self = Self(1 << 17);
    pub(crate) const RICH_TEXT: Self = Self(1 << 18);
    pub(crate) const INLINE_CONTENT: Self = Self(1 << 19);
    pub(crate) const INLINE_PARENT: Self = Self(1 << 20);
    pub(crate) const RICH_EDITOR_MARKS: Self = Self(1 << 21);
    pub(crate) const GLYPH_PRESENTATION: Self = Self(1 << 22);
    pub(crate) const TEXT_CONTENT: Self = Self(1 << 23);
    pub(crate) const TEXT_METRICS: Self = Self(1 << 24);
    pub(crate) const ACCESSIBILITY: Self = Self(1 << 25);
    pub(crate) const RENDER_ROOM_EPOCH: Self = Self(1 << 26);
    pub(crate) const TEXT_NODE: Self = Self(1 << 27);
    pub(crate) const THEME_DEP: Self = Self(1 << 28);
    pub(crate) const NODE_SCOPE: Self = Self(1 << 29);
    pub(crate) const SPACE_PIN: Self = Self(1 << 30);
    pub(crate) const PAGE_BOUND: Self = Self(1 << 31);

    #[inline]
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub(crate) const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub(crate) fn set(&mut self, bit: Self, present: bool) {
        if present {
            self.0 |= bit.0;
        } else {
            self.0 &= !bit.0;
        }
    }

    #[inline]
    pub(crate) fn clear(&mut self, bit: Self) {
        self.0 &= !bit.0;
    }
}

pub(crate) struct NodeRecord {
    pub document: DocumentId,
    pub kind: Arc<NodeKind>,
    pub hierarchy: Hierarchy,
    pub mount: MountState,
    pub style: Arc<NodeStyle>,
    /// [`Self::style`]'s layout with the node's design intent already applied
    /// (radius tier → `border_radius`), resolved against the installed
    /// `ThemeMetrics`.
    ///
    /// Resolved on write — when the style is set and when a theme install
    /// changes the metrics — rather than on every read. `LayoutStyle` is 4.8 KB,
    /// so resolving it inside extraction meant an `Arc::make_mut` copy per
    /// control per frame; a palette switch over 1k controls cost three times
    /// what it should. Resolving on write pays that once per change and hands
    /// every reader an `Arc` it can clone.
    ///
    /// [`Self::style`] stays exactly as authored, because that is what
    /// projection diffs against.
    pub resolved_layout: Arc<nana_ui_core::LayoutStyle>,
    pub resolved: ResolvedStyle,
    /// Whether [`Self::style`]'s layout reads the viewport, kept with it so
    /// presence bookkeeping after a visual or text write does not rescan
    /// the layout's lengths.
    pub layout_depends_on_viewport: bool,
    /// Whether this node's subtree reads the block extent it is offered,
    /// kept up to date as styles and child lists change. See
    /// `world/block_reads.rs`.
    pub block_reads: crate::world::BlockExtentReads,
    /// The writing mode, direction and text orientation this node inherits:
    /// its parent's computed values, as last resolved. Kept beside the
    /// record, not only inside the parent's `ComputedStyle`, so layout reads a
    /// node's writing context and its containing block's without a parent
    /// lookup or an `Arc` deref.
    pub inherited_writing: nana_ui_core::WritingContext,
    pub inherited_orientation: nana_ui_core::TextOrientationSpec,
    /// Whether [`NodeStore::text_content`] holds non-empty text for this node.
    pub has_text: bool,
    /// 1-based index into `BUILTIN_COMPONENT_IDS`, or `0` for none/extension.
    pub builtin_component: u8,
    /// 1-based index into `NodeStore::empty_text_backends` for empty non-`Text` nodes.
    pub empty_text_backend: u8,
    /// `0` = unset, `1` = `PaddingSpec::default()`, `2..=254` = distinct table slot, `255` = sparse map.
    pub layout_padding_slot: u8,
    /// Inline accessibility preset:
    /// `0` = default, `1` = plain `Text`, `2` = decorative `Text`, `3` = sparse map,
    /// `4` = plain `Button` (label = node text),
    /// `5` = plain unselected `ListItem` (label = node text),
    /// `6` = plain selected `ListItem` (label = node text).
    pub a11y_preset: u8,
    /// Non-zero when a `Stack` component view was elided from `AppContext::views`.
    pub elided_stack_flags: u8,
    /// Inline widget visual preset (`0` = none/map, `1` = shared default [`ListItemVisual`]).
    pub visual_preset: u8,
    pub layout_far_start: LayoutFarStart,
    /// Inline formatting-context change counter (`255` spills to `layout_context_generations`).
    pub layout_context_gen: u8,
    /// Positional auto-view slot (`#v{slot}`) when declared under its world parent; `u16::MAX` = none.
    pub auto_view_slot: u16,
    /// Levels from the root down to this node, counting it: 1 for a root.
    /// Moving a subtree renumbers it (`UiWorld::set_subtree_depth`), so
    /// checking an insert against `MAX_TREE_DEPTH` reads its parent's depth
    /// instead of walking to the root.
    pub depth: u16,
    /// How this node stands to the coordinate spaces around it, from
    /// [`Self::resolved_layout`], [`NodeStore::layout_frame`], its kind and its
    /// visual ([`crate::node_space::NodeSpace::of`]). Kept up to date by
    /// [`NodeStore::note_node_space`] wherever one of those is written.
    pub space: crate::node_space::NodeSpace,
    /// The formatting context the last layout ran for the children.
    pub layout_context: Option<crate::FormattingContextKind>,
    pub interaction: InteractionState,
    pub dirty: DirtyMask,
    /// Why the node is marked for render extraction, in the world's
    /// extraction epochs: the last epoch anything but a move marked it
    /// (or it was made), and the last one a layout move marked it. Only a
    /// node a move marked and nothing else did since the epoch began may
    /// reuse its last extracted record.
    pub render_full_epoch: u32,
    pub render_moved_epoch: u32,
    pub(crate) attachments: NodePresenceBits,
}

/// Compact 8-byte encoding of [`crate::LayoutFrame`].
///
/// `0` encodes [`crate::LayoutFrame::Parent`], `u64::MAX` encodes
/// [`crate::LayoutFrame::Page`], and any non-zero `n` encodes
/// [`crate::LayoutFrame::Block`] for `StableNodeId(n)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PackedLayoutFrame(u64);

impl PackedLayoutFrame {
    #[inline]
    pub(crate) const fn from_frame(frame: crate::LayoutFrame) -> Self {
        match frame {
            crate::LayoutFrame::Parent => Self(0),
            crate::LayoutFrame::Page => Self(u64::MAX),
            crate::LayoutFrame::Block(id) => {
                debug_assert!(id.get() != u64::MAX);
                Self(id.get())
            }
        }
    }

    #[inline]
    pub(crate) const fn get(self) -> crate::LayoutFrame {
        match self.0 {
            0 => crate::LayoutFrame::Parent,
            u64::MAX => crate::LayoutFrame::Page,
            n => match StableNodeId::new(n) {
                Some(id) => crate::LayoutFrame::Block(id),
                None => crate::LayoutFrame::Parent,
            },
        }
    }
}

pub(crate) const TEXT_SOURCE_NONE_SENTINEL: u32 = u32::MAX;
pub(crate) const TEXT_SOURCE_MISSING: u32 = u32::MAX - 1;
pub(crate) const TEXT_SOURCE_MAX_LEN: usize = (u32::MAX - 2) as usize;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PublishedRareGeometry {
    pub(crate) padding_box: LayoutBox,
    pub(crate) clip: Option<StableNodeId>,
    pub(crate) clip_parent: Option<StableNodeId>,
    pub(crate) containing_block: Option<StableNodeId>,
}

/// Compact per-node published layout metadata held inside [`NodeGeometry`].
#[derive(Debug, Clone)]
pub(crate) struct PublishedLayoutMeta {
    pub(crate) generation: u64,
    pub(crate) deps_hash: u64,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) content_box: LayoutBox,
    pub(crate) overflow: LayoutBox,
    pub(crate) first_baseline_raw: f32,
    pub(crate) single_text_source: u32,
    pub(crate) rare: Option<Box<PublishedRareGeometry>>,
    pub(crate) ext: Option<Box<PublishedLayoutExt>>,
    pub(crate) formatting_context: Option<crate::FormattingContextKind>,
    pub(crate) participation: Option<crate::ParticipationKind>,
    pub(crate) source: crate::LayoutResultSource,
    pub(crate) suppressed: bool,
    pub(crate) overflow_is_text_box: bool,
    #[cfg(test)]
    pub(crate) publish_seq: u64,
}

#[cfg(not(test))]
const _: () = assert!(std::mem::size_of::<PublishedLayoutMeta>() <= 88);

#[derive(Debug, Clone)]
pub(crate) struct PublishedLayoutExt {
    pub(crate) last_baseline: Option<f32>,
    pub(crate) scroll_extent: LayoutBox,
    pub(crate) fragments: Arc<[crate::LayoutFragment]>,
    pub(crate) parts: Arc<[crate::LayoutPart]>,
}

impl PublishedLayoutMeta {
    #[inline]
    pub(crate) fn bounds(&self) -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: self.width,
            height: self.height,
        }
    }

    #[inline]
    pub(crate) fn first_baseline(&self) -> Option<f32> {
        (!self.first_baseline_raw.is_nan()).then_some(self.first_baseline_raw)
    }

    #[inline]
    pub(crate) fn padding_box(&self) -> LayoutBox {
        match self.rare.as_deref() {
            Some(rare) => rare.padding_box,
            None => self.bounds(),
        }
    }

    #[inline]
    pub(crate) fn clip(&self) -> Option<StableNodeId> {
        self.rare.as_deref().and_then(|rare| rare.clip)
    }

    #[inline]
    pub(crate) fn clip_parent(&self) -> Option<StableNodeId> {
        self.rare.as_deref().and_then(|rare| rare.clip_parent)
    }

    #[inline]
    pub(crate) fn containing_block(&self) -> Option<StableNodeId> {
        self.rare.as_deref().and_then(|rare| rare.containing_block)
    }

    pub(crate) fn last_baseline(&self) -> Option<f32> {
        match self.ext.as_ref() {
            Some(ext) => ext.last_baseline,
            None => self.first_baseline(),
        }
    }

    pub(crate) fn overflow_box(&self) -> LayoutBox {
        if self.overflow_is_text_box {
            self.bounds()
        } else {
            self.overflow
        }
    }

    pub(crate) fn inline_text_box(&self) -> LayoutBox {
        if self.overflow_is_text_box {
            self.overflow
        } else {
            self.content_box
        }
    }

    pub(crate) fn scroll_extent(&self) -> LayoutBox {
        match self.ext.as_ref() {
            Some(ext) => ext.scroll_extent,
            None => self.overflow_box(),
        }
    }

    pub(crate) fn sticky_boxes(&self) -> crate::StickyBoxes {
        crate::StickyBoxes {
            padding_box: self.padding_box(),
            content_box: self.content_box,
            scroll_extent: self.scroll_extent(),
        }
    }

    pub(crate) fn extracted_layout(
        &self,
        scroll_metrics: Option<crate::ScrollMetrics>,
    ) -> crate::ExtractedLayout {
        crate::ExtractedLayout {
            generation: self.generation,
            clip_parent: self.clip_parent(),
            scroll_metrics,
            atomic_inline: self.participation == Some(crate::ParticipationKind::AtomicInline),
            sticky_boxes: self.sticky_boxes(),
        }
    }

    pub(crate) fn single_text_range(&self) -> Option<std::ops::Range<usize>> {
        match self.single_text_source {
            TEXT_SOURCE_NONE_SENTINEL | TEXT_SOURCE_MISSING => None,
            len => Some(0..len as usize),
        }
    }

    pub(crate) fn fragments_and_parts_eq(&self, other: &Self) -> bool {
        match (self.ext.as_ref(), other.ext.as_ref()) {
            (None, None) => {
                self.content_box == other.content_box
                    && self.inline_text_box() == other.inline_text_box()
                    && self.first_baseline() == other.first_baseline()
                    && self.single_text_source == other.single_text_source
            }
            (Some(a), Some(b)) => a.fragments == b.fragments && a.parts == b.parts,
            (Some(ext), None) => compact_matches_ext(other, ext),
            (None, Some(ext)) => compact_matches_ext(self, ext),
        }
    }

    pub(crate) fn geometry_eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.padding_box() == other.padding_box()
            && self.content_box == other.content_box
            && self.first_baseline() == other.first_baseline()
            && self.last_baseline() == other.last_baseline()
            && self.overflow_box() == other.overflow_box()
            && self.scroll_extent() == other.scroll_extent()
            && self.fragments_and_parts_eq(other)
            && self.clip() == other.clip()
            && self.clip_parent() == other.clip_parent()
            && self.containing_block() == other.containing_block()
            && self.deps_hash == other.deps_hash
            && self.formatting_context == other.formatting_context
            && self.participation == other.participation
    }
}

fn compact_matches_ext(compact: &PublishedLayoutMeta, ext: &PublishedLayoutExt) -> bool {
    if compact.single_text_source == TEXT_SOURCE_NONE_SENTINEL {
        return ext.fragments.is_empty()
            && ext.parts.len() == 1
            && ext.parts[0]
                == crate::LayoutPart::new(
                    crate::LayoutPartKind::ComponentContent,
                    compact.content_box,
                );
    }
    let text_box = compact.inline_text_box();
    let source = compact.single_text_range();
    let baseline = compact.first_baseline();
    ext.fragments.len() == 2
        && ext.fragments[0].kind == crate::LayoutFragmentKind::TextLine
        && ext.fragments[0].node.is_none()
        && ext.fragments[0].index == 0
        && ext.fragments[0].bounds == text_box
        && ext.fragments[0].first_baseline == baseline
        && ext.fragments[0].last_baseline == baseline
        && ext.fragments[0].source == source
        && ext.fragments[1].kind == crate::LayoutFragmentKind::TextRun
        && ext.fragments[1].node.is_none()
        && ext.fragments[1].index == 0
        && ext.fragments[1].bounds == text_box
        && ext.fragments[1].first_baseline == baseline
        && ext.fragments[1].last_baseline == baseline
        && ext.fragments[1].source == source
        && ext.parts.len() == 2
        && ext.parts[0] == crate::LayoutPart::new(crate::LayoutPartKind::TextContent, text_box)
        && ext.parts[1]
            == crate::LayoutPart::new(crate::LayoutPartKind::ComponentContent, compact.content_box)
}

/// Single authoritative per-node layout geometry record.
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeGeometry {
    pub(crate) local: LayoutBox,
    frame: PackedLayoutFrame,
    pub(crate) published: Option<PublishedLayoutMeta>,
}

impl NodeGeometry {
    #[inline]
    pub(crate) const fn frame(&self) -> crate::LayoutFrame {
        self.frame.get()
    }

    #[inline]
    pub(crate) fn set_frame(&mut self, frame: crate::LayoutFrame) {
        self.frame = PackedLayoutFrame::from_frame(frame);
    }
}

const _: () = assert!(std::mem::size_of::<NodeRecord>() <= 128);

static DEFAULT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> =
    std::sync::LazyLock::new(|| Arc::new(AccessibilityState::default()));
static TEXT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> = LazyLock::new(|| {
    Arc::new(AccessibilityState {
        role: crate::AccessibilityRole::Text,
        hidden: false,
        ..AccessibilityState::default()
    })
});
static DECORATIVE_TEXT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> =
    LazyLock::new(|| {
        Arc::new(AccessibilityState {
            role: crate::AccessibilityRole::Text,
            hidden: true,
            ..AccessibilityState::default()
        })
    });
static DEFAULT_TEXT_CONTENT: std::sync::LazyLock<TextContent> =
    std::sync::LazyLock::new(TextContent::default);

/// `state` as a record holds it: the one shared default when it is the
/// default, its own allocation otherwise.
pub(crate) fn shared_accessibility(state: &AccessibilityState) -> Arc<AccessibilityState> {
    if *state == **DEFAULT_ACCESSIBILITY {
        Arc::clone(&DEFAULT_ACCESSIBILITY)
    } else if *state == **TEXT_ACCESSIBILITY {
        Arc::clone(&TEXT_ACCESSIBILITY)
    } else if *state == **DECORATIVE_TEXT_ACCESSIBILITY {
        Arc::clone(&DECORATIVE_TEXT_ACCESSIBILITY)
    } else {
        Arc::new(state.clone())
    }
}

fn preset_control_accessibility(preset: u8, label: Arc<str>) -> AccessibilityState {
    match preset {
        4 => AccessibilityState {
            role: crate::AccessibilityRole::Button,
            label: Some(label),
            ..AccessibilityState::default()
        },
        5 => AccessibilityState {
            role: crate::AccessibilityRole::ListItem,
            label: Some(label),
            selected: Some(false),
            ..AccessibilityState::default()
        },
        6 => AccessibilityState {
            role: crate::AccessibilityRole::ListItem,
            label: Some(label),
            selected: Some(true),
            ..AccessibilityState::default()
        },
        _ => AccessibilityState::default(),
    }
}

fn matching_control_preset(state: &AccessibilityState, node_text: &str) -> Option<u8> {
    let label = state.label.as_ref()?;
    if label.as_ref() != node_text {
        return None;
    }
    let preset = match (state.role, state.selected) {
        (crate::AccessibilityRole::Button, None) => 4,
        (crate::AccessibilityRole::ListItem, Some(false)) => 5,
        (crate::AccessibilityRole::ListItem, Some(true)) => 6,
        _ => return None,
    };
    let candidate = preset_control_accessibility(preset, Arc::clone(label));
    (*state == candidate).then_some(preset)
}

impl NodeRecord {
    #[cfg(test)]
    pub fn new(document: DocumentId, kind: &NodeKind, interaction: InteractionState) -> Self {
        Self::with_kind(
            document,
            intern_kind(kind),
            Arc::new(NodeStyle::default()),
            interaction,
        )
    }

    /// [`Self::new`] with the kind already shared ([`NodeStore::intern_kind`])
    /// and the default style a store shares ([`NodeStore::default_style`]).
    pub(crate) fn with_kind(
        document: DocumentId,
        kind: Arc<NodeKind>,
        style: Arc<NodeStyle>,
        interaction: InteractionState,
    ) -> Self {
        let space = crate::node_space::NodeSpace::of(
            &style.layout,
            crate::LayoutFrame::Parent,
            &kind,
            None,
        );
        Self {
            document,
            kind,
            hierarchy: Hierarchy::default(),
            mount: MountState::default(),
            resolved_layout: Arc::clone(&style.layout),
            style,
            resolved: ResolvedStyle::interned_default(),
            layout_depends_on_viewport: false,
            block_reads: crate::world::BlockExtentReads::default(),
            inherited_writing: nana_ui_core::WritingContext::default(),
            inherited_orientation: nana_ui_core::TextOrientationSpec::Mixed,
            has_text: false,
            builtin_component: 0,
            empty_text_backend: 0,
            layout_padding_slot: 0,
            a11y_preset: 0,
            elided_stack_flags: 0,
            visual_preset: 0,
            layout_far_start: LayoutFarStart::Unset,
            layout_context_gen: 0,
            auto_view_slot: u16::MAX,
            depth: 1,
            space,
            layout_context: None,
            interaction,
            dirty: DirtyMask::all(),
            render_full_epoch: 0,
            render_moved_epoch: u32::MAX,
            attachments: NodePresenceBits::EMPTY,
        }
    }
}

macro_rules! sparse {
    ($($field:ident).+, $ty:ty, $get:ident, $set:ident, $bit:expr) => {
        pub fn $get(&self, id: StableNodeId) -> Option<&$ty> {
            self.$($field).+.get(&id)
        }

        pub fn $set(&mut self, id: StableNodeId, value: Option<$ty>) {
            match value {
                Some(value) => {
                    self.$($field).+.insert(id, value);
                    if let Some(record) = self.nodes.get_mut(id) {
                        record.attachments.set($bit, true);
                    }
                }
                None => {
                    self.$($field).+.remove(&id);
                    if let Some(record) = self.nodes.get_mut(id) {
                        record.attachments.clear($bit);
                    }
                }
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ScrollNodeState {
    pub offset: ScrollOffset,
    pub metrics: Option<ScrollMetrics>,
}

#[derive(Debug, Default)]
pub(crate) struct EditorState {
    pub editor: Option<EditorRecord>,
    pub highlight: Option<HighlightRequest>,
    pub text_presentation: Option<TextPresentation>,
    pub text_input_presentation: Option<TextInputPresentation>,
    pub fold_view: Option<TextFoldViewState>,
    pub inlays: Option<Arc<[crate::TextInlay]>>,
    pub snippet: Option<TextSnippetSession>,
    pub completion: Option<TextCompletionViewState>,
    pub hover: Option<TextHoverViewState>,
    pub signature: Option<crate::TextSignatureHelp>,
    pub viewport_pin: Option<ScrollOffset>,
}

impl EditorState {
    #[inline]
    fn is_empty(&self) -> bool {
        self.editor.is_none()
            && self.highlight.is_none()
            && self.text_presentation.is_none()
            && self.text_input_presentation.is_none()
            && self.fold_view.is_none()
            && self.inlays.is_none()
            && self.snippet.is_none()
            && self.completion.is_none()
            && self.hover.is_none()
            && self.signature.is_none()
            && self.viewport_pin.is_none()
    }
}

/// Per-node rectangles few nodes ever hold, kept behind one box so the
/// store (and the world that holds it by value) does not grow by a map each.
#[derive(Default)]
struct ColdRects {
    /// 拖拽移动选中文本的落点指示线（仅拖拽态编辑器持有条目；文本空间
    /// 矩形）。框架侧拖拽状态机写入，提取层翻译为节点空间图元。
    text_drop_indicators: crate::NodeMap<LayoutBox>,
    /// Explicit rects of free items (`display: free` children).
    explicit_rects: crate::NodeMap<crate::ExplicitRect>,
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
    /// 由诊断 span 指针命中写入。离开 span 时框架撤掉；宿主喂入的
    /// 符号 hover 把此位置为 false。
    pub diagnostic: bool,
}

/// One text editor, stored once: the session is the only copy of its
/// committed text, its selections and its IME composition (Issue #182).
#[derive(Debug)]
pub(crate) struct EditorRecord {
    pub session: nana_text::EditSession,
    /// An IME attached with nothing composed: the preedit a platform reports
    /// as empty between keystrokes, or when composition just ended and the
    /// commit is still on its way. Not a composition — the session has none,
    /// and caret motion and display do not treat it as one — but it is still
    /// the IME's turn: a submit waits for its commit or its end. Carries the
    /// IME's (empty) selection so the state reads back as it was written.
    pub empty_preedit: Option<Option<(usize, usize)>>,
    /// What the editor draws, built from the session and the overlays on it
    /// once per change of those, not once per consumer and pass.
    pub display: std::cell::RefCell<EditorDisplayMemo>,
}

/// Memoized display pieces of one editor, each with the inputs it was built
/// from. The keys are stamps and revisions, not dirty bits: a stale entry
/// cannot survive a change it did not see.
#[derive(Debug, Default)]
pub(crate) struct EditorDisplayMemo {
    /// The fold / inlay display view.
    pub view: Option<(DisplayViewKey, Option<crate::world::TextDisplayView>)>,
    /// The committed text with the preedit spliced in, stamped.
    pub composed: Option<((nana_text::TextStamp, u64), crate::TextValue)>,
}

/// What a fold / inlay view is a function of.
#[derive(Debug, Clone)]
pub(crate) struct DisplayViewKey {
    pub text: nana_text::TextStamp,
    pub collapsed: Vec<crate::TextCodeFold>,
    /// The inlay set, by identity: a new feed is a new `Arc`. Held, not
    /// just its address: a freed feed's address can come back for the next
    /// one, and a key that did not keep it alive would then match it.
    pub inlays: Option<Arc<[crate::TextInlay]>>,
    pub composing: bool,
}

impl PartialEq for DisplayViewKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.composing == other.composing
            && self.collapsed == other.collapsed
            && match (&self.inlays, &other.inlays) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl EditorRecord {
    pub fn new(session: nana_text::EditSession) -> Self {
        Self {
            session,
            empty_preedit: None,
            display: Default::default(),
        }
    }

    pub fn ime(&self) -> Option<ImeView<'_>> {
        if let Some(composition) = self.session.composition() {
            return Some(ImeView {
                text: &composition.text,
                selection: composition
                    .selection
                    .as_ref()
                    .map(|range| (range.start, range.end)),
            });
        }
        self.empty_preedit.map(|selection| ImeView {
            text: "",
            selection,
        })
    }
}

/// Ids below this index slots. A larger one, which the
/// allocator reaches only after sixteen million nodes and a caller can
/// otherwise only make up, stays in the overflow map: the directory of pages
/// is as long as the span of ids in use, and this bounds it at 2 MB.
const PAGED_IDS: u64 = 1 << 24;

/// The paged slot `id` has, if it has one.
#[inline]
fn paged_index(id: StableNodeId) -> Option<usize> {
    let raw = id.get();
    (raw < PAGED_IDS).then_some(raw as usize)
}

/// The id whose slot `index` is: [`paged_index`] backwards. A slot holds a
/// value only at an id's index, and ids are never zero.
#[inline]
fn slot_id(index: usize) -> StableNodeId {
    StableNodeId::new(index as u64).expect("a held slot is an id's")
}

/// A map keyed by node, laid out as [`NodeTable`] lays out the records:
/// ids index slots ([`paged::Slots`]: one run while they are close together,
/// pages once they are not), the few past [`PAGED_IDS`] fall back to a hash
/// map. A pass looks the same nodes up many times over,
/// and indexing costs neither a hash nor a probe; a walk in id order (the
/// order nodes are created in) reads the table front to back instead of at
/// random.
///
/// The API follows `HashMap<StableNodeId, V>`, so a table holding about one
/// entry per node can change type without its users changing, except that
/// iteration hands out ids by value. Iteration is in id order, then the
/// overflow's; nothing may depend on either. What it holds follows the
/// entries in it, not the highest id it has seen.
///
/// A slot holds the value alone: its index is the id. Holding the id beside
/// it took 8 bytes a slot in every table, and more where the value is small
/// (a context kind of one byte took a slot of 16).
#[derive(Clone)]
pub(crate) struct DenseNodeMap<V> {
    paged: paged::Slots<V>,
    overflow: crate::NodeMap<V>,
}

impl<V> Default for DenseNodeMap<V> {
    fn default() -> Self {
        Self {
            paged: paged::Slots::default(),
            overflow: crate::NodeMap::default(),
        }
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for DenseNodeMap<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V: PartialEq> PartialEq for DenseNodeMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(id, value)| other.get(&id) == Some(value))
    }
}

impl<V> DenseNodeMap<V> {
    /// A map about to hold `ids`, with room for them when they are close
    /// enough together to be held as one run.
    pub(crate) fn with_highest_of(ids: &[StableNodeId]) -> Self {
        let mut map = Self::default();
        if let Some(&highest) = ids.iter().max() {
            map.reserve_through(highest, ids.len());
        }
        map
    }

    pub(crate) fn len(&self) -> usize {
        self.paged.len() + self.overflow.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub(crate) fn get(&self, id: &StableNodeId) -> Option<&V> {
        match self.paged.run_slot(id.get() as usize) {
            Some(slot) => slot.as_ref(),
            None => self.past_run(id),
        }
    }

    /// [`Self::get`] for an id the run does not reach.
    #[inline(never)]
    fn past_run(&self, id: &StableNodeId) -> Option<&V> {
        match paged_index(*id) {
            Some(index) => self.paged.past_run(index),
            None => self.overflow.get(id),
        }
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, id: &StableNodeId) -> Option<&mut V> {
        let raw = id.get() as usize;
        if self.paged.run_slot(raw).is_some() {
            return self.paged.run_slot_mut(raw).and_then(|slot| slot.as_mut());
        }
        self.past_run_mut(id)
    }

    #[inline(never)]
    fn past_run_mut(&mut self, id: &StableNodeId) -> Option<&mut V> {
        match paged_index(*id) {
            Some(index) => self.paged.past_run_mut(index),
            None => self.overflow.get_mut(id),
        }
    }

    pub(crate) fn contains_key(&self, id: &StableNodeId) -> bool {
        self.get(id).is_some()
    }

    pub(crate) fn insert(&mut self, id: StableNodeId, value: V) -> Option<V> {
        match paged_index(id) {
            Some(index) => self.paged.insert(index, value),
            None => self.overflow.insert(id, value),
        }
    }

    pub(crate) fn remove(&mut self, id: &StableNodeId) -> Option<V> {
        match paged_index(*id) {
            Some(index) => self.paged.remove(index),
            None => self.overflow.remove(id),
        }
    }

    pub(crate) fn entry(&mut self, id: StableNodeId) -> DenseEntry<'_, V> {
        DenseEntry { map: self, id }
    }

    /// Room for every id up to `highest` without growing, for a table
    /// about to hold `entries` nodes: a table of every node of a document
    /// indexes ids from its first to the highest, which is more slots than
    /// nodes. Nothing is set aside when the ids are too far apart for one
    /// run of slots, or the table is on pages.
    pub(crate) fn reserve_through(&mut self, highest: StableNodeId, entries: usize) {
        if let Some(highest) = paged_index(highest) {
            self.paged.reserve_through(highest, entries);
        }
    }

    /// Move every entry of `from` into this map: the whole table when this
    /// one is empty (a first layout's retained cache), entry by entry
    /// otherwise.
    pub(crate) fn absorb(&mut self, from: &mut Self) {
        if self.is_empty() {
            std::mem::swap(self, from);
            from.clear();
            return;
        }
        if let Some(highest) = from.iter().map(|(id, _)| id).max() {
            self.reserve_through(highest, self.len() + from.len());
        }
        self.extend(from.drain());
    }

    /// Every entry, leaving the map empty.
    pub(crate) fn drain(&mut self) -> <Self as IntoIterator>::IntoIter {
        std::mem::take(self).into_iter()
    }

    pub(crate) fn clear(&mut self) {
        self.paged.clear();
        self.overflow.clear();
    }

    pub(crate) fn iter(&self) -> DenseIter<'_, V> {
        DenseIter {
            paged: self.paged.iter(),
            overflow: self.overflow.iter(),
        }
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&StableNodeId, &mut V) -> bool) {
        self.paged
            .retain(|index, value| keep(&slot_id(index), value));
        self.overflow.retain(|id, value| keep(id, value));
    }

    /// The value at `id`, inserting what `make` gives when there is none;
    /// `None` when `make` gives nothing.
    pub(crate) fn get_or_try_insert<E>(
        &mut self,
        id: StableNodeId,
        make: impl FnOnce() -> Result<Option<V>, E>,
    ) -> Result<Option<&V>, E> {
        if self.get(&id).is_none() {
            let Some(value) = make()? else {
                return Ok(None);
            };
            self.insert(id, value);
        }
        Ok(self.get(&id))
    }
}

/// [`DenseNodeMap::entry`]: the subset of `hash_map::Entry` its users need.
pub(crate) struct DenseEntry<'a, V> {
    map: &'a mut DenseNodeMap<V>,
    id: StableNodeId,
}

impl<'a, V> DenseEntry<'a, V> {
    pub(crate) fn or_insert_with(self, make: impl FnOnce() -> V) -> &'a mut V {
        if !self.map.contains_key(&self.id) {
            self.map.insert(self.id, make());
        }
        self.map.get_mut(&self.id).expect("inserted above")
    }

    pub(crate) fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

impl<V> std::ops::Index<&StableNodeId> for DenseNodeMap<V> {
    type Output = V;

    fn index(&self, id: &StableNodeId) -> &V {
        self.get(id).expect("no entry for node")
    }
}

impl<V> Extend<(StableNodeId, V)> for DenseNodeMap<V> {
    fn extend<I: IntoIterator<Item = (StableNodeId, V)>>(&mut self, items: I) {
        for (id, value) in items {
            self.insert(id, value);
        }
    }
}

impl<V> FromIterator<(StableNodeId, V)> for DenseNodeMap<V> {
    fn from_iter<I: IntoIterator<Item = (StableNodeId, V)>>(items: I) -> Self {
        let mut map = Self::default();
        map.extend(items);
        map
    }
}

impl<V> IntoIterator for DenseNodeMap<V> {
    type Item = (StableNodeId, V);
    type IntoIter = std::iter::Chain<
        std::iter::Map<paged::SlotsIntoIter<V>, fn((usize, V)) -> (StableNodeId, V)>,
        <crate::NodeMap<V> as IntoIterator>::IntoIter,
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn entry<V>((index, value): (usize, V)) -> (StableNodeId, V) {
            (slot_id(index), value)
        }
        let entry: fn((usize, V)) -> (StableNodeId, V) = entry::<V>;
        self.paged.into_iter().map(entry).chain(self.overflow)
    }
}

/// [`DenseNodeMap::iter`]: each entry's id by value, as no slot holds it.
pub(crate) struct DenseIter<'a, V> {
    paged: paged::SlotsIter<'a, V>,
    overflow: <&'a crate::NodeMap<V> as IntoIterator>::IntoIter,
}

impl<'a, V> Iterator for DenseIter<'a, V> {
    type Item = (StableNodeId, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.paged.next() {
            Some((index, value)) => Some((slot_id(index), value)),
            None => self.overflow.next().map(|(id, value)| (*id, value)),
        }
    }
}

impl<'a, V> IntoIterator for &'a DenseNodeMap<V> {
    type Item = (StableNodeId, &'a V);
    type IntoIter = DenseIter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The node records, by id: slots for ids below [`PAGED_IDS`]
/// ([`paged::Slots`]), a hash map for the rest. What it holds follows the nodes alive, not the
/// highest id the session has reached.
#[derive(Default)]
struct NodeTable {
    paged: paged::Slots<NodeRecord>,
    overflow: crate::NodeMap<NodeRecord>,
}

impl NodeTable {
    #[inline(always)]
    fn get(&self, id: StableNodeId) -> Option<&NodeRecord> {
        match self.paged.run_slot(id.get() as usize) {
            Some(slot) => slot.as_ref(),
            None => self.past_run(id),
        }
    }

    /// [`Self::get`] for an id the run does not reach.
    #[inline(never)]
    fn past_run(&self, id: StableNodeId) -> Option<&NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.past_run(index),
            None => self.overflow.get(&id),
        }
    }

    #[inline(always)]
    fn get_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        let raw = id.get() as usize;
        if self.paged.run_slot(raw).is_some() {
            return self.paged.run_slot_mut(raw).and_then(|slot| slot.as_mut());
        }
        self.past_run_mut(id)
    }

    #[inline(never)]
    fn past_run_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.past_run_mut(index),
            None => self.overflow.get_mut(&id),
        }
    }

    fn contains(&self, id: StableNodeId) -> bool {
        self.get(id).is_some()
    }

    fn len(&self) -> usize {
        self.paged.len() + self.overflow.len()
    }

    fn insert(&mut self, id: StableNodeId, record: NodeRecord) {
        match paged_index(id) {
            Some(index) => self.paged.insert(index, record),
            None => self.overflow.insert(id, record),
        };
    }

    fn reserve_through(&mut self, highest: StableNodeId, entries: usize) {
        if let Some(highest) = paged_index(highest) {
            self.paged.reserve_through(highest, entries);
        }
    }

    fn remove(&mut self, id: StableNodeId) -> Option<NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.remove(index),
            None => self.overflow.remove(&id),
        }
    }

    fn records(&self) -> impl Iterator<Item = &NodeRecord> + '_ {
        self.paged
            .iter()
            .map(|(_, record)| record)
            .chain(self.overflow.values())
    }

    fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.paged
            .iter()
            .filter_map(|(index, _)| StableNodeId::new(index as u64))
            .chain(self.overflow.keys().copied())
    }

    fn entries(&self) -> impl Iterator<Item = (StableNodeId, &NodeRecord)> + '_ {
        self.paged
            .iter()
            .filter_map(|(index, record)| StableNodeId::new(index as u64).map(|id| (id, record)))
            .chain(self.overflow.iter().map(|(&id, record)| (id, record)))
    }
}

#[derive(Default)]
pub(crate) struct NodeStore {
    nodes: NodeTable,
    /// Single authoritative per-node geometry (`local`, `frame`, and optional published layout).
    pub(crate) geometry: DenseNodeMap<NodeGeometry>,
    published_layout_count: usize,
    /// One shared kind per element tag ([`Self::intern_kind`]).
    kinds: HashMap<String, Arc<NodeKind>>,
    /// The style every new node starts with, one for all of them.
    default_style: Arc<NodeStyle>,
    widget_visuals: crate::NodeMap<crate::WidgetVisualHandle>,
    /// Number of live nodes using an inline widget-visual preset (`visual_preset != 0`).
    preset_visual_count: usize,
    /// Nodes whose space ([`NodeRecord::space`]) pins accessibility
    /// ([`crate::node_space::NodeSpace::pins_accessibility`]): their
    /// accessible bounds, or those of the boxes under them, may not keep
    /// their offset from their parent's when a box above them moves.
    space_pins: crate::NodeSet,
    /// Nodes whose space is page-bound
    /// ([`crate::node_space::NodeSpace::page_bound`]): an open select,
    /// whose menu a layout pass gives a new room ([`crate::SelectMenuRoom`]),
    /// or a visual whose geometry reads the page. The few a move above them
    /// looks at again for extraction.
    page_bound: crate::NodeSet,
    custom_render: crate::NodeMap<CustomRenderNode>,
    event_listeners: crate::NodeMap<EventListeners>,
    component_types: crate::NodeMap<ComponentTypeId>,
    overlay_hosts: crate::NodeMap<OverlayHostState>,
    overlay_mounts: crate::NodeMap<nana_ui_core::OverlayMount>,
    scroll: crate::NodeMap<ScrollNodeState>,
    non_zero_scroll_offsets: usize,
    editor_states: crate::NodeMap<Box<EditorState>>,
    editor_count: usize,
    empty_state_text: crate::NodeMap<EmptyStateTextPresentation>,
    modal_text: crate::NodeMap<ModalTextPresentation>,
    /// Text drop indicators and free items' explicit rects.
    cold_rects: Box<ColdRects>,
    text_natural_widths: crate::NodeMap<f32>,
    block_struts: crate::NodeMap<crate::FontStrut>,
    block_x_heights: crate::NodeMap<f32>,
    inline_struts: crate::NodeMap<(crate::FontStrut, Option<f32>)>,
    text_wrap_limits: crate::NodeMap<f32>,
    languages: crate::NodeMap<nana_text::font::LanguageTag>,
    text_scales: crate::NodeMap<f32>,
    rich_texts: crate::NodeMap<nana_ui_core::RichText>,
    inline_contents: crate::NodeMap<crate::inline_content::InlineRecord>,
    inline_parents: crate::NodeMap<StableNodeId>,
    rich_editor_marks: crate::NodeMap<crate::RichEditorMarks>,
    glyph_presentations: crate::NodeMap<nana_ui_core::GlyphPresentation>,
    /// Cold optional text content; absent means empty unstamped text.
    texts: crate::NodeMap<TextContent>,
    /// Cold shaped text metrics; absent means `TextMetrics::default()`.
    text_metrics: crate::NodeMap<TextMetrics>,
    /// Distinct non-zero layout padding specs (`layout_padding_slot` 2..=254).
    distinct_layout_paddings: Vec<nana_ui_core::PaddingSpec>,
    /// Overflow layout paddings beyond `distinct_layout_paddings` capacity.
    layout_paddings: crate::NodeMap<nana_ui_core::PaddingSpec>,
    /// Bumped each time `layout_context` changes on a container (overflow beyond `u8::MAX - 1`).
    layout_context_generations: crate::NodeMap<u64>,
    /// Distinct text backend epochs observed on empty non-`Text` nodes (indexed by `empty_text_backend`).
    empty_text_backends: Vec<crate::text_node::TextBackendEpoch>,
    /// Non-preset accessibility states (`a11y_preset == 3`).
    accessibilities: crate::NodeMap<Arc<AccessibilityState>>,
    /// Number of live nodes using inline decorative-text preset (`a11y_preset == 2`).
    decorative_a11y_count: usize,
    /// Epoch a move above an open select field asked if its room stayed valid; absent means `u32::MAX`.
    render_room_epochs: crate::NodeMap<u32>,
    /// Revisions, resolution stamp and retained layout handle of every node's
    /// text (Issue #95), one entry per node.
    text_nodes: DenseNodeMap<crate::text_node::TextNodeState>,
    /// Layouts retained by plain text nodes, addressed by
    /// [`crate::text_node::TextNodeState::layout`]. Released with the node.
    text_layouts: nana_text::TextLayoutStore,
    /// Per-node theme category dependencies (#106), recomputed on node write/mount.
    theme_deps: DenseNodeMap<nana_ui_core::ThemeDependencyMask>,
    /// Sparse map of nodes that establish a local `ThemeScopeId` (#104).
    node_scopes: crate::NodeMap<nana_ui_core::ThemeScopeId>,
}

/// minimap 视口钉住：显式视口导航（minimap 点击/拖动）写入的滚动偏移。
/// 几何层在「钉住值 == 请求滚动」期间跳过光标 reveal（视口停在用户导航
/// 到的位置）；光标移动后由 shape 趟清除，reveal 恢复权威。
pub(crate) type TextViewportPin = ScrollOffset;

impl NodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `kind` as a record holds it: every element of one tag shares an
    /// `Arc`, so a page of a few tags holds a few kinds, not one per node.
    /// Past [`KINDS`] distinct tags, a new tag gets its own.
    /// The style a new node starts with, shared ([`NodeRecord::style`]).
    pub(crate) fn default_style(&self) -> Arc<NodeStyle> {
        Arc::clone(&self.default_style)
    }

    pub(crate) fn intern_kind(&mut self, kind: &NodeKind) -> Arc<NodeKind> {
        let NodeKind::Element { tag } = kind else {
            return intern_kind(kind);
        };
        if let Some(kept) = self.kinds.get(tag.as_str()) {
            return Arc::clone(kept);
        }
        let kept = intern_kind(kind);
        if self.kinds.len() < KINDS {
            self.kinds.insert(tag.clone(), Arc::clone(&kept));
        }
        kept
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 0
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.nodes.contains(id)
    }

    pub fn get(&self, id: StableNodeId) -> Option<&NodeRecord> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        self.nodes.get_mut(id)
    }

    #[inline]
    pub(crate) fn local_layout(&self, id: StableNodeId) -> LayoutBox {
        self.geometry
            .get(&id)
            .map_or_else(LayoutBox::default, |g| g.local)
    }

    #[inline]
    pub(crate) fn layout_frame(&self, id: StableNodeId) -> crate::LayoutFrame {
        self.geometry
            .get(&id)
            .map_or(crate::LayoutFrame::Parent, NodeGeometry::frame)
    }

    #[inline]
    pub(crate) fn geometry_entry(&self, id: StableNodeId) -> Option<&NodeGeometry> {
        self.geometry.get(&id)
    }

    #[inline]
    pub(crate) fn geometry_entry_mut(&mut self, id: StableNodeId) -> &mut NodeGeometry {
        if self.geometry.get(&id).is_none() {
            self.geometry.insert(id, NodeGeometry::default());
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::GEOMETRY, true);
            }
        }
        self.geometry.get_mut(&id).expect("inserted geometry entry")
    }

    #[inline]
    pub(crate) fn published_layout(&self, id: StableNodeId) -> Option<&PublishedLayoutMeta> {
        self.geometry
            .get(&id)
            .and_then(|g| g.published.as_ref())
            .filter(|meta| !meta.suppressed)
    }

    #[inline]
    pub(crate) fn has_published_layouts(&self) -> bool {
        self.published_layout_count > 0
    }

    pub(crate) fn publish_layout(&mut self, id: StableNodeId, meta: PublishedLayoutMeta) {
        let entry = self.geometry_entry_mut(id);
        let was_none = entry.published.is_none();
        entry.published = Some(meta);
        if was_none {
            self.published_layout_count += 1;
        }
    }

    pub(crate) fn clear_published_layout(&mut self, id: StableNodeId) {
        if let Some(entry) = self.geometry.get_mut(&id)
            && entry.published.take().is_some()
        {
            self.published_layout_count -= 1;
        }
    }

    pub(crate) fn suppress_published_layout(&mut self, id: StableNodeId) {
        if let Some(meta) = self
            .geometry
            .get_mut(&id)
            .and_then(|g| g.published.as_mut())
        {
            meta.suppressed = true;
        }
    }

    pub(crate) fn unsuppress_published_layout(&mut self, id: StableNodeId) {
        if let Some(meta) = self
            .geometry
            .get_mut(&id)
            .and_then(|g| g.published.as_mut())
        {
            meta.suppressed = false;
        }
    }

    pub(crate) fn reserve_geometry_through(&mut self, highest: StableNodeId, entries: usize) {
        self.geometry.reserve_through(highest, entries);
    }

    pub fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.nodes.keys()
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &NodeRecord> + '_ {
        self.nodes.records()
    }

    /// 持有 hover 浮窗状态的节点（稀疏表直接迭代，命中测试路由用）。
    pub fn text_hover_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.editor_states
            .iter()
            .filter_map(|(id, state)| state.hover.as_ref().map(|_| *id))
    }

    pub fn insert(&mut self, id: StableNodeId, record: NodeRecord) {
        self.nodes.insert(id, record);
        self.note_node_space(id);
    }

    /// Bring `id`'s space ([`NodeRecord::space`]) up to date with its
    /// record and visual, and the indexes of it ([`Self::space_pins`],
    /// [`Self::page_bound`]). Called wherever a field that decides it is
    /// written: style resolution, the layout frame, the visual, an insert.
    pub(crate) fn note_node_space(&mut self, id: StableNodeId) {
        let visual = self.widget_visual(id);
        let frame = self.layout_frame(id);
        let Some(record) = self.nodes.get(id) else {
            return;
        };
        let space =
            crate::node_space::NodeSpace::of(&record.resolved_layout, frame, &record.kind, visual);
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        record.space = space;
        if space.pins_accessibility() {
            self.space_pins.insert(id);
            record.attachments.set(NodePresenceBits::SPACE_PIN, true);
        } else {
            self.space_pins.remove(&id);
            record.attachments.clear(NodePresenceBits::SPACE_PIN);
        }
        if space.page_bound() {
            self.page_bound.insert(id);
            record.attachments.set(NodePresenceBits::PAGE_BOUND, true);
        } else {
            self.page_bound.remove(&id);
            record.attachments.clear(NodePresenceBits::PAGE_BOUND);
        }
    }

    /// Every node whose space pins accessibility
    /// ([`crate::node_space::NodeSpace::pins_accessibility`]), in no
    /// particular order.
    pub(crate) fn space_pins(&self) -> impl ExactSizeIterator<Item = StableNodeId> + '_ {
        self.space_pins.iter().copied()
    }

    /// Every node whose space is page-bound
    /// ([`crate::node_space::NodeSpace::page_bound`]), in no particular
    /// order.
    pub(crate) fn page_bound(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.page_bound.iter().copied()
    }

    /// Whether any node is page-bound ([`Self::page_bound`]).
    pub(crate) fn has_page_bound(&self) -> bool {
        !self.page_bound.is_empty()
    }

    /// Nodes whose visual is an open Select, Dropdown or SearchDropdown
    /// menu ([`crate::node_space::NodeSpace::MENU_ROOM`]), in no particular
    /// order.
    pub(crate) fn open_select_menus(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.page_bound().filter(|&id| {
            self.nodes.get(id).is_some_and(|record| {
                record
                    .space
                    .contains(crate::node_space::NodeSpace::MENU_ROOM)
            })
        })
    }

    /// The space of every node is what its record and visual give, and the
    /// indexes hold exactly the nodes they should. Small worlds only.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(crate) fn check_node_spaces(&self) {
        if self.len() > 4096 {
            return;
        }
        let (mut pins, mut bound) = (0, 0);
        for id in self.keys() {
            let record = self.nodes.get(id).expect("a key has a record");
            let space = crate::node_space::NodeSpace::of(
                &record.resolved_layout,
                self.layout_frame(id),
                &record.kind,
                self.widget_visual(id),
            );
            assert_eq!(record.space, space, "{id:?}: its space is stale");
            assert_eq!(
                space.pins_accessibility(),
                self.space_pins.contains(&id),
                "{id:?}: the pins disagree with its space {space:?}"
            );
            assert_eq!(
                space.page_bound(),
                self.page_bound.contains(&id),
                "{id:?}: the page-bound nodes disagree with its space {space:?}"
            );
            pins += usize::from(space.pins_accessibility());
            bound += usize::from(space.page_bound());
        }
        assert_eq!(pins, self.space_pins.len(), "pins of nodes that are gone");
        assert_eq!(
            bound,
            self.page_bound.len(),
            "page-bound nodes that are gone"
        );
    }

    /// Drop the dense record and every sparse entry. The only despawn path.
    pub fn remove(&mut self, id: StableNodeId) -> Option<NodeRecord> {
        let mut record = self.nodes.remove(id)?;
        let mut bits = record.attachments;
        if record.a11y_preset == 2 {
            self.decorative_a11y_count -= 1;
        }
        if record.visual_preset != 0 {
            self.preset_visual_count -= 1;
        }
        if bits.contains(NodePresenceBits::GEOMETRY) {
            if let Some(geom) = self.geometry.remove(&id)
                && geom.published.is_some()
            {
                self.published_layout_count -= 1;
            }
            bits.clear(NodePresenceBits::GEOMETRY);
        }
        if !bits.is_empty() {
            if bits.contains(NodePresenceBits::WIDGET_VISUAL) {
                self.widget_visuals.remove(&id);
                bits.clear(NodePresenceBits::WIDGET_VISUAL);
            }
            if bits.contains(NodePresenceBits::SPACE_PIN) {
                self.space_pins.remove(&id);
                bits.clear(NodePresenceBits::SPACE_PIN);
            }
            if bits.contains(NodePresenceBits::PAGE_BOUND) {
                self.page_bound.remove(&id);
                bits.clear(NodePresenceBits::PAGE_BOUND);
            }
            if bits.contains(NodePresenceBits::CUSTOM_RENDER) {
                self.custom_render.remove(&id);
                bits.clear(NodePresenceBits::CUSTOM_RENDER);
            }
            if bits.contains(NodePresenceBits::EVENT_LISTENERS) {
                self.event_listeners.remove(&id);
                bits.clear(NodePresenceBits::EVENT_LISTENERS);
            }
            if bits.contains(NodePresenceBits::COMPONENT_TYPE) {
                self.component_types.remove(&id);
                bits.clear(NodePresenceBits::COMPONENT_TYPE);
            }
            if bits.contains(NodePresenceBits::OVERLAY_HOST) {
                self.overlay_hosts.remove(&id);
                bits.clear(NodePresenceBits::OVERLAY_HOST);
            }
            if bits.contains(NodePresenceBits::OVERLAY_MOUNT) {
                self.overlay_mounts.remove(&id);
                bits.clear(NodePresenceBits::OVERLAY_MOUNT);
            }
            if bits.contains(NodePresenceBits::SCROLL) {
                if let Some(prev) = self.scroll.remove(&id)
                    && prev.offset != ScrollOffset::default()
                {
                    self.non_zero_scroll_offsets -= 1;
                }
                bits.clear(NodePresenceBits::SCROLL);
            }
            if bits.contains(NodePresenceBits::EDITOR) {
                if let Some(prev) = self.editor_states.remove(&id)
                    && prev.editor.is_some()
                {
                    self.editor_count -= 1;
                }
                bits.clear(NodePresenceBits::EDITOR);
            }
            if bits.contains(NodePresenceBits::EMPTY_STATE_TEXT) {
                self.empty_state_text.remove(&id);
                bits.clear(NodePresenceBits::EMPTY_STATE_TEXT);
            }
            if bits.contains(NodePresenceBits::MODAL_TEXT) {
                self.modal_text.remove(&id);
                bits.clear(NodePresenceBits::MODAL_TEXT);
            }
            if bits.contains(NodePresenceBits::TEXT_DROP_INDICATOR) {
                self.cold_rects.text_drop_indicators.remove(&id);
                bits.clear(NodePresenceBits::TEXT_DROP_INDICATOR);
            }
            if bits.contains(NodePresenceBits::TEXT_NATURAL_WIDTH) {
                self.text_natural_widths.remove(&id);
                bits.clear(NodePresenceBits::TEXT_NATURAL_WIDTH);
            }
            if bits.contains(NodePresenceBits::BLOCK_STRUT) {
                self.block_struts.remove(&id);
                self.block_x_heights.remove(&id);
                bits.clear(NodePresenceBits::BLOCK_STRUT);
            }
            if bits.contains(NodePresenceBits::TEXT_WRAP_LIMIT) {
                self.text_wrap_limits.remove(&id);
                bits.clear(NodePresenceBits::TEXT_WRAP_LIMIT);
            }
            if bits.contains(NodePresenceBits::EXPLICIT_RECT) {
                self.cold_rects.explicit_rects.remove(&id);
                bits.clear(NodePresenceBits::EXPLICIT_RECT);
            }
            if bits.contains(NodePresenceBits::LANGUAGE) {
                self.languages.remove(&id);
                bits.clear(NodePresenceBits::LANGUAGE);
            }
            if bits.contains(NodePresenceBits::TEXT_SCALE) {
                self.text_scales.remove(&id);
                bits.clear(NodePresenceBits::TEXT_SCALE);
            }
            if bits.contains(NodePresenceBits::RICH_TEXT) {
                self.rich_texts.remove(&id);
                bits.clear(NodePresenceBits::RICH_TEXT);
            }
            if bits.contains(NodePresenceBits::INLINE_CONTENT) {
                self.inline_struts.remove(&id);
                bits.clear(NodePresenceBits::INLINE_CONTENT);
            }
            if bits.contains(NodePresenceBits::INLINE_PARENT) {
                self.inline_parents.remove(&id);
                bits.clear(NodePresenceBits::INLINE_PARENT);
            }
            if bits.contains(NodePresenceBits::RICH_EDITOR_MARKS) {
                self.rich_editor_marks.remove(&id);
                bits.clear(NodePresenceBits::RICH_EDITOR_MARKS);
            }
            if bits.contains(NodePresenceBits::GLYPH_PRESENTATION) {
                self.glyph_presentations.remove(&id);
                bits.clear(NodePresenceBits::GLYPH_PRESENTATION);
            }
            if bits.contains(NodePresenceBits::TEXT_CONTENT) {
                self.texts.remove(&id);
                bits.clear(NodePresenceBits::TEXT_CONTENT);
            }
            if bits.contains(NodePresenceBits::TEXT_METRICS) {
                self.text_metrics.remove(&id);
                bits.clear(NodePresenceBits::TEXT_METRICS);
            }
            if bits.contains(NodePresenceBits::ACCESSIBILITY) {
                self.accessibilities.remove(&id);
                bits.clear(NodePresenceBits::ACCESSIBILITY);
            }
            if bits.contains(NodePresenceBits::RENDER_ROOM_EPOCH) {
                self.render_room_epochs.remove(&id);
                bits.clear(NodePresenceBits::RENDER_ROOM_EPOCH);
            }
            if bits.contains(NodePresenceBits::THEME_DEP) {
                self.theme_deps.remove(&id);
                bits.clear(NodePresenceBits::THEME_DEP);
            }
            if bits.contains(NodePresenceBits::NODE_SCOPE) {
                self.node_scopes.remove(&id);
                bits.clear(NodePresenceBits::NODE_SCOPE);
            }
            if bits.contains(NodePresenceBits::TEXT_NODE) {
                if let Some(text) = self.text_nodes.remove(&id) {
                    self.text_layouts.remove(text.layout);
                }
                bits.clear(NodePresenceBits::TEXT_NODE);
            }
            debug_assert!(
                bits.is_empty(),
                "NodePresenceBits leaked on {id:?}: {bits:?}"
            );
        }
        if record
            .attachments
            .contains(NodePresenceBits::INLINE_CONTENT)
        {
            self.set_inline_atoms(id, Arc::from([]));
        }
        if record.layout_padding_slot == u8::MAX {
            self.layout_paddings.remove(&id);
        }
        if record.layout_context_gen == u8::MAX {
            self.layout_context_generations.remove(&id);
        }
        record.attachments = NodePresenceBits::EMPTY;
        Some(record)
    }

    pub(crate) fn theme_dependency(&self, id: StableNodeId) -> nana_ui_core::ThemeDependencyMask {
        self.theme_deps
            .get(&id)
            .copied()
            .unwrap_or(nana_ui_core::ThemeDependencyMask::NONE)
    }

    pub(crate) fn set_theme_dependency(
        &mut self,
        id: StableNodeId,
        mask: nana_ui_core::ThemeDependencyMask,
    ) {
        if mask.is_empty() {
            self.theme_deps.remove(&id);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.clear(NodePresenceBits::THEME_DEP);
            }
        } else {
            self.theme_deps.insert(id, mask);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::THEME_DEP, true);
            }
        }
    }

    pub(crate) fn theme_dependency_entries(
        &self,
    ) -> impl Iterator<Item = (StableNodeId, nana_ui_core::ThemeDependencyMask)> + '_ {
        self.theme_deps.iter().map(|(id, mask)| (id, *mask))
    }

    pub(crate) fn node_scope(&self, id: StableNodeId) -> nana_ui_core::ThemeScopeId {
        self.node_scopes
            .get(&id)
            .copied()
            .unwrap_or(nana_ui_core::ThemeScopeId::ROOT)
    }

    pub(crate) fn set_node_scope(&mut self, id: StableNodeId, scope: nana_ui_core::ThemeScopeId) {
        if !scope.is_root() {
            self.node_scopes.insert(id, scope);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::NODE_SCOPE, true);
            }
        } else if !self.node_scopes.is_empty() {
            self.node_scopes.remove(&id);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.clear(NodePresenceBits::NODE_SCOPE);
            }
        }
    }

    pub(crate) fn has_text_inputs(&self) -> bool {
        self.editor_count > 0
    }

    pub(crate) fn has_custom_render(&self) -> bool {
        !self.custom_render.is_empty()
    }

    pub(crate) fn has_widget_visuals(&self) -> bool {
        self.preset_visual_count > 0 || !self.widget_visuals.is_empty()
    }

    pub(crate) fn text_content(&self, id: StableNodeId) -> &TextContent {
        if self.texts.is_empty() {
            return &DEFAULT_TEXT_CONTENT;
        }
        self.texts.get(&id).unwrap_or(&DEFAULT_TEXT_CONTENT)
    }

    pub(crate) fn set_text_content(&mut self, id: StableNodeId, text: TextContent) {
        let materialize = if let Some(record) = self.nodes.get(id)
            && record.a11y_preset >= 4
        {
            let current = self.text_content(id);
            (current.value.as_str() != text.value.as_str())
                .then(|| (record.a11y_preset, current.value.to_shared_str()))
        } else {
            None
        };
        if let Some((preset, old_label)) = materialize {
            self.accessibilities.insert(
                id,
                Arc::new(preset_control_accessibility(preset, old_label)),
            );
            if let Some(record) = self.nodes.get_mut(id) {
                record.a11y_preset = 3;
                record
                    .attachments
                    .set(NodePresenceBits::ACCESSIBILITY, true);
            }
        }
        let non_empty = !text.value.is_empty();
        let present = non_empty || text.value.stamp().is_some();
        if let Some(record) = self.nodes.get_mut(id) {
            record.has_text = non_empty;
            if non_empty {
                record.empty_text_backend = 0;
            }
            record
                .attachments
                .set(NodePresenceBits::TEXT_CONTENT, present);
        }
        if present {
            self.texts.insert(id, text);
        } else {
            self.texts.remove(&id);
        }
    }

    pub(crate) fn text_metrics(&self, id: StableNodeId) -> TextMetrics {
        if self.text_metrics.is_empty() {
            return TextMetrics::default();
        }
        self.text_metrics.get(&id).copied().unwrap_or_default()
    }

    pub(crate) fn set_text_metrics(&mut self, id: StableNodeId, metrics: TextMetrics) {
        let present = metrics != TextMetrics::default();
        if present {
            self.text_metrics.insert(id, metrics);
        } else {
            self.text_metrics.remove(&id);
        }
        if let Some(record) = self.nodes.get_mut(id) {
            record
                .attachments
                .set(NodePresenceBits::TEXT_METRICS, present);
        }
    }

    pub(crate) fn layout_padding(&self, id: StableNodeId) -> Option<nana_ui_core::PaddingSpec> {
        let record = self.nodes.get(id)?;
        match record.layout_padding_slot {
            0 => None,
            1 => Some(nana_ui_core::PaddingSpec::default()),
            slot if slot < u8::MAX => self
                .distinct_layout_paddings
                .get(usize::from(slot - 2))
                .copied(),
            _ => self.layout_paddings.get(&id).copied(),
        }
    }

    pub(crate) fn set_layout_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
    ) {
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let previous_slot = record.layout_padding_slot;
        if padding == nana_ui_core::PaddingSpec::default() {
            record.layout_padding_slot = 1;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
            return;
        }
        if let Some(index) = self
            .distinct_layout_paddings
            .iter()
            .position(|existing| *existing == padding)
        {
            record.layout_padding_slot = (index + 2) as u8;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
            return;
        }
        if self.distinct_layout_paddings.len() < usize::from(u8::MAX) - 2 {
            self.distinct_layout_paddings.push(padding);
            record.layout_padding_slot = (self.distinct_layout_paddings.len() + 1) as u8;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
        } else {
            record.layout_padding_slot = u8::MAX;
            self.layout_paddings.insert(id, padding);
        }
    }

    pub(crate) fn layout_far_start(&self, id: StableNodeId) -> Option<[bool; 2]> {
        self.nodes.get(id)?.layout_far_start.to_option()
    }

    pub(crate) fn set_layout_far_start(&mut self, id: StableNodeId, far: [bool; 2]) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.layout_far_start = LayoutFarStart::from_pair(far);
        }
    }

    #[inline]
    pub(crate) fn auto_view_slot(&self, id: StableNodeId) -> Option<u16> {
        let slot = self.nodes.get(id)?.auto_view_slot;
        (slot != u16::MAX).then_some(slot)
    }

    #[inline]
    pub(crate) fn set_auto_view_slot(&mut self, id: StableNodeId, slot: Option<u16>) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.auto_view_slot = slot.unwrap_or(u16::MAX);
        }
    }

    pub(crate) fn layout_context_generation(&self, id: StableNodeId) -> u64 {
        let Some(record) = self.nodes.get(id) else {
            return 0;
        };
        if record.layout_context_gen < u8::MAX {
            return u64::from(record.layout_context_gen);
        }
        if self.layout_context_generations.is_empty() {
            return u64::from(u8::MAX);
        }
        self.layout_context_generations
            .get(&id)
            .copied()
            .unwrap_or(u64::from(u8::MAX))
    }

    pub(crate) fn bump_layout_context_generation(&mut self, id: StableNodeId) {
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        if record.layout_context_gen < u8::MAX - 1 {
            record.layout_context_gen += 1;
            return;
        }
        if record.layout_context_gen == u8::MAX - 1 {
            record.layout_context_gen = u8::MAX;
            self.layout_context_generations
                .insert(id, u64::from(u8::MAX));
            return;
        }
        let entry = self
            .layout_context_generations
            .entry(id)
            .or_insert(u64::from(u8::MAX));
        *entry = entry.wrapping_add(1);
    }

    pub(crate) fn has_scroll_offsets(&self) -> bool {
        self.non_zero_scroll_offsets > 0
    }

    pub(crate) fn scroll_offset(&self, id: StableNodeId) -> ScrollOffset {
        if self.non_zero_scroll_offsets == 0 {
            return ScrollOffset::default();
        }
        self.scroll
            .get(&id)
            .map_or_else(ScrollOffset::default, |state| state.offset)
    }

    pub(crate) fn set_scroll_offset(&mut self, id: StableNodeId, offset: ScrollOffset) {
        let is_zero = offset == ScrollOffset::default();
        let had_non_zero = self
            .scroll
            .get(&id)
            .is_some_and(|state| state.offset != ScrollOffset::default());
        match (had_non_zero, is_zero) {
            (false, false) => self.non_zero_scroll_offsets += 1,
            (true, true) => self.non_zero_scroll_offsets -= 1,
            _ => {}
        }
        let remove_entry = if let Some(state) = self.scroll.get_mut(&id) {
            state.offset = offset;
            is_zero && state.metrics.is_none()
        } else if !is_zero {
            self.scroll.insert(
                id,
                ScrollNodeState {
                    offset,
                    metrics: None,
                },
            );
            false
        } else {
            false
        };
        if remove_entry {
            self.scroll.remove(&id);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.clear(NodePresenceBits::SCROLL);
            }
        } else if !is_zero || self.scroll.contains_key(&id) {
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::SCROLL, true);
            }
        }
    }

    pub fn scroll_metrics(&self, id: StableNodeId) -> Option<&ScrollMetrics> {
        self.scroll
            .get(&id)
            .and_then(|state| state.metrics.as_ref())
    }

    pub fn set_scroll_metrics(&mut self, id: StableNodeId, metrics: Option<ScrollMetrics>) {
        if let Some(metrics) = metrics {
            let entry = self.scroll.entry(id).or_default();
            entry.metrics = Some(metrics);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::SCROLL, true);
            }
        } else if let Some(state) = self.scroll.get_mut(&id) {
            state.metrics = None;
            if state.offset == ScrollOffset::default() {
                self.scroll.remove(&id);
                if let Some(record) = self.nodes.get_mut(id) {
                    record.attachments.clear(NodePresenceBits::SCROLL);
                }
            }
        }
    }

    #[inline]
    pub(crate) fn elided_stack_flags(&self, id: StableNodeId) -> u8 {
        self.nodes
            .get(id)
            .map_or(0, |record| record.elided_stack_flags)
    }

    #[inline]
    pub(crate) fn set_elided_stack_flags(&mut self, id: StableNodeId, flags: u8) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.elided_stack_flags = flags;
        }
    }

    pub(crate) fn reserve_nodes_through(&mut self, highest: StableNodeId, entries: usize) {
        self.nodes.reserve_through(highest, entries);
    }

    pub(crate) fn reserve_text_nodes_through(&mut self, highest: StableNodeId, entries: usize) {
        self.text_nodes.reserve_through(highest, entries);
    }

    pub(crate) fn text_nodes_len(&self) -> usize {
        self.text_nodes.len()
    }

    pub(crate) fn any_hidden_accessibility(&self) -> bool {
        self.decorative_a11y_count > 0 || self.accessibilities.values().any(|state| state.hidden)
    }

    pub(crate) fn accessibility(
        &self,
        id: StableNodeId,
    ) -> std::borrow::Cow<'_, AccessibilityState> {
        use std::borrow::Cow;
        if let Some(record) = self.nodes.get(id) {
            return match record.a11y_preset {
                0 => Cow::Borrowed(DEFAULT_ACCESSIBILITY.as_ref()),
                1 => Cow::Borrowed(TEXT_ACCESSIBILITY.as_ref()),
                2 => Cow::Borrowed(DECORATIVE_TEXT_ACCESSIBILITY.as_ref()),
                preset @ 4..=6 => Cow::Owned(preset_control_accessibility(
                    preset,
                    self.text_content(id).value.to_shared_str(),
                )),
                _ => Cow::Borrowed(
                    self.accessibilities
                        .get(&id)
                        .unwrap_or(&DEFAULT_ACCESSIBILITY)
                        .as_ref(),
                ),
            };
        }
        if self.accessibilities.is_empty() {
            return Cow::Borrowed(DEFAULT_ACCESSIBILITY.as_ref());
        }
        Cow::Borrowed(
            self.accessibilities
                .get(&id)
                .unwrap_or(&DEFAULT_ACCESSIBILITY)
                .as_ref(),
        )
    }

    pub(crate) fn accessibility_eq(&self, id: StableNodeId, expected: &AccessibilityState) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        match record.a11y_preset {
            0 => *expected == **DEFAULT_ACCESSIBILITY,
            1 => *expected == **TEXT_ACCESSIBILITY,
            2 => *expected == **DECORATIVE_TEXT_ACCESSIBILITY,
            preset @ 4..=6 => {
                matching_control_preset(expected, self.text_content(id).value.as_str())
                    == Some(preset)
            }
            _ => {
                *expected
                    == **self
                        .accessibilities
                        .get(&id)
                        .unwrap_or(&DEFAULT_ACCESSIBILITY)
            }
        }
    }

    pub(crate) fn accessible_label_arc(&self, id: StableNodeId) -> Option<Arc<str>> {
        let record = self.nodes.get(id)?;
        match record.a11y_preset {
            0..=2 => None,
            4..=6 => Some(self.text_content(id).value.to_shared_str()),
            _ => self
                .accessibilities
                .get(&id)
                .and_then(|state| state.label.as_ref().map(Arc::clone)),
        }
    }

    pub(crate) fn set_accessibility(&mut self, id: StableNodeId, state: &AccessibilityState) {
        let preset = if *state == **DEFAULT_ACCESSIBILITY {
            0
        } else if *state == **TEXT_ACCESSIBILITY {
            1
        } else if *state == **DECORATIVE_TEXT_ACCESSIBILITY {
            2
        } else if self.nodes.get(id).is_some_and(|record| record.has_text)
            && let Some(control_preset) =
                matching_control_preset(state, self.text_content(id).value.as_str())
        {
            control_preset
        } else {
            3
        };
        if let Some(record) = self.nodes.get_mut(id) {
            if record.a11y_preset == 2 && preset != 2 {
                self.decorative_a11y_count -= 1;
            } else if record.a11y_preset != 2 && preset == 2 {
                self.decorative_a11y_count += 1;
            }
            record.a11y_preset = preset;
            record
                .attachments
                .set(NodePresenceBits::ACCESSIBILITY, preset == 3);
        }
        if preset == 3 {
            self.accessibilities.insert(id, shared_accessibility(state));
        } else {
            self.accessibilities.remove(&id);
        }
    }

    pub(crate) fn render_room_epoch(&self, id: StableNodeId) -> u32 {
        if self.render_room_epochs.is_empty() {
            return u32::MAX;
        }
        self.render_room_epochs
            .get(&id)
            .copied()
            .unwrap_or(u32::MAX)
    }

    pub(crate) fn set_render_room_epoch(&mut self, id: StableNodeId, epoch: u32) {
        if epoch == u32::MAX {
            self.render_room_epochs.remove(&id);
            if let Some(record) = self.nodes.get_mut(id) {
                record
                    .attachments
                    .clear(NodePresenceBits::RENDER_ROOM_EPOCH);
            }
        } else {
            self.render_room_epochs.insert(id, epoch);
            if let Some(record) = self.nodes.get_mut(id) {
                record
                    .attachments
                    .set(NodePresenceBits::RENDER_ROOM_EPOCH, true);
            }
        }
    }

    pub fn widget_visual(&self, id: StableNodeId) -> Option<&crate::WidgetVisualHandle> {
        if self
            .nodes
            .get(id)
            .is_some_and(|record| record.visual_preset == 1)
        {
            return Some(&EMPTY_LIST_ITEM_VISUAL);
        }
        self.widget_visuals.get(&id)
    }

    pub fn set_widget_visual(
        &mut self,
        id: StableNodeId,
        value: Option<crate::WidgetVisualHandle>,
    ) {
        let text_key_changed = self.widget_visual(id).and_then(|v| v.text_visual_key())
            != value.as_ref().and_then(|v| v.text_visual_key());
        let is_empty_list_item_preset = value
            .as_ref()
            .is_some_and(|handle| handle.ptr_eq(&EMPTY_LIST_ITEM_VISUAL));
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let prev_preset = record.visual_preset;
        if is_empty_list_item_preset {
            record.visual_preset = 1;
            if prev_preset == 0 {
                self.preset_visual_count += 1;
            }
            self.widget_visuals.remove(&id);
            record.attachments.clear(NodePresenceBits::WIDGET_VISUAL);
        } else {
            if prev_preset != 0 {
                record.visual_preset = 0;
                self.preset_visual_count -= 1;
            }
            match value {
                Some(value) => {
                    self.widget_visuals.insert(id, value);
                    record
                        .attachments
                        .set(NodePresenceBits::WIDGET_VISUAL, true);
                }
                None => {
                    self.widget_visuals.remove(&id);
                    record.attachments.clear(NodePresenceBits::WIDGET_VISUAL);
                }
            }
        }
        if text_key_changed {
            self.invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
        }
        self.note_node_space(id);
    }
    sparse!(
        custom_render,
        CustomRenderNode,
        custom_render,
        set_custom_render,
        NodePresenceBits::CUSTOM_RENDER
    );
    sparse!(
        event_listeners,
        EventListeners,
        event_listeners,
        set_event_listeners,
        NodePresenceBits::EVENT_LISTENERS
    );
    pub fn component_type(&self, id: StableNodeId) -> Option<&ComponentTypeId> {
        let record = self.nodes.get(id)?;
        if record.builtin_component != 0 {
            return crate::component_descriptors::builtin_component_type(record.builtin_component);
        }
        self.component_types.get(&id)
    }

    pub fn set_component_type(&mut self, id: StableNodeId, value: Option<ComponentTypeId>) {
        match value {
            Some(value) => {
                if let Some(index) =
                    crate::component_descriptors::builtin_component_index(value.as_str())
                    && let Some(record) = self.nodes.get_mut(id)
                {
                    record.builtin_component = index;
                    record.attachments.clear(NodePresenceBits::COMPONENT_TYPE);
                    self.component_types.remove(&id);
                    return;
                }
                if let Some(record) = self.nodes.get_mut(id) {
                    record.builtin_component = 0;
                    record
                        .attachments
                        .set(NodePresenceBits::COMPONENT_TYPE, true);
                }
                self.component_types.insert(id, value);
            }
            None => {
                if let Some(record) = self.nodes.get_mut(id) {
                    record.builtin_component = 0;
                    record.attachments.clear(NodePresenceBits::COMPONENT_TYPE);
                }
                self.component_types.remove(&id);
            }
        }
    }
    sparse!(
        overlay_hosts,
        OverlayHostState,
        overlay_host,
        set_overlay_host,
        NodePresenceBits::OVERLAY_HOST
    );
    pub(crate) fn has_overlay_mounts(&self) -> bool {
        !self.overlay_mounts.is_empty()
    }

    sparse!(
        overlay_mounts,
        nana_ui_core::OverlayMount,
        overlay_mount,
        set_overlay_mount,
        NodePresenceBits::OVERLAY_MOUNT
    );

    fn ensure_editor_state(&mut self, id: StableNodeId) -> &mut EditorState {
        if !self.editor_states.contains_key(&id) {
            self.editor_states.insert(id, Box::default());
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.set(NodePresenceBits::EDITOR, true);
            }
        }
        self.editor_states
            .get_mut(&id)
            .expect("inserted editor state")
    }

    fn prune_editor_state_if_empty(&mut self, id: StableNodeId) {
        if self
            .editor_states
            .get(&id)
            .is_some_and(|state| state.is_empty())
        {
            self.editor_states.remove(&id);
            if let Some(record) = self.nodes.get_mut(id) {
                record.attachments.clear(NodePresenceBits::EDITOR);
            }
        }
    }

    pub fn text_input(&self, id: StableNodeId) -> Option<TextInputView<'_>> {
        self.editor(id)
            .map(|editor| TextInputView::of(&editor.session))
    }

    pub fn ime(&self, id: StableNodeId) -> Option<ImeView<'_>> {
        self.editor(id)?.ime()
    }

    pub(crate) fn editor(&self, id: StableNodeId) -> Option<&EditorRecord> {
        self.editor_states
            .get(&id)
            .and_then(|state| state.editor.as_ref())
    }

    pub(crate) fn editor_mut(&mut self, id: StableNodeId) -> Option<&mut EditorRecord> {
        self.editor_states
            .get_mut(&id)
            .and_then(|state| state.editor.as_mut())
    }

    pub(crate) fn set_editor(&mut self, id: StableNodeId, editor: Option<EditorRecord>) {
        match editor {
            Some(editor) => {
                let inserted = {
                    let state = self.ensure_editor_state(id);
                    let was_none = state.editor.is_none();
                    state.editor = Some(editor);
                    was_none
                };
                if inserted {
                    self.editor_count += 1;
                }
            }
            None => {
                if let Some(state) = self.editor_states.get_mut(&id)
                    && state.editor.take().is_some()
                {
                    self.editor_count -= 1;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }

    pub fn highlight(&self, id: StableNodeId) -> Option<&HighlightRequest> {
        self.editor_states.get(&id)?.highlight.as_ref()
    }
    pub fn set_highlight(&mut self, id: StableNodeId, value: Option<HighlightRequest>) {
        match value {
            Some(value) => self.ensure_editor_state(id).highlight = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.highlight = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_presentation(&self, id: StableNodeId) -> Option<&TextPresentation> {
        self.editor_states.get(&id)?.text_presentation.as_ref()
    }
    pub fn set_text_presentation(&mut self, id: StableNodeId, value: Option<TextPresentation>) {
        match value {
            Some(value) => self.ensure_editor_state(id).text_presentation = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.text_presentation = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_input_presentation(&self, id: StableNodeId) -> Option<&TextInputPresentation> {
        self.editor_states
            .get(&id)?
            .text_input_presentation
            .as_ref()
    }
    pub fn set_text_input_presentation(
        &mut self,
        id: StableNodeId,
        value: Option<TextInputPresentation>,
    ) {
        match value {
            Some(value) => self.ensure_editor_state(id).text_input_presentation = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.text_input_presentation = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    sparse!(
        empty_state_text,
        EmptyStateTextPresentation,
        empty_state_text,
        set_empty_state_text,
        NodePresenceBits::EMPTY_STATE_TEXT
    );
    sparse!(
        modal_text,
        ModalTextPresentation,
        modal_text,
        set_modal_text,
        NodePresenceBits::MODAL_TEXT
    );
    pub fn text_fold_view(&self, id: StableNodeId) -> Option<&TextFoldViewState> {
        self.editor_states.get(&id)?.fold_view.as_ref()
    }
    pub fn set_text_fold_view(&mut self, id: StableNodeId, value: Option<TextFoldViewState>) {
        match value {
            Some(value) => self.ensure_editor_state(id).fold_view = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.fold_view = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_inlays(&self, id: StableNodeId) -> Option<&Arc<[crate::TextInlay]>> {
        self.editor_states.get(&id)?.inlays.as_ref()
    }
    pub fn set_text_inlays(&mut self, id: StableNodeId, value: Option<Arc<[crate::TextInlay]>>) {
        match value {
            Some(value) => self.ensure_editor_state(id).inlays = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.inlays = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_snippet_session(&self, id: StableNodeId) -> Option<&TextSnippetSession> {
        self.editor_states.get(&id)?.snippet.as_ref()
    }
    pub fn set_text_snippet_session(
        &mut self,
        id: StableNodeId,
        value: Option<TextSnippetSession>,
    ) {
        match value {
            Some(value) => self.ensure_editor_state(id).snippet = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.snippet = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_completion_view(&self, id: StableNodeId) -> Option<&TextCompletionViewState> {
        self.editor_states.get(&id)?.completion.as_ref()
    }
    pub fn set_text_completion_view(
        &mut self,
        id: StableNodeId,
        value: Option<TextCompletionViewState>,
    ) {
        match value {
            Some(value) => self.ensure_editor_state(id).completion = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.completion = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_hover_view(&self, id: StableNodeId) -> Option<&TextHoverViewState> {
        self.editor_states.get(&id)?.hover.as_ref()
    }
    pub fn set_text_hover_view(&mut self, id: StableNodeId, value: Option<TextHoverViewState>) {
        match value {
            Some(value) => self.ensure_editor_state(id).hover = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.hover = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_signature(&self, id: StableNodeId) -> Option<&crate::TextSignatureHelp> {
        self.editor_states.get(&id)?.signature.as_ref()
    }
    pub fn set_text_signature(
        &mut self,
        id: StableNodeId,
        value: Option<crate::TextSignatureHelp>,
    ) {
        match value {
            Some(value) => self.ensure_editor_state(id).signature = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.signature = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    pub fn text_viewport_pin(&self, id: StableNodeId) -> Option<&ScrollOffset> {
        self.editor_states.get(&id)?.viewport_pin.as_ref()
    }
    pub fn set_text_viewport_pin(&mut self, id: StableNodeId, value: Option<ScrollOffset>) {
        match value {
            Some(value) => self.ensure_editor_state(id).viewport_pin = Some(value),
            None => {
                if let Some(state) = self.editor_states.get_mut(&id) {
                    state.viewport_pin = None;
                    self.prune_editor_state_if_empty(id);
                }
            }
        }
    }
    sparse!(
        cold_rects.text_drop_indicators,
        LayoutBox,
        text_drop_indicator,
        set_text_drop_indicator,
        NodePresenceBits::TEXT_DROP_INDICATOR
    );
    sparse!(
        text_natural_widths,
        f32,
        text_natural_width,
        set_text_natural_width,
        NodePresenceBits::TEXT_NATURAL_WIDTH
    );
    sparse!(
        text_wrap_limits,
        f32,
        text_wrap_limit,
        set_text_wrap_limit,
        NodePresenceBits::TEXT_WRAP_LIMIT
    );
    sparse!(
        cold_rects.explicit_rects,
        crate::ExplicitRect,
        explicit_rect,
        set_explicit_rect,
        NodePresenceBits::EXPLICIT_RECT
    );
    sparse!(
        block_struts,
        crate::FontStrut,
        block_strut,
        set_block_strut,
        NodePresenceBits::BLOCK_STRUT
    );

    /// Every block holding a strut: what a new font set can change.
    pub(crate) fn strut_holders(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.block_struts
            .keys()
            .copied()
            .chain(self.inline_struts.keys().copied())
    }

    pub(crate) fn block_x_height(&self, id: StableNodeId) -> Option<f32> {
        self.block_x_heights.get(&id).copied()
    }

    pub(crate) fn set_block_x_height(&mut self, id: StableNodeId, value: Option<f32>) {
        match value {
            Some(x_height) => {
                self.block_x_heights.insert(id, x_height);
            }
            None => {
                self.block_x_heights.remove(&id);
            }
        }
    }

    pub(crate) fn inline_strut(&self, id: StableNodeId) -> Option<(crate::FontStrut, Option<f32>)> {
        self.inline_struts.get(&id).copied()
    }

    pub(crate) fn set_inline_strut(
        &mut self,
        id: StableNodeId,
        value: Option<(crate::FontStrut, Option<f32>)>,
    ) {
        match value {
            Some(strut) => {
                self.inline_struts.insert(id, strut);
            }
            None => {
                self.inline_struts.remove(&id);
            }
        }
    }
    sparse!(
        languages,
        nana_text::font::LanguageTag,
        language,
        set_language,
        NodePresenceBits::LANGUAGE
    );
    sparse!(
        text_scales,
        f32,
        text_scale,
        set_text_scale,
        NodePresenceBits::TEXT_SCALE
    );
    sparse!(
        rich_texts,
        nana_ui_core::RichText,
        rich_text,
        set_rich_text,
        NodePresenceBits::RICH_TEXT
    );
    sparse!(
        glyph_presentations,
        nana_ui_core::GlyphPresentation,
        glyph_presentation,
        set_glyph_presentation,
        NodePresenceBits::GLYPH_PRESENTATION
    );
    sparse!(
        rich_editor_marks,
        crate::RichEditorMarks,
        rich_editor_marks,
        set_rich_editor_marks,
        NodePresenceBits::RICH_EDITOR_MARKS
    );

    pub(crate) fn text_layouts(&self) -> &nana_text::TextLayoutStore {
        &self.text_layouts
    }

    pub(crate) fn text_node(&self, id: StableNodeId) -> Option<&crate::text_node::TextNodeState> {
        self.text_nodes.get(&id)
    }
    pub(crate) fn note_text_fragments(&mut self, id: StableNodeId) {
        if let Some(text) = self.text_nodes.get_mut(&id) {
            text.fragments_layout = text.layout;
        }
    }

    fn intern_empty_text_backend(&mut self, backend: crate::text_node::TextBackendEpoch) -> u8 {
        if let Some(index) = self
            .empty_text_backends
            .iter()
            .position(|existing| *existing == backend)
        {
            return (index + 1) as u8;
        }
        if self.empty_text_backends.len() < u8::MAX as usize {
            self.empty_text_backends.push(backend);
            return self.empty_text_backends.len() as u8;
        }
        0
    }

    pub(crate) fn is_empty_text_current(
        &self,
        id: StableNodeId,
        backend: crate::text_node::TextBackendEpoch,
    ) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        if record.has_text
            || matches!(record.kind.as_ref(), crate::world::NodeKind::Text)
            || record.empty_text_backend == 0
        {
            return false;
        }
        self.empty_text_backends
            .get(usize::from(record.empty_text_backend) - 1)
            .copied()
            == Some(backend)
    }

    pub(crate) fn note_text_scale(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::note_scale)
    }

    pub(crate) fn take_text_scale_pending(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::take_scale_pending)
    }

    /// Records a change to `id`'s text. See [`crate::text_node::TextDirty`].
    pub(crate) fn invalidate_text(&mut self, id: StableNodeId, dirty: crate::text_node::TextDirty) {
        if dirty.is_empty() {
            return;
        }
        if let Some(text) = self.text_nodes.get_mut(&id) {
            text.invalidate(dirty);
            return;
        }
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let work = dirty.work();
        if dirty.intersects(crate::text_node::TextDirty::CONTENT)
            || work.intersects(crate::text_node::TextWork::SHAPE)
            || work.intersects(crate::text_node::TextWork::LAYOUT)
        {
            record.empty_text_backend = 0;
        }
        if record.has_text || matches!(record.kind.as_ref(), crate::world::NodeKind::Text) {
            record.attachments.set(NodePresenceBits::TEXT_NODE, true);
            self.text_nodes.entry(id).or_default().invalidate(dirty);
        }
    }

    /// Stamps `id`'s text as resolved at its current revisions. Returns
    /// whether a language change had invalidated it in vain; see
    /// [`crate::text_node::TextNodeState::mark_resolved`].
    pub(crate) fn mark_text_resolved(
        &mut self,
        id: StableNodeId,
        backend: crate::text_node::TextBackendEpoch,
        constraints: Option<crate::TextShapeConstraints>,
    ) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let text_node =
            record.has_text || matches!(record.kind.as_ref(), crate::world::NodeKind::Text);
        if !text_node && !self.text_nodes.contains_key(&id) {
            let backend_index = self.intern_empty_text_backend(backend);
            if let Some(record) = self.nodes.get_mut(id) {
                record.empty_text_backend = backend_index;
            }
            return false;
        }
        let constraints =
            constraints.map(|constraints| (constraints, record.style.text_horizontal_alignment));
        let language = record.resolved.0.language.clone();
        if let Some(record) = self.nodes.get_mut(id) {
            record.attachments.set(NodePresenceBits::TEXT_NODE, true);
        }
        self.text_nodes.entry(id).or_default().mark_resolved(
            backend,
            constraints,
            text_node,
            language.as_ref(),
        )
    }

    /// The `nana-text` source for `id`'s current text, built once per content
    /// revision, and whether this call built it.
    pub(crate) fn text_source(
        &mut self,
        id: StableNodeId,
        style: &crate::ComputedStyle,
    ) -> Option<(&nana_text::TextSource, bool)> {
        if let Some(record) = self.nodes.get_mut(id) {
            record.attachments.set(NodePresenceBits::TEXT_NODE, true);
        } else {
            return None;
        }
        let text = self.texts.get(&id).unwrap_or(&DEFAULT_TEXT_CONTENT);
        let rich = self.rich_texts.get(&id);
        Some(
            self.text_nodes.entry(id).or_default().source_for_objects(
                &text.value,
                rich,
                style,
                self.inline_contents
                    .get(&id)
                    .map_or(&[], |inline| inline.objects.as_ref()),
            ),
        )
    }

    pub(crate) fn inline_record(
        &self,
        id: StableNodeId,
    ) -> Option<&crate::inline_content::InlineRecord> {
        self.inline_contents.get(&id)
    }
    pub(crate) fn has_inline_content(&self) -> bool {
        !self.inline_contents.is_empty()
    }
    pub(crate) fn inline_parent(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.inline_parents.get(&id).copied()
    }
    pub(crate) fn set_inline_atoms(
        &mut self,
        id: StableNodeId,
        atoms: Arc<[crate::InlineAtomic]>,
    ) -> bool {
        if self
            .inline_contents
            .get(&id)
            .is_some_and(|previous| previous.atoms == atoms)
            || (atoms.is_empty() && !self.inline_contents.contains_key(&id))
        {
            return false;
        }
        if let Some(previous) = self.inline_contents.get_mut(&id)
            && previous.atoms.len() == atoms.len()
            && previous
                .atoms
                .iter()
                .zip(atoms.iter())
                .all(|(a, b)| (a.offset, a.node) == (b.offset, b.node))
        {
            previous.atoms = atoms;
            return false;
        }
        let previous = self.inline_contents.remove(&id).unwrap_or_default();
        if let Some(record) = self.nodes.get_mut(id) {
            record.attachments.clear(NodePresenceBits::INLINE_CONTENT);
        }
        let old: crate::NodeMap<_> = previous
            .atoms
            .iter()
            .zip(previous.objects.iter())
            .map(|(atom, object)| (atom.node, object.metrics))
            .collect();
        for atom in previous.atoms.iter() {
            if self.inline_parents.get(&atom.node) == Some(&id) {
                self.inline_parents.remove(&atom.node);
                if let Some(record) = self.nodes.get_mut(atom.node) {
                    record.attachments.clear(NodePresenceBits::INLINE_PARENT);
                }
            }
        }
        if atoms.is_empty() {
            if self.inline_contents.is_empty() {
                self.inline_contents = crate::NodeMap::default();
                // A destination already installed earlier in this batch keeps
                // its reverse binding when an old owner is cleared.
                if self.inline_parents.is_empty() {
                    self.inline_parents = crate::NodeMap::default();
                }
            }
            return true;
        }
        let objects = atoms
            .iter()
            .map(|atom| {
                self.inline_parents.insert(atom.node, id);
                if let Some(record) = self.nodes.get_mut(atom.node) {
                    record
                        .attachments
                        .set(NodePresenceBits::INLINE_PARENT, true);
                }
                nana_text::InlineObject {
                    offset: atom.offset,
                    id: atom.node.get(),
                    metrics: old.get(&atom.node).copied().unwrap_or(
                        nana_text::InlineObjectMetrics {
                            width_px: 0.0,
                            ascent_px: 0.0,
                            descent_px: 0.0,
                            align: nana_text::InlineObjectAlign::Baseline,
                            baseline_shift_px: 0.0,
                            envelope: None,
                        },
                    ),
                }
            })
            .collect::<Vec<_>>()
            .into();
        self.inline_contents
            .insert(id, crate::inline_content::InlineRecord { atoms, objects });
        if let Some(record) = self.nodes.get_mut(id) {
            record
                .attachments
                .set(NodePresenceBits::INLINE_CONTENT, true);
        }
        true
    }
    pub(crate) fn set_inline_objects(
        &mut self,
        id: StableNodeId,
        objects: Arc<[nana_text::InlineObject]>,
    ) -> bool {
        let Some(inline) = self.inline_contents.get_mut(&id) else {
            return false;
        };
        if inline.objects == objects {
            return false;
        }
        inline.objects = objects;
        true
    }

    /// Retains `layout` for `id`, replacing what the node held. A node handed
    /// the very layout it already holds keeps its handle. Returns whether the
    /// handle was kept.
    pub(crate) fn retain_text_layout(
        &mut self,
        id: StableNodeId,
        layout: Arc<nana_text::TextLayout>,
    ) -> bool {
        if let Some(record) = self.nodes.get_mut(id) {
            record.attachments.set(NodePresenceBits::TEXT_NODE, true);
        } else {
            return false;
        }
        let text = self.text_nodes.entry(id).or_default();
        if self
            .text_layouts
            .get(text.layout)
            .is_some_and(|current| Arc::ptr_eq(current, &layout))
        {
            return true;
        }
        self.text_layouts.remove(text.layout);
        text.layout = self.text_layouts.insert(layout);
        false
    }

    /// Drops the layout `id` retains, if any. O(1) when it holds none.
    pub(crate) fn release_text_layout(&mut self, id: StableNodeId) {
        if let Some(text) = self.text_nodes.get_mut(&id)
            && !text.layout.is_null()
        {
            let held = std::mem::take(&mut text.layout);
            self.text_layouts.remove(held);
            // The source copy served only the released layout.
            text.release_source();
        }
    }

    /// Every node whose text a new font set can change: text that was measured
    /// (an empty Text node's line box included), plus the visuals that measure text of their own every pass
    /// and so are never stamped.
    pub(crate) fn text_bearing_nodes(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        let resolved = self
            .text_nodes
            .iter()
            .filter(|(_, text)| text.stamp().is_some_and(|stamp| stamp.measured))
            .map(|(id, _)| id);
        let widget_measuring = self
            .widget_visuals
            .iter()
            .filter(|(_, visual)| visual.measures_text())
            .map(|(id, _)| *id);
        let preset_measuring = (self.preset_visual_count > 0)
            .then(|| {
                self.nodes
                    .entries()
                    .filter(|(_, record)| record.visual_preset == 1)
                    .map(|(id, _)| id)
            })
            .into_iter()
            .flatten();
        resolved.chain(widget_measuring).chain(preset_measuring)
    }

    pub(crate) fn text_completion_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextCompletionViewState> {
        self.editor_states
            .get_mut(&id)
            .and_then(|state| state.completion.as_mut())
    }

    pub(crate) fn text_hover_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextHoverViewState> {
        self.editor_states
            .get_mut(&id)
            .and_then(|state| state.hover.as_mut())
    }

    pub(crate) fn diagnostic_hover_ids(&self) -> Vec<StableNodeId> {
        self.editor_states
            .iter()
            .filter(|(_, state)| state.hover.as_ref().is_some_and(|h| h.diagnostic))
            .map(|(id, _)| *id)
            .collect()
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
                origin_x: 0.0,
                origin_y: 0.0,
            }),
        );
        store.set_component_type(id, Some(ComponentTypeId::new("nana.button").unwrap()));
        store.set_editor(
            id,
            Some(EditorRecord {
                empty_preedit: Some(None),
                ..EditorRecord::new(nana_text::EditSession::new("a"))
            }),
        );
        store.set_text_content(id, TextContent { value: "hi".into() });
        store.set_text_metrics(
            id,
            TextMetrics {
                width: 12.0,
                height: 16.0,
                ascent: Some(12.0),
            },
        );
        store.set_layout_padding(id, nana_ui_core::PaddingSpec::uniform(4.0));
        store.set_layout_far_start(id, [true, false]);
        store.bump_layout_context_generation(id);
        store.set_scroll_offset(id, ScrollOffset { x: 0.0, y: 8.0 });
        store.set_accessibility(
            id,
            &AccessibilityState {
                hidden: true,
                ..AccessibilityState::default()
            },
        );
        store.set_render_room_epoch(id, 42);
        assert!(store.scroll_metrics(id).is_some());
        assert!(store.remove(id).is_some());
        assert!(!store.contains(id));
        assert!(store.scroll_metrics(id).is_none());
        assert!(store.component_type(id).is_none());
        assert!(store.ime(id).is_none());
        assert_eq!(store.text_content(id).value, "");
        assert_eq!(store.text_metrics(id), TextMetrics::default());
        assert_eq!(store.layout_padding(id), None);
        assert_eq!(store.layout_far_start(id), None);
        assert_eq!(store.layout_context_generation(id), 0);
        assert_eq!(store.scroll_offset(id), ScrollOffset::default());
        assert!(!store.accessibility(id).hidden);
        assert_eq!(store.render_room_epoch(id), u32::MAX);
    }

    #[test]
    fn node_record_fits_size_budget() {
        let size = std::mem::size_of::<NodeRecord>();
        eprintln!("size_of::<NodeRecord>() = {size}");
        assert!(
            size <= 152,
            "NodeRecord grew to {size} bytes (budget <= 152 bytes)"
        );
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

    #[test]
    fn dense_ids_and_overflow_ids_round_trip() {
        let mut store = NodeStore::new();
        let record = || {
            NodeRecord::new(
                document(1),
                &NodeKind::Text,
                InteractionState {
                    pointer_events: true,
                    focusable: false,
                },
            )
        };
        store.insert(node(1), record());
        store.insert(node(2), record());
        // A hole large enough to stay out of the contiguous table.
        store.insert(node(20_000), record());
        store.insert(node(80_000), record());
        assert_eq!(store.len(), 4);
        assert!(store.contains(node(1)));
        assert!(store.contains(node(20_000)));
        assert!(store.contains(node(80_000)));
        assert!(!store.contains(node(3)));
        let mut keys = store.keys().map(|id| id.get()).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 2, 20_000, 80_000]);
        assert!(store.remove(node(20_000)).is_some());
        assert!(store.remove(node(1)).is_some());
        assert_eq!(store.len(), 2);
        store.insert(node(1), record());
        assert!(store.get(node(1)).is_some());
        assert!(store.get(node(80_000)).is_some());
        assert_eq!(store.len(), 3);
    }
}
