use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LayoutStyle, LengthSpec,
    OverflowSpec, SemanticColorRole, TooltipConfig,
};

use crate::view_components::{
    IconButton, ListItem, ListItemSlots, ScrollAxes, ScrollView, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextContent, UiWorld,
};

const FRAME_PADDING_TOP: f32 = 10.0;
const FRAME_PADDING_RIGHT: f32 = 8.0;
const FRAME_PADDING_BOTTOM: f32 = 10.0;
const FRAME_PADDING_LEFT: f32 = 12.0;
const FRAME_GAP: f32 = 14.0;
const ROW_PADDING_LEFT: f32 = 8.0;
const ROW_PADDING_RIGHT: f32 = 8.0;
const ROW_TREE_FIRST_DEPTH_INSET: f32 = 30.0;
const ROW_TREE_DEPTH_STEP: f32 = 12.0;
const SECTION_ANIMATION_DURATION: Duration = Duration::from_millis(160);

/// Visual selection contract for a navigation row. This is not a generic list item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SidebarRowState {
    #[default]
    Idle,
    Active,
    AncestorActive,
    Disabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SidebarRowTone {
    #[default]
    Default,
    Warning,
    Error,
}

pub fn sidebar_row_depth_inset(depth: u16) -> f32 {
    if depth == 0 {
        ROW_PADDING_LEFT
    } else {
        ROW_TREE_FIRST_DEPTH_INSET + f32::from(depth - 1) * ROW_TREE_DEPTH_STEP
    }
}

/// Host-sampled expand/collapse. The host owns the clock; this type never starts a thread.
#[derive(Debug, Clone)]
pub struct SidebarSectionState {
    expansion: nana_ui_core::ExpansionState,
}

impl SidebarSectionState {
    pub fn new(expanded: bool) -> Self {
        Self {
            expansion: nana_ui_core::ExpansionState::new(expanded, SECTION_ANIMATION_DURATION),
        }
    }

    pub fn expanded(&self) -> bool {
        self.expansion.expanded()
    }

    pub fn set_expanded(&mut self, expanded: bool, now: Duration) -> bool {
        self.expansion.set_expanded(expanded, now)
    }

    pub fn toggle(&mut self, now: Duration) -> bool {
        self.expansion.toggle(now)
    }

    pub fn is_animating(&self, now: Duration) -> bool {
        self.expansion.is_animating_at(now)
    }

    pub fn expansion(&self, now: Duration) -> f32 {
        self.expansion.value_at(now)
    }

    pub fn animation_duration() -> Duration {
        SECTION_ANIMATION_DURATION
    }
}

impl Default for SidebarSectionState {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Fixed top/footer with an independently scrolling body.
///
/// `body` is the vertical [`ScrollView`] slot. Content is a child of that
/// scrollport; top and footer stay unscoped siblings.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarFrame {
    pub top: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub footer: Option<StableNodeId>,
    pub gap: f32,
    pub style: NodeStyle,
}

fn apply_body_scroll_layout(layout: &mut LayoutStyle) {
    layout.overflow_y = OverflowSpec::Scroll;
    if layout.flex_grow.is_none_or(|grow| grow <= 0.0) {
        layout.flex_grow = Some(1.0);
    }
    if layout.flex_shrink.is_none() {
        layout.flex_shrink = Some(1.0);
    }
    if layout.height.is_none() {
        layout.height = Some(LengthSpec::Fill);
    }
    if layout.width.is_none() {
        layout.width = Some(LengthSpec::Fill);
    }
    if layout.min_height.is_none() {
        layout.min_height = Some(LengthSpec::Px(0.0));
    }
    if layout.min_width.is_none() {
        layout.min_width = Some(LengthSpec::Px(0.0));
    }
    layout.align_items = AlignSpec::Stretch;
}

impl SidebarFrame {
    pub fn new() -> Self {
        Self {
            top: None,
            body: None,
            footer: None,
            gap: FRAME_GAP,
            style: NodeStyle::default(),
        }
    }

    /// Vertical Runtime scrollport used as the independently scrolling body.
    pub fn vertical_body_scroll() -> ScrollView {
        Self::scroll_body(LayoutStyle::default())
    }

    /// Project host layout onto the body slot as a vertical [`ScrollView`].
    pub fn scroll_body(layout: LayoutStyle) -> ScrollView {
        let mut style = NodeStyle {
            layout: Arc::new(layout),
            ..NodeStyle::default()
        };
        apply_body_scroll_layout(Arc::make_mut(&mut style.layout));
        ScrollView::new(ScrollAxes::Vertical).style(style)
    }

