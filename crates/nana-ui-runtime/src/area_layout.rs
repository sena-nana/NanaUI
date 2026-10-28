//! Area layout (#340): the window cut into rectangles that never overlap, the
//! way 3D and animation tools lay out their editors. Each area shows one
//! editor the application chooses, under a header the application fills.
//!
//! - The dividers between areas drag; each side keeps at least
//!   [`AreaLayout::min_size`] px.
//! - Dragging from an area's top corner into the area splits it there: mostly
//!   sideways cuts it side by side, mostly down cuts it top and bottom. Dragging
//!   into the area beside it, when the two are the halves of one split, joins
//!   them: the area dragged from takes the other's place. A preview shows the
//!   cut line or the area that goes.
//! - An area can be maximized over the whole layout and restored.
//! - The layout ([`AreaLayoutModel`]) is plain data that serializes with each
//!   area's editor type id and state key; switching between saved layouts
//!   (workspaces) only moves the contents, so their state survives.
//!
//! Contents come from the view: a child keyed with an area's id is its
//! content, one keyed `{id}:header` its header. A child keyed
//! `editor:{type}` (and `editor:{type}:header`) is an editor's content and
//! goes to an area showing that editor, so areas split off or switched to
//! another editor show it (#351). Keyed children whose area is not in the
//! layout wait, hidden, until a layout shows it again.
//!
//! With [`AreaLayout::editor_select`] every header starts with a select of
//! editor types; choosing one is [`AppContext::set_area_editor`].

mod model;

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, CursorSpec, DisplaySpec, FlexDirection, LengthSpec, OverflowSpec, PositionSpec,
    SemanticColorRole,
};

pub use model::{
    Area, AreaAxis, AreaGeometry, AreaLayoutError, AreaLayoutModel, AreaNode, AreaSplitter,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, LayoutBox, MutationQueue, NodeKind, NodePointer, NodePointerHooks, NodeStyle,
    NodeWheel, Select, SelectChanged, SelectOption, StableNodeId, UiWorld,
};

/// The key prefix of a content placed by its editor type (#351).
const EDITOR_KEY: &str = "editor:";

/// The key of `editor`'s content.
fn editor_key(editor: &str) -> String {
    format!("{EDITOR_KEY}{editor}")
}

/// A corner drag moves this far before it means anything, px.
const DRAG_SLOP: f32 = nana_ui_core::space::SM;
/// The square at an area's top corners a split or join starts from, px.
const CORNER: f32 = 12.0;
/// The cut line a split previews, px.
const PREVIEW_LINE: f32 = 2.0;
/// The least a divider grabs, however thin the gap it draws, px.
const GRIP: f32 = 6.0;

/// What changed in the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaChange {
    /// A divider moved.
    Resized,
    /// `area` was cut; `new` is the other half, showing the same editor.
    Split { area: String, new: String },
    /// `removed` went; `kept` took its place.
    Joined { kept: String, removed: String },
    /// The maximized area, `None` when everything shows again.
    Maximized(Option<String>),
    /// `area` shows another editor.
    Editor { area: String },
    /// The application set a whole new layout.
    Replaced,
}

/// The layout changed; `model` is the new one, to keep or save.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaLayoutChanged {
    pub change: AreaChange,
    pub model: AreaLayoutModel,
}

/// A corner drag in progress.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CornerDrag {
    area: String,
    origin: (f32, f32),
    action: Option<CornerAction>,
}

/// What releasing a corner drag does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CornerAction {
    Split { axis: AreaAxis, ratio: f32 },
    Join { remove: String },
}

/// The nodes built for one node of the tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AreaPart {
    Split {
        path: Vec<bool>,
        axis: AreaAxis,
        container: StableNodeId,
        bar: StableNodeId,
        first: StableNodeId,
        second: StableNodeId,
    },
    Area {
        id: String,
        frame: StableNodeId,
        header: StableNodeId,
        content: StableNodeId,
        corners: [StableNodeId; 2],
        /// The editor type select at the start of the header.
        select: Option<StableNodeId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AreaParts {
    /// The tree the nodes were built for, ratios aside.
    shape: AreaNode,
    pub(crate) top: StableNodeId,
    pub(crate) nodes: Vec<AreaPart>,
    pub(crate) preview: StableNodeId,
    /// Where keyed contents whose area is not shown wait.
    pub(crate) stash: StableNodeId,
}

/// Areas cut from one rectangle; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaLayout {
    pub model: AreaLayoutModel,
    /// Space between areas, the dividers' grab width, px.
    pub gap: f32,
    /// The least either side of a divider keeps, px.
    pub min_size: f32,
    pub style: NodeStyle,
    /// How every area's frame and header are painted.
    pub chrome: AreaChrome,
    /// What assistive technology calls an area, by its editor type id, in the
    /// interface's language (#350). An editor without one is called by its id.
    /// Not part of [`AreaLayoutModel`], which stays language-free data.
    pub labels: std::collections::BTreeMap<String, Arc<str>>,
    /// The editor type select at the start of every header (#351); `None`
    /// leaves the headers to the application.
    pub editor_select: Option<AreaEditorSelect>,
    /// The area each editor type was last chosen in: its `editor:` content
    /// goes there.
    pub(crate) hosts: std::collections::BTreeMap<String, String>,
    pub(crate) drag: Option<CornerDrag>,
    pub(crate) parts: Option<AreaParts>,
}

/// How every area is painted. The default is the tiled look: small corners and
/// a header filled apart from the content. A larger corner tier, a header in
/// the frame's own fill and a hairline under it make each area read as a
/// separate panel when the layout's [`AreaLayout::gap`] shows the background
/// between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaChrome {
    /// The corner tier of each area's frame.
    pub frame_radius: nana_ui_core::RadiusTier,
    /// The fill of each area's header; the frame itself is the surface.
    pub header_background: SemanticColorRole,
    /// A hairline between the header and the content.
    pub header_divider: bool,
    /// The header's height in logical pixels, its content centred in it.
    /// `None` is the small control height: one row of compact controls and
    /// nothing around them. A taller header leaves room for controls grouped
    /// on a track, or a band that reads as the area's title bar.
    pub header_height: Option<f32>,
}

