use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, FlexDirection, JustifySpec, LengthSpec, OverflowSpec, SemanticColorRole, SplitAxis,
    SplitPaneModel, SplitPaneMutation,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, TextContent, UiWorld,
};

pub(crate) const HANDLE_SIZE: f32 = 8.0;
pub(crate) const INDICATOR_SIZE: f32 = 2.0;

/// Two children and an 8px resize handle. Size comes from [`SplitPaneModel`].
///
/// Host applies [`SplitPaneMutation`]; this view reflects the model. Application
/// content stays in the `first` / `second` slots. Assign `handle` to a
/// host-created node; its optional first child is the 2px indicator.
#[derive(Debug, Clone)]
pub struct SplitPane {
    pub first: Option<StableNodeId>,
    pub second: Option<StableNodeId>,
    pub handle: Option<StableNodeId>,
    pub model: SplitPaneModel,
}

impl SplitPane {
    pub fn from_model(model: &SplitPaneModel, first: StableNodeId, second: StableNodeId) -> Self {
        Self {
            first: Some(first),
            second: Some(second),
            handle: None,
            model: model.clone(),
        }
    }

    pub fn handle(mut self, handle: StableNodeId) -> Self {
        self.handle = Some(handle);
        self
    }

    pub fn apply(&mut self, mutation: SplitPaneMutation) -> bool {
        self.model.update(mutation)
    }

    fn first_is_sized(&self) -> bool {
        !self.model.from_end()
    }

    fn handle_color(&self) -> SemanticColorRole {
        if self.model.is_active() {
            SemanticColorRole::BorderStrong
        } else {
            SemanticColorRole::Border
        }
    }

    fn handle_focusable(&self) -> bool {
        true
    }

    fn root_style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(split_direction(self.model.axis()));
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        style
    }

    fn project_root(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
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
            &self.root_style(),
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

    fn project_pane(
        &self,
        id: StableNodeId,
        sized: bool,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        if world.node(id).is_none() {
            return;
        }
        let horizontal = self.model.axis() == SplitAxis::Horizontal;
        let size = self.model.size();
        let (min_size, max_size) = self.model.limits();
        let mut style = world.node_style(id).cloned().unwrap_or_default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        layout.flex_grow = Some(if sized { 0.0 } else { 1.0 });
        layout.flex_shrink = Some(if sized { 0.0 } else { 1.0 });
        if horizontal {
            layout.width = Some(if sized {
                LengthSpec::Px(size)
            } else {
                LengthSpec::Fill
            });
            layout.height = Some(LengthSpec::Fill);
            if sized {
                layout.min_width = Some(LengthSpec::Px(min_size));
                layout.max_width = Some(LengthSpec::Px(max_size));
            } else {
                layout.min_width = Some(LengthSpec::Px(0.0));
                layout.max_width = None;
            }
        } else {
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(if sized {
                LengthSpec::Px(size)
            } else {
                LengthSpec::Fill
            });
            if sized {
                layout.min_height = Some(LengthSpec::Px(min_size));
                layout.max_height = Some(LengthSpec::Px(max_size));
            } else {
                layout.min_height = Some(LengthSpec::Px(0.0));
                layout.max_height = None;
            }
        }
        if world.node_style(id) != Some(&style) {
            mutations.set_style(id, style);
        }
    }

    fn project_handle(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(id) = self.handle else {
            return;
        };
        if world.node(id).is_none() {
            return;
        }
        let horizontal = self.model.axis() == SplitAxis::Horizontal;
        let indicator = world
            .node(id)
            .and_then(|node| node.children.first().copied());
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
        let color = self.handle_color();
        let mut style = NodeStyle::default();
        style.background = indicator.is_none().then_some(color);
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(if horizontal {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        });
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Center;
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        if horizontal {
            layout.width = Some(LengthSpec::Px(HANDLE_SIZE));
            layout.min_width = Some(LengthSpec::Px(HANDLE_SIZE));
            layout.max_width = Some(LengthSpec::Px(HANDLE_SIZE));
            layout.height = Some(LengthSpec::Fill);
        } else {
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(HANDLE_SIZE));
            layout.min_height = Some(LengthSpec::Px(HANDLE_SIZE));
            layout.max_height = Some(LengthSpec::Px(HANDLE_SIZE));
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: self.handle_focusable(),
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from("Resize")),
                ..AccessibilityState::default()
            },
        );
        if let Some(indicator) = indicator {
            self.project_indicator(indicator, horizontal, color, world, mutations);
        }
    }

    fn project_indicator(
        &self,
        id: StableNodeId,
        horizontal: bool,
        color: SemanticColorRole,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        if world.node(id).is_none() {
            return;
        }
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
        let mut style = NodeStyle::default();
        style.background = Some(color);
        let layout = Arc::make_mut(&mut style.layout);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        if horizontal {
            layout.width = Some(LengthSpec::Px(INDICATOR_SIZE));
            layout.min_width = Some(LengthSpec::Px(INDICATOR_SIZE));
            layout.max_width = Some(LengthSpec::Px(INDICATOR_SIZE));
            layout.height = Some(LengthSpec::Fill);
        } else {
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(INDICATOR_SIZE));
            layout.min_height = Some(LengthSpec::Px(INDICATOR_SIZE));
            layout.max_height = Some(LengthSpec::Px(INDICATOR_SIZE));
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
                ..AccessibilityState::default()
            },
        );
    }
}

pub(crate) fn split_direction(axis: SplitAxis) -> FlexDirection {
    match axis {
        SplitAxis::Horizontal => FlexDirection::Row,
        SplitAxis::Vertical => FlexDirection::Column,
    }
}

