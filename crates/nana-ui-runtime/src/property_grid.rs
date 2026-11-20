//! Property grid (#343): the dense two-column panel of professional tools'
//! inspectors. Rows sit in collapsible sections; each row is a label on the
//! left and the application's control on the right. The label column is one
//! width for the whole grid and drags.
//!
//! Controls come from the view: a child keyed with a row's id is that row's
//! control (a [`crate::ScrubField`], a select, a colour field). A row can be
//! [`PropertyRow::mixed`] — several objects disagree; the grid marks the
//! label and the control shows its own mixed state — or
//! [`PropertyRow::disabled`]: its control takes no pointer and the row shows
//! why.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, CursorSpec, DisplaySpec, FlexDirection, LengthSpec, OverflowSpec,
    PositionSpec, SemanticColorRole,
};

use crate::view_components::{Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, MutationQueue, NodeKind, NodePointer, NodePointerHooks, NodeStyle,
    StableNodeId, UiWorld,
};

/// The least the label column or the controls keep, px.
const MIN_COLUMN: f32 = 48.0;
/// The column divider's grab width, px.
const GRIP: f32 = nana_ui_core::space::SM;

/// One property: a label and the control keyed with `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyRow {
    pub id: Arc<str>,
    pub label: Arc<str>,
    /// The objects being edited disagree: the label is marked; the control
    /// shows its own mixed state.
    pub mixed: bool,
    /// The control takes no pointer.
    pub disabled: bool,
    /// Why it is disabled, shown under the row.
    pub reason: Option<Arc<str>>,
}

impl PropertyRow {
    pub fn new(id: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            mixed: false,
            disabled: false,
            reason: None,
        }
    }

    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// Disable the row; `reason` is shown under it.
    pub fn disabled(mut self, reason: Option<impl Into<Arc<str>>>) -> Self {
        self.disabled = true;
        self.reason = reason.map(Into::into);
        self
    }
}

/// A titled group of rows that folds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySection {
    pub id: Arc<str>,
    pub title: Arc<str>,
    pub rows: Vec<PropertyRow>,
    pub collapsed: bool,
}

impl PropertySection {
    pub fn new(
        id: impl Into<Arc<str>>,
        title: impl Into<Arc<str>>,
        rows: impl IntoIterator<Item = PropertyRow>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            rows: rows.into_iter().collect(),
            collapsed: false,
        }
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

/// A section folded or opened; keep it to restore the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySectionToggled {
    pub id: Arc<str>,
    pub collapsed: bool,
}

/// The label column was dragged to `width`; keep it to restore the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropertyLabelWidthChanged {
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowParts {
    id: Arc<str>,
    pub(crate) row: StableNodeId,
    pub(crate) label: StableNodeId,
    pub(crate) slot: StableNodeId,
    pub(crate) veil: StableNodeId,
    pub(crate) reason: StableNodeId,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SectionParts {
    id: Arc<str>,
    pub(crate) header: StableNodeId,
    pub(crate) mark: StableNodeId,
    pub(crate) title: StableNodeId,
    pub(crate) body: StableNodeId,
    pub(crate) rows: Vec<RowParts>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridParts {
    pub(crate) sections: Vec<SectionParts>,
    pub(crate) divider: StableNodeId,
    /// Where keyed controls of rows no longer shown wait.
    pub(crate) stash: StableNodeId,
}

/// The two-column property panel; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyGrid {
    pub sections: Vec<PropertySection>,
    /// The label column's width, px.
    pub label_width: f32,
    pub style: NodeStyle,
    pub(crate) dragging: bool,
    pub(crate) parts: Option<GridParts>,
}

impl PropertyGrid {
    pub fn new(sections: impl IntoIterator<Item = PropertySection>) -> Self {
        Self {
            sections: sections.into_iter().collect(),
            label_width: 96.0,
            style: grid_style(),
            dragging: false,
            parts: None,
        }
    }

    /// Replaces the node style wholesale.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label_width(mut self, width: f32) -> Self {
        self.label_width = width.max(MIN_COLUMN);
        self
    }

    /// Which sections are folded, to save with the panel.
    pub fn collapsed_sections(&self) -> Vec<Arc<str>> {
        self.sections
            .iter()
            .filter(|section| section.collapsed)
            .map(|section| Arc::clone(&section.id))
            .collect()
    }
}

