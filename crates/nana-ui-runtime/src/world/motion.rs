//! Retained surface presence: input closes immediately, paint survives the exit.
use super::*;
use nana_ui_core::motion as tokens;

#[derive(Clone, Copy)]
pub(super) struct SurfaceMotion {
    pub open: bool,
    menu: bool,
    start: Duration,
    from: [f32; 2],
    value: [f32; 2],
    pub running: bool,
}

impl SurfaceMotion {
    fn sample(&self, now: Duration) -> [f32; 2] {
        let elapsed = now.saturating_sub(self.start).as_secs_f32();
        let opacity_duration = if self.menu {
            tokens::MENU_OPACITY
        } else {
            tokens::OVERLAY_FADE
        };
        let opacity = crate::Easing::EaseOutCubic
            .sample((elapsed / opacity_duration.as_secs_f32()).clamp(0.0, 1.0));
        let pop = crate::Easing::MENU_POP
            .sample((elapsed / tokens::MENU_POP.as_secs_f32()).clamp(0.0, 1.0));
        let to = f32::from(self.open);
        [
            self.from[0] + (to - self.from[0]) * opacity,
            self.from[1] + (to - self.from[1]) * pop,
        ]
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
        let from = self
            .surface_motion
            .get(&id)
            .map(|motion| motion.value)
            .unwrap_or([f32::from(!open); 2]);
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
                start: self.animation_now,
                from,
                value: from,
                running: true,
            },
        );
        self.start_component_animation(
            id,
            crate::component_animation_kinds::SURFACE,
            if menu {
                tokens::MENU_POP
            } else {
                tokens::OVERLAY_FADE
            },
            crate::Easing::Linear,
        );
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
        let Some(motion) = self.surface_motion.get_mut(&sample.target) else {
            return;
        };
        motion.value = motion.sample(self.animation_now);
        motion.running = !sample.finished;
        if sample.finished {
            self.closing_surfaces.remove(&sample.target);
        }
        self.mark_subtree(
            sample.target,
            DirtyMask::STYLE | DirtyMask::RENDER | DirtyMask::INPUT,
        );
    }

    pub(super) fn motion_layout(
        &self,
        id: StableNodeId,
        source: &Arc<LayoutStyle>,
    ) -> Arc<LayoutStyle> {
        let Some(motion) = self.surface_motion.get(&id) else {
            return source.clone();
        };
        if !motion.running || motion.value == [1.0; 2] {
            return source.clone();
        }
        let mut source = source.clone();
        let layout = Arc::make_mut(&mut source);
        layout.opacity = Some(layout.opacity.unwrap_or(1.0) * motion.value[0]);
        if motion.menu {
            let scale = 0.9 + 0.1 * motion.value[1];
            let transform = layout.transform.get_or_insert_default();
            transform.a *= scale;
            transform.b *= scale;
            transform.c *= scale;
            transform.d *= scale;
            layout
                .transform_origin
                .get_or_insert(nana_ui_core::TransformOrigin {
                    x: nana_ui_core::LengthSpec::Px(0.0),
                    y: nana_ui_core::LengthSpec::Px(0.0),
                });
        }
        source
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
        cx.world().extract_nodes(&[id])[0]
            .source_style
            .layout
            .opacity
            .unwrap_or(1.0)
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
        tick(&mut cx, 80);
        let opening = alpha(&cx, menu.stable_id());
        assert!((opening - 0.875).abs() < 1e-5);
        let pop = cx.world().extract_nodes(&[menu.stable_id()])[0]
            .source_style
            .layout
            .transform
            .unwrap()
            .a;
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
}