impl Default for AreaChrome {
    fn default() -> Self {
        Self {
            frame_radius: nana_ui_core::RadiusTier::Sm,
            header_background: SemanticColorRole::Subtle,
            header_divider: false,
            header_height: None,
        }
    }
}

/// The editor types an area's header select offers, in order, named by
/// [`AreaLayout::labels`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaEditorSelect {
    pub editors: Vec<String>,
    /// What assistive technology calls the select; `None` is the framework
    /// strings' `area_layout.editor_type`.
    pub label: Option<Arc<str>>,
}

impl AreaEditorSelect {
    pub fn new<E: Into<String>>(editors: impl IntoIterator<Item = E>) -> Self {
        Self {
            editors: editors.into_iter().map(Into::into).collect(),
            label: None,
        }
    }

    /// See [`Self::label`]; an empty name is none.
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        let label = label.into();
        self.label = (!label.is_empty()).then_some(label);
        self
    }
}

impl AreaLayout {
    pub fn new(model: AreaLayoutModel) -> Self {
        Self {
            model,
            gap: nana_ui_core::space::XXS,
            min_size: 48.0,
            style: layout_style(),
            chrome: AreaChrome::default(),
            labels: Default::default(),
            editor_select: None,
            hosts: Default::default(),
            drag: None,
            parts: None,
        }
    }

    /// Start every area's header with a select of `select`'s editor types;
    /// choosing one shows it in that area ([`AppContext::set_area_editor`]).
    /// [`AppContext::set_area_editor_select`] changes it later.
    pub fn editor_select(mut self, select: AreaEditorSelect) -> Self {
        self.editor_select = Some(select);
        self
    }

    /// Name the areas showing each editor, `(editor type id, label)`; see
    /// [`Self::labels`]. [`AppContext::set_area_labels`] changes them later.
    pub fn labels<E: Into<String>, L: Into<Arc<str>>>(
        mut self,
        labels: impl IntoIterator<Item = (E, L)>,
    ) -> Self {
        self.labels = labels
            .into_iter()
            .map(|(editor, label)| (editor.into(), label.into()))
            .collect();
        self
    }

    /// Replaces the node style wholesale.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(1.0);
        self
    }

    /// See [`AreaChrome`].
    pub fn chrome(mut self, chrome: AreaChrome) -> Self {
        self.chrome = chrome;
        self
    }

    pub fn min_size(mut self, min_size: f32) -> Self {
        self.min_size = min_size.max(0.0);
        self
    }
}

impl ComponentView for AreaLayout {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::assemble_area_layout),
        slot_assembler: Some(AppContext::assemble_area_layout),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // the layout is the user's once they have dragged it; a view that
        // means to replace it calls `set_area_layout`
        next.model = std::mem::replace(&mut self.model, next.model);
        next.hosts = std::mem::take(&mut self.hosts);
        next.drag = self.drag.take();
        next.parts = self.parts.take();
        *self = next;
    }

    fn keep_framework_state(&mut self, previous: &Self) {
        if self.parts.is_none() {
            self.parts = previous.parts.clone();
        }
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "area-layout".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }
}

/// A container, a frame, a slot or the preview: a styled box the layout keeps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AreaBox {
    tag: &'static str,
    pub(crate) style: NodeStyle,
    accessibility: AccessibilityState,
}

impl AreaBox {
    fn new(tag: &'static str, style: NodeStyle) -> Self {
        Self {
            tag,
            style,
            accessibility: AccessibilityState::default(),
        }
    }
}

impl ComponentView for AreaBox {
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: self.tag.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            self.accessibility.clone(),
        );
    }
}

/// A divider: dragging it moves the split's ratio.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AreaBar {
    layout: StableNodeId,
    path: Vec<bool>,
    axis: AreaAxis,
    style: NodeStyle,
    dragging: bool,
}

impl ComponentView for AreaBar {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&BAR_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "area-divider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.style.clone();
        if self.dragging {
            style.background = Some(SemanticColorRole::Accent);
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Separator,
                orientation: Some(match self.axis {
                    AreaAxis::Horizontal => crate::SelectionOrientation::Vertical,
                    AreaAxis::Vertical => crate::SelectionOrientation::Horizontal,
                }),
                ..AccessibilityState::default()
            },
        );
    }
}

/// An area's top corner: a drag from it splits or joins.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AreaCorner {
    layout: StableNodeId,
    area: String,
    style: NodeStyle,
}

impl ComponentView for AreaCorner {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&CORNER_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "area-corner".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                hidden: true,
                ..AccessibilityState::default()
            },
        );
    }
}

fn layout_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Relative;
    layout.direction = Some(FlexDirection::Column);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.min_height = Some(LengthSpec::Px(0.0));
    style
}

/// A split's container, or a frame, as one side of its parent split.
fn side_style(mut style: NodeStyle, grow: f32, shown: bool) -> NodeStyle {
    let layout = Arc::make_mut(&mut style.layout);
    layout.flex_grow = Some(grow);
    layout.flex_shrink = Some(1.0);
    layout.align_self = Some(AlignSpec::Stretch);
    layout.flex_basis = Some(LengthSpec::Px(0.0));
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.min_height = Some(LengthSpec::Px(0.0));
    layout.display = (!shown).then_some(DisplaySpec::None);
    style
}