impl ComponentView for PropertyGrid {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::assemble_property_grid),
        slot_assembler: Some(AppContext::assemble_property_grid),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // folding and the column width are the user's once they changed them
        for section in &mut next.sections {
            if let Some(current) = self.sections.iter().find(|s| s.id == section.id) {
                section.collapsed = current.collapsed;
            }
        }
        next.label_width = self.label_width;
        next.dragging = self.dragging;
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
            tag: "property-grid".into(),
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
            AccessibilityState {
                role: AccessibilityRole::Form,
                ..AccessibilityState::default()
            },
        );
    }
}

/// A header, a row, a slot or a veil: a styled box the grid keeps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridBox {
    tag: &'static str,
    pub(crate) style: NodeStyle,
    accessibility: AccessibilityState,
    /// Takes the pointer (a disabled row's veil) without acting on it.
    blocks: bool,
    /// Whether this control slot dims to the active theme's `opacity().disabled`.
    disabled_opacity: bool,
}

impl GridBox {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            style: NodeStyle::default(),
            accessibility: AccessibilityState::default(),
            blocks: false,
            disabled_opacity: false,
        }
    }
}

impl ComponentView for GridBox {
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: self.tag.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.style.clone();
        if self.disabled_opacity {
            Arc::make_mut(&mut style.layout).opacity =
                Some(world.node_theme(id).opacity().disabled);
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: self.blocks,
                focusable: false,
            },
            self.accessibility.clone(),
        );
    }
}

/// A section header: a press folds or opens the section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridHeader {
    grid: StableNodeId,
    section: Arc<str>,
    style: NodeStyle,
    collapsed: bool,
    title: Arc<str>,
}

impl ComponentView for GridHeader {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(|cx, header| {
            let (grid, section) = cx.read(header, |h| (h.grid, Arc::clone(&h.section)))?;
            let Some(grid) = cx.view_entity::<PropertyGrid>(grid) else {
                return Ok(false);
            };
            cx.toggle_property_section(grid, &section)
        }),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "property-section".into(),
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
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.title)),
                selected: Some(!self.collapsed),
                ..AccessibilityState::default()
            },
        );
    }
}

/// The label column's edge: dragging it sets the width for every row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridDivider {
    grid: StableNodeId,
    style: NodeStyle,
}

impl ComponentView for GridDivider {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&DIVIDER_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "property-divider".into(),
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

const DIVIDER_POINTER: NodePointerHooks<GridDivider> = NodePointerHooks {
    moved: divider_moved,
    left: |_, _| Ok(()),
    pressed: |cx, divider, pointer| {
        if pointer.button != 0 {
            return Ok(false);
        }
        let grid = cx.read(divider, |d| d.grid)?;
        if let Some(grid) = cx.view_entity::<PropertyGrid>(grid) {
            cx.update_component(grid, |grid, _| grid.dragging = true)?;
        }
        Ok(true)
    },
    released: |cx, divider, _| {
        let grid = cx.read(divider, |d| d.grid)?;
        if let Some(grid) = cx.view_entity::<PropertyGrid>(grid) {
            cx.update_component(grid, |grid, view| {
                grid.dragging = false;
                view.emit(PropertyLabelWidthChanged {
                    width: grid.label_width,
                });
            })?;
        }
        Ok(true)
    },
    wheel: |_, _, _| Ok(false),
};

fn divider_moved(
    cx: &mut AppContext,
    divider: Entity<GridDivider>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    let grid = cx.read(divider, |d| d.grid)?;
    let (Some(grid), Some(own), Some(bounds)) = (
        cx.view_entity::<PropertyGrid>(grid),
        cx.world().component_layout_box(divider.stable_id()),
        cx.world().component_layout_box(grid),
    ) else {
        return Ok(false);
    };
    let x = pointer.x + own.x - bounds.x;
    let width = x.clamp(MIN_COLUMN, (bounds.width - MIN_COLUMN).max(MIN_COLUMN));
    cx.update_component(grid, |grid, _| grid.label_width = width)?;
    Ok(true)
}

fn grid_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Relative;
    layout.direction = Some(FlexDirection::Column);
    layout.width = Some(LengthSpec::Fill);
    layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XXS));
    style
}

