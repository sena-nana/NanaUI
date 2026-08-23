use std::sync::{Arc, Mutex};

use nana_ui::runtime::{
    AlignSpec, AppContext, ComponentView, DesktopShell, Entity, FlexDirection, FrameworkError,
    IconButton, InteractionState, JustifySpec, LengthSpec, MutationQueue, NodeKind, NodeStyle,
    RuntimeDocument, SemanticColorRole, StableNodeId, Text, UiWorld, View, Workspace,
};
use nana_ui::{ButtonKind, ControlSize, Icon, LogicalPoint, ThemeMode, TitleBarDragTracker};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use super::GalleryMessage;

pub(super) const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 800.0);

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeSceneInput {
    PointerMove(LogicalPoint),
    PointerDown {
        button: i16,
        point: LogicalPoint,
    },
    PointerUp {
        button: i16,
        point: LogicalPoint,
    },
    Scroll {
        delta_y: f32,
        line_delta: bool,
    },
    Key {
        pressed: bool,
        key: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
}

#[derive(Default)]
pub(super) struct RuntimeChrome {
    pub last_pointer: LogicalPoint,
    title_bar: TitleBarDragTracker,
}

impl RuntimeChrome {
    pub(super) fn title_bar_chrome_messages(
        &mut self,
        document: &RuntimeDocument,
        event: &InputEvent,
    ) -> Vec<GalleryMessage> {
        self.title_bar
            .events(document.context(), document.document(), event)
            .into_iter()
            .map(GalleryMessage::WindowChrome)
            .collect()
    }
}

pub(super) fn apply_title_bar_insets(
    context: &mut AppContext,
    shell: Entity<DesktopShell>,
    leading_inset: f32,
    trailing_inset: f32,
    maximized: bool,
    show_window_controls: bool,
) {
    if let Ok(Some(title_bar)) = context.read(shell, |shell| {
        shell
            .title_bar
            .map(Entity::<nana_ui::runtime::AppTitleBar>::from_stable_id)
    }) {
        let _ = context.update_component(title_bar, |bar, _| {
            bar.leading_inset = leading_inset;
            bar.trailing_inset = trailing_inset;
            bar.maximized = maximized;
            bar.show_window_controls = show_window_controls;
        });
    }
}

pub(super) fn apply_workspace_corners(
    context: &mut AppContext,
    shell: Entity<DesktopShell>,
    workspace_corners: bool,
) {
    if let Ok(Some(workspace)) = context.read(shell, |shell| {
        shell.workspace.map(Entity::<Workspace>::from_stable_id)
    }) {
        let _ = context.update_component(workspace, |workspace, _| {
            workspace.workspace_corners = workspace_corners;
        });
    }
}

pub(super) fn node_is_or_under(
    context: &AppContext,
    target: StableNodeId,
    root: StableNodeId,
) -> bool {
    let mut current = Some(target);
    while let Some(id) = current {
        if id == root {
            return true;
        }
        current = context.world().node(id).and_then(|node| node.parent);
    }
    false
}

pub(super) fn styled_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    size: f32,
    weight: u16,
) -> Text {
    labeled_text(value, color, size, weight, Some(LengthSpec::Fill))
}

pub(super) fn hugging_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    size: f32,
    weight: u16,
) -> Text {
    labeled_text(value, color, size, weight, Some(LengthSpec::Shrink))
}

pub(super) fn labeled_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    size: f32,
    weight: u16,
    width: Option<LengthSpec>,
) -> Text {
    let mut style = NodeStyle {
        foreground: Some(color),
        ..NodeStyle::default()
    };
    let layout = std::sync::Arc::make_mut(&mut style.layout);
    layout.font_size = Some(size);
    layout.font_weight = Some(weight);
    layout.width = width;
    Text::new(value).style(style)
}

pub(super) fn sidebar_toggle_button(collapsed: bool) -> IconButton {
    IconButton::new(Icon::Sidebar, "切换侧栏")
        .size(ControlSize::Small)
        .selected(collapsed)
        .kind(if collapsed {
            ButtonKind::Selected
        } else {
            ButtonKind::Ghost
        })
}

pub(super) fn theme_toggle_button(theme: ThemeMode) -> IconButton {
    let icon = match theme {
        ThemeMode::Dark => Icon::Appearance,
        ThemeMode::Light => Icon::Moon,
    };
    IconButton::new(icon, "切换主题")
        .size(ControlSize::Small)
        .kind(ButtonKind::Text)
}

pub(super) fn search_command_button() -> IconButton {
    IconButton::new(Icon::Search, "搜索命令")
        .size(ControlSize::Small)
        .kind(ButtonKind::Text)
}