    fn project_body_scroll(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(body) = self.body else {
            return;
        };
        let mut style = world.node_style(body).cloned().unwrap_or_default();
        apply_body_scroll_layout(Arc::make_mut(&mut style.layout));
        ScrollView::new(ScrollAxes::Vertical)
            .style(style)
            .project(body, world, mutations);
    }

    pub fn top(mut self, top: StableNodeId) -> Self {
        self.top = Some(top);
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn footer(mut self, footer: StableNodeId) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(layout.direction.unwrap_or(FlexDirection::Column));
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(layout.width.unwrap_or(LengthSpec::Fill));
        layout.height = Some(layout.height.unwrap_or(LengthSpec::Fill));
        layout.gap = Some(layout.gap.unwrap_or(LengthSpec::Px(self.gap)));
        if layout.padding.is_none()
            && layout.padding_top.is_none()
            && layout.padding_right.is_none()
            && layout.padding_bottom.is_none()
            && layout.padding_left.is_none()
        {
            layout.padding_top = Some(LengthSpec::Px(FRAME_PADDING_TOP));
            layout.padding_right = Some(LengthSpec::Px(FRAME_PADDING_RIGHT));
            layout.padding_bottom = Some(LengthSpec::Px(FRAME_PADDING_BOTTOM));
            layout.padding_left = Some(LengthSpec::Px(FRAME_PADDING_LEFT));
        }
        if !layout.overflow_x.clips() {
            layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
        }
        if !layout.overflow_y.clips() {
            layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;
        }
        style
    }
}

impl Default for SidebarFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for SidebarFrame {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "sidebar-frame".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
        self.project_body_scroll(world, mutations);
    }
}

/// Navigation row. Reuses ListItem slot paint; keeps a distinct catalog identity.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarRow {
    pub label: Arc<str>,
    pub slots: ListItemSlots,
    pub tools: Option<StableNodeId>,
    pub depth: u16,
    pub size: ControlSize,
    pub gap: f32,
    pub state: SidebarRowState,
    pub tone: SidebarRowTone,
    pub disclosure: Option<bool>,
    pub style: NodeStyle,
}

impl SidebarRow {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            slots: ListItemSlots::default(),
            tools: None,
            depth: 0,
            size: ControlSize::Small,
            gap: 6.0,
            state: SidebarRowState::Idle,
            tone: SidebarRowTone::Default,
            disclosure: None,
            style: NodeStyle::default(),
        }
    }

    pub fn slots(mut self, slots: ListItemSlots) -> Self {
        self.slots = slots;
        self
    }

    pub fn tools(mut self, tools: StableNodeId) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn state(mut self, state: SidebarRowState) -> Self {
        self.state = state;
        self
    }

    pub fn tone(mut self, tone: SidebarRowTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn disclosure(mut self, expanded: bool) -> Self {
        self.disclosure = Some(expanded);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn selected(&self) -> bool {
        matches!(
            self.state,
            SidebarRowState::Active | SidebarRowState::AncestorActive
        )
    }

    pub fn disabled(&self) -> bool {
        self.state == SidebarRowState::Disabled
    }

    fn effective_style(&self) -> NodeStyle {
        let mut item = ListItem::new(self.label.as_ref())
            .slots(self.slots)
            .gap(self.gap)
            .size(self.size)
            .selected(self.selected())
            .disabled(self.disabled());
        item.style.layout = Arc::clone(&self.style.layout);
        let mut style = item.style;
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(self.gap));
        layout.min_height = Some(LengthSpec::Px(self.size.height()));
        layout.height = Some(LengthSpec::Px(self.size.height()));
        layout.padding_left = Some(LengthSpec::Px(sidebar_row_depth_inset(self.depth)));
        layout.padding_right = Some(LengthSpec::Px(ROW_PADDING_RIGHT));
        layout.font_size = Some(self.size.text_size());
        layout.font_weight = Some(if self.state == SidebarRowState::AncestorActive {
            600
        } else {
            500
        });
        style.foreground = Some(match (self.tone, self.state) {
            (SidebarRowTone::Warning, _) => SemanticColorRole::Warning,
            (SidebarRowTone::Error, _) => SemanticColorRole::Danger,
            (_, SidebarRowState::Disabled) => SemanticColorRole::Faint,
            (_, SidebarRowState::Active | SidebarRowState::AncestorActive) => {
                SemanticColorRole::Text
            }
            (_, SidebarRowState::Idle) => SemanticColorRole::Muted,
        });
        style.background = self.selected().then_some(SemanticColorRole::Selected);
        style.interaction = InteractionStyle {
            selected: SemanticPaint {
                background: Some(SemanticColorRole::Selected),
                ..SemanticPaint::default()
            },
            selected_hovered: SemanticPaint {
                background: Some(SemanticColorRole::SelectedHover),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(SemanticColorRole::SelectedPressed),
                ..SemanticPaint::default()
            },
            hovered: SemanticPaint {
                background: Some(if self.selected() {
                    SemanticColorRole::SelectedHover
                } else {
                    SemanticColorRole::Hover
                }),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(if self.selected() {
                    SemanticColorRole::SelectedPressed
                } else {
                    SemanticColorRole::Active
                }),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                ..SemanticPaint::default()
            },
        };
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        style
    }
}

