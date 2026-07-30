use std::collections::HashMap;

use iced::widget::{column, container, mouse_area, row, space, stack};
use iced::{Animation, Element, Length, Padding, Point, Subscription};

use crate::geometry::{RESIZE_HANDLE_SIZE, WorkspaceGeometry};
use crate::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use crate::theme::Colors;
use crate::widgets::{canvas_style, workspace_region_style};

const REGION_COLLAPSE_DURATION: iced::time::Duration = iced::time::Duration::from_millis(240);

/// Framework-owned workspace interaction message.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceAction {
    ToggleRegion(RegionId),
    SetRegionCollapsed(RegionId, bool),
    SetRegionVisible(RegionId, bool),
    SetRegionSize(RegionId, f32),
    ResetRegionSize(RegionId),
    ResizeStart(RegionId),
    ResizeHover(Option<RegionId>),
    ResizeMove { x: f32, y: f32 },
    ResizeEnd,
    WindowResized { width: f32, height: f32 },
    WindowScaleFactorChanged(f32),
    AnimationFrame(iced::time::Instant),
}

#[derive(Debug, Clone)]
struct ResizeState {
    region: RegionId,
    last_position: Option<Point>,
}

#[derive(Debug, Clone)]
struct RegionTransition {
    expansion: Animation<bool>,
    target_collapsed: bool,
    overlay: bool,
}

/// Owns region registrations, persisted layout, resize interaction, and host
/// viewport geometry. Application content remains outside the controller.
#[derive(Debug, Clone)]
pub struct WorkspaceController {
    layout: WorkspaceLayout,
    transitions: HashMap<RegionId, RegionTransition>,
    resizing: Option<ResizeState>,
    hovered_resize: Option<RegionId>,
    window_width: f32,
    window_height: f32,
    scale_factor: f32,
}

impl Default for WorkspaceController {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceController {
    pub fn new() -> Self {
        Self::with_layout(WorkspaceLayout::default())
    }

    pub fn with_layout(layout: WorkspaceLayout) -> Self {
        Self {
            layout,
            transitions: HashMap::new(),
            resizing: None,
            hovered_resize: None,
            window_width: 1440.0,
            window_height: 900.0,
            scale_factor: 1.0,
        }
    }

