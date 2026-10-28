//! Toolbar and status bar: the two horizontal strips a desktop shell has that
//! `AppTitleBar` does not cover; and the tool group, a run of tool buttons on
//! one track inside either, or down the side of a canvas.
//!
//! Both are containers — the application puts its own controls in them. What
//! they add over a plain [`crate::Stack`] row is the shell surface treatment
//! and, more importantly, the accessibility role: a screen reader announces a
//! toolbar's contents as a group of controls and a status bar as a live status
//! region, which a bare layout box cannot convey.

use std::sync::Arc;

use nana_ui_core::{AlignSpec, FlexDirection, JustifySpec, LengthSpec, SemanticColorRole, space};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, SelectionOrientation, StableNodeId, UiWorld,
};

/// Strip of actions above the content (`nana.toolbar`).
#[derive(Debug, Clone, PartialEq)]
pub struct Toolbar {
    /// Accessible name, so a window with several toolbars is navigable.
    pub label: Option<Arc<str>>,
    /// Draws the shell surface and a bottom hairline. Off for a toolbar nested
    /// in a surface that already provides them.
    pub chrome: bool,
    pub style: NodeStyle,
}

impl Toolbar {
    pub fn new() -> Self {
        Self {
            label: None,
            chrome: true,
            style: bar_style(true, false),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn chrome(mut self, chrome: bool) -> Self {
        self.chrome = chrome;
        self.style = bar_style(chrome, false);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip of state below the content (`nana.status-bar`).
///
/// Announced as a live status region: assistive technology reports changes
/// here without the user moving focus to it.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBar {
    pub label: Option<Arc<str>>,
    pub chrome: bool,
    pub style: NodeStyle,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            label: None,
            chrome: true,
            style: bar_style(true, true),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn chrome(mut self, chrome: bool) -> Self {
        self.chrome = chrome;
        self.style = bar_style(chrome, true);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared strip geometry. The hairline sits on the edge facing the content.
fn bar_style(chrome: bool, below_content: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = AlignSpec::Center;
    layout.justify_content = JustifySpec::Start;
    layout.gap = Some(LengthSpec::Px(space::SM));
    layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
    layout.width = Some(LengthSpec::Fill);
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.padding_left = Some(LengthSpec::Px(space::MD));
    layout.theme.padding_left = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
    layout.padding_right = Some(LengthSpec::Px(space::MD));
    layout.theme.padding_right = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
    layout.padding_top = Some(LengthSpec::Px(space::XS));
    layout.theme.padding_top = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
    layout.padding_bottom = Some(LengthSpec::Px(space::XS));
    layout.theme.padding_bottom = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
    // A strip of controls: when they do not fit, its gaps and then their
    // declared padding close up before anything clips (Issue #212).
    layout.adaptation = Some(nana_ui_core::dynamic_layout::AdaptationProfile::strip());
    if chrome {
        style.background = Some(SemanticColorRole::Surface);
        style.border = Some(SemanticColorRole::BorderSoft);
        layout.border_width = Some(nana_ui_core::HAIRLINE);
        layout.theme.border_width =
            Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        // Only the edge that meets the content is drawn.
        if below_content {
            layout.border_top_width = Some(nana_ui_core::HAIRLINE);
            layout.theme.border_top_width =
                Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
            layout.border_right_width = Some(0.0);
            layout.border_bottom_width = Some(0.0);
            layout.border_left_width = Some(0.0);
        } else {
            layout.border_top_width = Some(0.0);
            layout.border_right_width = Some(0.0);
            layout.border_bottom_width = Some(nana_ui_core::HAIRLINE);
            layout.theme.border_bottom_width =
                Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
            layout.border_left_width = Some(0.0);
        }
    }
    style
}

fn project_bar(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    role: AccessibilityRole,
    label: Option<&Arc<str>>,
) {
    project_common(
        id,
        world,
        mutations,
        style,
        InteractionState {
            pointer_events: false,
            focusable: false,
        },
        AccessibilityState {
            role,
            label: label.cloned(),
            ..AccessibilityState::default()
        },
    );
}

/// How the buttons of a [`ToolGroup`] relate: what assistive technology
/// hears of the group. It does not change how the buttons look; each button
/// shows its own `selected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolGroupMode {
    /// Each press does something and nothing stays on: create, delete,
    /// reorder. Announced as a toolbar.
    #[default]
    Actions,
    /// One of the buttons is the current tool, the one the application marks
    /// `selected`. Announced as a radio group. Several groups can share one
    /// current tool: the application marks it in whichever group holds it.
    Exclusive,
    /// Each button is on or off by itself, such as display toggles.
    /// Announced as a toolbar; each button reports its own `selected`.
    Toggles,
}

/// A run of tool buttons on one track (`nana.tool-group`): the group of a
/// drawing tool's toolbar, a canvas's display toggles, a panel's own actions.
///
/// The application puts its own [`crate::IconButton`]s in it and keeps which
/// is selected, as it does for a lone icon button. What the group adds is
/// the track that reads them as one group, quieter than the surface around
/// it, with the buttons a hairline apart, laid out down a column or along a
/// row; and the group's role, so assistive technology announces the buttons
/// as one set. Keyboard focus moves through the buttons with the arrow keys
/// of the group's direction when the view adds
/// `.roving_focus(RovingFocusGroup::vertical())` (or `horizontal`).
///
/// Groups next to each other are separated by the gap between them, not by
/// a divider: each track already ends its group.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolGroup {
    /// Accessible name of the set, such as "Selection tools".
    pub label: Option<Arc<str>>,
    pub orientation: SelectionOrientation,
    pub mode: ToolGroupMode,
    pub style: NodeStyle,
}

impl ToolGroup {
    /// A row of buttons on a track.
    pub fn new() -> Self {
        Self {
            label: None,
            orientation: SelectionOrientation::Horizontal,
            mode: ToolGroupMode::Actions,
            style: tool_group_style(),
        }
    }

    /// A column of buttons on a track, such as a canvas's tool shelf.
    pub fn vertical() -> Self {
        Self::new().orientation(SelectionOrientation::Vertical)
    }

    pub fn orientation(mut self, orientation: SelectionOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn mode(mut self, mode: ToolGroupMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Replaces the node style wholesale; the group still lays its buttons
    /// out along its orientation.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        Arc::make_mut(&mut style.layout).direction = Some(match self.orientation {
            SelectionOrientation::Horizontal => FlexDirection::Row,
            SelectionOrientation::Vertical => FlexDirection::Column,
        });
        style
    }
}

impl Default for ToolGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// The track: the subtle fill, the buttons a hairline gap apart and a
/// hairline from its edge, the card radius so a button's own corners nest
/// inside it.
fn tool_group_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Subtle);
    style.radius = Some(nana_ui_core::RadiusTier::Md);
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = AlignSpec::Center;
    layout.justify_content = JustifySpec::Start;
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.gap = Some(LengthSpec::Px(space::XXS));
    layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
    let inset = Some(LengthSpec::Px(space::XXS));
    layout.padding_left = inset;
    layout.padding_right = inset;
    layout.padding_top = inset;
    layout.padding_bottom = inset;
    let inset = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
    layout.theme.padding_left = inset.clone();
    layout.theme.padding_right = inset.clone();
    layout.theme.padding_top = inset.clone();
    layout.theme.padding_bottom = inset;
    style
}