fn header_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.align_self = Some(AlignSpec::Stretch);
        layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.padding_left = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.cursor = Some(CursorSpec::Pointer);
    }
    style.background = Some(SemanticColorRole::Subtle);
    style.radius = Some(nana_ui_core::RadiusTier::Sm);
    style.control_height = Some(nana_ui_core::ControlHeight::Exact(ControlSize::Small));
    style.interaction.hovered.background = Some(SemanticColorRole::Hover);
    style
}

fn body_style(collapsed: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.align_self = Some(AlignSpec::Stretch);
    layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XXS));
    layout.display = collapsed.then_some(DisplaySpec::None);
    style
}

fn row_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Relative;
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.align_self = Some(AlignSpec::Stretch);
        layout.flex_wrap = nana_ui_core::FlexWrap::Wrap;
        layout.min_height = Some(LengthSpec::Px(0.0));
    }
    style.control_height = Some(nana_ui_core::ControlHeight::Min(ControlSize::Small));
    style
}

fn label_style(width: f32, mixed: bool, disabled: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.flex_shrink = Some(0.0);
        layout.padding_left = Some(LengthSpec::Px(nana_ui_core::space::SM));
        layout.padding_right = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.overflow_x = OverflowSpec::Hidden;
        layout.font_size = Some(ControlSize::Small.text_size());
    }
    style.foreground = Some(if disabled {
        SemanticColorRole::Faint
    } else if mixed {
        SemanticColorRole::Text
    } else {
        SemanticColorRole::Muted
    });
    style
}

fn slot_style(disabled: bool, disabled_opacity: f32) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Relative;
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.flex_grow = Some(1.0);
        layout.flex_basis = Some(LengthSpec::Px(0.0));
        layout.min_width = Some(LengthSpec::Px(0.0));
        if disabled {
            layout.opacity = Some(disabled_opacity);
        }
    }
    style
}

fn veil_style(shown: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.offset_left = Some(LengthSpec::Px(0.0));
    layout.offset_top = Some(LengthSpec::Px(0.0));
    layout.width = Some(LengthSpec::Percent(100.0));
    layout.height = Some(LengthSpec::Percent(100.0));
    layout.z_index = Some(1);
    layout.cursor = Some(CursorSpec::NotAllowed);
    layout.display = (!shown).then_some(DisplaySpec::None);
    style
}

fn reason_style(width: f32, shown: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.padding_left = Some(LengthSpec::Px(width));
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Hint);
        layout.display = (!shown).then_some(DisplaySpec::None);
    }
    style.foreground = Some(SemanticColorRole::Faint);
    style
}

fn divider_style(x: f32) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_left = Some(LengthSpec::Px(x - GRIP / 2.0));
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.width = Some(LengthSpec::Px(GRIP));
        layout.height = Some(LengthSpec::Percent(100.0));
        layout.cursor = Some(CursorSpec::EwResize);
        layout.z_index = Some(2);
    }
    style.interaction.hovered.background = Some(SemanticColorRole::BorderStrong);
    style
}

impl AppContext {
    /// Fold or open `section`.
    pub fn toggle_property_section(
        &mut self,
        grid: Entity<PropertyGrid>,
        section: &str,
    ) -> Result<bool, FrameworkError> {
        self.update_component(grid, |grid, view| {
            let Some(target) = grid.sections.iter_mut().find(|s| s.id.as_ref() == section) else {
                return false;
            };
            target.collapsed = !target.collapsed;
            view.emit(PropertySectionToggled {
                id: Arc::clone(&target.id),
                collapsed: target.collapsed,
            });
            true
        })
    }