    pub fn layout(&self) -> &WorkspaceLayout {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut WorkspaceLayout {
        self.transitions.clear();
        &mut self.layout
    }

    pub fn replace_layout(&mut self, layout: WorkspaceLayout) -> WorkspaceLayout {
        self.resizing = None;
        self.hovered_resize = None;
        self.transitions.clear();
        std::mem::replace(&mut self.layout, layout)
    }

    pub fn inline_size(&self) -> f32 {
        self.window_width
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        self.layout.to_json()
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        self.layout.restore_json(value)?;
        self.resizing = None;
        self.hovered_resize = None;
        self.transitions.clear();
        Ok(())
    }

    pub fn geometry(
        &self,
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
    ) -> WorkspaceGeometry {
        if self.transitions.is_empty() {
            WorkspaceGeometry::new(&self.layout, logical_width, logical_height, scale_factor)
        } else {
            let layout = self.layout.with_transient_extents(
                self.transitions
                    .keys()
                    .map(|id| (id.clone(), self.region_extent(id))),
            );
            WorkspaceGeometry::new(&layout, logical_width, logical_height, scale_factor)
        }
    }

    pub fn viewport_geometry(&self) -> WorkspaceGeometry {
        self.geometry(self.window_width, self.window_height, self.scale_factor)
    }

    pub fn subscription(&self) -> Subscription<WorkspaceAction> {
        let mut subscriptions = vec![iced::event::listen_with(window_event)];
        if self.resizing.is_some() {
            subscriptions.push(iced::event::listen_with(resize_event));
        }
        if !self.transitions.is_empty() {
            subscriptions.push(iced::window::frames().map(WorkspaceAction::AnimationFrame));
        }
        Subscription::batch(subscriptions)
    }

    /// Applies one framework action and reports whether observable state changed.
    pub fn update(&mut self, action: WorkspaceAction) -> bool {
        match action {
            WorkspaceAction::ToggleRegion(region) => {
                let Some(state) = self.layout.region(&region) else {
                    return false;
                };
                let collapsed = self
                    .transitions
                    .get(&region)
                    .map_or(state.collapsed_value(), |transition| {
                        transition.target_collapsed
                    });
                self.set_region_collapsed(region, !collapsed)
            }
            WorkspaceAction::SetRegionCollapsed(region, collapsed) => {
                self.set_region_collapsed(region, collapsed)
            }
            WorkspaceAction::SetRegionVisible(region, visible) => {
                self.transitions.remove(&region);
                self.layout.set_hidden(&region, !visible)
            }
            WorkspaceAction::SetRegionSize(region, size) => self.layout.set_size(&region, size),
            WorkspaceAction::ResetRegionSize(region) => self.layout.reset_size(&region),
            WorkspaceAction::ResizeStart(region) => {
                let Some(state) = self.layout.region(&region) else {
                    return false;
                };
                if !state.resizable_value()
                    || state.disabled_value()
                    || !state.requested_visible()
                    || state.fill_priority_value() > 0
                    || self.transitions.contains_key(&region)
                {
                    return false;
                }
                self.hovered_resize = Some(region.clone());
                self.resizing = Some(ResizeState {
                    region,
                    last_position: None,
                });
                true
            }
            WorkspaceAction::ResizeHover(region) => {
                if self.hovered_resize == region {
                    false
                } else {
                    self.hovered_resize = region;
                    true
                }
            }
            WorkspaceAction::ResizeMove { x, y } => {
                let Some(resizing) = &mut self.resizing else {
                    return false;
                };
                let Some(region) = self.layout.region(&resizing.region) else {
                    self.resizing = None;
                    return false;
                };
                let placement = region.placement_value();
                let position = Point::new(x, y);
                let changed = resizing.last_position.is_some_and(|last_position| {
                    let delta = resize_delta(placement, last_position, position);
                    self.layout.resize_by(&resizing.region, delta)
                });
                resizing.last_position = Some(position);
                changed
            }
            WorkspaceAction::ResizeEnd => {
                let changed = self.resizing.is_some() || self.hovered_resize.is_some();
                self.resizing = None;
                self.hovered_resize = None;
                changed
            }
            WorkspaceAction::WindowResized { width, height } => {
                let width = finite_non_negative(width);
                let height = finite_non_negative(height);
                let changed = self.window_width != width || self.window_height != height;
                self.window_width = width;
                self.window_height = height;
                changed
            }
            WorkspaceAction::WindowScaleFactorChanged(scale_factor) => {
                if !scale_factor.is_finite() || scale_factor <= 0.0 {
                    return false;
                }
                let changed = self.scale_factor != scale_factor;
                self.scale_factor = scale_factor;
                changed
            }
            WorkspaceAction::AnimationFrame(now) => {
                let had_transitions = !self.transitions.is_empty();
                self.transitions
                    .retain(|_, transition| transition.expansion.is_animating(now));
                had_transitions
            }
        }
    }

    fn set_region_collapsed(&mut self, region: RegionId, collapsed: bool) -> bool {
        let Some(state) = self.layout.region(&region) else {
            return false;
        };
        let current_target = self
            .transitions
            .get(&region)
            .map_or(state.collapsed_value(), |transition| {
                transition.target_collapsed
            });
        if current_target == collapsed {
            return false;
        }
        if !state.collapsible_value() || state.hidden_value() || state.disabled_value() {
            return false;
        }

        let now = iced::time::Instant::now();
        let overlay = self.transitions.get(&region).map_or_else(
            || state.responsive_overlay(self.window_width),
            |value| value.overlay,
        );
        if !self.layout.set_collapsed(&region, collapsed) {
            return false;
        }
        if let Some(transition) = self.transitions.get_mut(&region) {
            transition.expansion.go_mut(!collapsed, now);
            transition.target_collapsed = collapsed;
        } else {
            let mut expansion = Animation::new(!current_target)
                .duration(REGION_COLLAPSE_DURATION)
                .easing(iced::animation::Easing::EaseOutCubic);
            expansion.go_mut(!collapsed, now);
            self.transitions.insert(
                region,
                RegionTransition {
                    expansion,
                    target_collapsed: collapsed,
                    overlay,
                },
            );
        }
        true
    }

    fn region_visible(&self, state: &RegionState) -> bool {
        if self.transitions.contains_key(state.id()) {
            !state.hidden_value() && !state.responsive_collapsed(self.inline_size())
        } else {
            state.visible_at(self.inline_size())
        }
    }

    fn region_overlay(&self, state: &RegionState) -> bool {
        self.transitions.get(state.id()).map_or_else(
            || state.responsive_overlay(self.inline_size()),
            |value| value.overlay,
        )
    }

    fn region_extent(&self, region: &RegionId) -> f32 {
        self.region_extent_at(region, iced::time::Instant::now())
    }

    fn region_extent_at(&self, region: &RegionId, at: iced::time::Instant) -> f32 {
        let Some(state) = self.layout.region(region) else {
            return 0.0;
        };
        self.transitions.get(region).map_or_else(
            || state.extent(),
            |transition| transition.expansion.interpolate(0.0, state.extent(), at),
        )
    }

    fn resize_highlighted(&self, region: &RegionId) -> bool {
        self.hovered_resize.as_ref() == Some(region)
            || self
                .resizing
                .as_ref()
                .is_some_and(|state| &state.region == region)
    }
}

/// Application content registered for one workspace region.
pub struct WorkspaceRegion<'a, Message> {
    id: RegionId,
    content: Element<'a, Message>,
}

impl<'a, Message> WorkspaceRegion<'a, Message> {
    pub fn new(id: RegionId, content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            id,
            content: content.into(),
        }
    }
}