impl ComponentView for ToolGroup {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tool-group".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
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
                role: match self.mode {
                    ToolGroupMode::Exclusive => AccessibilityRole::RadioGroup,
                    ToolGroupMode::Actions | ToolGroupMode::Toggles => AccessibilityRole::Toolbar,
                },
                label: self.label.clone(),
                orientation: Some(self.orientation),
                ..AccessibilityState::default()
            },
        );
    }
}

impl ComponentView for Toolbar {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "toolbar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_bar(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Toolbar,
            self.label.as_ref(),
        );
    }
}

impl ComponentView for StatusBar {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "status-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_bar(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Status,
            self.label.as_ref(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn the_bars_carry_the_roles_a_screen_reader_needs() {
        let mut cx = AppContext::new();
        let toolbar = cx
            .create_component(document(), Toolbar::new().label("Formatting"))
            .unwrap();
        let status = cx
            .create_component(document(), StatusBar::new().label("Document status"))
            .unwrap();

        let semantics = |id| {
            let state = cx.world().accessibility(id).cloned().unwrap();
            (state.role, state.label.map(|label| label.to_string()))
        };
        assert_eq!(
            semantics(toolbar.stable_id()),
            (AccessibilityRole::Toolbar, Some("Formatting".to_owned()))
        );
        assert_eq!(
            semantics(status.stable_id()),
            (
                AccessibilityRole::Status,
                Some("Document status".to_owned())
            )
        );
    }

    #[test]
    fn a_tool_group_is_one_set_along_its_orientation() {
        let mut cx = AppContext::new();
        let shelf = cx
            .create_component(
                document(),
                ToolGroup::vertical()
                    .mode(ToolGroupMode::Exclusive)
                    .label("Selection tools"),
            )
            .unwrap();
        let toggles = cx
            .create_component(document(), ToolGroup::new().mode(ToolGroupMode::Toggles))
            .unwrap();
        let actions = cx.create_component(document(), ToolGroup::new()).unwrap();

        let semantics = |id| {
            let state = cx.world().accessibility(id).cloned().unwrap();
            (
                state.role,
                state.orientation,
                state.label.map(|l| l.to_string()),
            )
        };
        assert_eq!(
            semantics(shelf.stable_id()),
            (
                AccessibilityRole::RadioGroup,
                Some(SelectionOrientation::Vertical),
                Some("Selection tools".to_owned())
            )
        );
        assert_eq!(semantics(toggles.stable_id()).0, AccessibilityRole::Toolbar);
        assert_eq!(
            semantics(actions.stable_id()),
            (
                AccessibilityRole::Toolbar,
                Some(SelectionOrientation::Horizontal),
                None
            )
        );
        let direction = |id| cx.world().node_style(id).unwrap().layout.direction;
        assert_eq!(direction(shelf.stable_id()), Some(FlexDirection::Column));
        assert_eq!(direction(actions.stable_id()), Some(FlexDirection::Row));
        let track = cx.world().node_style(actions.stable_id()).unwrap();
        assert_eq!(track.background, Some(SemanticColorRole::Subtle));
    }

    #[test]
    fn the_hairline_sits_on_the_edge_that_meets_the_content() {
        let toolbar = Toolbar::new();
        assert_eq!(
            toolbar.style.layout.border_bottom_width,
            Some(nana_ui_core::HAIRLINE)
        );
        assert_eq!(toolbar.style.layout.border_top_width, Some(0.0));

        let status = StatusBar::new();
        assert_eq!(
            status.style.layout.border_top_width,
            Some(nana_ui_core::HAIRLINE)
        );
        assert_eq!(status.style.layout.border_bottom_width, Some(0.0));

        // Nested in a surface that already draws them, a bar takes no chrome.
        let bare = Toolbar::new().chrome(false);
        assert_eq!(bare.style.background, None);
        assert_eq!(bare.style.layout.border_width, None);
    }
}