impl ComponentView for SidebarRow {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "sidebar-row".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut item = ListItem::new(self.label.as_ref())
            .slots(self.slots)
            .gap(self.gap)
            .size(self.size)
            .selected(self.selected())
            .disabled(self.disabled());
        item.style = self.effective_style();
        item.project(id, world, mutations);
    }
}

/// Titled, optionally collapsible group. Expansion is host-sampled.
#[derive(Debug, Clone)]
pub struct SidebarSection {
    pub title: Arc<str>,
    pub count: Option<usize>,
    pub tools: Option<StableNodeId>,
    pub empty_text: Option<Arc<str>>,
    pub size: ControlSize,
    pub collapsible: bool,
    pub disabled: bool,
    pub state: SidebarSectionState,
    pub animation_progress: f32,
    pub body: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl SidebarSection {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            count: None,
            tools: None,
            empty_text: None,
            size: ControlSize::Small,
            collapsible: false,
            disabled: false,
            state: SidebarSectionState::new(true),
            animation_progress: 1.0,
            body: None,
            style: NodeStyle::default(),
        }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub fn tools(mut self, tools: StableNodeId) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn empty_text(mut self, empty_text: impl Into<Arc<str>>) -> Self {
        self.empty_text = Some(empty_text.into());
        self
    }

    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = collapsible;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.state = SidebarSectionState::new(expanded);
        self.animation_progress = if expanded { 1.0 } else { 0.0 };
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.width = Some(LengthSpec::Fill);
        layout.align_items = AlignSpec::Stretch;
        style.foreground = Some(SemanticColorRole::Faint);
        style.interaction = if self.collapsible && !self.disabled {
            InteractionStyle {
                hovered: SemanticPaint {
                    background: Some(SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    background: Some(SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint {
                    border: Some(SemanticColorRole::Accent),
                    ..SemanticPaint::default()
                },
                ..InteractionStyle::default()
            }
        } else {
            InteractionStyle::default()
        };
        style
    }
}

impl ComponentView for SidebarSection {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "sidebar-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.title.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.title.as_ref().to_owned(),
                },
            );
        }
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        let interactive = self.collapsible && !self.disabled;
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: interactive,
                focusable: interactive,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.title)),
                value: self.count.map(|count| Arc::from(count.to_string())),
                disabled: self.disabled,
                selected: self.collapsible.then_some(self.state.expanded()),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Fixed footer row. Children are application-owned actions.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarFooter {
    pub style: NodeStyle,
}