/// Ordered content registrations consumed by [`workspace_view`].
pub struct WorkspaceRegions<'a, Message> {
    regions: Vec<WorkspaceRegion<'a, Message>>,
}

impl<'a, Message> WorkspaceRegions<'a, Message> {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    pub fn with_region(mut self, id: RegionId, content: impl Into<Element<'a, Message>>) -> Self {
        self.regions.push(WorkspaceRegion::new(id, content));
        self
    }

    pub fn push(&mut self, region: WorkspaceRegion<'a, Message>) {
        self.regions.push(region);
    }
}

impl<Message> Default for WorkspaceRegions<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience content bundle for the standard six-region workspace.
pub struct WorkspaceSlots<'a, Message> {
    pub global_navigation: Element<'a, Message>,
    pub resources: Element<'a, Message>,
    pub primary_toolbar: Element<'a, Message>,
    pub primary: Element<'a, Message>,
    pub inspector: Element<'a, Message>,
    pub diagnostics: Element<'a, Message>,
}

impl<'a, Message> WorkspaceSlots<'a, Message> {
    pub fn new(
        global_navigation: impl Into<Element<'a, Message>>,
        resources: impl Into<Element<'a, Message>>,
        primary_toolbar: impl Into<Element<'a, Message>>,
        primary: impl Into<Element<'a, Message>>,
        inspector: impl Into<Element<'a, Message>>,
        diagnostics: impl Into<Element<'a, Message>>,
    ) -> Self {
        Self {
            global_navigation: global_navigation.into(),
            resources: resources.into(),
            primary_toolbar: primary_toolbar.into(),
            primary: primary.into(),
            inspector: inspector.into(),
            diagnostics: diagnostics.into(),
        }
    }
}

impl<'a, Message> From<WorkspaceSlots<'a, Message>> for WorkspaceRegions<'a, Message> {
    fn from(slots: WorkspaceSlots<'a, Message>) -> Self {
        Self::new()
            .with_region(RegionId::GlobalNavigation, slots.global_navigation)
            .with_region(RegionId::Resources, slots.resources)
            .with_region(RegionId::PrimaryToolbar, slots.primary_toolbar)
            .with_region(RegionId::Primary, slots.primary)
            .with_region(RegionId::Inspector, slots.inspector)
            .with_region(RegionId::Diagnostics, slots.diagnostics)
    }
}

struct RegionView<'a, Message> {
    state: &'a RegionState,
    content: Element<'a, Message>,
}

