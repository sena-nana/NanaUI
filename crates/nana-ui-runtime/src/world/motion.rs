//! Retained surface presence: input closes immediately, paint survives the exit.
use super::*;
use nana_ui_core::{PaintTransform, motion as tokens};

use crate::MotionTargetId;

#[derive(Clone, Copy)]
pub(super) struct SurfaceMotion {
    pub open: bool,
    menu: bool,
    pub running: bool,
}

#[derive(Clone, Copy, Default)]
struct LayoutClassOverlay {
    width: Option<f32>,
    height: Option<f32>,
    padding: Option<f32>,
    margin: Option<f32>,
}

fn scale_transform(scale: f32) -> PaintTransform {
    PaintTransform {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        e: 0.0,
        f: 0.0,
    }
}

impl UiWorld {
    pub(super) fn set_surface_open(&mut self, id: StableNodeId, open: bool, menu: bool) {
        if !self.is_mounted(id) {
            return;
        }
        if self
            .surface_motion
            .get(&id)
            .is_some_and(|motion| motion.open == open)
        {
            return;
        }
        let from_opacity = MotionTargetId::new(id.get())
            .and_then(|target| {
                self.presentation.applied_value(
                    target,
                    crate::AnimatableProperty::Opacity,
                    self.animation_now,
                )
            })
            .and_then(|value| match value {
                crate::MotionValue::Scalar(value) => Some(value),
                _ => None,
            })
            .unwrap_or(f32::from(!open));
        let from_pop = MotionTargetId::new(id.get())
            .and_then(|target| {
                self.presentation.applied_value(
                    target,
                    crate::AnimatableProperty::Transform,
                    self.animation_now,
                )
            })
            .and_then(|value| match value {
                crate::MotionValue::Transform(transform) => Some(transform.a),
                _ => None,
            })
            .unwrap_or(if open { 0.9 } else { 1.0 });
        if open {
            self.closing_surfaces.remove(&id);
        } else {
            self.closing_surfaces.insert(id);
            self.clear_surface_pointer_interactions(id);
        }
        self.surface_motion.insert(
            id,
            SurfaceMotion {
                open,
                menu,
                running: true,
            },
        );
        let to_opacity = f32::from(open);
        self.start_component_track(
            id,
            crate::component_animation_kinds::SURFACE,
            if menu {
                tokens::MENU_OPACITY
            } else {
                tokens::OVERLAY_FADE
            },
            crate::Easing::EaseOutCubic,
            crate::AnimatableProperty::Opacity,
            crate::MotionValue::Scalar(from_opacity),
            crate::MotionValue::Scalar(to_opacity),
            crate::MotionInterrupt::Retarget,
            None,
        );
        if menu {
            let to_scale = if open { 1.0 } else { 0.9 };
            self.start_component_track(
                id,
                crate::component_animation_kinds::SURFACE_POP,
                tokens::MENU_POP,
                crate::Easing::MENU_POP,
                crate::AnimatableProperty::Transform,
                crate::MotionValue::Transform(scale_transform(from_pop)),
                crate::MotionValue::Transform(scale_transform(to_scale)),
                crate::MotionInterrupt::Retarget,
                None,
            );
        }
        self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::RENDER | DirtyMask::INPUT);
    }

    pub(crate) fn project_menu_presence(
        &self,
        id: StableNodeId,
        requested: bool,
        mutations: &mut MutationQueue,
    ) -> bool {
        // OverlayHost owns presence while its child is activated or closing.
        if self
            .node(id)
            .and_then(|node| node.parent)
            .and_then(|parent| self.overlay_host(parent))
            .is_some_and(|host| host.active == Some(id))
            && self.surface_closed(id)
        {
            return requested || self.surface_closing(id);
        }
        let motion = self.surface_motion.get(&id);
        if motion.map(|motion| motion.open).unwrap_or(false) != requested {
            mutations.set_surface_open(id, requested, true);
        }
        requested || motion.is_some_and(|motion| motion.open || motion.running)
    }

    pub(crate) fn surface_closed(&self, id: StableNodeId) -> bool {
        self.surface_motion
            .get(&id)
            .is_some_and(|motion| !motion.open)
    }

    pub(crate) fn surface_closing(&self, id: StableNodeId) -> bool {
        self.surface_motion
            .get(&id)
            .is_some_and(|motion| !motion.open && motion.running)
    }

    pub(crate) fn motion_blocks_input(&self, id: StableNodeId) -> bool {
        if self.closing_surfaces.is_empty() {
            return false;
        }
        let mut current = Some(id);
        while let Some(id) = current {
            if self.surface_closing(id) {
                return true;
            }
            current = self.node(id).and_then(|node| node.parent);
        }
        false
    }

    pub(super) fn advance_surface_motion(&mut self, sample: &crate::AnimationSample) {
        let Some(motion) = self.surface_motion.get(&sample.target) else {
            return;
        };
        if !sample.finished {
            return;
        }
        let opacity_done =
            crate::component_animation_id(crate::component_animation_kinds::SURFACE, sample.target)
                .is_none_or(|id| !self.animation_is_active(id) || sample.id == id);
        let pop_done = !motion.menu
            || crate::component_animation_id(
                crate::component_animation_kinds::SURFACE_POP,
                sample.target,
            )
            .is_none_or(|id| !self.animation_is_active(id) || sample.id == id);
        if !(opacity_done && pop_done) {
            return;
        }
        let Some(motion) = self.surface_motion.get_mut(&sample.target) else {
            return;
        };
        motion.running = false;
        if !motion.open {
            self.closing_surfaces.remove(&sample.target);
        }
        self.mark_subtree(
            sample.target,
            DirtyMask::STYLE | DirtyMask::RENDER | DirtyMask::INPUT,
        );
        self.account_animation_dirty(DirtyMask::STYLE | DirtyMask::RENDER | DirtyMask::INPUT);
    }

    pub(super) fn motion_layout(
        &self,
        id: StableNodeId,
        source: &Arc<LayoutStyle>,
    ) -> Arc<LayoutStyle> {
        let Some(overlay) = self.layout_class_overlay(id) else {
            return Arc::clone(source);
        };
        let mut layout = (**source).clone();
        if let Some(width) = overlay.width {
            layout.width = Some(LengthSpec::Px(width));
        }
        if let Some(height) = overlay.height {
            layout.height = Some(LengthSpec::Px(height));
        }
        if let Some(padding) = overlay.padding {
            layout.padding = Some(LengthSpec::Px(padding));
        }
        if let Some(margin) = overlay.margin {
            layout.margin = Some(LengthSpec::Px(margin));
        }
        Arc::new(layout)
    }

    fn layout_class_overlay(&self, id: StableNodeId) -> Option<LayoutClassOverlay> {
        let now = self.animation_now;
        let mut overlay = LayoutClassOverlay::default();
        let mut any = false;
        for animation in self.animations.values() {
            if animation.spec.target != id
                || animation.spec.property.animation_class() != crate::AnimationClass::Layout
            {
                continue;
            }
            let Some(track) = animation.spec.to_motion_track() else {
                continue;
            };
            let sample = nana_ui_core::motion::evaluate_track(&track, now);
            let Some(crate::MotionValue::Scalar(px)) = sample
                .applied_value()
                .or(sample.finished.then_some(sample.value))
            else {
                continue;
            };
            if !px.is_finite() {
                continue;
            }
            match animation.spec.property {
                crate::AnimatableProperty::Width => overlay.width = Some(px),
                crate::AnimatableProperty::Height => overlay.height = Some(px),
                crate::AnimatableProperty::Padding => overlay.padding = Some(px),
                crate::AnimatableProperty::Margin => overlay.margin = Some(px),
                _ => continue,
            }
            any = true;
        }
        any.then_some(overlay)
    }

    pub(super) fn apply_layout_class_sample(&mut self, sample: &crate::AnimationSample) -> bool {
        if sample.property.animation_class() != crate::AnimationClass::Layout {
            return false;
        }
        let Some(crate::MotionValue::Scalar(px)) = sample
            .applied_value()
            .or(sample.finished.then_some(sample.value))
        else {
            return false;
        };
        if !px.is_finite() || !self.contains(sample.target) {
            return false;
        }
        let length = LengthSpec::Px(px);
        let layout = Arc::make_mut(&mut self.record_mut(sample.target).style.layout);
        let changed = match sample.property {
            crate::AnimatableProperty::Width => {
                let changed = layout.width != Some(length);
                layout.width = Some(length);
                changed
            }
            crate::AnimatableProperty::Height => {
                let changed = layout.height != Some(length);
                layout.height = Some(length);
                changed
            }
            crate::AnimatableProperty::Padding => {
                let changed = layout.padding != Some(length);
                layout.padding = Some(length);
                changed
            }
            crate::AnimatableProperty::Margin => {
                let changed = layout.margin != Some(length);
                layout.margin = Some(length);
                changed
            }
            _ => return false,
        };
        if !changed {
            return false;
        }
        self.mark_subtree(
            sample.target,
            DirtyMask::LAYOUT | DirtyMask::INPUT | DirtyMask::ACCESSIBILITY | DirtyMask::RENDER,
        );
        self.account_animation_dirty(DirtyMask::LAYOUT | DirtyMask::RENDER);
        true
    }

    pub(super) fn advance_loading_phase(&mut self, sample: &crate::AnimationSample) {
        let Some(mut visual) = self.standard_visual(sample.target) else {
            return;
        };
        match &mut visual {
            StandardVisual::Button { loading_phase, .. }
            | StandardVisual::Switch { loading_phase, .. }
            | StandardVisual::Card { loading_phase, .. } => {
                *loading_phase = sample.progress;
            }
            _ => return,
        }
        self.nodes.set_visual(sample.target, Some(visual));
        self.mark(sample.target, DirtyMask::RENDER);
        self.account_animation_dirty(DirtyMask::RENDER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionMenu, AnchoredActionMenu, AppContext, Button, Dialog, Entity, OverlayHost,
        OverlayKey, Switch,
    };

    fn tick(cx: &mut AppContext, ms: u64) {
        cx.advance_animations(Duration::from_millis(ms));
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
    }

    fn alpha(cx: &AppContext, id: StableNodeId) -> f32 {
        match cx.world().presentation_applied_value(
            id,
            crate::AnimatableProperty::Opacity,
            cx.world().animation_now(),
        ) {
            Some(crate::MotionValue::Scalar(value)) => value,
            _ => 1.0,
        }
    }

    fn logical_opacity(cx: &AppContext, id: StableNodeId) -> Option<f32> {
        cx.world()
            .node_style(id)
            .and_then(|style| style.layout.opacity)
    }

    #[test]
    fn menu_exit_keeps_paint_and_reopening_reverses_without_an_old_completion() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let menu = cx
            .create_component(doc, ActionMenu::new().open(true))
            .unwrap();
        tick(&mut cx, 0);
        assert_eq!(alpha(&cx, menu.stable_id()), 0.0);
        assert_eq!(logical_opacity(&cx, menu.stable_id()), None);
        tick(&mut cx, 80);
        let opening = alpha(&cx, menu.stable_id());
        assert!((opening - 0.875).abs() < 1e-5);
        assert_eq!(logical_opacity(&cx, menu.stable_id()), None);
        let pop = match cx.world().presentation_applied_value(
            menu.stable_id(),
            crate::AnimatableProperty::Transform,
            cx.world().animation_now(),
        ) {
            Some(crate::MotionValue::Transform(transform)) => transform.a,
            _ => panic!("menu pop must live on the transform overlay"),
        };
        assert!(pop > 0.9 && pop < 1.0);
        cx.update_component(menu, |menu, _| menu.popover.open = false)
            .unwrap();
        assert_eq!(alpha(&cx, menu.stable_id()), opening);
        assert!(matches!(
            cx.world().standard_visual(menu.stable_id()),
            Some(StandardVisual::MenuSurface { open: true, .. })
        ));
        tick(&mut cx, 120);
        let closing = alpha(&cx, menu.stable_id());
        assert!(closing < opening);
        cx.update_component(menu, |menu, _| menu.popover.open = true)
            .unwrap();
        assert_eq!(alpha(&cx, menu.stable_id()), closing);
        tick(&mut cx, 260);
        assert!(matches!(
            cx.world().standard_visual(menu.stable_id()),
            Some(StandardVisual::MenuSurface { open: true, .. })
        ));
        tick(&mut cx, 300);
        assert_eq!(alpha(&cx, menu.stable_id()), 1.0);
        assert_eq!(cx.next_animation_deadline(), None);
        cx.update_component(menu, |menu, _| menu.popover.open = false)
            .unwrap();
        tick(&mut cx, 480);
        assert_eq!(cx.world().standard_visual(menu.stable_id()), None);
        assert_eq!(cx.next_animation_deadline(), None);
    }

    #[test]
    fn anchored_menu_unmounts_its_surface_only_after_the_exit() {
        let mut cx = AppContext::new();
        let menu = cx
            .create_component(
                DocumentId::new(1).unwrap(),
                AnchoredActionMenu::new(20.0, 20.0),
            )
            .unwrap();
        tick(&mut cx, 180);
        cx.update_component(menu, |menu, _| menu.open = false)
            .unwrap();
        assert!(
            !cx.world()
                .node_style(menu.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        tick(&mut cx, 359);
        assert!(
            !cx.world()
                .node_style(menu.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        tick(&mut cx, 360);
        assert!(
            cx.world()
                .node_style(menu.stable_id())
                .unwrap()
                .layout
                .hidden
        );
    }

    #[test]
    fn dialog_escape_restores_focus_and_excludes_hits_before_delayed_unload() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let base = cx.create_component(doc, Button::new("Open")).unwrap();
        let host = cx.create_component(doc, OverlayHost::new()).unwrap();
        let dialog = cx.create_component(doc, Dialog::new("Settings")).unwrap();
        cx.append_child(host, dialog).unwrap();
        cx.focus_node(doc, base.stable_id()).unwrap();
        cx.activate_overlay(host, dialog).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dialog.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        cx.commit_mutations(layout).unwrap();
        tick(&mut cx, 140);
        cx.rebuild_hit_test(doc);
        assert!(
            cx.world()
                .hit_test_candidates(doc, 10.0, 10.0)
                .contains(&dialog.stable_id())
        );
        assert!(cx.route_overlay_key(doc, OverlayKey::Escape).unwrap());
        assert_eq!(cx.world().focused(doc), Some(base.stable_id()));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(dialog.stable_id())
        );
        assert!(
            !cx.world()
                .hit_test_candidates(doc, 10.0, 10.0)
                .contains(&dialog.stable_id())
        );
        assert!(cx.active_runtime_overlay(doc).is_none());
        tick(&mut cx, 200);
        assert_eq!(cx.world().focused(doc), Some(base.stable_id()));
        let closing = alpha(&cx, dialog.stable_id());
        assert!(closing > 0.0 && closing < 1.0);
        cx.activate_overlay(host, dialog).unwrap();
        assert_eq!(alpha(&cx, dialog.stable_id()), closing);
        assert_eq!(cx.world().focused(doc), Some(dialog.stable_id()));
        tick(&mut cx, 340);
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(dialog.stable_id())
        );
        cx.dismiss_overlay(host).unwrap();
        cx.focus_node(doc, base.stable_id()).unwrap();
        tick(&mut cx, 480);
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        assert_eq!(cx.world().focused(doc), Some(base.stable_id()));
        assert_eq!(cx.next_animation_deadline(), None);
    }

    fn thumb(cx: &AppContext, switch: Entity<Switch>) -> f32 {
        match cx.world().standard_visual(switch.stable_id()).unwrap() {
            StandardVisual::Switch { thumb_progress, .. } => thumb_progress,
            _ => unreachable!(),
        }
    }

    #[test]
    fn switch_reverses_from_current_position_and_releases_its_deadline() {
        let mut cx = AppContext::new();
        let switch = cx
            .create_component(DocumentId::new(1).unwrap(), Switch::new("Enabled", false))
            .unwrap();
        assert_eq!(cx.next_animation_deadline(), None);
        cx.update_component(switch, |switch, _| switch.checked = true)
            .unwrap();
        assert_eq!(thumb(&cx, switch), 0.0);
        tick(&mut cx, 70);
        assert!((thumb(&cx, switch) - 0.875).abs() < 1e-5);
        cx.update_component(switch, |switch, _| switch.checked = false)
            .unwrap();
        assert!((thumb(&cx, switch) - 0.875).abs() < 1e-5);
        tick(&mut cx, 140);
        assert!(thumb(&cx, switch) > 0.0 && thumb(&cx, switch) < 0.875);
        tick(&mut cx, 210);
        assert_eq!(thumb(&cx, switch), 0.0);
        assert_eq!(cx.next_animation_deadline(), None);
    }

    #[test]
    fn hover_interpolates_both_directions_without_layout_or_idle_frames() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let button = cx.create_component(doc, Button::new("Hover")).unwrap();
        tick(&mut cx, 0);
        let id = button.stable_id();
        let idle = cx.world().computed_style(id).unwrap().background;
        cx.set_pointer_hover_at(doc, 1, Some(id), Duration::ZERO)
            .unwrap();
        tick(&mut cx, 0);
        assert_eq!(cx.world().computed_style(id).unwrap().background, idle);
        assert!(
            cx.world().animation_is_active(
                crate::component_animation_id(crate::component_animation_kinds::HOVER, id)
                    .expect("hover animation id")
            ),
            "hover color is a Motion IR track, not a parallel timer"
        );
        tick(&mut cx, 60);
        let middle = cx.world().computed_style(id).unwrap().background;
        assert_ne!(middle, idle);
        cx.set_pointer_hover_at(doc, 1, None, Duration::from_millis(60))
            .unwrap();
        let work = cx.take_system_work();
        assert!(work.layout.is_empty());
        cx.resolve_styles(&work.style).unwrap();
        assert_eq!(cx.world().computed_style(id).unwrap().background, middle);
        tick(&mut cx, 180);
        assert_eq!(cx.world().computed_style(id).unwrap().background, idle);
        assert_eq!(cx.next_animation_deadline(), None);
        assert!(
            !cx.advance_animations(Duration::from_millis(1000))
                .has_updates()
        );
    }

    #[test]
    fn hover_foreground_repaints_inheriting_children_through_the_final_frame() {
        let mut world = UiWorld::new();
        let doc = DocumentId::new(1).unwrap();
        let root = StableNodeId::new(1).unwrap();
        let parent = StableNodeId::new(2).unwrap();
        let child = StableNodeId::new(3).unwrap();
        let mut create = MutationQueue::new();
        create.create(root, doc, NodeKind::Document);
        create.create(parent, doc, NodeKind::Element { tag: "row".into() });
        create.create(child, doc, NodeKind::Text);
        create.insert(root, parent, None);
        create.insert(parent, child, None);
        let mut style = NodeStyle {
            foreground: Some(SemanticColorRole::Text),
            ..NodeStyle::default()
        };
        style.interaction.hovered.foreground = Some(SemanticColorRole::Accent);
        create.set_style(parent, style);
        create.set_interaction(
            parent,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
        );
        world.commit(create).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let idle = world.computed_style(child).unwrap().color;
        world.set_pointer_hover(doc, 1, Some(parent)).unwrap();
        for ms in [0, 60, 120] {
            world.advance_animations(Duration::from_millis(ms));
            let work = world.take_system_work();
            assert!(work.style.contains(&child));
            assert!(work.render_extraction.contains(&child));
            assert!(work.layout.is_empty());
            world.resolve_styles(&work.style).unwrap();
            assert_eq!(
                world.computed_style(child).unwrap().color,
                world.computed_style(parent).unwrap().color
            );
            if ms == 0 {
                assert_eq!(world.computed_style(child).unwrap().color, idle);
            } else {
                assert_ne!(world.computed_style(child).unwrap().color, idle);
            }
        }
        assert_eq!(world.next_animation_deadline(), None);
    }

    #[test]
    fn l3_transition_spring_and_timeline_compile_to_motion_ir() {
        use crate::{
            AnimationPlayback, MotionCurve, MotionGraph, MotionTargetId, MotionTiming, MotionTrack,
            MotionTrackId, Spring, Timeline,
        };

        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let button = cx.create_component(doc, Button::new("Motion")).unwrap();
        let id = button.stable_id();
        cx.update_component(button, |_, ctx| {
            ctx.transition()
                .opacity(0.0)
                .duration(Duration::from_millis(100))
                .ease(crate::Easing::Linear);
        })
        .unwrap();
        tick(&mut cx, 0);
        assert_eq!(logical_opacity(&cx, id), None);
        assert!((alpha(&cx, id) - 1.0).abs() < 1e-5);
        tick(&mut cx, 50);
        assert!((alpha(&cx, id) - 0.5).abs() < 1e-4);
        tick(&mut cx, 100);
        assert!((alpha(&cx, id) - 0.0).abs() < 1e-5);

        cx.update_component(button, |_, ctx| {
            ctx.node().motion(Spring::to(1.0)).opacity();
        })
        .unwrap();
        tick(&mut cx, 100);
        let spring_start = alpha(&cx, id);
        tick(&mut cx, 250);
        let spring_later = alpha(&cx, id);
        assert!(spring_later > spring_start);

        let track = MotionTrack::transition(
            MotionTrackId::new(1).unwrap(),
            MotionTargetId::new(1).unwrap(),
            crate::AnimatableProperty::Opacity,
            crate::MotionValue::Scalar(1.0),
            crate::MotionValue::Scalar(0.25),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(80),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(crate::Easing::Linear),
            AnimationPlayback::default(),
        );
        cx.update_component(button, |_, ctx| {
            ctx.node()
                .timeline(Timeline::parallel([MotionGraph::track(track)]));
        })
        .unwrap();
        tick(&mut cx, 250);
        tick(&mut cx, 290);
        let sequenced = alpha(&cx, id);
        assert!(sequenced < 1.0 && sequenced > 0.25);
    }
}