impl ComponentView for SplitPane {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "split-pane".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_root(id, world, mutations);
        if let Some(first) = self.first {
            self.project_pane(first, self.first_is_sized(), world, mutations);
        }
        self.project_handle(world, mutations);
        if let Some(second) = self.second {
            self.project_pane(second, !self.first_is_sized(), world, mutations);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn slot(context: &mut AppContext, tag: &str) -> StableNodeId {
        context
            .create_view(document(), NodeKind::Element { tag: tag.into() }, ())
            .unwrap()
            .stable_id()
    }

    fn mount(
        context: &mut AppContext,
        model: &SplitPaneModel,
        first: StableNodeId,
        second: StableNodeId,
        handle: StableNodeId,
    ) -> crate::Entity<SplitPane> {
        context
            .create_component(
                document(),
                SplitPane::from_model(model, first, second).handle(handle),
            )
            .unwrap()
    }

    #[test]
    fn from_model_horizontal_size_goes_to_first() {
        let mut context = AppContext::new();
        let first = slot(&mut context, "first");
        let second = slot(&mut context, "second");
        let handle = slot(&mut context, "handle");
        let mut model = SplitPaneModel::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        model.update(SplitPaneMutation::SetSize(400.0));
        assert_eq!(model.size(), 260.0);
        let split = mount(&mut context, &model, first, second, handle);

        assert_eq!(
            context.world().node(split.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "split-pane".into(),
            }
        );
        assert_eq!(
            context
                .world()
                .accessibility(split.stable_id())
                .unwrap()
                .role,
            AccessibilityRole::Generic
        );
        let first_layout = &context.world().node_style(first).unwrap().layout;
        assert_eq!(first_layout.width, Some(LengthSpec::Px(260.0)));
        assert_eq!(first_layout.min_width, Some(LengthSpec::Px(140.0)));
        assert_eq!(first_layout.max_width, Some(LengthSpec::Px(260.0)));
        assert_eq!(first_layout.height, Some(LengthSpec::Fill));
        assert_eq!(first_layout.flex_grow, Some(0.0));
        let second_layout = &context.world().node_style(second).unwrap().layout;
        assert_eq!(second_layout.width, Some(LengthSpec::Fill));
        assert_eq!(second_layout.flex_grow, Some(1.0));
        assert_eq!(
            context
                .world()
                .node_style(split.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Row)
        );
    }

    #[test]
    fn from_end_assigns_size_to_second() {
        let mut context = AppContext::new();
        let first = slot(&mut context, "first");
        let second = slot(&mut context, "second");
        let handle = slot(&mut context, "handle");
        let model =
            SplitPaneModel::new(SplitAxis::Horizontal, 180.0, 120.0, 240.0).with_from_end(true);
        let _ = mount(&mut context, &model, first, second, handle);

        let first_layout = &context.world().node_style(first).unwrap().layout;
        assert_eq!(first_layout.width, Some(LengthSpec::Fill));
        assert_eq!(first_layout.flex_grow, Some(1.0));
        let second_layout = &context.world().node_style(second).unwrap().layout;
        assert_eq!(second_layout.width, Some(LengthSpec::Px(180.0)));
        assert_eq!(second_layout.min_width, Some(LengthSpec::Px(120.0)));
        assert_eq!(second_layout.max_width, Some(LengthSpec::Px(240.0)));
        assert_eq!(second_layout.flex_grow, Some(0.0));
    }

    #[test]
    fn handle_stays_a_pointer_target_when_not_focused() {
        let mut context = AppContext::new();
        let first = slot(&mut context, "first");
        let second = slot(&mut context, "second");
        let handle = slot(&mut context, "handle");
        let model = SplitPaneModel::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        let split = mount(&mut context, &model, first, second, handle);

        assert!(!model.focused());
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
        assert_eq!(
            context
                .world()
                .accessibility(handle)
                .unwrap()
                .label
                .as_deref(),
            Some("Resize")
        );
        let handle_layout = &context.world().node_style(handle).unwrap().layout;
        assert_eq!(handle_layout.width, Some(LengthSpec::Px(HANDLE_SIZE)));
        assert_eq!(handle_layout.height, Some(LengthSpec::Fill));

        context
            .update_component(split, |pane, _| {
                pane.apply(SplitPaneMutation::Focus);
            })
            .unwrap();
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
    }

    #[test]
    fn handle_active_style_uses_border_strong() {
        let mut context = AppContext::new();
        let first = slot(&mut context, "first");
        let second = slot(&mut context, "second");
        let handle = slot(&mut context, "handle");
        let model = SplitPaneModel::new(SplitAxis::Vertical, 160.0, 80.0, 280.0);
        let split = mount(&mut context, &model, first, second, handle);

        assert_eq!(
            context.world().node_style(handle).unwrap().background,
            Some(SemanticColorRole::Border)
        );
        let handle_layout = &context.world().node_style(handle).unwrap().layout;
        assert_eq!(handle_layout.height, Some(LengthSpec::Px(HANDLE_SIZE)));
        assert_eq!(
            context
                .world()
                .node_style(split.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Column)
        );

        context
            .update_component(split, |pane, _| {
                pane.apply(SplitPaneMutation::Hover(true));
            })
            .unwrap();
        assert_eq!(
            context.world().node_style(handle).unwrap().background,
            Some(SemanticColorRole::BorderStrong)
        );
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let first = slot(&mut context, "first");
        let second = slot(&mut context, "second");
        let handle = slot(&mut context, "handle");
        let model = SplitPaneModel::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        let split = mount(&mut context, &model, first, second, handle);
        let _ = context.take_system_work();
        context.update_component(split, |_, _| {}).unwrap();
        assert!(context.take_system_work().is_empty());
    }
}