/// Composes registered application content using the same start/primary/end,
/// workspace/primary top, and workspace/primary bottom model as LiliaUI.
pub fn workspace_view<'a, Message>(
    controller: &'a WorkspaceController,
    regions: impl Into<WorkspaceRegions<'a, Message>>,
    colors: Colors,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let mut content = regions.into().regions;
    let mut starts = Vec::new();
    let mut primaries = Vec::new();
    let mut ends = Vec::new();
    let mut workspace_top = Vec::new();
    let mut primary_top = Vec::new();
    let mut primary_bottom = Vec::new();
    let mut workspace_bottom = Vec::new();
    let mut overlays = Vec::new();

    for state in controller.layout().regions() {
        let Some(index) = content.iter().position(|region| region.id == *state.id()) else {
            continue;
        };
        let region = content.remove(index);
        if !controller.region_visible(state) {
            continue;
        }
        let view = RegionView {
            state,
            content: region.content,
        };
        if controller.region_overlay(state) {
            overlays.push(view);
            continue;
        }
        match (state.placement_value(), state.scope_value()) {
            (RegionPlacement::Start, _) => starts.push(view),
            (RegionPlacement::Primary, _) => primaries.push(view),
            (RegionPlacement::End, _) => ends.push(view),
            (RegionPlacement::Top, RegionScope::Workspace) => workspace_top.push(view),
            (RegionPlacement::Top, RegionScope::Primary) => primary_top.push(view),
            (RegionPlacement::Bottom, RegionScope::Workspace) => workspace_bottom.push(view),
            (RegionPlacement::Bottom, RegionScope::Primary) => primary_bottom.push(view),
        }
    }

    let mut primary_column = column![].width(Length::Fill).height(Length::Fill);
    for region in primary_top {
        primary_column = primary_column.push(render_region(controller, region, colors, on_action));
    }

    let mut primary_row = row![].width(Length::Fill).height(Length::Fill);
    if primaries.is_empty() {
        primary_row = primary_row.push(space().width(Length::Fill).height(Length::Fill));
    } else {
        for region in primaries {
            primary_row = primary_row.push(render_region(controller, region, colors, on_action));
        }
    }
    primary_column = primary_column.push(primary_row);

    for region in primary_bottom {
        primary_column = primary_column.push(render_region(controller, region, colors, on_action));
    }

    let mut middle = row![].width(Length::Fill).height(Length::Fill);
    for region in starts {
        middle = middle.push(render_region(controller, region, colors, on_action));
    }
    middle = middle.push(primary_column);
    for region in ends {
        middle = middle.push(render_region(controller, region, colors, on_action));
    }

    let mut base = column![].width(Length::Fill).height(Length::Fill);
    for region in workspace_top {
        base = base.push(render_region(controller, region, colors, on_action));
    }
    base = base.push(middle);
    for region in workspace_bottom {
        base = base.push(render_region(controller, region, colors, on_action));
    }

    let base = container(base)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.background)
                .color(colors.text)
        });

    let mut layers = stack![base];
    for overlay in overlays {
        layers = layers.push(render_overlay(controller, overlay, colors, on_action));
    }
    layers.width(Length::Fill).height(Length::Fill).into()
}