pub(super) fn bind_event<V, E>(
    context: &mut AppContext,
    entity: Entity<V>,
    pending: Arc<Mutex<Vec<GalleryMessage>>>,
    map: impl Fn(&E) -> GalleryMessage + Send + 'static,
) -> Result<(), FrameworkError>
where
    V: View,
    E: Send + 'static,
{
    context.on(entity, move |_, event: &E, _| {
        if let Ok(mut pending) = pending.lock() {
            pending.push(map(event));
        }
    })
}

pub(super) fn take_pending(pending: &Arc<Mutex<Vec<GalleryMessage>>>) -> Vec<GalleryMessage> {
    pending
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default()
}

pub(super) fn runtime_input_event(
    input: &RuntimeSceneInput,
    last_pointer: LogicalPoint,
) -> InputEvent {
    match *input {
        RuntimeSceneInput::PointerMove(point) => runtime_pointer(PointerPhase::Move, point, 0),
        RuntimeSceneInput::PointerDown { button, point } => {
            runtime_pointer(PointerPhase::Down, point, button)
        }
        RuntimeSceneInput::PointerUp { button, point } => {
            runtime_pointer(PointerPhase::Up, point, button)
        }
        RuntimeSceneInput::Scroll {
            delta_y,
            line_delta,
        } => InputEvent::Wheel {
            x: last_pointer.x,
            y: last_pointer.y,
            delta_x: 0.0,
            delta_y,
            line_delta,
            modifiers: InputModifiers::default(),
        },
        RuntimeSceneInput::Key {
            pressed,
            ref key,
            repeat,
            modifiers,
        } => InputEvent::Keyboard {
            pressed,
            key: key.clone(),
            text: None,
            code: key.clone(),
            repeat,
            modifiers,
        },
    }
}

pub(super) fn runtime_pointer(phase: PointerPhase, point: LogicalPoint, button: i16) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x: point.x,
        y: point.y,
        screen_x: point.x,
        screen_y: point.y,
        button,
        buttons: if matches!(phase, PointerPhase::Down | PointerPhase::Move) && button == 0 {
            1
        } else {
            0
        },
        pressure: 0.5,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: button == 0,
        modifiers: InputModifiers::default(),
    }
}

#[derive(Clone)]
pub(super) struct HostStack {
    direction: FlexDirection,
    gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    width: Option<LengthSpec>,
    height: Option<LengthSpec>,
    min_width: Option<LengthSpec>,
    min_height: Option<LengthSpec>,
    max_width: Option<LengthSpec>,
    padding: Option<f32>,
    padding_x: Option<f32>,
    padding_y: Option<f32>,
    background: Option<SemanticColorRole>,
    grow: Option<f32>,
    shrink: Option<f32>,
}

impl HostStack {
    fn base(direction: FlexDirection, gap: f32, align: AlignSpec, justify: JustifySpec) -> Self {
        Self {
            direction,
            gap,
            align,
            justify,
            width: None,
            height: None,
            min_width: None,
            min_height: None,
            max_width: None,
            padding: None,
            padding_x: None,
            padding_y: None,
            background: None,
            grow: None,
            shrink: None,
        }
    }