fn container_style(axis: AreaAxis, grow: f32, shown: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    Arc::make_mut(&mut style.layout).direction = Some(match axis {
        AreaAxis::Horizontal => FlexDirection::Row,
        AreaAxis::Vertical => FlexDirection::Column,
    });
    side_style(style, grow, shown)
}

fn bar_style(axis: AreaAxis, gap: f32, shown: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        // a thin seam still grabs GRIP px: the bar reaches over the areas'
        // edges by negative margins and stays above them
        let grip = gap.max(GRIP);
        let reach = Some(LengthSpec::Px(-(grip - gap) / 2.0));
        match axis {
            AreaAxis::Horizontal => {
                layout.width = Some(LengthSpec::Px(grip));
                layout.margin_left = reach;
                layout.margin_right = reach;
            }
            AreaAxis::Vertical => {
                layout.height = Some(LengthSpec::Px(grip));
                layout.margin_top = reach;
                layout.margin_bottom = reach;
            }
        }
        layout.position = PositionSpec::Relative;
        layout.z_index = Some(1);
        layout.flex_shrink = Some(0.0);
        layout.align_self = Some(AlignSpec::Stretch);
        layout.cursor = Some(match axis {
            AreaAxis::Horizontal => CursorSpec::EwResize,
            AreaAxis::Vertical => CursorSpec::NsResize,
        });
        layout.display = (!shown).then_some(DisplaySpec::None);
    }
    style.interaction.hovered.background = Some(SemanticColorRole::BorderStrong);
    style
}

fn frame_style(grow: f32, shown: bool, chrome: AreaChrome) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Relative;
        layout.direction = Some(FlexDirection::Column);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
    }
    style.background = Some(SemanticColorRole::Surface);
    style.radius = Some(chrome.frame_radius);
    side_style(style, grow, shown)
}

fn header_style(chrome: AreaChrome) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.padding_left = Some(LengthSpec::Px(CORNER));
        layout.padding_right = Some(LengthSpec::Px(CORNER));
        layout.flex_shrink = Some(0.0);
        layout.align_self = Some(AlignSpec::Stretch);
        if chrome.header_divider {
            // only the edge that meets the content
            layout.border_width = Some(0.0);
            layout.border_bottom_width = Some(nana_ui_core::HAIRLINE);
            layout.theme.border_bottom_width =
                Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        }
    }
    style.background = Some(chrome.header_background);
    if chrome.header_divider {
        style.border = Some(SemanticColorRole::BorderSoft);
    }
    match chrome
        .header_height
        .filter(|height| height.is_finite() && *height > 0.0)
    {
        Some(height) => {
            let layout = Arc::make_mut(&mut style.layout);
            layout.height = Some(LengthSpec::Px(height));
            layout.min_height = Some(LengthSpec::Px(height));
        }
        None => {
            style.control_height = Some(nana_ui_core::ControlHeight::Exact(
                nana_ui_core::ControlSize::Small,
            ));
        }
    }
    style
}

fn content_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.flex_grow = Some(1.0);
        layout.flex_basis = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.align_self = Some(AlignSpec::Stretch);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
    }
    style
}

fn corner_style(right: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_top = Some(LengthSpec::Px(0.0));
        if right {
            layout.offset_right = Some(LengthSpec::Px(0.0));
        } else {
            layout.offset_left = Some(LengthSpec::Px(0.0));
        }
        layout.width = Some(LengthSpec::Px(CORNER));
        layout.height = Some(LengthSpec::Px(CORNER));
        layout.cursor = Some(CursorSpec::Crosshair);
        layout.z_index = Some(1);
    }
    style.interaction.hovered.background = Some(SemanticColorRole::Hover);
    style
}

fn hidden_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    Arc::make_mut(&mut style.layout).display = Some(DisplaySpec::None);
    style
}

/// The preview of a corner drag, `rect` in the layout's own px.
fn preview_style(rect: Option<LayoutBox>, join: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.z_index = Some(2);
    match rect {
        Some(rect) => {
            layout.offset_left = Some(LengthSpec::Px(rect.x));
            layout.offset_top = Some(LengthSpec::Px(rect.y));
            layout.width = Some(LengthSpec::Px(rect.width));
            layout.height = Some(LengthSpec::Px(rect.height));
            if join {
                layout.border_width = Some(2.0);
                style.background = Some(SemanticColorRole::AccentSoft);
                style.border = Some(SemanticColorRole::Accent);
                style.radius = Some(nana_ui_core::RadiusTier::Sm);
            } else {
                style.background = Some(SemanticColorRole::Accent);
            }
        }
        None => layout.display = Some(DisplaySpec::None),
    }
    style
}

/// Ratios aside: whether the nodes built for one tree fit another.
fn shape_of(node: &AreaNode) -> AreaNode {
    match node {
        AreaNode::Area { id } => AreaNode::Area { id: id.clone() },
        AreaNode::Split {
            axis,
            first,
            second,
            ..
        } => AreaNode::Split {
            axis: *axis,
            ratio: 0.0,
            first: Box::new(shape_of(first)),
            second: Box::new(shape_of(second)),
        },
    }
}

/// Whether `node` holds the maximized area (or any area when none is).
fn holds(node: &AreaNode, maximized: Option<&str>) -> bool {
    match (node, maximized) {
        (_, None) => true,
        (AreaNode::Area { id }, Some(max)) => id == max,
        (AreaNode::Split { first, second, .. }, max) => holds(first, max) || holds(second, max),
    }
}

const BAR_POINTER: NodePointerHooks<AreaBar> = NodePointerHooks {
    moved: bar_moved,
    left: |_, _| Ok(()),
    pressed: |cx, bar, pointer| {
        if pointer.button != 0 {
            return Ok(false);
        }
        cx.update_component(bar, |bar, _| bar.dragging = true)?;
        Ok(true)
    },
    released: |cx, bar, _| {
        let layout = cx.read(bar, |bar| bar.layout)?;
        cx.update_component(bar, |bar, _| bar.dragging = false)?;
        if let Some(layout) = cx.view_entity::<AreaLayout>(layout) {
            cx.emit_area_change(layout, AreaChange::Resized)?;
        }
        Ok(true)
    },
    wheel: no_wheel,
};

