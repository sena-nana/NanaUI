use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, CardKind, ControlSize, FlexDirection, Icon, LengthSpec, SemanticColorRole,
};

use super::row::{HintPlacement, SettingsRow};
use super::{document_of, reconcile_children};
use crate::view_components::{Card, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, InteractionStyle, MutationQueue, NodeKind, NodeStyle, SemanticPaint,
    StableNodeId, UiWorld,
};

/// Surface grouping for settings rows.
///
/// [`Self::hint_placement`](field@Self::hint_placement) says where the hints of every
/// [`SettingsRow`] in the card go, through the groups inside, as on a
/// [`SettingsGroup`]: a card of inspector groups sets it once.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCard {
    pub title: Arc<str>,
    pub style: NodeStyle,
    /// Where the hints of the rows inside go; see
    /// [`SettingsGroup::hint_placement`](field@SettingsGroup::hint_placement).
    pub hint_placement: Option<HintPlacement>,
}

impl SettingsCard {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            style: NodeStyle::default(),
            hint_placement: None,
        }
    }

    /// Put the hints of the rows inside where `placement` says.
    pub fn hint_placement(mut self, placement: HintPlacement) -> Self {
        self.hint_placement = Some(placement);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        if !self.title.is_empty() {
            card = card.title(Arc::clone(&self.title));
        }
        let mut style = self.style.clone();
        Arc::make_mut(&mut style.layout)
            .width
            .get_or_insert(LengthSpec::Fill);
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCard {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SETTINGS_CARD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        lifecycle: Some(sync_settings_card),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
    }
}

/// A column of settings rows and other content whose group edges follow what
/// is shown.
///
/// The first shown member of the group gets the group's edge air above it,
/// the last one below it, and a divided row draws no divider under the last
/// shown member — however the rows are shown or hidden. A [`SettingsRow`]
/// directly in the group has its `first_in_group` / `last_in_group` written
/// by the group; any other member (a badge, a value list, a row of buttons)
/// takes its place in the order without being written. The container of a
/// keyed list or a conditional block is read through, so its rows are
/// members in their own right. A [`crate::Divider`] member is a separator:
/// it shows only between two shown members.
///
/// [`Self::hint_placement`](field@Self::hint_placement) says where the hints of the rows inside go, for
/// every [`SettingsRow`] below the group (through any container, a nested
/// group or card that sets its own aside); a row's own placement wins.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsGroup {
    pub style: NodeStyle,
    /// Where the hints of the rows inside go; `None` leaves it to the group
    /// or card around this one, and to [`HintPlacement::Below`] when none
    /// says.
    pub hint_placement: Option<HintPlacement>,
}

impl Default for SettingsGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsGroup {
    pub fn new() -> Self {
        let mut style = NodeStyle::default();
        let layout = style.layout_mut();
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        Self {
            style,
            hint_placement: None,
        }
    }

    /// Put the hints of the rows inside where `placement` says; see
    /// [`Self::hint_placement`](field@Self::hint_placement).
    pub fn hint_placement(mut self, placement: HintPlacement) -> Self {
        self.hint_placement = Some(placement);
        self
    }

