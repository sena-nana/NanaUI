use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LayoutStyle, LengthSpec,
    LineHeightSpec, SemanticColorRole,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, StandardVisual, TextContent,
    TextHorizontalAlignment, TextVerticalAlignment, UiWorld,
};

/// Backend-neutral selection orientation. SegmentedControl currently supports
/// only its design-language horizontal form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionOrientation {
    #[default]
    Horizontal,
}

/// Shared selection contract with two product surfaces.
///
/// `Segmented` keeps the bordered pill. `Tabs` is the same RadioGroup/roving
/// behavior on an independent tab strip (no outer chrome). Professional
/// reorder, close, and drag/lease behavior lives on [`crate::Tabs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionChrome {
    #[default]
    Segmented,
    Tabs,
}

/// Selection-independent intent consumed by roving-focus components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RovingFocusIntent {
    Previous,
    Next,
    First,
    Last,
}

/// Resolves one enabled tab stop without knowing or changing selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RovingFocusPolicy {
    pub wrap: bool,
}

impl Default for RovingFocusPolicy {
    fn default() -> Self {
        Self { wrap: true }
    }
}

impl RovingFocusPolicy {
    pub fn resolve<T: Copy + Eq>(
        self,
        items: &[(T, bool)],
        current: Option<T>,
        intent: RovingFocusIntent,
    ) -> Option<T> {
        let enabled = items
            .iter()
            .filter_map(|(id, enabled)| enabled.then_some(*id))
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return None;
        }
        match intent {
            RovingFocusIntent::First => enabled.first().copied(),
            RovingFocusIntent::Last => enabled.last().copied(),
            RovingFocusIntent::Previous | RovingFocusIntent::Next => {
                let current =
                    current.and_then(|current| enabled.iter().position(|id| *id == current));
                let next = match (current, intent) {
                    (Some(0), RovingFocusIntent::Previous) if self.wrap => enabled.len() - 1,
                    (Some(0), RovingFocusIntent::Previous) => 0,
                    (Some(index), RovingFocusIntent::Previous) => index - 1,
                    (Some(index), RovingFocusIntent::Next) if index + 1 < enabled.len() => {
                        index + 1
                    }
                    (Some(_), RovingFocusIntent::Next) if self.wrap => 0,
                    (Some(index), RovingFocusIntent::Next) => index,
                    (None, RovingFocusIntent::Previous) => enabled.len() - 1,
                    (None, RovingFocusIntent::Next) => 0,
                    _ => unreachable!(),
                };
                enabled.get(next).copied()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedControl {
    pub label: Option<Arc<str>>,
    pub orientation: SelectionOrientation,
    pub(crate) chrome: SelectionChrome,
    pub(crate) fill: bool,
    pub(crate) size: ControlSize,
    pub(crate) options: Vec<StableNodeId>,
    pub(crate) selected: Option<StableNodeId>,
    pub(crate) focus_target: Option<StableNodeId>,
    pub(crate) roving_focus: RovingFocusPolicy,
    pub style: NodeStyle,
}

impl SegmentedControl {
    pub fn new() -> Self {
        Self {
            label: None,
            orientation: SelectionOrientation::Horizontal,
            chrome: SelectionChrome::Segmented,
            fill: false,
            size: ControlSize::Medium,
            options: Vec::new(),
            selected: None,
            focus_target: None,
            roving_focus: RovingFocusPolicy::default(),
            style: selection_chrome_style(SelectionChrome::Segmented, ControlSize::Medium, false),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.apply_size(size);
        self
    }

    pub(crate) fn apply_size(&mut self, size: ControlSize) {
        self.size = size;
        self.style = selection_chrome_style(self.chrome, size, self.fill);
    }
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self.style = selection_chrome_style(self.chrome, self.size, fill);
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn fill_value(&self) -> bool {
        self.fill
    }

    pub const fn size_value(&self) -> ControlSize {
        self.size
    }

    pub fn options(&self) -> &[StableNodeId] {
        &self.options
    }

    pub const fn selected(&self) -> Option<StableNodeId> {
        self.selected
    }

    pub const fn focus_target(&self) -> Option<StableNodeId> {
        self.focus_target
    }
}

impl Default for SegmentedControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for SegmentedControl {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "segmented-control".into(),
        }
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.style.clone();
        Arc::make_mut(&mut style.layout).border_radius = Some(world.theme_metrics().radius_md);
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
                role: AccessibilityRole::RadioGroup,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedOption {
    pub label: Arc<str>,
    pub icon: Option<Icon>,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
    pub(crate) size: ControlSize,
    pub(crate) chrome: SelectionChrome,
    pub(crate) fill: bool,
    pub style: NodeStyle,
}

impl SegmentedOption {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            selected: false,
            disabled: false,
            size: ControlSize::Medium,
            chrome: SelectionChrome::Segmented,
            fill: false,
            style: segmented_option_style(ControlSize::Medium, SelectionChrome::Segmented, false),
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.synchronize_surface(size, self.chrome, self.fill);
        self
    }
    pub fn surface(mut self, size: ControlSize, chrome: SelectionChrome, fill: bool) -> Self {
        self.synchronize_surface(size, chrome, fill);
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn selected(&self) -> bool {
        self.selected
    }

    pub const fn disabled_value(&self) -> bool {
        self.disabled
    }

    pub const fn size_value(&self) -> ControlSize {
        self.size
    }

    pub(crate) fn synchronize_surface(
        &mut self,
        size: ControlSize,
        chrome: SelectionChrome,
        fill: bool,
    ) {
        self.size = size;
        self.chrome = chrome;
        self.fill = fill;
        self.style = segmented_option_style(size, chrome, fill);
    }
}

pub(crate) fn selection_chrome_style(
    chrome: SelectionChrome,
    size: ControlSize,
    fill: bool,
) -> NodeStyle {
    let (gap, padding, border_width, background, border) = match chrome {
        SelectionChrome::Segmented => (
            2.0,
            2.0,
            1.0,
            Some(SemanticColorRole::Background),
            Some(SemanticColorRole::Border),
        ),
        SelectionChrome::Tabs => (4.0, 0.0, 0.0, None, None),
    };
    let layout = LayoutStyle {
        direction: Some(FlexDirection::Row),
        gap: Some(LengthSpec::Px(gap)),
        padding: Some(LengthSpec::Px(padding)),
        width: Some(if fill {
            LengthSpec::Percent(100.0)
        } else {
            LengthSpec::Shrink
        }),
        height: Some(LengthSpec::Px(size.height())),
        align_items: AlignSpec::Center,
        border_width: Some(border_width),
        border_radius: Some(if matches!(chrome, SelectionChrome::Segmented) {
            10.0
        } else {
            0.0
        }),
        ..LayoutStyle::default()
    };
    NodeStyle {
        layout: Arc::new(layout),
        background,
        border,
        ..NodeStyle::default()
    }
}

fn option_height(size: ControlSize, chrome: SelectionChrome) -> f32 {
    match chrome {
        SelectionChrome::Segmented => (size.height() - 6.0).max(0.0),
        SelectionChrome::Tabs => size.height(),
    }
}

fn segmented_option_style(size: ControlSize, chrome: SelectionChrome, fill: bool) -> NodeStyle {
    let padding = size.padding_x() + 2.0;
    let layout = LayoutStyle {
        height: Some(LengthSpec::Px(option_height(size, chrome))),
        padding_left: Some(LengthSpec::Px(padding)),
        padding_right: Some(LengthSpec::Px(padding)),
        font_size: Some(size.text_size()),
        font_weight: Some(500),
        line_height: Some(LineHeightSpec::Absolute(size.line_height())),
        white_space_nowrap: true,
        align_self: Some(AlignSpec::Center),
        justify_content: JustifySpec::Center,
        border_radius: Some(match chrome {
            SelectionChrome::Segmented => 7.0,
            SelectionChrome::Tabs => 6.0,
        }),
        flex_grow: fill.then_some(1.0),
        flex_shrink: fill.then_some(1.0),
        width: fill.then_some(LengthSpec::Fill),
        ..LayoutStyle::default()
    };
    NodeStyle {
        layout: Arc::new(layout),
        foreground: Some(SemanticColorRole::Muted),
        interaction: InteractionStyle {
            selected: SemanticPaint {
                foreground: Some(SemanticColorRole::Text),
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
                foreground: Some(SemanticColorRole::Text),
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
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                ..SemanticPaint::default()
            },
        },
        text_horizontal_alignment: TextHorizontalAlignment::Center,
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

impl ComponentView for SegmentedOption {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: match self.chrome {
                SelectionChrome::Segmented => "segmented-option".into(),
                SelectionChrome::Tabs => "tab".into(),
            },
        }
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.to_string(),
                },
            );
        }
        let visual = StandardVisual::SelectionOption {
            label: Arc::clone(&self.label),
            icon: self.icon,
            selected: self.selected,
            disabled: self.disabled,
            size: self.size,
            show_focus_ring: matches!(self.chrome, SelectionChrome::Segmented),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let radius = match self.chrome {
            SelectionChrome::Segmented => (world.theme_metrics().radius_md - 3.0).max(0.0),
            SelectionChrome::Tabs => world.theme_metrics().radius_sm,
        };
        Arc::make_mut(&mut effective_style.layout).border_radius = Some(radius);
        if self.icon.is_some() {
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.padding_left = Some(LengthSpec::Px(
                self.size.padding_x() + 2.0 + self.size.icon_size() + 5.0,
            ));
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: match self.chrome {
                    SelectionChrome::Segmented => AccessibilityRole::Radio,
                    SelectionChrome::Tabs => AccessibilityRole::Tab,
                },
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                selected: matches!(self.chrome, SelectionChrome::Tabs).then_some(self.selected),
                checked: matches!(self.chrome, SelectionChrome::Segmented).then_some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentedSelectionRequested {
    pub option: StableNodeId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    #[test]
    fn roving_focus_skips_disabled_and_wraps() {
        let items = [(id(1), true), (id(2), false), (id(3), true)];
        let policy = RovingFocusPolicy::default();
        assert_eq!(
            policy.resolve(&items, Some(id(1)), RovingFocusIntent::Next),
            Some(id(3))
        );
        assert_eq!(
            policy.resolve(&items, Some(id(3)), RovingFocusIntent::Next),
            Some(id(1))
        );
        assert_eq!(
            policy.resolve(&items, Some(id(1)), RovingFocusIntent::Previous),
            Some(id(3))
        );
        assert_eq!(
            policy.resolve(&items, None, RovingFocusIntent::First),
            Some(id(1))
        );
        assert_eq!(
            policy.resolve(&items, None, RovingFocusIntent::Last),
            Some(id(3))
        );
    }

    #[test]
    fn segmented_sizes_preserve_concentric_layout_and_semantic_states() {
        for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
            let option = SegmentedOption::new("Preview").size(size);
            assert_eq!(
                option.style.layout.height,
                Some(LengthSpec::Px(size.height() - 6.0))
            );
            assert_eq!(option.style.layout.font_size, Some(size.text_size()));
            assert!(option.style.layout.white_space_nowrap);
            assert_eq!(option.style.layout.border_radius, Some(7.0));
            assert_eq!(
                option.style.interaction.selected.background,
                Some(SemanticColorRole::Selected)
            );
            let control = SegmentedControl::new().size(size);
            assert_eq!(
                control.style.layout.height,
                Some(LengthSpec::Px(size.height()))
            );
            assert_eq!(control.style.layout.border_width, Some(1.0));
            assert_eq!(control.style.layout.border_radius, Some(10.0));
            assert_eq!(control.style.layout.width, Some(LengthSpec::Shrink));
        }
    }

    #[test]
    fn tabs_chrome_uses_independent_surface_and_tab_roles() {
        let tabs = crate::Tabs::new("preview");
        assert_eq!(tabs.size, ControlSize::Small);
        assert_eq!(tabs.style.layout.border_width, Some(0.0));
        assert_eq!(tabs.style.layout.gap, Some(LengthSpec::Px(4.0)));
        assert!(tabs.style.background.is_none());
        assert_eq!(tabs.node_kind(), NodeKind::Element { tag: "tabs".into() });

        let option = SegmentedOption::new("Preview").surface(
            ControlSize::Small,
            SelectionChrome::Tabs,
            false,
        );
        assert_eq!(
            option.style.layout.height,
            Some(LengthSpec::Px(ControlSize::Small.height()))
        );
        assert_eq!(option.node_kind(), NodeKind::Element { tag: "tab".into() });
    }

    #[test]
    fn tabs_options_do_not_request_a_focus_ring() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let tab = context
            .create_component(
                document,
                SegmentedOption::new("Code").surface(
                    ControlSize::Small,
                    SelectionChrome::Tabs,
                    false,
                ),
            )
            .unwrap();
        let segmented = context
            .create_component(
                document,
                SegmentedOption::new("Code").size(ControlSize::Medium),
            )
            .unwrap();
        assert!(matches!(
            context.world().standard_visual(tab.stable_id()),
            Some(StandardVisual::SelectionOption {
                show_focus_ring: false,
                ..
            })
        ));
        assert!(matches!(
            context.world().standard_visual(segmented.stable_id()),
            Some(StandardVisual::SelectionOption {
                show_focus_ring: true,
                ..
            })
        ));
    }
}