impl SidebarFooter {
    pub fn new() -> Self {
        Self {
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(layout.gap.unwrap_or(LengthSpec::Px(2.0)));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl Default for SidebarFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for SidebarFooter {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "sidebar-footer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
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

/// Footer icon action. Composes IconButton paint and tooltip.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarFooterButton {
    pub label: Arc<str>,
    pub icon: Icon,
    pub size: ControlSize,
    pub selected: bool,
    pub disabled: bool,
}

impl SidebarFooterButton {
    pub fn new(label: impl Into<Arc<str>>, icon: Icon) -> Self {
        Self {
            label: label.into(),
            icon,
            size: ControlSize::Small,
            selected: false,
            disabled: false,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl ComponentView for SidebarFooterButton {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "sidebar-footer-button".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        IconButton::new(self.icon, Arc::clone(&self.label))
            .size(self.size)
            .selected(self.selected)
            .disabled(self.disabled)
            .tooltip(Arc::clone(&self.label), TooltipConfig::default())
            .project(id, world, mutations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, StandardVisual};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn section_state_animates_from_a_host_sample() {
        let started = Duration::from_millis(100);
        let mut state = SidebarSectionState::new(true);
        assert!(state.set_expanded(false, started));
        assert!(!state.expanded());
        let middle = state.expansion(started + Duration::from_millis(80));
        assert!(middle > 0.0 && middle < 1.0);
        assert_eq!(state.expansion(started + Duration::from_millis(200)), 0.0);
        assert!(state.set_expanded(true, started + Duration::from_millis(80)));
        assert!(state.expanded());
        assert_eq!(state.expansion(started + Duration::from_millis(280)), 1.0);
    }

    #[test]
    fn tree_depth_aligns_the_first_child_with_a_leading_row_label() {
        assert_eq!(sidebar_row_depth_inset(0), 8.0);
        assert_eq!(sidebar_row_depth_inset(1), 30.0);
        assert_eq!(sidebar_row_depth_inset(2), 42.0);
    }

    #[test]
    fn frame_is_layout_only_with_fixed_outer_slots() {
        let mut context = AppContext::new();
        let top = context
            .create_component(document(), SidebarRow::new("返回"))
            .unwrap();
        let body = context
            .create_component(document(), SidebarFrame::vertical_body_scroll())
            .unwrap();
        let footer = context
            .create_component(document(), SidebarFooter::new())
            .unwrap();
        let frame = context
            .create_component(
                document(),
                SidebarFrame::new()
                    .top(top.stable_id())
                    .body(body.stable_id())
                    .footer(footer.stable_id()),
            )
            .unwrap();
        let id = frame.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "sidebar-frame".into(),
            }
        );
        assert_eq!(context.world().standard_visual(id), None);
        let layout = &context.world().node_style(id).unwrap().layout;
        assert_eq!(layout.direction, Some(FlexDirection::Column));
        assert_eq!(layout.gap, Some(LengthSpec::Px(FRAME_GAP)));
        assert_eq!(
            layout.padding_left,
            Some(LengthSpec::Px(FRAME_PADDING_LEFT))
        );
        assert!(!context.world().interaction(id).unwrap().focusable);
        let body_layout = &context.world().node_style(body.stable_id()).unwrap().layout;
        assert_eq!(body_layout.overflow_y, OverflowSpec::Scroll);
        assert_eq!(
            context.world().node(body.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert!(
            !context
                .world()
                .node_style(top.stable_id())
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !context
                .world()
                .node_style(footer.stable_id())
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
    }

    #[test]
    fn frame_body_scrolls_without_moving_top_or_footer() {
        let mut context = AppContext::new();
        let top = context
            .create_component(document(), SidebarRow::new("返回"))
            .unwrap();
        let body = context
            .create_component(document(), SidebarFrame::vertical_body_scroll())
            .unwrap();
        let mut rows = Vec::new();
        for label in [
            "外观",
            "工作区",
            "资源",
            "设置",
            "关于",
            "日志",
            "调试",
            "扩展",
        ] {
            let row = context
                .create_component(document(), SidebarRow::new(label))
                .unwrap();
            context.append_child(body, row).unwrap();
            rows.push(row);
        }
        let footer = context
            .create_component(document(), SidebarFooter::new())
            .unwrap();
        let frame = context
            .create_component(
                document(),
                SidebarFrame::new()
                    .top(top.stable_id())
                    .body(body.stable_id())
                    .footer(footer.stable_id()),
            )
            .unwrap();
        context.append_child(frame, top).unwrap();
        context.append_child(frame, body).unwrap();
        context.append_child(frame, footer).unwrap();

        context
            .layout_document(document(), crate::LayoutViewport::new(220.0, 160.0))
            .unwrap();
        let top_before = context.world().layout_box(top.stable_id()).unwrap();
        let body_box = context.world().layout_box(body.stable_id()).unwrap();
        let footer_before = context.world().layout_box(footer.stable_id()).unwrap();
        let content = rows[0];
        let content_before = context.world().layout_box(content.stable_id()).unwrap();
        assert!(top_before.y < body_box.y);
        assert!(footer_before.y >= body_box.y + body_box.height - 0.5);
        assert!(
            context
                .world()
                .node_style(body.stable_id())
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );

        context
            .set_scroll_metrics(
                body,
                crate::ScrollMetrics {
                    viewport_width: body_box.width,
                    viewport_height: body_box.height,
                    content_width: body_box.width,
                    content_height: body_box.height + 80.0,
                },
            )
            .unwrap();
        assert!(
            context
                .scroll_to(body, crate::ScrollOffset { x: 0.0, y: 40.0 })
                .unwrap()
        );
        assert_eq!(
            context.world().scroll_offset(body.stable_id()),
            Some(crate::ScrollOffset { x: 0.0, y: 40.0 })
        );
        assert_eq!(
            context.world().layout_box(top.stable_id()).unwrap(),
            top_before
        );
        assert_eq!(
            context.world().layout_box(footer.stable_id()).unwrap(),
            footer_before
        );
        assert_eq!(
            context.world().layout_box(content.stable_id()).unwrap(),
            content_before
        );

        context.rebuild_hit_test(document());
        assert_eq!(
            context
                .world()
                .hit_test(document(), top_before.x + 4.0, top_before.y + 4.0),
            Some(top.stable_id())
        );
        let footer_hit =
            context
                .world()
                .hit_test(document(), footer_before.x + 4.0, footer_before.y + 4.0);
        assert_ne!(footer_hit, Some(content.stable_id()));
        assert_ne!(footer_hit, Some(body.stable_id()));
        let scrolled_y = content_before.y - 40.0 + 4.0;
        if scrolled_y >= body_box.y && scrolled_y < body_box.y + body_box.height {
            assert_eq!(
                context
                    .world()
                    .hit_test(document(), content_before.x + 4.0, scrolled_y),
                Some(content.stable_id())
            );
        }
        assert_ne!(
            context
                .world()
                .hit_test(document(), content_before.x + 4.0, content_before.y + 4.0),
            Some(content.stable_id())
        );
    }

    #[test]
    fn row_select_and_disabled_project_list_item_slots() {
        let mut context = AppContext::new();
        let leading = context
            .create_component(document(), crate::Text::new("◇"))
            .unwrap();
        let selected = context
            .create_component(
                document(),
                SidebarRow::new("工作区")
                    .state(SidebarRowState::Active)
                    .slots(ListItemSlots {
                        leading: Some(leading.stable_id()),
                        content: None,
                        trailing: None,
                    })
                    .depth(1)
                    .tone(SidebarRowTone::Warning),
            )
            .unwrap();
        let id = selected.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "sidebar-row".into(),
            }
        );
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::ListItem {
                leading: Some(leading.stable_id()),
                content: None,
                trailing: None,
            })
        );
        assert_eq!(context.world().text(id), Some("工作区"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.foreground, Some(SemanticColorRole::Warning));
        assert_eq!(
            style.layout.padding_left,
            Some(LengthSpec::Px(sidebar_row_depth_inset(1)))
        );
        assert!(context.world().interaction(id).unwrap().focusable);
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(true)
        );

        let disabled = context
            .create_component(
                document(),
                SidebarRow::new("停用").state(SidebarRowState::Disabled),
            )
            .unwrap();
        let disabled_id = disabled.stable_id();
        assert!(
            !context
                .world()
                .interaction(disabled_id)
                .unwrap()
                .pointer_events
        );
        assert!(!context.world().interaction(disabled_id).unwrap().focusable);
        assert!(context.world().accessibility(disabled_id).unwrap().disabled);
        assert!(!context.activate_sidebar_row(disabled).unwrap());
        assert!(context.activate_sidebar_row(selected).unwrap());
    }

    #[test]
    fn section_header_toggles_when_collapsible() {
        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                SidebarSection::new("资源")
                    .count(3)
                    .collapsible(true)
                    .expanded(true),
            )
            .unwrap();
        let id = section.stable_id();
        assert_eq!(context.world().text(id), Some("资源"));
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(true)
        );
        assert!(context.activate_sidebar_section(section).unwrap());
        context
            .read(section, |section| {
                assert!(!section.state.expanded());
                assert!(section.state.is_animating(std::time::Duration::ZERO));
            })
            .unwrap();
        let _ = context.advance_animations(std::time::Duration::from_millis(80));
        context
            .read(section, |section| {
                assert!(section.animation_progress > 0.0 && section.animation_progress < 1.0);
            })
            .unwrap();
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(false)
        );

        let locked = context
            .create_component(
                document(),
                SidebarSection::new("锁定").collapsible(true).disabled(true),
            )
            .unwrap();
        assert!(!context.activate_sidebar_section(locked).unwrap());
    }

    #[test]
    fn footer_button_press_is_real() {
        let mut context = AppContext::new();
        let pressed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = pressed.clone();
        let button = context
            .create_component(
                document(),
                SidebarFooterButton::new("设置", Icon::Settings).selected(true),
            )
            .unwrap();
        context
            .on(button, move |_button, _event: &crate::Activate, _cx| {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            })
            .unwrap();
        assert!(context.activate_sidebar_footer_button(button).unwrap());
        assert!(pressed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            context
                .world()
                .accessibility(button.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("设置")
        );
        assert_eq!(
            context
                .world()
                .accessibility(button.stable_id())
                .unwrap()
                .selected,
            Some(true)
        );

        let disabled = context
            .create_component(
                document(),
                SidebarFooterButton::new("禁用", Icon::Settings).disabled(true),
            )
            .unwrap();
        assert!(!context.activate_sidebar_footer_button(disabled).unwrap());
    }
}