    /// Space between members.
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.layout_mut().gap = Some(LengthSpec::Px(gap.max(0.0)));
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for SettingsGroup {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        lifecycle: Some(sync_settings_group),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-group".into(),
        }
    }

    /// A member shown or hidden moves the group's edges.
    fn wants_child_reproject() -> bool {
        true
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

/// Whether `id` takes part in layout: neither hidden nor `display: none`.
pub(super) fn generates_box(world: &UiWorld, id: StableNodeId) -> bool {
    world.node_style(id).is_some_and(|style| {
        !style.layout.hidden && style.layout.display != Some(nana_ui_core::DisplaySpec::None)
    })
}

/// A group's members in order, read through structural containers, which
/// land in `containers`. A separator is listed whether it shows or not: the
/// group decides that.
fn group_members(
    context: &AppContext,
    node: StableNodeId,
    members: &mut Vec<(StableNodeId, bool)>,
    containers: &mut Vec<StableNodeId>,
) {
    let Some(snapshot) = context.world().node(node) else {
        return;
    };
    for child in snapshot.children {
        let separator = context
            .read(Entity::<crate::Divider>::from_stable_id(child), |_| ())
            .is_ok();
        if separator {
            members.push((child, true));
            continue;
        }
        if !generates_box(context.world(), child) {
            continue;
        }
        let structural = context
            .read(Entity::<crate::Stack>::from_stable_id(child), |stack| {
                stack.is_structural()
            })
            .unwrap_or(false);
        if structural {
            containers.push(child);
            group_members(context, child, members, containers);
        } else {
            members.push((child, false));
        }
    }
}

/// A row shown or hidden notifies its group (through any list or conditional
/// block container) so group edges update cleanly.
pub(super) fn sync_row_group(
    context: &mut AppContext,
    row: Entity<SettingsRow>,
) -> Result<(), FrameworkError> {
    let mut node = row.stable_id();
    while let Some(parent) = context.world().parent_id(node) {
        let group = Entity::<SettingsGroup>::from_stable_id(parent);
        if context.read(group, |_| ()).is_ok() {
            return sync_settings_group(context, group);
        }
        let structural = context
            .read(Entity::<crate::Stack>::from_stable_id(parent), |stack| {
                stack.is_structural()
            })
            .unwrap_or(false);
        if !structural {
            break;
        }
        node = parent;
    }
    Ok(())
}

fn sync_settings_group(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    if context.defer_post_flush(group.stable_id(), mark_settings_group_node) {
        return Ok(());
    }
    mark_settings_group(context, group)
}

fn mark_settings_group_node(
    context: &mut AppContext,
    group: StableNodeId,
) -> Result<(), FrameworkError> {
    mark_settings_group(context, Entity::from_stable_id(group))
}

/// Mark `group`'s edges now.
pub(crate) fn mark_settings_group(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    mark_group_edges(context, group)
}

fn mark_group_edges(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    let mut members = Vec::new();
    let mut containers = Vec::new();
    group_members(context, group.stable_id(), &mut members, &mut containers);
    // Rows inside a list or a conditional block hide on their container,
    // not on the group, so the group watches those containers too.
    context.watch_child_structures(group.stable_id(), &containers);

    let shown: Vec<StableNodeId> = members
        .iter()
        .filter(|(_, separator)| !separator)
        .map(|(id, _)| *id)
        .collect();
    let (first, last) = (shown.first().copied(), shown.last().copied());
    // A row under a shown member that is not a settings row rules its top
    // edge: a settings row's edge is lined whichever side the other content
    // is on (docs/guide/composition.md「分隔线」).
    let is_row = |id: StableNodeId| {
        context
            .read(Entity::<SettingsRow>::from_stable_id(id), |_| ())
            .is_ok()
    };
    let after_other: HashSet<StableNodeId> = shown
        .windows(2)
        .filter(|pair| !is_row(pair[0]) && is_row(pair[1]))
        .map(|pair| pair[1])
        .collect();
    for (index, &(id, separator)) in members.iter().enumerate() {
        if separator {
            let before = members[..index].iter().any(|(_, separator)| !separator);
            let after = members[index + 1..].iter().any(|(_, separator)| !separator);
            // Of two separators with nothing shown between them, the later
            // one stands.
            let next_is_separator = members
                .get(index + 1)
                .is_some_and(|(_, separator)| *separator);
            let hidden = !(before && after) || next_is_separator;
            let divider = Entity::<crate::Divider>::from_stable_id(id);
            let current = context.read(divider, |divider| divider.style.layout.hidden);
            if current.is_ok_and(|current| current != hidden) {
                context.update_component(divider, |divider, _| {
                    divider.style.layout_mut().hidden = hidden;
                })?;
            }
            continue;
        }
        let row = Entity::<SettingsRow>::from_stable_id(id);
        let edges = (
            Some(id) == first,
            Some(id) == last,
            after_other.contains(&id),
        );
        let current = context.read(row, |row| {
            (row.first_in_group, row.last_in_group, row.rule_above)
        });
        if current.is_ok_and(|current| current != edges) {
            context.update_component(row, |row, _| {
                (row.first_in_group, row.last_in_group, row.rule_above) = edges;
            })?;
        }
    }
    hand_down_hint_placement(context, group.stable_id())
}

/// Whether `id` is a [`SettingsGroup`] or a [`SettingsCard`], and what
/// placement it sets when it is.
fn hint_scope(context: &AppContext, id: StableNodeId) -> Option<Option<HintPlacement>> {
    context
        .read(Entity::<SettingsGroup>::from_stable_id(id), |group| {
            group.hint_placement
        })
        .or_else(|_| {
            context.read(Entity::<SettingsCard>::from_stable_id(id), |card| {
                card.hint_placement
            })
        })
        .ok()
}

/// The placement the nearest group or card above `id` that sets one gives.
fn inherited_hint_placement(context: &AppContext, id: StableNodeId) -> Option<HintPlacement> {
    let mut node = context.world().parent_id(id);
    while let Some(parent) = node {
        if let Some(Some(placement)) = hint_scope(context, parent) {
            return Some(placement);
        }
        node = context.world().parent_id(parent);
    }
    None
}

/// Write the placement `row`'s containers give it. While a view is being
/// built the row may not stand in its container yet: it is looked at once
/// the build is done.
pub(crate) fn resolve_row_hint_placement(
    context: &mut AppContext,
    row: StableNodeId,
) -> Result<(), FrameworkError> {
    if context.defer_post_flush(row, resolve_row_hint_placement_now) {
        return Ok(());
    }
    resolve_row_hint_placement_now(context, row)
}

pub(crate) fn resolve_row_hint_placement_now(
    context: &mut AppContext,
    row: StableNodeId,
) -> Result<(), FrameworkError> {
    let inherited = inherited_hint_placement(context, row);
    let row = Entity::<SettingsRow>::from_stable_id(row);
    if context.read(row, |row| row.inherited_hint_placement != inherited)? {
        context.update_component(row, |row, _| row.inherited_hint_placement = inherited)?;
    }
    Ok(())
}

/// The rows under `node` that take their placement from it: every
/// [`SettingsRow`] below it, through any container, except under a group or
/// card that sets a placement of its own.
fn scope_rows(context: &AppContext, node: StableNodeId, rows: &mut Vec<StableNodeId>) {
    let Some(snapshot) = context.world().node(node) else {
        return;
    };
    for child in snapshot.children {
        if context
            .read(Entity::<SettingsRow>::from_stable_id(child), |_| ())
            .is_ok()
        {
            rows.push(child);
        } else if !matches!(hint_scope(context, child), Some(Some(_))) {
            scope_rows(context, child, rows);
        }
    }
}

/// Hand a group or card's hint placement down to every [`SettingsRow`] in
/// scope. Individual rows diff-check `inherited_hint_placement` before writing.
fn hand_down_hint_placement(
    context: &mut AppContext,
    scope: StableNodeId,
) -> Result<(), FrameworkError> {
    let mut rows = Vec::new();
    scope_rows(context, scope, &mut rows);
    for row in rows {
        resolve_row_hint_placement(context, row)?;
    }
    Ok(())
}

fn sync_settings_card(
    context: &mut AppContext,
    card: Entity<SettingsCard>,
) -> Result<(), FrameworkError> {
    hand_down_hint_placement(context, card.stable_id())
}

const COLLAPSIBLE_HEADER_GAP: f32 = nana_ui_core::space::MD;
const COLLAPSIBLE_BODY_GAP: f32 = nana_ui_core::space::XL;

/// Controlled collapse. Header is keyboard-activatable; values stay host-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCollapsibleCard {
    pub summary: Option<StableNodeId>,
    pub details: Option<StableNodeId>,
    pub accessory: Option<StableNodeId>,
    pub(crate) header: Option<StableNodeId>,
    pub(crate) disclosure: Option<StableNodeId>,
    pub(crate) divider: Option<StableNodeId>,
    pub expanded: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl SettingsCollapsibleCard {
    pub fn new(expanded: bool) -> Self {
        Self {
            summary: None,
            details: None,
            accessory: None,
            header: None,
            disclosure: None,
            divider: None,
            expanded,
            disabled: false,
            style: NodeStyle::default(),
        }
    }

    pub fn summary(mut self, summary: StableNodeId) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn details(mut self, details: StableNodeId) -> Self {
        self.details = Some(details);
        self
    }

    pub fn accessory(mut self, accessory: StableNodeId) -> Self {
        self.accessory = Some(accessory);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        let mut style = card.style.clone();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Fill);
            layout.direction = Some(FlexDirection::Column);
            layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_BODY_GAP));
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xl));
        }
        style.interaction = if self.disabled {
            InteractionStyle {
                disabled: SemanticPaint {
                    foreground: Some(SemanticColorRole::Faint),
                    ..SemanticPaint::default()
                },
                ..InteractionStyle::default()
            }
            .into()
        } else {
            InteractionStyle {
                hovered: SemanticPaint {
                    background: Some(SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    background: Some(SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint::FOCUS_SURFACE,
                ..InteractionStyle::default()
            }
            .into()
        };
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCollapsibleCard {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::SETTINGS_COLLAPSIBLE_CARD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_settings_collapsible_card),
        slot_assembler: Some(AppContext::assemble_settings_collapsible_card),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Lays out and hides the summary and detail slots the application provides.
    const ALWAYS_REPROJECT: bool = true;

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
        project_common(
            id,
            world,
            mutations,
            &self.card().style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                selected: Some(self.expanded),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
        self.project_slots(world, mutations);
    }
}

fn project_slots_hidden(
    id: StableNodeId,
    hidden: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    if current.layout.hidden == hidden {
        return;
    }
    let mut style = current.clone();
    Arc::make_mut(&mut style.layout).hidden = hidden;
    mutations.set_style(id, style);
}

fn project_summary_slot(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    let layout = current.layout.as_ref();
    let needs_width = layout.width.is_none();
    let needs_grow = layout.flex_grow.is_none();
    let needs_color = current.foreground != Some(SemanticColorRole::Text);
    if !needs_width && !needs_grow && !needs_color {
        return;
    }
    let mut style = current.clone();
    if needs_color {
        style.foreground = Some(SemanticColorRole::Text);
    }
    let layout = Arc::make_mut(&mut style.layout);
    if needs_width {
        layout.width = Some(LengthSpec::Fill);
    }
    if needs_grow {
        layout.flex_grow = Some(1.0);
    }
    mutations.set_style(id, style);
}

fn disclosure_icon_kind(expanded: bool) -> Icon {
    if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    }
}

impl SettingsCollapsibleCard {
    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(header) = self.header {
            SettingsCollapsibleHeader.project(header, world, mutations);
        }
        if let Some(summary) = self.summary {
            project_summary_slot(summary, world, mutations);
        }
        if let Some(disclosure) = self.disclosure {
            SettingsDisclosure {
                expanded: self.expanded,
            }
            .project(disclosure, world, mutations);
        }
        if let Some(divider) = self.divider {
            SettingsCollapsibleDivider {
                hidden: !self.expanded,
            }
            .project(divider, world, mutations);
        }
        if let Some(details) = self.details {
            project_slots_hidden(details, !self.expanded, world, mutations);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsCollapsibleHeader;

impl SettingsCollapsibleHeader {
    fn style() -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_HEADER_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
        style
    }
}

impl ComponentView for SettingsCollapsibleHeader {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-header".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &Self::style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsCollapsibleDivider {
    hidden: bool,
}

impl SettingsCollapsibleDivider {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.background = Some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(nana_ui_core::HAIRLINE));
        layout.min_height = Some(LengthSpec::Px(nana_ui_core::HAIRLINE));
        let extent = nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline);
        layout.theme.height = Some(extent.into());
        layout.theme.min_height = Some(extent.into());
        layout.hidden = self.hidden;
        style
    }
}

