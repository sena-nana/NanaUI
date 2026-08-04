use super::*;
use iced::Vector;
use iced::widget::column;

struct RegionView<'a, Message> {
    state: RegionState,
    content: Element<'a, Message>,
    edges: RegionEdges,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RegionEdges {
    pub(super) start: bool,
    pub(super) end: bool,
}

/// Composes registered application content using the same start/primary/end,
/// workspace/primary top, and workspace/primary bottom model as LiliaUI.
pub fn workspace_view<'a, Message>(
    controller: &WorkspaceController,
    regions: impl Into<WorkspaceRegions<'a, Message>>,
    theme: impl Into<ThemeTokens>,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tokens = theme.into();
    let colors = tokens.colors;
    let mut ordered_content = VecDeque::from(regions.into().regions);
    let mut indexed_content: Option<HashMap<RegionId, Element<'a, Message>>> = None;
    let mut starts = Vec::new();
    let mut primaries = Vec::new();
    let mut ends = Vec::new();
    let mut workspace_top = Vec::new();
    let mut primary_top = Vec::new();
    let mut primary_bottom = Vec::new();
    let mut workspace_bottom = Vec::new();
    let mut overlays = Vec::new();

    for state in controller.layout().regions() {
        let content = if let Some(indexed) = &mut indexed_content {
            indexed.remove(state.id())
        } else if ordered_content
            .front()
            .is_some_and(|region| &region.id == state.id())
        {
            ordered_content.pop_front().map(|region| region.content)
        } else {
            let mut indexed = HashMap::with_capacity(ordered_content.len());
            for region in ordered_content.drain(..) {
                indexed.entry(region.id).or_insert(region.content);
            }
            let content = indexed.remove(state.id());
            indexed_content = Some(indexed);
            content
        };
        let Some(content) = content else {
            continue;
        };
        if !controller.region_visible(state) {
            continue;
        }
        let overlay = controller.region_overlay(state);
        let placement = state.placement_value();
        let scope = state.scope_value();
        let view = RegionView {
            state: state.clone(),
            content,
            edges: RegionEdges::default(),
        };
        if overlay {
            overlays.push(view);
            continue;
        }
        match (placement, scope) {
            (RegionPlacement::Start, _) => starts.push(view),
            (RegionPlacement::Primary, _) => primaries.push(view),
            (RegionPlacement::End, _) => ends.push(view),
            (RegionPlacement::Top, RegionScope::Workspace) => workspace_top.push(view),
            (RegionPlacement::Top, RegionScope::Primary) => primary_top.push(view),
            (RegionPlacement::Bottom, RegionScope::Workspace) => workspace_bottom.push(view),
            (RegionPlacement::Bottom, RegionScope::Primary) => primary_bottom.push(view),
        }
    }

    resolve_primary_edges(&starts, &mut primaries, &ends);

    let mut primary_column = column![].width(Length::Fill).height(Length::Fill);
    for region in primary_top {
        primary_column = primary_column.push(render_region(controller, region, tokens, on_action));
    }

    let mut primary_row = row![].width(Length::Fill).height(Length::Fill);
    if primaries.is_empty() {
        primary_row = primary_row.push(space().width(Length::Fill).height(Length::Fill));
    } else {
        for region in primaries {
            primary_row = primary_row.push(render_region(controller, region, tokens, on_action));
        }
    }
    primary_column = primary_column.push(primary_row);

    for region in primary_bottom {
        primary_column = primary_column.push(render_region(controller, region, tokens, on_action));
    }

    let mut middle = row![].width(Length::Fill).height(Length::Fill);
    for region in starts {
        middle = middle.push(render_region(controller, region, tokens, on_action));
    }
    middle = middle.push(primary_column);
    for region in ends {
        middle = middle.push(render_region(controller, region, tokens, on_action));
    }

    let mut base = column![].width(Length::Fill).height(Length::Fill);
    for region in workspace_top {
        base = base.push(render_region(controller, region, tokens, on_action));
    }
    base = base.push(middle);
    for region in workspace_bottom {
        base = base.push(render_region(controller, region, tokens, on_action));
    }

    let base = container(base)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.surface)
                .color(colors.text)
        });

    let mut layers = stack![base];
    for overlay in overlays {
        layers = layers.push(render_overlay(controller, overlay, tokens, on_action));
    }
    layers.width(Length::Fill).height(Length::Fill).into()
}