    pub(super) fn column(gap: f32) -> Self {
        Self::base(
            FlexDirection::Column,
            gap,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
    }

    pub(super) fn row(gap: f32) -> Self {
        Self::base(FlexDirection::Row, gap, AlignSpec::Center, JustifySpec::End)
            .width(LengthSpec::Shrink)
    }

    pub(super) fn leading_row(gap: f32) -> Self {
        Self::base(
            FlexDirection::Row,
            gap,
            AlignSpec::Center,
            JustifySpec::Start,
        )
        .width(LengthSpec::Shrink)
    }

    /// Horizontal track. Does not grow on a column parent; children opt into grow.
    pub(super) fn fill_row(gap: f32) -> Self {
        Self::base(
            FlexDirection::Row,
            gap,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
        .min_width(LengthSpec::Px(0.0))
        .grow(0.0)
        .shrink(1.0)
    }

    pub(super) fn fill_column(gap: f32) -> Self {
        Self::base(
            FlexDirection::Column,
            gap,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
        .height(LengthSpec::Fill)
        .min_width(LengthSpec::Px(0.0))
        .min_height(LengthSpec::Px(0.0))
        .grow(1.0)
        .shrink(1.0)
    }

    /// Equal-width row child. `min_width: 0` so siblings share leftover space.
    pub(super) fn flex_child() -> Self {
        Self::base(
            FlexDirection::Column,
            0.0,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
        .min_width(LengthSpec::Px(0.0))
        .grow(1.0)
        .shrink(1.0)
    }

    pub(super) fn spacer() -> Self {
        Self::base(
            FlexDirection::Row,
            0.0,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
        .min_width(LengthSpec::Px(0.0))
        .height(LengthSpec::Px(0.0))
        .grow(1.0)
        .shrink(1.0)
    }

    /// Outer workspace-region host. Workspace overwrites this node's style.
    pub(super) fn region_slot() -> Self {
        Self::fill_column(0.0).grow(0.0)
    }

    pub(super) fn canvas() -> Self {
        Self::fill_column(12.0)
            .padding(16.0)
            .background(SemanticColorRole::Background)
    }

    pub(super) fn panel(gap: f32) -> Self {
        Self::base(
            FlexDirection::Column,
            gap,
            AlignSpec::Stretch,
            JustifySpec::Start,
        )
        .width(LengthSpec::Fill)
        .min_width(LengthSpec::Px(0.0))
        .padding_xy(12.0, 10.0)
        .background(SemanticColorRole::Surface)
        .grow(0.0)
        .shrink(1.0)
    }

    pub(super) fn width(mut self, width: LengthSpec) -> Self {
        self.width = Some(width);
        self
    }

    pub(super) fn height(mut self, height: LengthSpec) -> Self {
        self.height = Some(height);
        self
    }

    pub(super) fn min_width(mut self, min_width: LengthSpec) -> Self {
        self.min_width = Some(min_width);
        self
    }

    pub(super) fn min_height(mut self, min_height: LengthSpec) -> Self {
        self.min_height = Some(min_height);
        self
    }

    pub(super) fn max_width(mut self, max_width: LengthSpec) -> Self {
        self.max_width = Some(max_width);
        self
    }

    pub(super) fn padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding);
        self
    }

    pub(super) fn padding_xy(mut self, x: f32, y: f32) -> Self {
        self.padding_x = Some(x);
        self.padding_y = Some(y);
        self
    }

    pub(super) fn background(mut self, background: SemanticColorRole) -> Self {
        self.background = Some(background);
        self
    }

    pub(super) fn align(mut self, align: AlignSpec) -> Self {
        self.align = align;
        self
    }

    pub(super) fn grow(mut self, grow: f32) -> Self {
        self.grow = Some(grow);
        self
    }

    pub(super) fn shrink(mut self, shrink: f32) -> Self {
        self.shrink = Some(shrink);
        self
    }
}

impl ComponentView for HostStack {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "stack".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = NodeStyle {
            background: self.background,
            ..NodeStyle::default()
        };
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        layout.direction = Some(self.direction);
        layout.gap = Some(LengthSpec::Px(self.gap));
        layout.align_items = self.align;
        layout.justify_content = self.justify;
        layout.width = self.width;
        layout.height = self.height;
        layout.min_width = self.min_width;
        layout.min_height = self.min_height;
        layout.max_width = self.max_width;
        if self.min_width == Some(LengthSpec::Px(0.0))
            || self.min_height == Some(LengthSpec::Px(0.0))
        {
            layout.allow_shrink = true;
        }
        if let Some(padding) = self.padding {
            layout.padding = Some(LengthSpec::Px(padding));
        }
        if let Some(x) = self.padding_x {
            layout.padding_left = Some(LengthSpec::Px(x));
            layout.padding_right = Some(LengthSpec::Px(x));
        }
        if let Some(y) = self.padding_y {
            layout.padding_top = Some(LengthSpec::Px(y));
            layout.padding_bottom = Some(LengthSpec::Px(y));
        }
        if let Some(grow) = self.grow {
            layout.flex_grow = Some(grow);
        }
        if let Some(shrink) = self.shrink {
            layout.flex_shrink = Some(shrink);
        }
        if self.background.is_some() {
            layout.border_radius = Some(8.0);
        }
        if world.node_style(id) != Some(&style) {
            mutations.set_style(id, style);
        }
        let interaction = InteractionState {
            pointer_events: false,
            focusable: false,
        };
        if world.interaction(id) != Some(interaction) {
            mutations.set_interaction(id, interaction);
        }
    }
}

pub(super) fn reconcile_children(
    context: &mut AppContext,
    parent: StableNodeId,
    ordered: &[StableNodeId],
) -> Result<(), FrameworkError> {
    let ordered = ordered
        .iter()
        .copied()
        .filter(|id| *id != parent && context.world().contains(*id))
        .collect::<Vec<_>>();
    let current = context
        .world()
        .node(parent)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    if current.as_slice() == ordered.as_slice() {
        return Ok(());
    }
    let keep = ordered
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut mutations = MutationQueue::new();
    for child in &current {
        if !keep.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    for child in ordered {
        mutations.insert(parent, child, None);
    }
    context.commit_mutations(mutations)?;
    Ok(())
}
