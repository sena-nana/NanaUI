use std::sync::Arc;

use nana_ui_core::{
    DialogClosePolicy, DialogSize, DrawerSide, LayoutStyle, LengthSpec, PositionSpec,
};

use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld, view_components::project_common,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalSurfaceKind {
    Dialog(DialogSize),
    Confirm(DialogSize),
    Drawer(DrawerSide),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalInitialFocus {
    Surface,
    #[default]
    FirstAction,
    Target(StableNodeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalBehavior {
    pub close_policy: DialogClosePolicy,
    pub initial_focus: ModalInitialFocus,
}

impl Default for ModalBehavior {
    fn default() -> Self {
        Self {
            close_policy: DialogClosePolicy::default(),
            initial_focus: ModalInitialFocus::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModalSlots {
    pub body: Option<StableNodeId>,
    pub footer: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub actions: Vec<StableNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmSlots {
    pub body: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub cancel: StableNodeId,
    pub secondary: Option<StableNodeId>,
    pub confirm: StableNodeId,
}

impl ConfirmSlots {
    pub(crate) fn modal_slots(&self) -> ModalSlots {
        ModalSlots {
            body: self.body,
            close_action: self.close_action,
            actions: std::iter::once(self.cancel)
                .chain(self.secondary)
                .chain([self.confirm])
                .collect(),
            ..ModalSlots::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmIntent {
    Cancel,
    Secondary,
    Confirm { danger: bool },
}

impl ModalSlots {
    pub(crate) fn ordered(&self) -> Vec<StableNodeId> {
        self.body
            .into_iter()
            .chain(self.footer)
            .chain(self.close_action)
            .chain(self.actions.iter().copied())
            .collect()
    }
}

pub trait ModalSurface: ComponentView {
    fn slots(&self) -> &ModalSlots;
    fn slots_mut(&mut self) -> &mut ModalSlots;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmDialog {
    pub title: Arc<str>,
    pub message: Arc<str>,
    pub size: DialogSize,
    pub danger: bool,
    pub busy: bool,
    behavior: ModalBehavior,
    slots: ModalSlots,
    confirm_slots: Option<ConfirmSlots>,
    pub style: NodeStyle,
}

impl ConfirmDialog {
    pub fn new(title: impl Into<Arc<str>>, message: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            size: DialogSize::Compact,
            danger: false,
            busy: false,
            behavior: ModalBehavior::default(),
            slots: ModalSlots::default(),
            confirm_slots: None,
            style: modal_root_style(),
        }
    }

    pub fn close_policy(mut self, close_policy: DialogClosePolicy) -> Self {
        self.behavior.close_policy = close_policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: ModalInitialFocus) -> Self {
        self.behavior.initial_focus = initial_focus;
        self
    }

    pub fn behavior(&self) -> ModalBehavior {
        self.behavior
    }

    pub fn confirm_slots(&self) -> Option<&ConfirmSlots> {
        self.confirm_slots.as_ref()
    }

    pub(crate) fn set_confirm_slots_state(&mut self, slots: ConfirmSlots) {
        self.confirm_slots = Some(slots);
    }
}

impl ModalSurface for ConfirmDialog {
    fn slots(&self) -> &ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut ModalSlots {
        &mut self.slots
    }
}

impl ComponentView for ConfirmDialog {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "confirm-dialog".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::AlertDialog,
            &self.title,
            Some(&self.message),
            ModalSurfaceKind::Confirm(self.size),
            self.busy,
            self.danger,
            &self.slots,
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawer {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub side: DrawerSide,
    behavior: ModalBehavior,
    slots: ModalSlots,
    pub style: NodeStyle,
}

impl Drawer {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            description: None,
            side: DrawerSide::Right,
            behavior: ModalBehavior::default(),
            slots: ModalSlots::default(),
            style: modal_root_style(),
        }
    }

    pub fn side(mut self, side: DrawerSide) -> Self {
        self.side = side;
        self
    }
    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn close_policy(mut self, close_policy: DialogClosePolicy) -> Self {
        self.behavior.close_policy = close_policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: ModalInitialFocus) -> Self {
        self.behavior.initial_focus = initial_focus;
        self
    }

    pub fn behavior(&self) -> ModalBehavior {
        self.behavior
    }
}

impl ModalSurface for Drawer {
    fn slots(&self) -> &ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut ModalSlots {
        &mut self.slots
    }
}

impl ComponentView for Drawer {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "drawer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Dialog,
            &self.title,
            self.description.as_deref(),
            ModalSurfaceKind::Drawer(self.side),
            false,
            false,
            &self.slots,
        );
    }
}

pub(crate) fn project_modal(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    role: AccessibilityRole,
    title: &Arc<str>,
    description: Option<&str>,
    kind: ModalSurfaceKind,
    busy: bool,
    danger: bool,
    slots: &ModalSlots,
) {
    let visual = StandardVisual::ModalFrame {
        title: Arc::clone(title),
        description: description.map(Arc::from),
        kind,
        busy,
        danger,
        slots: slots.clone(),
    };
    if world.standard_visual(id) != Some(visual.clone()) {
        mutations.set_standard_visual(id, Some(visual));
    }
    project_common(
        id,
        world,
        mutations,
        style,
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
        AccessibilityState {
            role,
            label: Some(Arc::clone(title)),
            description: description.map(Arc::from),
            modal: true,
            busy,
            ..Default::default()
        },
    );
}

pub(crate) fn modal_root_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            position: PositionSpec::Fixed,
            width: Some(LengthSpec::Percent(100.0)),
            height: Some(LengthSpec::Percent(100.0)),
            z_index: Some(1_000),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, Button, DocumentId, MountState, StandardVisual};
    use std::sync::{Arc, Mutex};
    use unicode_segmentation::UnicodeSegmentation;

    #[derive(Default)]
    struct WrappingShaper;

    impl crate::TextShaper for WrappingShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &crate::TextContent,
            style: &crate::ComputedStyle,
            constraints: crate::TextShapeConstraints,
        ) -> crate::TextMetrics {
            let count = text.value.graphemes(true).count();
            if count == 0 {
                return crate::TextMetrics::default();
            }
            let advance = style.font_size;
            let natural_width = count as f32 * advance;
            let columns = constraints
                .max_width
                .filter(|_| constraints.wrap)
                .map(|width| (width / advance).floor().max(1.0) as usize)
                .unwrap_or(count);
            crate::TextMetrics {
                width: constraints
                    .max_width
                    .map_or(natural_width, |width| natural_width.min(width)),
                height: count.div_ceil(columns) as f32 * style.font_size * 1.2,
            }
        }
    }

    #[test]
    fn modal_slots_replace_by_parking_and_remounting_owned_children() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let confirm = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();

        let first = ModalSlots {
            actions: vec![cancel.stable_id()],
            ..Default::default()
        };
        assert!(cx.set_modal_slots(dialog, first).unwrap());
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Mounted)
        );

        let second = ModalSlots {
            actions: vec![confirm.stable_id()],
            ..Default::default()
        };
        assert!(cx.set_modal_slots(dialog, second).unwrap());
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Parked)
        );
        assert_eq!(
            cx.world().mount_state(confirm.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn modal_slot_failure_is_atomic_across_documents() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let foreign_document = DocumentId::new(2).unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Inspector"))
            .unwrap();
        let current = cx
            .create_detached_component(document, Button::new("Done"))
            .unwrap();
        let foreign = cx
            .create_detached_component(foreign_document, Button::new("Foreign"))
            .unwrap();
        let slots = ModalSlots {
            actions: vec![current.stable_id()],
            ..Default::default()
        };
        cx.set_modal_slots(drawer, slots.clone()).unwrap();

        let error = cx
            .set_modal_slots(
                drawer,
                ModalSlots {
                    actions: vec![foreign.stable_id()],
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::FrameworkError::InvalidModalSlots { .. }
        ));
        assert_eq!(
            cx.read(drawer, |drawer| drawer.slots.clone()).unwrap(),
            slots
        );
        assert_eq!(
            cx.world().mount_state(current.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn confirm_slot_validation_failure_preserves_typed_and_retained_authority() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let foreign_document = DocumentId::new(2).unwrap();
        let dialog = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let confirm = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();
        let foreign = cx
            .create_detached_component(foreign_document, Button::new("Foreign"))
            .unwrap();
        let slots = ConfirmSlots {
            body: None,
            close_action: None,
            cancel: cancel.stable_id(),
            secondary: None,
            confirm: confirm.stable_id(),
        };
        cx.set_confirm_slots(dialog, slots.clone()).unwrap();
        let children = cx.world().node(dialog.stable_id()).unwrap().children;

        assert!(matches!(
            cx.set_confirm_slots(
                dialog,
                ConfirmSlots {
                    cancel: foreign.stable_id(),
                    ..slots.clone()
                }
            ),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.read(dialog, |dialog| dialog.confirm_slots().cloned())
                .unwrap(),
            Some(slots)
        );
        assert_eq!(
            cx.world().node(dialog.stable_id()).unwrap().children,
            children
        );
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Mounted)
        );
        assert_eq!(
            cx.world().mount_state(confirm.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn modal_slots_reject_mounted_roots_and_activation_rejects_rogue_children() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Inspector"))
            .unwrap();
        let mounted_root = cx
            .create_component(document, Button::new("Owned elsewhere"))
            .unwrap();
        assert!(matches!(
            cx.set_modal_slots(
                drawer,
                ModalSlots {
                    body: Some(mounted_root.stable_id()),
                    ..Default::default()
                }
            ),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.world().mount_state(mounted_root.stable_id()),
            Some(MountState::Mounted)
        );
        let rogue = cx
            .create_detached_component(document, Button::new("Rogue"))
            .unwrap();
        cx.append_child(drawer, rogue).unwrap();
        cx.append_child(host, drawer).unwrap();
        assert!(matches!(
            cx.activate_overlay(host, drawer),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
    }

    #[test]
    fn bottom_drawer_projects_full_scrim_and_bottom_surface() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Console").side(DrawerSide::Bottom))
            .unwrap();
        let body = cx
            .create_detached_component(document, Button::new("Body"))
            .unwrap();
        let body_child = cx
            .create_detached_component(document, Button::new("Nested body action"))
            .unwrap();
        cx.append_child(body, body_child).unwrap();
        let action = cx
            .create_detached_component(document, Button::new("Done"))
            .unwrap();
        cx.set_modal_slots(
            drawer,
            ModalSlots {
                body: Some(body.stable_id()),
                actions: vec![action.stable_id()],
                ..Default::default()
            },
        )
        .unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let crate::ComponentGeometry::ModalFrame {
            scrim,
            surface,
            body: body_region,
            ..
        } = cx.world().component_geometry(drawer.stable_id()).unwrap()
        else {
            panic!("modal geometry")
        };
        assert_eq!((scrim.width, scrim.height), (800.0, 600.0));
        assert_eq!(surface.width, 800.0);
        assert_eq!(surface.y + surface.height, 600.0);
        let body_bounds = cx.world().layout_box(body.stable_id()).unwrap();
        let action_bounds = cx.world().layout_box(action.stable_id()).unwrap();
        assert!(surface.contains(body_bounds.x, body_bounds.y));
        assert!(surface.contains(action_bounds.x, action_bounds.y));
        assert!(body_bounds.y + body_bounds.height <= action_bounds.y);
        let root_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == drawer.stable_id())
            .unwrap();
        assert_eq!(root_a11y.bounds, surface);

        let escaped_body = crate::LayoutBox {
            x: body_region.x,
            y: body_region.y - 8.0,
            width: 24.0,
            height: 16.0,
        };
        let mut layout = MutationQueue::new();
        layout.write_layout(body.stable_id(), escaped_body);
        cx.commit_mutations(layout).unwrap();
        cx.rebuild_hit_test(document);
        assert!(
            !cx.world()
                .hit_test_candidates(document, escaped_body.x + 1.0, escaped_body.y + 1.0)
                .contains(&body.stable_id())
        );
        assert!(
            cx.world()
                .hit_test_candidates(document, escaped_body.x + 1.0, body_region.y + 1.0)
                .contains(&body.stable_id())
        );

        let translated_body = crate::LayoutBox {
            x: body_region.x,
            y: body_region.y + body_region.height - 8.0,
            width: 24.0,
            height: 8.0,
        };
        let mut translated_style = cx.world().node_style(body.stable_id()).unwrap().clone();
        Arc::make_mut(&mut translated_style.layout).transform =
            Some(nana_ui_core::PaintTransform {
                f: 20.0,
                ..nana_ui_core::PaintTransform::default()
            });
        let mut translated = MutationQueue::new();
        translated.write_layout(body.stable_id(), translated_body);
        translated.write_layout(body_child.stable_id(), translated_body);
        translated.set_style(body.stable_id(), translated_style);
        cx.commit_mutations(translated).unwrap();
        cx.rebuild_hit_test(document);
        let footer_point = (
            translated_body.x + 1.0,
            translated_body.y + translated_body.height / 2.0 + 20.0,
        );
        let footer_candidates =
            cx.world()
                .hit_test_candidates(document, footer_point.0, footer_point.1);
        assert!(!footer_candidates.contains(&body.stable_id()));
        assert!(!footer_candidates.contains(&body_child.stable_id()));
    }

    #[test]
    fn side_drawers_anchor_to_the_requested_viewport_edge() {
        for (index, side) in [DrawerSide::Left, DrawerSide::Right]
            .into_iter()
            .enumerate()
        {
            let mut cx = AppContext::new();
            let document = DocumentId::new(index as u64 + 1).unwrap();
            let drawer = cx
                .create_component(document, Drawer::new("Inspector").side(side))
                .unwrap();
            cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
                .unwrap();
            let crate::ComponentGeometry::ModalFrame { surface, .. } =
                cx.world().component_geometry(drawer.stable_id()).unwrap()
            else {
                panic!("drawer geometry")
            };
            assert_eq!(surface.width, 420.0);
            assert_eq!(surface.height, 600.0);
            assert_eq!(
                surface.x,
                if side == DrawerSide::Left { 0.0 } else { 380.0 }
            );
        }
    }

    #[test]
    fn dialog_wraps_against_final_surface_width_and_settles_at_twelve_vh() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(
                document,
                crate::Dialog::new("一个很长的设置标题👩‍💻需要根据最终宽度换行")
                    .description("说明文字同样按surface内容宽度换行🙂且不会覆盖正文区域"),
            )
            .unwrap();
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        cx.shape_text(&work.text, &mut shaper).unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(220.0, 600.0))
            .unwrap();
        assert!(cx.shape_text_for_layout(document, &mut shaper).unwrap());
        cx.layout_document(document, crate::LayoutViewport::new(220.0, 600.0))
            .unwrap();
        assert!(!cx.shape_text_for_layout(document, &mut shaper).unwrap());

        let crate::ComponentGeometry::ModalFrame {
            surface,
            title,
            description: Some(description),
            body,
            ..
        } = cx.world().component_geometry(dialog.stable_id()).unwrap()
        else {
            panic!("dialog geometry")
        };
        assert_eq!(surface.y, 72.0);
        assert!(surface.height <= 456.0);
        assert!(title.bounds.height > 14.0 * 1.2);
        assert!(description.bounds.height > 12.0 * 1.2);
        assert!(title.bounds.width <= surface.width - 32.0);
        assert!(description.bounds.width <= surface.width - 32.0);
        assert!(description.bounds.y + description.bounds.height <= body.y);
    }

    #[test]
    fn confirm_initial_focus_and_busy_close_are_one_modal_authority() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let confirm = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let body = cx
            .create_detached_component(document, Button::new("Body action"))
            .unwrap();
        let close = cx
            .create_detached_component(document, Button::new("Close"))
            .unwrap();
        let commit = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();
        let commit_child = cx
            .create_detached_component(document, Button::new("Delete details"))
            .unwrap();
        cx.append_child(commit, commit_child).unwrap();
        cx.set_confirm_slots(
            confirm,
            ConfirmSlots {
                body: Some(body.stable_id()),
                close_action: Some(close.stable_id()),
                cancel: cancel.stable_id(),
                secondary: None,
                confirm: commit.stable_id(),
            },
        )
        .unwrap();
        let intents = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&intents);
        cx.on(confirm, move |_dialog, intent: &ConfirmIntent, _| {
            captured.lock().unwrap().push(*intent)
        })
        .unwrap();
        cx.append_child(host, confirm).unwrap();
        assert!(cx.activate_overlay(host, confirm).unwrap());
        assert_eq!(cx.world().focused(document), Some(cancel.stable_id()));
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        assert!(cx.activate_button(commit).unwrap());
        assert_eq!(
            *intents.lock().unwrap(),
            vec![ConfirmIntent::Confirm { danger: false }]
        );
        assert!(cx.focus_node(document, commit_child.stable_id()).unwrap());
        cx.take_system_work();
        cx.set_confirm_state(confirm, true, true).unwrap();
        assert_eq!(cx.world().focused(document), Some(confirm.stable_id()));
        let busy_work = cx.take_system_work();
        let busy_delta = cx.world().project_accessibility_delta(&busy_work);
        assert!(
            busy_delta
                .updated
                .iter()
                .any(|node| node.id == commit.stable_id() && node.disabled)
        );
        cx.restore_system_work(busy_work);
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        assert!(!cx.activate_button(close).unwrap());
        assert!(!cx.activate_button(commit).unwrap());
        assert!(!cx.focus_node(document, cancel.stable_id()).unwrap());
        assert!(
            !cx.apply_accessibility_action(
                document,
                crate::AccessibilityActionRequest {
                    target: commit_child.stable_id(),
                    action: crate::AccessibilityAction::Focus,
                },
            )
            .unwrap()
        );
        assert!(cx.focus_node(document, body.stable_id()).unwrap());
        assert!(cx.focus_node(document, confirm.stable_id()).unwrap());
        let commit_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == commit.stable_id())
            .unwrap();
        assert!(commit_a11y.disabled);
        let extracted = cx
            .world()
            .extract_nodes(&[commit.stable_id()])
            .pop()
            .unwrap();
        assert!(matches!(
            extracted.standard_visual,
            Some(StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Danger,
                loading: true,
                ..
            })
        ));
        cx.rebuild_hit_test(document);
        let commit_bounds = cx.world().layout_box(commit.stable_id()).unwrap();
        let commit_center = (
            commit_bounds.x + commit_bounds.width / 2.0,
            commit_bounds.y + commit_bounds.height / 2.0,
        );
        assert!(
            !cx.world()
                .hit_test_candidates(document, commit_center.0, commit_center.1)
                .contains(&commit.stable_id())
        );
        assert!(
            cx.world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active
                .is_some()
        );
        assert!(
            cx.route_overlay_key(document, crate::OverlayKey::Escape)
                .unwrap()
        );
        assert!(
            cx.world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active
                .is_some()
        );
        cx.set_confirm_state(confirm, false, false).unwrap();
        let restored_work = cx.take_system_work();
        let restored_delta = cx.world().project_accessibility_delta(&restored_work);
        assert!(
            restored_delta
                .updated
                .iter()
                .any(|node| node.id == commit.stable_id() && !node.disabled)
        );
        cx.restore_system_work(restored_work);
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        cx.rebuild_hit_test(document);
        assert!(
            cx.world()
                .hit_test_candidates(document, commit_center.0, commit_center.1)
                .contains(&commit.stable_id())
        );
        let restored_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == commit.stable_id())
            .unwrap();
        assert!(!restored_a11y.disabled);
        let restored = cx
            .world()
            .extract_nodes(&[commit.stable_id()])
            .pop()
            .unwrap();
        assert!(matches!(
            restored.standard_visual,
            Some(StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Primary,
                loading: false,
                ..
            })
        ));
    }

    #[test]
    fn invalid_explicit_initial_focus_rejects_activation_atomically() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let foreign = cx
            .create_component(document, Button::new("Outside"))
            .unwrap();
        let dialog = cx
            .create_component(
                document,
                Drawer::new("Inspector")
                    .initial_focus(ModalInitialFocus::Target(foreign.stable_id())),
            )
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        let error = cx.activate_overlay(host, dialog).unwrap_err();
        assert!(matches!(
            error,
            crate::FrameworkError::InvalidComponentHierarchy { .. }
        ));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        assert_eq!(cx.world().focused(document), None);
    }
}