fn resolve_primary_edges<Message>(
    starts: &[RegionView<'_, Message>],
    primaries: &mut [RegionView<'_, Message>],
    ends: &[RegionView<'_, Message>],
) {
    let mut has_track_before = starts.iter().any(|region| !region.state.collapsed_value());
    let has_end_track = ends.iter().any(|region| !region.state.collapsed_value());
    let mut expanded_after = primaries
        .iter()
        .filter(|region| !region.state.collapsed_value())
        .count();

    for region in primaries {
        let expanded = !region.state.collapsed_value();
        expanded_after -= usize::from(expanded);
        region.edges = primary_edges(
            expanded,
            has_track_before,
            expanded_after > 0 || has_end_track,
        );
        has_track_before |= expanded;
    }
}

pub(super) fn primary_edges(
    expanded: bool,
    has_track_before: bool,
    has_track_after: bool,
) -> RegionEdges {
    RegionEdges {
        start: expanded && !has_track_before,
        end: expanded && !has_track_after,
    }
}

fn render_region<'a, Message>(
    controller: &WorkspaceController,
    region: RegionView<'a, Message>,
    tokens: ThemeTokens,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let colors = tokens.colors;
    let state = region.state;
    let horizontal = matches!(
        state.placement_value(),
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let (width, height) = track_lengths(controller, &state);
    let surface = region_surface(region.content, &state, region.edges, width, height, tokens);
    if !state.resizable_value()
        || state.disabled_value()
        || state.fill_priority_value() > 0
        || controller.transitions.contains_key(state.id())
    {
        return surface;
    }

    let handle = resize_handle(controller, &state, colors, on_action);
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
    controller: &WorkspaceController,
    region: RegionView<'a, Message>,
    tokens: ThemeTokens,
    on_action: impl Fn(WorkspaceAction) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let placement = region.state.placement_value();
    let overlay = render_region(controller, region, tokens, on_action);
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
    edges: RegionEdges,
    width: Length,
    height: Length,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let colors = tokens.colors;
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
        let radius = primary_region_radius(tokens, edges.start, edges.end);
        let surface = surface.style(primary_region_style(tokens, edges.start, edges.end));
        if radius == iced::border::Radius::default() {
            return surface.into();
        }
        let mask = canvas(PrimaryCornerMask {
            radius,
            color: colors.surface,
        })
        .width(Length::Fill)
        .height(Length::Fill);
        stack![surface, mask].width(width).height(height).into()
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

#[derive(Debug, Clone, Copy)]
struct PrimaryCornerMask {
    radius: iced::border::Radius,
    color: iced::Color,
}

impl<Message> canvas::Program<Message> for PrimaryCornerMask {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let mask = Path::new(|builder| {
            builder.rectangle(Point::ORIGIN, bounds.size());
            builder.rounded_rectangle(Point::ORIGIN, bounds.size(), self.radius);
        });
        frame.fill(
            &mask,
            Fill {
                style: Style::Solid(self.color),
                rule: fill::Rule::EvenOdd,
            },
        );
        vec![frame.into_geometry()]
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
    on_action: impl Fn(WorkspaceAction) -> Message + Copy + 'a,
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

    crate::drag_handle::DragHandle::new(
        content,
        on_action(WorkspaceAction::ResizeStart(id.clone())),
        move |position| {
            on_action(WorkspaceAction::ResizeMove {
                x: position.x,
                y: position.y,
            })
        },
        on_action(WorkspaceAction::ResizeEnd),
        on_action(WorkspaceAction::ResetRegionSize(id.clone())),
        move |hovered| on_action(WorkspaceAction::ResizeHover(hovered.then_some(id.clone()))),
        if horizontal {
            iced::mouse::Interaction::ResizingHorizontally
        } else {
            iced::mouse::Interaction::ResizingVertically
        },
    )
    .translate(resize_handle_translation(state.placement_value()))
    .into()
}

pub(super) fn resize_handle_translation(placement: RegionPlacement) -> Vector {
    let offset = RESIZE_HANDLE_SIZE / 2.0;
    match placement {
        RegionPlacement::Start | RegionPlacement::Primary => Vector::new(offset, 0.0),
        RegionPlacement::End => Vector::new(-offset, 0.0),
        RegionPlacement::Top => Vector::new(0.0, offset),
        RegionPlacement::Bottom => Vector::new(0.0, -offset),
    }
}