fn render_region<'a, Message>(
    controller: &'a WorkspaceController,
    region: RegionView<'a, Message>,
    colors: Colors,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let state = region.state;
    let horizontal = matches!(
        state.placement_value(),
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let (width, height) = track_lengths(controller, state);
    let surface = region_surface(region.content, state, width, height, colors);
    if !state.resizable_value()
        || state.disabled_value()
        || state.fill_priority_value() > 0
        || controller.transitions.contains_key(state.id())
    {
        return surface;
    }

    let handle = resize_handle(controller, state, colors, on_action);
    let handle = match state.placement_value() {
        RegionPlacement::Start | RegionPlacement::Primary => container(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill),
        RegionPlacement::End => container(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_left(Length::Fill),
        RegionPlacement::Top => container(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_bottom(Length::Fill),
        RegionPlacement::Bottom => container(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_top(Length::Fill),
    };
    stack![surface, handle]
        .width(if horizontal { width } else { Length::Fill })
        .height(if horizontal { Length::Fill } else { height })
        .into()
}

fn render_overlay<'a, Message>(
    controller: &'a WorkspaceController,
    region: RegionView<'a, Message>,
    colors: Colors,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let placement = region.state.placement_value();
    let overlay = render_region(controller, region, colors, on_action);
    let aligned = match placement {
        RegionPlacement::Start | RegionPlacement::Primary => container(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_left(Length::Fill),
        RegionPlacement::End => container(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill),
        RegionPlacement::Top => container(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_top(Length::Fill),
        RegionPlacement::Bottom => container(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_bottom(Length::Fill),
    };
    aligned.into()
}

fn region_surface<'a, Message>(
    content: Element<'a, Message>,
    state: &RegionState,
    width: Length,
    height: Length,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let content = if state.placement_value() == RegionPlacement::Bottom {
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: 1.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            })
            .into()
    } else {
        content
    };
    let surface = container(content).width(width).height(height).clip(true);
    if state.role() == RegionRole::Primary {
        surface.style(canvas_style(colors)).into()
    } else if state.placement_value() == RegionPlacement::Bottom {
        let separator = container(space())
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(move |_theme| {
                iced::widget::container::Style::default().background(colors.border_soft)
            });
        stack![
            surface.style(workspace_region_style(colors)),
            container(separator)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_top(Length::Fill)
        ]
        .width(width)
        .height(height)
        .into()
    } else {
        surface.style(workspace_region_style(colors)).into()
    }
}

fn track_lengths(controller: &WorkspaceController, state: &RegionState) -> (Length, Length) {
    let track = if state.fill_priority_value() > 0 {
        Length::FillPortion(state.fill_priority_value())
    } else if state.size_value().is_some() {
        Length::Fixed(controller.region_extent(state.id()))
    } else {
        Length::Shrink
    };
    match state.placement_value() {
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End => {
            (track, Length::Fill)
        }
        RegionPlacement::Top | RegionPlacement::Bottom => (Length::Fill, track),
    }
}

fn resize_handle<'a, Message>(
    controller: &WorkspaceController,
    state: &RegionState,
    colors: Colors,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let horizontal = matches!(
        state.placement_value(),
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let indicator_color = if controller.resize_highlighted(state.id()) {
        colors.border_strong
    } else {
        iced::Color::TRANSPARENT
    };
    let indicator = container(space())
        .width(if horizontal {
            Length::Fixed(2.0)
        } else {
            Length::Fill
        })
        .height(if horizontal {
            Length::Fill
        } else {
            Length::Fixed(2.0)
        })
        .style(move |_theme| iced::widget::container::Style::default().background(indicator_color));
    let content = container(indicator)
        .width(if horizontal {
            Length::Fixed(RESIZE_HANDLE_SIZE)
        } else {
            Length::Fill
        })
        .height(if horizontal {
            Length::Fill
        } else {
            Length::Fixed(RESIZE_HANDLE_SIZE)
        })
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center);
    let id = state.id().clone();

    mouse_area(content)
        .on_press(on_action(WorkspaceAction::ResizeStart(id.clone())))
        .on_double_click(on_action(WorkspaceAction::ResetRegionSize(id.clone())))
        .on_release(on_action(WorkspaceAction::ResizeEnd))
        .on_enter(on_action(WorkspaceAction::ResizeHover(Some(id))))
        .on_exit(on_action(WorkspaceAction::ResizeHover(None)))
        .interaction(if horizontal {
            iced::mouse::Interaction::ResizingHorizontally
        } else {
            iced::mouse::Interaction::ResizingVertically
        })
        .into()
}

fn resize_delta(placement: RegionPlacement, previous: Point, current: Point) -> f32 {
    match placement {
        RegionPlacement::Start | RegionPlacement::Primary => current.x - previous.x,
        RegionPlacement::End => previous.x - current.x,
        RegionPlacement::Top => current.y - previous.y,
        RegionPlacement::Bottom => previous.y - current.y,
    }
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn resize_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<WorkspaceAction> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(WorkspaceAction::ResizeMove {
                x: position.x,
                y: position.y,
            })
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(WorkspaceAction::ResizeEnd)
        }
        _ => None,
    }
}