const CORNER_POINTER: NodePointerHooks<AreaCorner> = NodePointerHooks {
    moved: corner_moved,
    left: |cx, corner| {
        // a cancelled press: nothing happens
        let layout = cx.read(corner, |corner| corner.layout)?;
        if let Some(layout) = cx.view_entity::<AreaLayout>(layout)
            && cx.read(layout, |layout| layout.drag.is_some())?
        {
            cx.update_component(layout, |layout, _| layout.drag = None)?;
        }
        Ok(())
    },
    pressed: corner_pressed,
    released: corner_released,
    wheel: no_wheel,
};

fn no_wheel<C: crate::View>(
    _: &mut AppContext,
    _: Entity<C>,
    _: NodeWheel,
) -> Result<bool, FrameworkError> {
    Ok(false)
}

/// The pointer in layout px.
fn absolute(cx: &AppContext, node: StableNodeId, pointer: &NodePointer) -> Option<(f32, f32)> {
    let origin = cx.world().component_layout_box(node)?;
    Some((pointer.x + origin.x, pointer.y + origin.y))
}

fn bar_moved(
    cx: &mut AppContext,
    bar: Entity<AreaBar>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    let (layout, path, axis) = cx.read(bar, |bar| (bar.layout, bar.path.clone(), bar.axis))?;
    let Some(layout) = cx.view_entity::<AreaLayout>(layout) else {
        return Ok(false);
    };
    let (Some(at), Some(span)) = (
        absolute(cx, bar.stable_id(), &pointer),
        cx.world()
            .parent_id(bar.stable_id())
            .and_then(|container| cx.world().component_layout_box(container)),
    ) else {
        return Ok(true);
    };
    let (gap, min) = cx.read(layout, |layout| (layout.gap, layout.min_size))?;
    let (start, extent, at) = match axis {
        AreaAxis::Horizontal => (span.x, span.width, at.0),
        AreaAxis::Vertical => (span.y, span.height, at.1),
    };
    let usable = (extent - gap).max(1.0);
    let ratio = model::clamp_ratio((at - start - gap / 2.0) / usable, extent, gap, min);
    cx.update_component(layout, |layout, _| {
        layout.model.set_ratio(&path, ratio);
    })?;
    Ok(true)
}

fn corner_pressed(
    cx: &mut AppContext,
    corner: Entity<AreaCorner>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    let (layout, area) = cx.read(corner, |corner| (corner.layout, corner.area.clone()))?;
    let (Some(layout), Some(origin)) = (
        cx.view_entity::<AreaLayout>(layout),
        absolute(cx, corner.stable_id(), &pointer),
    ) else {
        return Ok(false);
    };
    cx.update_component(layout, |layout, _| {
        layout.drag = Some(CornerDrag {
            area,
            origin,
            action: None,
        });
    })?;
    Ok(true)
}

fn corner_moved(
    cx: &mut AppContext,
    corner: Entity<AreaCorner>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    let layout = cx.read(corner, |corner| corner.layout)?;
    let (Some(layout), Some(at)) = (
        cx.view_entity::<AreaLayout>(layout),
        absolute(cx, corner.stable_id(), &pointer),
    ) else {
        return Ok(false);
    };
    let Some(drag) = cx.read(layout, |layout| layout.drag.clone())? else {
        return Ok(true);
    };
    let action = cx.corner_action(layout, &drag, at)?;
    if action != drag.action {
        cx.update_component(layout, |layout, _| {
            if let Some(drag) = layout.drag.as_mut() {
                drag.action = action;
            }
        })?;
    }
    Ok(true)
}

fn corner_released(
    cx: &mut AppContext,
    corner: Entity<AreaCorner>,
    _: NodePointer,
) -> Result<bool, FrameworkError> {
    let layout = cx.read(corner, |corner| corner.layout)?;
    let Some(layout) = cx.view_entity::<AreaLayout>(layout) else {
        return Ok(false);
    };
    let Some(drag) = cx.read(layout, |layout| layout.drag.clone())? else {
        return Ok(true);
    };
    cx.update_component(layout, |layout, _| layout.drag = None)?;
    match drag.action {
        Some(CornerAction::Split { axis, ratio }) => {
            cx.split_area(layout, &drag.area, axis, ratio)?;
        }
        Some(CornerAction::Join { remove }) => {
            cx.join_areas(layout, &drag.area, &remove)?;
        }
        None => {}
    }
    Ok(true)
}

impl AppContext {
    /// Each shown area's box as last laid out, by id: where to put an
    /// overlay over an area, or a pointer in a test.
    pub fn area_layout_boxes(
        &self,
        layout: Entity<AreaLayout>,
    ) -> Result<Vec<(String, LayoutBox)>, FrameworkError> {
        self.area_boxes(layout)
    }