impl ComponentView for SettingsCollapsibleDivider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-divider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual chevron only. Activation stays on the card so accessory controls
/// remain independently interactive.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsDisclosure {
    expanded: bool,
}

impl ComponentView for SettingsDisclosure {
    fn wants_metrics_reproject() -> bool {
        true
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-disclosure".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::IconVisual {
            icon: disclosure_icon_kind(self.expanded),
            size: world.node_theme(id).typography().line,
            tooltip: None,
        };
        if world.widget_visual_as::<crate::IconVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut style = NodeStyle::default();
        style.foreground = Some(SemanticColorRole::Muted);
        style.square = Some(nana_ui_core::SquareSize::Control(ControlSize::Small));
        style.control_height = Some(nana_ui_core::ControlHeight::Exact(ControlSize::Small));
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.aspect_ratio = Some(1.0);
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                selected: Some(self.expanded),
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    pub fn assemble_settings_collapsible_card(
        &mut self,
        card: Entity<SettingsCollapsibleCard>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, card.stable_id())?;
        let snapshot = self.read(card, |card| {
            (
                card.summary,
                card.details,
                card.accessory,
                card.header,
                card.disclosure,
                card.divider,
                card.expanded,
            )
        })?;
        let (summary, details, accessory, header, disclosure, divider, expanded) = snapshot;
        let header = if let Some(id) = header {
            Entity::from_stable_id(id)
        } else {
            self.create_detached_component(document, SettingsCollapsibleHeader)?
        };
        let disclosure = if let Some(id) = disclosure {
            let entity = Entity::<SettingsDisclosure>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.expanded = expanded;
            })?;
            entity
        } else {
            self.create_detached_component(document, SettingsDisclosure { expanded })?
        };
        let divider = if let Some(id) = divider {
            let entity = Entity::<SettingsCollapsibleDivider>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.hidden = !expanded;
            })?;
            entity
        } else {
            self.create_detached_component(
                document,
                SettingsCollapsibleDivider { hidden: !expanded },
            )?
        };
        let mut header_children = Vec::new();
        if let Some(summary) = summary {
            header_children.push(summary);
        }
        if let Some(accessory) = accessory {
            header_children.push(accessory);
        }
        header_children.push(disclosure.stable_id());
        reconcile_children(self, header, &header_children)?;
        self.update_component(card, |card, _| {
            card.header = Some(header.stable_id());
            card.disclosure = Some(disclosure.stable_id());
            card.divider = Some(divider.stable_id());
        })?;
        let mut ordered = vec![header.stable_id(), divider.stable_id()];
        if let Some(details) = details {
            ordered.push(details);
        }
        reconcile_children(self, card, &ordered)
    }
}