    /// Build or update the sections, rows and divider; place each keyed
    /// control in its row.
    pub fn assemble_property_grid(
        &mut self,
        grid: Entity<PropertyGrid>,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(grid.stable_id())?;
        let snapshot = self.read(grid, Clone::clone)?;
        let shape = snapshot
            .sections
            .iter()
            .map(|section| {
                (
                    Arc::clone(&section.id),
                    section
                        .rows
                        .iter()
                        .map(|row| Arc::clone(&row.id))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let alive = snapshot.parts.clone().filter(|parts| {
            self.world().contains(parts.divider)
                && self.world().contains(parts.stash)
                && parts.sections.len() == shape.len()
                && parts
                    .sections
                    .iter()
                    .zip(&shape)
                    .all(|(built, (id, rows))| {
                        built.id == *id
                            && built.rows.len() == rows.len()
                            && built.rows.iter().zip(rows).all(|(row, id)| row.id == *id)
                    })
        });
        let created = alive.is_none();
        let mut gone = Vec::new();
        let parts = match alive {
            Some(parts) => parts,
            None => {
                // controls move out of the old rows before those go
                if let Some(old) = &snapshot.parts {
                    gone.extend(old.sections.iter().map(|section| section.header));
                    gone.extend(old.sections.iter().map(|section| section.body));
                }
                let divider = match &snapshot.parts {
                    Some(old) if self.world().contains(old.divider) => old.divider,
                    _ => self
                        .create_detached_component(
                            document,
                            GridDivider {
                                grid: grid.stable_id(),
                                style: NodeStyle::default(),
                            },
                        )?
                        .stable_id(),
                };
                let stash = match &snapshot.parts {
                    Some(old) if self.world().contains(old.stash) => old.stash,
                    _ => {
                        let mut stash = GridBox::new("property-stash");
                        Arc::make_mut(&mut stash.style.layout).display = Some(DisplaySpec::None);
                        self.create_detached_component(document, stash)?.stable_id()
                    }
                };
                let mut sections = Vec::with_capacity(shape.len());
                for section in &snapshot.sections {
                    sections.push(self.build_property_section(document, grid, section)?);
                }
                GridParts {
                    sections,
                    divider,
                    stash,
                }
            }
        };

        // contents and state
        let width = snapshot.label_width;
        for (section, built) in snapshot.sections.iter().zip(&parts.sections) {
            let (collapsed, title) = (section.collapsed, Arc::clone(&section.title));
            self.update_component(
                Entity::<GridHeader>::from_stable_id(built.header),
                |header, _| {
                    header.collapsed = collapsed;
                    header.title = Arc::clone(&title);
                },
            )?;
            let mark = if collapsed { "▸" } else { "▾" };
            self.update_component(Entity::<Text>::from_stable_id(built.mark), |text, _| {
                text.value = mark.to_owned();
            })?;
            let title_text = section.title.to_string();
            self.update_component(Entity::<Text>::from_stable_id(built.title), |text, _| {
                text.value = title_text;
            })?;
            self.update_component(Entity::<GridBox>::from_stable_id(built.body), |body, _| {
                body.style = body_style(collapsed);
            })?;
            for (row, parts) in section.rows.iter().zip(&built.rows) {
                let label = row.label.to_string();
                let (mixed, disabled) = (row.mixed, row.disabled);
                self.update_component(Entity::<Text>::from_stable_id(parts.label), |text, _| {
                    text.value = label;
                    text.style = label_style(width, mixed, disabled);
                })?;
                let disabled_opacity = self.world().node_theme(parts.slot).opacity().disabled;
                self.update_component(Entity::<GridBox>::from_stable_id(parts.slot), |slot, _| {
                    slot.style = slot_style(disabled, disabled_opacity);
                    slot.disabled_opacity = disabled;
                })?;
                self.update_component(Entity::<GridBox>::from_stable_id(parts.veil), |veil, _| {
                    veil.style = veil_style(disabled);
                    veil.blocks = disabled;
                })?;
                let reason = row.reason.as_deref().unwrap_or("").to_owned();
                let shown = disabled && !reason.is_empty();
                self.update_component(Entity::<Text>::from_stable_id(parts.reason), |text, _| {
                    text.value = reason;
                    text.style = reason_style(width, shown);
                })?;
                let (spoken, description) = (
                    Arc::clone(&row.label),
                    if mixed {
                        Some(Arc::from("mixed values"))
                    } else {
                        row.reason.clone().filter(|_| disabled)
                    },
                );
                self.update_component(Entity::<GridBox>::from_stable_id(parts.row), |box_, _| {
                    box_.accessibility = AccessibilityState {
                        role: AccessibilityRole::Generic,
                        label: Some(spoken),
                        description,
                        disabled,
                        mixed,
                        ..AccessibilityState::default()
                    };
                })?;
                // the application's control, keyed with the row's id
                if let Some(control) = self
                    .assembled_child(grid.stable_id(), &row.id)
                    .filter(|control| self.world().contains(*control))
                    && self.world().parent_id(control) != Some(parts.slot)
                {
                    self.place_assembled(control, parts.slot)?;
                }
            }
        }
        self.update_component(
            Entity::<GridDivider>::from_stable_id(parts.divider),
            |divider, _| divider.style = divider_style(width),
        )?;
        // controls whose row is gone wait hidden, so the old rows can go
        let shown = parts
            .sections
            .iter()
            .flat_map(|section| section.rows.iter().map(|row| row.slot))
            .collect::<Vec<_>>();
        for (_, control) in self.assembled_children(grid.stable_id()) {
            if self.world().contains(control)
                && self
                    .world()
                    .parent_id(control)
                    .is_none_or(|parent| !shown.contains(&parent) && parent != parts.stash)
            {
                self.place_assembled(control, parts.stash)?;
            }
        }

        if !gone.is_empty() {
            let mut mutations = MutationQueue::new();
            for id in gone {
                mutations.despawn_subtree(id);
            }
            self.commit_mutations(mutations)?;
        }
        if snapshot.parts.as_ref() != Some(&parts) {
            let stored = parts.clone();
            self.update_component(grid, |grid, _| grid.parts = Some(stored))?;
        }
        let mut children = Vec::new();
        for section in &parts.sections {
            children.push(section.header);
            children.push(section.body);
        }
        children.push(parts.divider);
        children.push(parts.stash);
        self.append_children(grid.stable_id(), &children)?;
        Ok(created)
    }

    fn build_property_section(
        &mut self,
        document: crate::DocumentId,
        grid: Entity<PropertyGrid>,
        section: &PropertySection,
    ) -> Result<SectionParts, FrameworkError> {
        let header = self
            .create_detached_component(
                document,
                GridHeader {
                    grid: grid.stable_id(),
                    section: Arc::clone(&section.id),
                    style: header_style(),
                    collapsed: section.collapsed,
                    title: Arc::clone(&section.title),
                },
            )?
            .stable_id();
        let mut mark = Text::new("");
        mark.decorative = true;
        mark.style.foreground = Some(SemanticColorRole::Muted);
        let mark = self.create_detached_component(document, mark)?.stable_id();
        let mut title = Text::new("");
        title.decorative = true;
        title.style.type_weight = Some(nana_ui_core::TextWeight::Semibold);
        let title = self.create_detached_component(document, title)?.stable_id();
        self.append_children(header, &[mark, title])?;
        let body = self
            .create_detached_component(document, GridBox::new("property-section-body"))?
            .stable_id();
        let mut rows = Vec::with_capacity(section.rows.len());
        for row in &section.rows {
            let mut box_ = GridBox::new("property-row");
            box_.style = row_style();
            let row_node = self.create_detached_component(document, box_)?.stable_id();
            let mut label = Text::new("");
            label.decorative = true;
            let label = self.create_detached_component(document, label)?.stable_id();
            let slot = self
                .create_detached_component(document, GridBox::new("property-control"))?
                .stable_id();
            let veil = self
                .create_detached_component(document, GridBox::new("property-veil"))?
                .stable_id();
            let mut reason = Text::new("");
            reason.decorative = true;
            let reason = self
                .create_detached_component(document, reason)?
                .stable_id();
            self.append_children(slot, &[veil])?;
            self.append_children(row_node, &[label, slot, reason])?;
            rows.push(RowParts {
                id: Arc::clone(&row.id),
                row: row_node,
                label,
                slot,
                veil,
                reason,
            });
        }
        let ids = rows.iter().map(|row| row.row).collect::<Vec<_>>();
        self.append_children(body, &ids)?;
        Ok(SectionParts {
            id: Arc::clone(&section.id),
            header,
            mark,
            title,
            body,
            rows,
        })
    }
}