    fn area_boxes(
        &self,
        layout: Entity<AreaLayout>,
    ) -> Result<Vec<(String, LayoutBox)>, FrameworkError> {
        let parts = self.read(layout, |layout| layout.parts.clone())?;
        Ok(parts
            .map(|parts| {
                parts
                    .nodes
                    .iter()
                    .filter_map(|part| match part {
                        AreaPart::Area { id, frame, .. } => {
                            // a hidden frame keeps its last box
                            let shown = self
                                .read(Entity::<AreaBox>::from_stable_id(*frame), |frame| {
                                    frame.style.layout.display.is_none()
                                })
                                .unwrap_or(false);
                            self.world()
                                .component_layout_box(*frame)
                                .filter(|b| shown && b.width > 0.0 && b.height > 0.0)
                                .map(|b| (id.clone(), b))
                        }
                        AreaPart::Split { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// What a corner drag at `at` would do: split its own area along the
    /// way it has moved most, or join the area it is over into its own.
    fn corner_action(
        &self,
        layout: Entity<AreaLayout>,
        drag: &CornerDrag,
        at: (f32, f32),
    ) -> Result<Option<CornerAction>, FrameworkError> {
        let (dx, dy) = (at.0 - drag.origin.0, at.1 - drag.origin.1);
        if dx.abs().max(dy.abs()) < DRAG_SLOP {
            return Ok(None);
        }
        let boxes = self.area_boxes(layout)?;
        let Some((over, bounds)) = boxes.iter().find(|(_, b)| b.contains(at.0, at.1)).cloned()
        else {
            return Ok(None);
        };
        let (model, min) = self.read(layout, |layout| (layout.model.clone(), layout.min_size))?;
        if over == drag.area {
            let (axis, ratio, extent) = if dx.abs() >= dy.abs() {
                (
                    AreaAxis::Horizontal,
                    (at.0 - bounds.x) / bounds.width,
                    bounds.width,
                )
            } else {
                (
                    AreaAxis::Vertical,
                    (at.1 - bounds.y) / bounds.height,
                    bounds.height,
                )
            };
            // both halves at least the minimum
            if extent * ratio < min || extent * (1.0 - ratio) < min {
                return Ok(None);
            }
            return Ok(Some(CornerAction::Split { axis, ratio }));
        }
        Ok(model
            .can_join(&drag.area, &over)
            .then_some(CornerAction::Join { remove: over }))
    }

    fn emit_area_change(
        &mut self,
        layout: Entity<AreaLayout>,
        change: AreaChange,
    ) -> Result<(), FrameworkError> {
        self.update_component(layout, |layout, view| {
            let model = layout.model.clone();
            view.emit(AreaLayoutChanged { change, model });
        })
    }

    /// Cut `area` along `axis`, the first half `ratio` of it. Answers the
    /// new area's id.
    pub fn split_area(
        &mut self,
        layout: Entity<AreaLayout>,
        area: &str,
        axis: AreaAxis,
        ratio: f32,
    ) -> Result<Option<String>, FrameworkError> {
        let new = self.update_component(layout, |layout, _| {
            let new = layout.model.fresh_id();
            layout
                .model
                .split(area, axis, ratio, new.clone())
                .then_some(new)
        })?;
        if let Some(new) = &new {
            self.emit_area_change(
                layout,
                AreaChange::Split {
                    area: area.to_owned(),
                    new: new.clone(),
                },
            )?;
        }
        Ok(new)
    }

    /// Join `remove` into `keep`, the two halves of one split.
    pub fn join_areas(
        &mut self,
        layout: Entity<AreaLayout>,
        keep: &str,
        remove: &str,
    ) -> Result<bool, FrameworkError> {
        let joined = self.update_component(layout, |layout, _| layout.model.join(keep, remove))?;
        if joined {
            self.emit_area_change(
                layout,
                AreaChange::Joined {
                    kept: keep.to_owned(),
                    removed: remove.to_owned(),
                },
            )?;
        }
        Ok(joined)
    }

    /// Maximize `area` over the whole layout, or restore when it already is
    /// (`None` restores).
    pub fn toggle_area_maximized(
        &mut self,
        layout: Entity<AreaLayout>,
        area: Option<&str>,
    ) -> Result<(), FrameworkError> {
        let maximized = self.update_component(layout, |layout, _| {
            layout.model.toggle_maximized(area);
            layout.model.maximized.clone()
        })?;
        self.emit_area_change(layout, AreaChange::Maximized(maximized))
    }

    /// Show `editor` in `area`. Its `editor:` content moves there; a content
    /// keyed by the area's id is the application's to swap.
    pub fn set_area_editor(
        &mut self,
        layout: Entity<AreaLayout>,
        area: &str,
        editor: &str,
    ) -> Result<bool, FrameworkError> {
        let set = self.update_component(layout, |layout, _| {
            let set = layout.model.area(area).is_some_and(|a| a.editor != editor)
                && layout.model.set_editor(area, editor);
            if set {
                layout.hosts.insert(editor.to_owned(), area.to_owned());
            }
            set
        })?;
        if set {
            self.emit_area_change(
                layout,
                AreaChange::Editor {
                    area: area.to_owned(),
                },
            )?;
        }
        Ok(set)
    }

    /// Rename the areas for assistive technology, `(editor type id, label)`,
    /// for instance after the interface's language changed; see
    /// [`AreaLayout::labels`]. The layout itself does not change.
    pub fn set_area_labels<E: Into<String>, L: Into<Arc<str>>>(
        &mut self,
        layout: Entity<AreaLayout>,
        labels: impl IntoIterator<Item = (E, L)>,
    ) -> Result<(), FrameworkError> {
        let labels = labels
            .into_iter()
            .map(|(editor, label)| (editor.into(), label.into()))
            .collect();
        self.update_component(layout, |layout, _| layout.labels = labels)
    }

    /// Change the headers' editor type select, or remove it (`None`); see
    /// [`AreaLayout::editor_select`]. The layout itself does not change.
    pub fn set_area_editor_select(
        &mut self,
        layout: Entity<AreaLayout>,
        select: Option<AreaEditorSelect>,
    ) -> Result<(), FrameworkError> {
        self.update_component(layout, |layout, _| layout.editor_select = select)
    }

    /// Show `model` instead, a saved layout or another workspace. Contents
    /// keep their state: an area in both layouts keeps its content, the
    /// others' wait hidden.
    pub fn set_area_layout(
        &mut self,
        layout: Entity<AreaLayout>,
        model: AreaLayoutModel,
    ) -> Result<(), FrameworkError> {
        model.validate().map_err(|_| FrameworkError::InvalidInput)?;
        self.update_component(layout, |layout, _| {
            layout.model = model;
            layout.hosts.clear();
            layout.drag = None;
        })?;
        self.emit_area_change(layout, AreaChange::Replaced)
    }

    /// Build the area tree's nodes, or update them when only ratios, the
    /// maximized area or a drag's preview changed; place each keyed content
    /// and header in its area.
    pub fn assemble_area_layout(
        &mut self,
        layout: Entity<AreaLayout>,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(layout.stable_id())?;
        let snapshot = self.read(layout, Clone::clone)?;
        let shape = shape_of(&snapshot.model.root);
        let alive = snapshot.parts.clone().filter(|parts| {
            [parts.top, parts.preview, parts.stash]
                .iter()
                .all(|id| self.world().contains(*id))
        });
        let created = alive.as_ref().is_none_or(|parts| parts.shape != shape);
        let mut gone = Vec::new();
        let mut parts = match alive {
            Some(parts) if parts.shape == shape => parts,
            old => {
                // areas that stay keep their nodes: a corner holding the
                // pointer must outlive the split it made
                let mut reuse = std::collections::HashMap::new();
                let (preview, stash) = match &old {
                    Some(old) => {
                        for part in &old.nodes {
                            if let AreaPart::Area {
                                id,
                                frame,
                                header,
                                content,
                                corners,
                                select,
                            } = part
                                && snapshot.model.area(id).is_some()
                            {
                                reuse.insert(
                                    id.clone(),
                                    (*frame, *header, *content, *corners, *select),
                                );
                            }
                        }
                        let top_reused = reuse.values().any(|(frame, ..)| *frame == old.top);
                        if !top_reused {
                            gone.push(old.top);
                        }
                        (old.preview, old.stash)
                    }
                    None => (
                        self.create_detached_component(
                            document,
                            AreaBox::new("area-preview", preview_style(None, false)),
                        )?
                        .stable_id(),
                        self.create_detached_component(
                            document,
                            AreaBox::new("area-stash", hidden_style()),
                        )?
                        .stable_id(),
                    ),
                };
                let mut nodes = Vec::new();
                let top = self.build_area_node(
                    document,
                    layout.stable_id(),
                    &snapshot.model.root,
                    &mut Vec::new(),
                    &mut nodes,
                    &mut reuse,
                )?;
                AreaParts {
                    shape,
                    top,
                    nodes,
                    preview,
                    stash,
                }
            }
        };

        self.assemble_editor_selects(layout, document, &snapshot, &mut parts)?;

        // contents into their areas, the rest to wait
        let model = &snapshot.model;
        let hosts = self.editor_hosts(layout, &snapshot, &parts);
        let mut placed = Vec::new();
        for part in &parts.nodes {
            let AreaPart::Area {
                id,
                header,
                content,
                ..
            } = part
            else {
                continue;
            };
            let mut keys = vec![(id.clone(), *content), (format!("{id}:header"), *header)];
            if let Some(area) = model.area(id)
                && hosts.get(area.editor.as_str()) == Some(id)
            {
                let key = editor_key(&area.editor);
                keys.push((format!("{key}:header"), *header));
                keys.push((key, *content));
            }
            for (key, slot) in keys {
                if let Some(child) = self
                    .assembled_child(layout.stable_id(), &key)
                    .filter(|child| self.world().contains(*child))
                {
                    if self.world().parent_id(child) != Some(slot) {
                        self.place_assembled(child, slot)?;
                    }
                    placed.push(child);
                }
            }
        }
        for (_, child) in self.assembled_children(layout.stable_id()) {
            if !placed.contains(&child)
                && self.world().contains(child)
                && self.world().parent_id(child) != Some(parts.stash)
            {
                self.place_assembled(child, parts.stash)?;
            }
        }

        // sizes, the maximized area, the preview
        let maximized = model.maximized.as_deref();
        let mut shown = Vec::new();
        collect_shown(&model.root, maximized, true, 1.0, &mut shown);
        let mut index = 0;
        let gap = snapshot.gap;
        for part in &parts.nodes {
            match part {
                AreaPart::Split {
                    axis,
                    container,
                    bar,
                    ..
                } => {
                    let (visible, grow, both) = shown[index];
                    index += 1;
                    let axis = *axis;
                    self.update_component(
                        Entity::<AreaBox>::from_stable_id(*container),
                        |container, _| container.style = container_style(axis, grow, visible),
                    )?;
                    self.update_component(Entity::<AreaBar>::from_stable_id(*bar), |bar, _| {
                        bar.style = bar_style(axis, gap, visible && both);
                    })?;
                }
                AreaPart::Area {
                    id, frame, header, ..
                } => {
                    let (visible, grow, _) = shown[index];
                    index += 1;
                    let editor = model.area(id).map(|area| {
                        snapshot
                            .labels
                            .get(&area.editor)
                            .cloned()
                            .unwrap_or_else(|| Arc::from(area.editor.as_str()))
                    });
                    self.update_component(
                        Entity::<AreaBox>::from_stable_id(*frame),
                        |frame, _| {
                            frame.style = frame_style(grow, visible, snapshot.chrome);
                            frame.accessibility = AccessibilityState {
                                role: AccessibilityRole::Region,
                                label: editor,
                                ..AccessibilityState::default()
                            };
                        },
                    )?;
                    self.update_component(
                        Entity::<AreaBox>::from_stable_id(*header),
                        |header, _| header.style = header_style(snapshot.chrome),
                    )?;
                }
            }
        }
        let preview = self.area_preview(layout, &snapshot)?;
        self.update_component(Entity::<AreaBox>::from_stable_id(parts.preview), |p, _| {
            p.style = preview_style(
                preview.map(|(rect, _)| rect),
                preview.is_some_and(|(_, j)| j),
            );
        })?;

        self.append_children(layout.stable_id(), &[parts.top, parts.preview, parts.stash])?;
        if !gone.is_empty() {
            let mut mutations = MutationQueue::new();
            for id in gone {
                mutations.despawn_subtree(id);
            }
            self.commit_mutations(mutations)?;
        }
        if snapshot.parts.as_ref() != Some(&parts) {
            parts.shape = shape_of(&model.root);
            let stored = parts;
            self.update_component(layout, |layout, _| layout.parts = Some(stored))?;
        }
        Ok(created)
    }

    /// Build the nodes for `node`; answers its root.
    fn build_area_node(
        &mut self,
        document: crate::DocumentId,
        layout: StableNodeId,
        node: &AreaNode,
        path: &mut Vec<bool>,
        out: &mut Vec<AreaPart>,
        reuse: &mut ReusedAreas,
    ) -> Result<StableNodeId, FrameworkError> {
        match node {
            AreaNode::Area { id } => {
                if let Some((frame, header, content, corners, select)) = reuse.remove(id) {
                    out.push(AreaPart::Area {
                        id: id.clone(),
                        frame,
                        header,
                        content,
                        corners,
                        select,
                    });
                    return Ok(frame);
                }
                let frame = self
                    .create_detached_component(
                        document,
                        AreaBox::new("area", frame_style(1.0, true, AreaChrome::default())),
                    )?
                    .stable_id();
                let header = self
                    .create_detached_component(
                        document,
                        AreaBox::new("area-header", header_style(AreaChrome::default())),
                    )?
                    .stable_id();
                let content = self
                    .create_detached_component(
                        document,
                        AreaBox::new("area-content", content_style()),
                    )?
                    .stable_id();
                let mut corners = [frame; 2];
                for (index, corner) in corners.iter_mut().enumerate() {
                    *corner = self
                        .create_detached_component(
                            document,
                            AreaCorner {
                                layout,
                                area: id.clone(),
                                style: corner_style(index == 1),
                            },
                        )?
                        .stable_id();
                }
                self.append_children(frame, &[header, content, corners[0], corners[1]])?;
                out.push(AreaPart::Area {
                    id: id.clone(),
                    frame,
                    header,
                    content,
                    corners,
                    select: None,
                });
                Ok(frame)
            }
            AreaNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                let container = self
                    .create_detached_component(
                        document,
                        AreaBox::new("area-split", container_style(*axis, 1.0, true)),
                    )?
                    .stable_id();
                let bar = self
                    .create_detached_component(
                        document,
                        AreaBar {
                            layout,
                            path: path.clone(),
                            axis: *axis,
                            style: NodeStyle::default(),
                            dragging: false,
                        },
                    )?
                    .stable_id();
                let at = out.len();
                out.push(AreaPart::Split {
                    path: path.clone(),
                    axis: *axis,
                    container,
                    bar,
                    first: container,
                    second: container,
                });
                path.push(false);
                let one = self.build_area_node(document, layout, first, path, out, reuse)?;
                path.pop();
                path.push(true);
                let two = self.build_area_node(document, layout, second, path, out, reuse)?;
                path.pop();
                if let AreaPart::Split { first, second, .. } = &mut out[at] {
                    *first = one;
                    *second = two;
                }
                self.append_children(container, &[one, bar, two])?;
                Ok(container)
            }
        }
    }

    /// What the preview shows for the drag in progress, in the layout's own
    /// px, and whether it is a join.
    fn area_preview(
        &self,
        layout: Entity<AreaLayout>,
        snapshot: &AreaLayout,
    ) -> Result<Option<(LayoutBox, bool)>, FrameworkError> {
        let Some(drag) = &snapshot.drag else {
            return Ok(None);
        };
        let Some(action) = &drag.action else {
            return Ok(None);
        };
        let Some(origin) = self.world().component_layout_box(layout.stable_id()) else {
            return Ok(None);
        };
        let boxes = self.area_boxes(layout)?;
        let local = |b: LayoutBox| LayoutBox {
            x: b.x - origin.x,
            y: b.y - origin.y,
            ..b
        };
        Ok(match action {
            CornerAction::Split { axis, ratio } => {
                boxes.iter().find(|(id, _)| *id == drag.area).map(|(_, b)| {
                    let line = match axis {
                        AreaAxis::Horizontal => LayoutBox {
                            x: b.x + b.width * ratio - PREVIEW_LINE / 2.0,
                            width: PREVIEW_LINE,
                            ..*b
                        },
                        AreaAxis::Vertical => LayoutBox {
                            y: b.y + b.height * ratio - PREVIEW_LINE / 2.0,
                            height: PREVIEW_LINE,
                            ..*b
                        },
                    };
                    (local(line), false)
                })
            }
            CornerAction::Join { remove } => boxes
                .iter()
                .find(|(id, _)| id == remove)
                .map(|(_, b)| (local(*b), true)),
        })
    }

    /// Give each header its editor type select (or take it away), showing
    /// the area's editor among the layout's choices.
    fn assemble_editor_selects(
        &mut self,
        layout: Entity<AreaLayout>,
        document: crate::DocumentId,
        snapshot: &AreaLayout,
        parts: &mut AreaParts,
    ) -> Result<(), FrameworkError> {
        let name = |editor: &str| {
            snapshot
                .labels
                .get(editor)
                .cloned()
                .unwrap_or_else(|| Arc::from(editor))
        };
        for part in &mut parts.nodes {
            let AreaPart::Area {
                id, header, select, ..
            } = part
            else {
                continue;
            };
            let Some(choice) = &snapshot.editor_select else {
                if let Some(node) = select.take() {
                    let mut mutations = MutationQueue::new();
                    mutations.despawn_subtree(node);
                    self.commit_mutations(mutations)?;
                }
                continue;
            };
            let node = match *select {
                Some(node) if self.world().contains(node) => node,
                _ => {
                    let mut field = Select::new(None::<&str>)
                        .size(nana_ui_core::ControlSize::Small)
                        .fit_options(true);
                    {
                        let layout = Arc::make_mut(&mut field.style.layout);
                        layout.width = Some(LengthSpec::Shrink);
                        layout.flex_shrink = Some(0.0);
                    }
                    let field = self.create_detached_component(document, field)?;
                    // first in the header, before the application's controls
                    let first = self
                        .world()
                        .node(*header)
                        .and_then(|node| node.children.first().copied());
                    let mut mutations = MutationQueue::new();
                    mutations.insert(*header, field.stable_id(), first);
                    self.commit_mutations(mutations)?;
                    let area = id.clone();
                    self.observe(field, layout, move |layout, e: &SelectChanged, cx| {
                        let editor = e.value.to_string();
                        if layout.model.area(&area).is_none_or(|a| a.editor == editor) {
                            return;
                        }
                        layout.model.set_editor(&area, editor.clone());
                        layout.hosts.insert(editor, area.clone());
                        cx.emit(AreaLayoutChanged {
                            change: AreaChange::Editor { area: area.clone() },
                            model: layout.model.clone(),
                        });
                        cx.reassemble();
                    })?;
                    *select = Some(field.stable_id());
                    field.stable_id()
                }
            };
            let editor = snapshot.model.area(id).map(|area| area.editor.clone());
            let mut options: Vec<SelectOption> = choice
                .editors
                .iter()
                .map(|e| SelectOption::new(e.as_str(), name(e)))
                .collect();
            // an editor the choices leave out still shows as the value
            if let Some(editor) = &editor
                && !choice.editors.contains(editor)
            {
                options.push(SelectOption::new(editor.as_str(), name(editor)));
            }
            let value = editor.map(Arc::<str>::from);
            let label = choice
                .label
                .clone()
                .unwrap_or_else(|| Arc::clone(&self.world().framework_strings().area_editor_type));
            let field = Entity::<Select>::from_stable_id(node);
            let stale = self.read(field, |field| {
                field.options != options
                    || field.value != value
                    || field.label.as_ref() != Some(&label)
            })?;
            if stale {
                self.update_component(field, |field, _| {
                    field.options = options;
                    field.value = value;
                    field.label = Some(label);
                })?;
            }
        }
        Ok(())
    }

    /// The area each `editor:` content goes to, by editor type: one area per
    /// type, the one it was last chosen in, else the one holding it, else
    /// the first; a shown area first. An area with a content keyed by its own
    /// id takes none.
    fn editor_hosts(
        &self,
        layout: Entity<AreaLayout>,
        snapshot: &AreaLayout,
        parts: &AreaParts,
    ) -> std::collections::HashMap<String, String> {
        let keyed = |key: &str| {
            self.assembled_child(layout.stable_id(), key)
                .filter(|child| self.world().contains(*child))
        };
        let model = &snapshot.model;
        let areas: Vec<(&str, &str, StableNodeId, StableNodeId)> = parts
            .nodes
            .iter()
            .filter_map(|part| match part {
                AreaPart::Area {
                    id,
                    header,
                    content,
                    ..
                } => model
                    .area(id)
                    .map(|area| (id.as_str(), area.editor.as_str(), *header, *content)),
                AreaPart::Split { .. } => None,
            })
            .collect();
        let shown = |id: &str| model.maximized.as_deref().is_none_or(|max| max == id);
        let mut hosts = std::collections::HashMap::new();
        for &(_, editor, ..) in &areas {
            if hosts.contains_key(editor) {
                continue;
            }
            let key = editor_key(editor);
            let Some(child) = keyed(&key).or_else(|| keyed(&format!("{key}:header"))) else {
                continue;
            };
            let candidates: Vec<&str> = areas
                .iter()
                .filter(|(id, e, ..)| *e == editor && keyed(id).is_none())
                .map(|(id, ..)| *id)
                .collect();
            let holder = self.world().parent_id(child).and_then(|slot| {
                areas
                    .iter()
                    .find(|(_, _, header, content)| slot == *header || slot == *content)
                    .map(|(id, ..)| *id)
            });
            let preferred: Vec<&str> = snapshot
                .hosts
                .get(editor)
                .map(String::as_str)
                .into_iter()
                .chain(holder)
                .chain(candidates.iter().copied())
                .filter(|id| candidates.contains(id))
                .collect();
            if let Some(host) = preferred.iter().find(|id| shown(id)).or(preferred.first()) {
                hosts.insert(editor.to_owned(), (*host).to_owned());
            }
        }
        hosts
    }
}

/// An area's frame, header, content, corners and editor select, by id.
type ReusedAreas = std::collections::HashMap<
    String,
    (
        StableNodeId,
        StableNodeId,
        StableNodeId,
        [StableNodeId; 2],
        Option<StableNodeId>,
    ),
>;

/// For each tree node in build order: whether it shows, how much it grows
/// within its parent, and (for a split) whether both its sides show.
fn collect_shown(
    node: &AreaNode,
    maximized: Option<&str>,
    visible: bool,
    grow: f32,
    out: &mut Vec<(bool, f32, bool)>,
) {
    match node {
        AreaNode::Area { .. } => out.push((visible, grow, false)),
        AreaNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            let one = visible && holds(first, maximized);
            let two = visible && holds(second, maximized);
            out.push((visible, grow, one && two));
            let (g1, g2) = if one && two {
                (*ratio, 1.0 - *ratio)
            } else {
                (1.0, 1.0)
            };
            collect_shown(first, maximized, one, g1, out);
            collect_shown(second, maximized, two, g2, out);
        }
    }
}