fn window_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<WorkspaceAction> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(WorkspaceAction::WindowResized {
                width: size.width,
                height: size.height,
            })
        }
        iced::Event::Window(iced::window::Event::Rescaled(scale_factor)) => {
            Some(WorkspaceAction::WindowScaleFactorChanged(scale_factor))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceAction, WorkspaceController};
    use crate::layout::{RegionId, RegionRole, RegionState, WorkspaceLayout};

    #[test]
    fn controller_resizes_regions_in_their_visual_direction() {
        let mut controller = WorkspaceController::new();
        let resources = size(&controller, &RegionId::Resources);
        let inspector = size(&controller, &RegionId::Inspector);
        let diagnostics = size(&controller, &RegionId::Diagnostics);

        resize(
            &mut controller,
            RegionId::Resources,
            (10.0, 0.0),
            (34.0, 0.0),
        );
        resize(
            &mut controller,
            RegionId::Inspector,
            (100.0, 0.0),
            (76.0, 0.0),
        );
        resize(
            &mut controller,
            RegionId::Diagnostics,
            (0.0, 100.0),
            (0.0, 76.0),
        );

        assert_eq!(size(&controller, &RegionId::Resources), resources + 24.0);
        assert_eq!(size(&controller, &RegionId::Inspector), inspector + 24.0);
        assert_eq!(
            size(&controller, &RegionId::Diagnostics),
            diagnostics + 24.0
        );
    }

    #[test]
    fn controller_resizes_application_defined_regions() {
        let pull_requests = RegionId::custom("pull-requests");
        let layout = WorkspaceLayout::new([
            RegionState::new(pull_requests.clone(), RegionRole::SectionNavigation)
                .size(230.0)
                .resizable(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary),
        ])
        .expect("dynamic layout");
        let mut controller = WorkspaceController::with_layout(layout);

        resize(
            &mut controller,
            pull_requests.clone(),
            (0.0, 0.0),
            (30.0, 0.0),
        );
        assert_eq!(size(&controller, &pull_requests), 260.0);
    }

    #[test]
    fn controller_rejects_non_resizable_and_collapsed_regions() {
        let mut controller = WorkspaceController::new();
        assert!(!controller.update(WorkspaceAction::ResizeStart(RegionId::GlobalNavigation)));

        assert!(controller.update(WorkspaceAction::ToggleRegion(RegionId::Resources)));
        assert!(!controller.update(WorkspaceAction::ResizeStart(RegionId::Resources)));
    }

    #[test]
    fn controller_restores_a_resized_region_to_its_default() {
        let mut controller = WorkspaceController::new();
        resize(
            &mut controller,
            RegionId::Resources,
            (10.0, 0.0),
            (50.0, 0.0),
        );
        assert!(controller.update(WorkspaceAction::ResetRegionSize(RegionId::Resources)));
        assert_eq!(size(&controller, &RegionId::Resources), 260.0);
    }

    #[test]
    fn controller_applies_deterministic_region_state() {
        let mut controller = WorkspaceController::new();
        assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            true,
        )));
        assert!(!controller.update(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            true,
        )));
        assert!(controller.update(WorkspaceAction::SetRegionSize(RegionId::Inspector, 320.0,)));
        assert_eq!(size(&controller, &RegionId::Inspector), 320.0);
    }

    #[test]
    fn controller_animates_region_extent_and_commits_the_target_immediately() {
        let mut controller = WorkspaceController::new();
        let started = iced::time::Instant::now();

        assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            true,
        )));
        assert!(
            controller
                .layout()
                .region(&RegionId::Resources)
                .expect("resources")
                .collapsed_value()
        );

        let middle = controller.region_extent_at(
            &RegionId::Resources,
            started + iced::time::Duration::from_millis(120),
        );
        assert!(middle > 0.0 && middle < 260.0);

        let finished = started + iced::time::Duration::from_millis(300);
        assert_eq!(
            controller.region_extent_at(&RegionId::Resources, finished),
            0.0
        );
        assert!(controller.update(WorkspaceAction::AnimationFrame(finished)));
        assert!(!controller.transitions.contains_key(&RegionId::Resources));
    }

    #[test]
    fn controller_reverses_an_active_collapse_without_losing_region_state() {
        let mut controller = WorkspaceController::new();
        assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            true,
        )));
        assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            false,
        )));
        assert!(
            !controller
                .layout()
                .region(&RegionId::Resources)
                .expect("resources")
                .collapsed_value()
        );
        assert!(
            !controller
                .transitions
                .get(&RegionId::Resources)
                .expect("transition")
                .target_collapsed
        );
    }

    #[test]
    fn controller_owns_serialized_layout_and_viewport_geometry() {
        let mut controller = WorkspaceController::new();
        controller.update(WorkspaceAction::ToggleRegion(RegionId::Inspector));
        controller.update(WorkspaceAction::WindowResized {
            width: 1000.0,
            height: 700.0,
        });
        controller.update(WorkspaceAction::WindowScaleFactorChanged(1.5));

        let encoded = controller.layout_json().expect("layout serializes");
        let mut restored = WorkspaceController::new();
        restored
            .restore_layout_json(&encoded)
            .expect("layout restores");

        assert_eq!(restored.layout(), controller.layout());
        assert_eq!(controller.viewport_geometry().physical_size, (1500, 1050));
    }

    fn resize(
        controller: &mut WorkspaceController,
        region: RegionId,
        start: (f32, f32),
        end: (f32, f32),
    ) {
        assert!(controller.update(WorkspaceAction::ResizeStart(region)));
        controller.update(WorkspaceAction::ResizeMove {
            x: start.0,
            y: start.1,
        });
        assert!(controller.update(WorkspaceAction::ResizeMove { x: end.0, y: end.1 }));
        assert!(controller.update(WorkspaceAction::ResizeEnd));
    }

    fn size(controller: &WorkspaceController, region: &RegionId) -> f32 {
        controller
            .layout()
            .region(region)
            .and_then(RegionState::size_value)
            .expect("fixed region size")
    }
}
