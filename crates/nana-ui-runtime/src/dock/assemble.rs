use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::ControlSize;

use super::component::{
    Dock, DockChrome, DockDropOverlay, DockHandle, DockHandleMark, DockItemDrag, DockItemFrame,
    DockSplitFrame, DockSplitResize, DockTabsFrame, DockTitle, chrome_find, chrome_has_handle,
    chrome_has_strip, chrome_has_title, collect_drop_leaves, drop_zone_at, point_near_box,
    split_info_for_handle, strip_containing_item, strip_for_tabs, title_item_id,
};
use super::model::{
    DEFAULT_FLOATING_HEIGHT, DEFAULT_FLOATING_WIDTH, DOCK_DIVIDER_HIT_SIZE, DockAxis, DockDropZone,
    DockNode, DockWorkspaceEvent, activate_runtime_dock_tab, clamp_ratio, dock_nudge_split_ratio,
    dock_split_ratio_from_pointer, effective_active, host_content_ids, reorder_dock_tab,
    set_split_ratio_for_first_ids, visible_branch,
};
use crate::tabs::{TabOption, Tabs};
use crate::{
    AppContext, ComponentView, DocumentId, Entity, FrameworkError, MutationQueue, StableNodeId,
};

impl AppContext {
    /// Mount retained chrome for `dock`. Host content slots are reparented, not recreated.
    ///
    /// One [`Dock`] is one window surface. Floating surfaces are additional
    /// `Entity<Dock>` values assembled on the document that owns that window.
    /// The dock's assembler in a view (`TypeBehavior::slot_assembler`):
    /// the child a view keyed with an item's id is that item's content
    /// (`widget(Dock::new(layout)).children(files().key("files"))`), then
    /// [`Self::assemble_dock`]. A child whose key names no item is parked.
    pub fn assemble_dock_panels(&mut self, dock: Entity<Dock>) -> Result<bool, FrameworkError> {
        let mut root = self.read(dock, |dock| dock.root.clone())?;
        let mut bound = root.clear_missing_content(&|id| self.world().contains(id));
        for id in root.flatten() {
            if let Some(content) = self
                .assembled_child(dock.stable_id(), &id)
                .filter(|content| self.world().contains(*content))
            {
                bound |= root.bind_content(&id, content);
            }
        }
        if bound {
            self.update_component(dock, |dock, _| dock.root = root)?;
        }
        Ok(self.assemble_dock(dock)? || bound)
    }

    pub fn assemble_dock(&mut self, dock: Entity<Dock>) -> Result<bool, FrameworkError> {
        let document = document_of(self, dock.stable_id())?;
        self.update_component(dock, |dock, _| {
            dock.root.clamp_ratios();
        })?;
        let old = self.take_assembled_slot::<DockChrome>(dock.stable_id());
        let (visible, drop_target, locked, titles, hosts) = self.read(dock, |dock| {
            let visible = visible_branch(&dock.root, &dock.hidden)
                .map(|node| node.into_owned())
                .unwrap_or_else(|| dock.root.clone());
            (
                visible,
                dock.drop_target.clone(),
                dock.locked,
                dock.titles.clone(),
                host_content_ids(&dock.root),
            )
        })?;
        if let Some(old) = old.as_ref() {
            park_hosts(self, old, &hosts)?;
        }
        let chrome = assemble_branch(
            self,
            document,
            dock.stable_id(),
            &visible,
            old,
            &drop_target,
            locked,
            &titles,
            &hosts,
            true,
        )?;
        let children = branch_children(&chrome, &visible);
        self.set_assembled_slot(dock.stable_id(), chrome);
        self.reproject_component(dock)?;
        reconcile_children(self, dock, &children)
    }

    /// Float `id` off `dock` and rebuild that surface. The host opens the window.
    pub fn float_dock_item(
        &mut self,
        dock: Entity<Dock>,
        id: impl AsRef<str>,
    ) -> Result<Option<DockWorkspaceEvent>, FrameworkError> {
        let id = id.as_ref();
        let event = self.update_component(dock, |dock, _| {
            dock.float_item(id).map(DockWorkspaceEvent::OpenFloating)
        })?;
        if event.is_some() {
            self.assemble_dock(dock)?;
        }
        Ok(event)
    }

    pub fn is_dock(&self, id: StableNodeId) -> bool {
        self.read(Entity::<Dock>::from_stable_id(id), |_| ())
            .is_ok()
    }

    pub fn is_dock_handle(&self, id: StableNodeId) -> bool {
        self.dock_handle_id(id).is_some()
    }

    pub fn is_dock_title(&self, id: StableNodeId) -> bool {
        self.dock_title_id(id).is_some()
    }

    pub fn is_dock_tab_strip(&self, id: StableNodeId) -> bool {
        self.dock_tab_strip_id(id).is_some()
    }

    /// Handle under the pointer, including a few pixels of slop around the 8px bar.
    pub fn dock_handle_near(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        self.dock_handle_near_hit(document, x, y, self.pointer_target(document, x, y))
    }

    /// [`Self::dock_handle_near`] with the point already hit-tested.
    pub fn dock_handle_near_hit(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        hit: Option<StableNodeId>,
    ) -> Option<StableNodeId> {
        const SLOP: f32 = nana_ui_core::space::SM;
        if let Some(target) = hit {
            if let Some(handle) = self.unlocked_dock_handle(target) {
                return Some(handle);
            }
            if let Some(parent) = self.world().parent_id(target)
                && let Some(handle) = self.unlocked_dock_handle(parent)
            {
                return Some(handle);
            }
        }
        // Every pointer move off a handle reaches here, so ask the docks for
        // their handles instead of walking the document.
        self.unlocked_dock_chrome_find(document, |chrome| {
            chrome_find(chrome, &mut |node| match node {
                DockChrome::Split { handle, .. } => self
                    .world()
                    .component_layout_box(*handle)
                    .is_some_and(|bounds| point_near_box(bounds, x, y, SLOP))
                    .then_some(*handle),
                _ => None,
            })
        })
    }

    /// The first answer `find` gives for the chrome of an unlocked dock in
    /// `document`, docks in no particular order.
    fn unlocked_dock_chrome_find(
        &self,
        document: DocumentId,
        find: impl Fn(&DockChrome) -> Option<StableNodeId>,
    ) -> Option<StableNodeId> {
        self.world()
            .nodes_of_component(document, crate::component_descriptors::DOCK.type_id)
            .find_map(|dock| {
                self.read(Entity::<Dock>::from_stable_id(dock), |dock| !dock.locked)
                    .ok()
                    .filter(|unlocked| *unlocked)
                    .and_then(|_| self.assembled_slot::<DockChrome>(dock))
                    .and_then(&find)
            })
    }

    /// Tab strip under the pointer, including option children of assembled chrome.
    pub fn dock_tab_strip_near(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<StableNodeId> {
        self.dock_tab_strip_near_hit(document, x, y, self.pointer_target(document, x, y))
    }

    /// [`Self::dock_tab_strip_near`] with the point already hit-tested.
    pub fn dock_tab_strip_near_hit(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        hit: Option<StableNodeId>,
    ) -> Option<StableNodeId> {
        if let Some(target) = hit {
            if let Some(strip) = self.unlocked_dock_tab_strip(target) {
                return Some(strip);
            }
            let mut current = Some(target);
            while let Some(id) = current {
                if let Some(strip) = self.unlocked_dock_tab_strip(id) {
                    return Some(strip);
                }
                current = self.world().parent_id(id);
            }
        }
        self.unlocked_dock_chrome_find(document, |chrome| {
            chrome_find(chrome, &mut |node| match node {
                DockChrome::Tabs { strip, .. } => self
                    .world()
                    .component_layout_box(*strip)
                    .is_some_and(|bounds| bounds.contains(x, y))
                    .then_some(*strip),
                _ => None,
            })
        })
    }

    pub fn begin_dock_split_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(handle) = self.dock_handle_id(target) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(handle) else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.locked)? {
            return Ok(false);
        }
        let Some(chrome) = self.assembled_slot::<DockChrome>(dock.stable_id()) else {
            return Ok(false);
        };
        let Some((frame, axis, ratio, first_ids)) = self.read(dock, |dock| {
            let visible = dock.visible_root()?;
            split_info_for_handle(chrome, &visible, handle)
        })?
        else {
            return Ok(false);
        };
        let start = match axis {
            DockAxis::Horizontal => x,
            DockAxis::Vertical => y,
        };
        let started = self.update_component(dock, |dock, cx| {
            dock.item_drag = None;
            dock.split_resize = Some(DockSplitResize {
                handle,
                frame,
                axis,
                start: Some(start),
                start_ratio: ratio,
                first_ids,
            });
            cx.mutations().request_focus(document, Some(handle));
            cx.mutations().capture_pointer(pointer_id, handle);
            true
        })?;
        if started {
            self.begin_active_drag(document, pointer_id, dock.stable_id(), &DOCK_SPLIT_DRAG_OPS);
        }
        Ok(started)
    }

    pub fn update_dock_split_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(handle) = self.dock_handle_id(target) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(handle) else {
            return Ok(false);
        };
        let Some((frame, axis, start, start_ratio, first_ids)) = self.read(dock, |dock| {
            dock.split_resize.as_ref().map(|session| {
                (
                    session.frame,
                    session.axis,
                    session.start,
                    session.start_ratio,
                    session.first_ids.clone(),
                )
            })
        })?
        else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.locked)? {
            return Ok(false);
        }
        let Some(bounds) = self.world().component_layout_box(frame) else {
            return Ok(false);
        };
        let extent = match axis {
            DockAxis::Horizontal => bounds.width,
            DockAxis::Vertical => bounds.height,
        } - DOCK_DIVIDER_HIT_SIZE;
        let position = match axis {
            DockAxis::Horizontal => x,
            DockAxis::Vertical => y,
        };
        if !position.is_finite() {
            return Ok(false);
        }
        let start = start.unwrap_or(position);
        let ratio = dock_split_ratio_from_pointer(start_ratio, start, position, extent);
        self.update_component(dock, |dock, _| {
            set_split_ratio_for_first_ids(&mut dock.root, &first_ids, ratio)
        })
    }

    pub fn end_dock_split_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(handle) = self.dock_handle_id(target) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(handle) else {
            return Ok(false);
        };
        self.clear_active_drag(document, pointer_id, dock.stable_id());
        self.update_component(dock, |dock, cx| {
            let Some(session) = dock.split_resize.take() else {
                return false;
            };
            if cancel {
                set_split_ratio_for_first_ids(
                    &mut dock.root,
                    &session.first_ids,
                    session.start_ratio,
                );
            }
            cx.mutations().release_pointer(pointer_id, handle);
            true
        })
    }

    /// Nudge the focused dock split handle by one keyboard step. Locked docks refuse.
    pub fn adjust_focused_dock_split(
        &mut self,
        document: DocumentId,
        direction: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.world().focused(document) else {
            return Ok(false);
        };
        let Some(handle) = self.dock_handle_id(focused) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(handle) else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.locked)? {
            return Ok(false);
        }
        let Some(chrome) = self.assembled_slot::<DockChrome>(dock.stable_id()) else {
            return Ok(false);
        };
        let Some((_, _, ratio, first_ids)) = self.read(dock, |dock| {
            let visible = dock.visible_root()?;
            split_info_for_handle(chrome, &visible, handle)
        })?
        else {
            return Ok(false);
        };
        if !direction.is_finite() || direction == 0.0 {
            return Ok(false);
        }
        let next = dock_nudge_split_ratio(ratio, direction);
        let changed = self.update_component(dock, |dock, _| {
            set_split_ratio_for_first_ids(&mut dock.root, &first_ids, next)
        })?;
        if changed {
            self.assemble_dock(dock)?;
        }
        Ok(changed)
    }

    pub fn begin_dock_item_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        _x: f32,
        _y: f32,
    ) -> Result<bool, FrameworkError> {
        if self
            .world()
            .node(target)
            .is_none_or(|node| node.document != document)
        {
            return Ok(false);
        }
        let Some(dock) = self.dock_entity_of(target) else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.locked)? {
            return Ok(false);
        }
        let Some(id) = self.dock_item_for_target(dock, target)? else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.primary.as_deref() == Some(id.as_ref()))? {
            return Ok(false);
        }
        let started = self.update_component(dock, |dock, cx| {
            activate_runtime_dock_tab(&mut dock.root, id.as_ref());
            dock.split_resize = None;
            dock.drop_target = None;
            dock.item_drag = Some(DockItemDrag { id, reorder: None });
            cx.mutations().capture_pointer(pointer_id, target);
            true
        })?;
        if started {
            self.begin_active_drag(document, pointer_id, dock.stable_id(), &DOCK_ITEM_DRAG_OPS);
        }
        Ok(started)
    }

    pub fn update_dock_item_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(target) else {
            return Ok(false);
        };
        let Some(dragged) = self.read(dock, |dock| {
            dock.item_drag.as_ref().map(|drag| Arc::clone(&drag.id))
        })?
        else {
            return Ok(false);
        };
        if self.read(dock, |dock| dock.locked)? {
            return Ok(false);
        }
        let next_reorder = self.dock_reorder_at(dock, dragged.as_ref(), x, y)?;
        let next_zone = if next_reorder.is_some() {
            None
        } else {
            self.dock_drop_target_at(dock, dragged.as_ref(), x, y)?
        };
        let changed = self.update_component(dock, |dock, _| {
            let Some(drag) = dock.item_drag.as_mut() else {
                return false;
            };
            let changed = dock.drop_target != next_zone || drag.reorder != next_reorder;
            dock.drop_target = next_zone;
            drag.reorder = next_reorder;
            changed
        })?;
        if changed {
            self.assemble_dock(dock)?;
        }
        Ok(true)
    }

    pub fn end_dock_item_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(dock) = self.dock_entity_of(target) else {
            return Ok(false);
        };
        self.clear_active_drag(document, pointer_id, dock.stable_id());
        let Some((dragged, reorder)) = self.read(dock, |dock| {
            dock.item_drag
                .as_ref()
                .map(|drag| (Arc::clone(&drag.id), drag.reorder.clone()))
        })?
        else {
            return Ok(false);
        };
        let drop_target = self.read(dock, |dock| dock.drop_target.clone())?;
        let locked = self.read(dock, |dock| dock.locked)?;
        let outside = self
            .world()
            .component_layout_box(dock.stable_id())
            .is_none_or(|bounds| !bounds.contains(x, y));
        self.update_component(dock, |dock, cx| {
            dock.item_drag = None;
            dock.drop_target = None;
            cx.mutations().release_pointer(pointer_id, target);
            if cancel || locked {
                return;
            }
            if let Some((target_id, before)) = reorder {
                reorder_dock_tab(&mut dock.root, dragged.as_ref(), target_id.as_ref(), before);
            } else if let Some((target_id, zone)) = drop_target {
                dock.retarget(dragged.as_ref(), target_id.as_ref(), zone);
            } else if outside {
                let _ = dock.float_item_at(
                    dragged.as_ref(),
                    x,
                    y,
                    DEFAULT_FLOATING_WIDTH,
                    DEFAULT_FLOATING_HEIGHT,
                );
            }
        })?;
        self.assemble_dock(dock)?;
        Ok(true)
    }

    pub fn is_dock_item_source(&self, id: StableNodeId) -> bool {
        self.dock_title_id(id).is_some()
            || self
                .dock_entity_of(id)
                .and_then(|dock| self.dock_item_for_target(dock, id).ok().flatten())
                .is_some()
    }

    fn dock_entity_of(&self, id: StableNodeId) -> Option<Entity<Dock>> {
        let mut current = Some(id);
        while let Some(id) = current {
            if self.is_dock(id) {
                return Some(Entity::from_stable_id(id));
            }
            current = self.world().parent_id(id);
        }
        None
    }

    fn dock_is_locked(&self, id: StableNodeId) -> bool {
        self.dock_entity_of(id)
            .and_then(|dock| self.read(dock, |dock| dock.locked).ok())
            .unwrap_or(false)
    }

    fn unlocked_dock_handle(&self, id: StableNodeId) -> Option<StableNodeId> {
        let handle = self.dock_handle_id(id)?;
        (!self.dock_is_locked(handle)).then_some(handle)
    }

    fn unlocked_dock_tab_strip(&self, id: StableNodeId) -> Option<StableNodeId> {
        let strip = self.dock_tab_strip_id(id)?;
        (!self.dock_is_locked(strip)).then_some(strip)
    }

    fn dock_handle_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        if self
            .read(Entity::<DockHandle>::from_stable_id(id), |_| ())
            .is_ok()
        {
            let dock = self.dock_entity_of(id)?;
            return self
                .assembled_slot::<DockChrome>(dock.stable_id())
                .is_some_and(|chrome| chrome_has_handle(chrome, id))
                .then_some(id);
        }
        let parent = self.world().parent_id(id)?;
        if self
            .read(Entity::<DockHandle>::from_stable_id(parent), |_| ())
            .is_ok()
        {
            return self.dock_handle_id(parent);
        }
        None
    }

    fn dock_title_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        if self
            .read(Entity::<DockTitle>::from_stable_id(id), |_| ())
            .is_err()
        {
            return None;
        }
        let dock = self.dock_entity_of(id)?;
        self.assembled_slot::<DockChrome>(dock.stable_id())
            .is_some_and(|chrome| chrome_has_title(chrome, id))
            .then_some(id)
    }

    fn dock_tab_strip_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        if self
            .read(Entity::<Tabs>::from_stable_id(id), |_| ())
            .is_ok()
        {
            let dock = self.dock_entity_of(id)?;
            return self
                .assembled_slot::<DockChrome>(dock.stable_id())
                .is_some_and(|chrome| chrome_has_strip(chrome, id))
                .then_some(id);
        }
        None
    }

    fn dock_item_for_target(
        &self,
        dock: Entity<Dock>,
        target: StableNodeId,
    ) -> Result<Option<Arc<str>>, FrameworkError> {
        enum TargetMatch {
            Found(Arc<str>),
            Strip(StableNodeId),
        }
        let Some(chrome) = self.assembled_slot::<DockChrome>(dock.stable_id()) else {
            return Ok(None);
        };
        let found = self.read(dock, |dock| {
            let visible = visible_branch(&dock.root, &dock.hidden)?;
            if let Some(id) = title_item_id(chrome, visible.as_ref(), target) {
                return Some(TargetMatch::Found(id));
            }
            let mut current = Some(target);
            while let Some(id) = current {
                if chrome_has_strip(chrome, id) {
                    return Some(TargetMatch::Strip(id));
                }
                current = self.world().parent_id(id);
            }
            None
        })?;
        match found {
            Some(TargetMatch::Found(id)) => Ok(Some(id)),
            Some(TargetMatch::Strip(id)) => {
                if let Some(value) = self
                    .read(Entity::<Tabs>::from_stable_id(id), |tabs| {
                        tabs.option_nodes()
                            .iter()
                            .find(|(_, option)| *option == target)
                            .map(|(value, _)| Arc::clone(value))
                            .or_else(|| tabs.selected.clone())
                    })
                    .ok()
                    .flatten()
                {
                    return Ok(Some(value));
                }
                self.read(dock, |dock| {
                    let visible = visible_branch(&dock.root, &dock.hidden)?;
                    strip_for_tabs(chrome, visible.as_ref(), id)
                })
            }
            None => Ok(None),
        }
    }

    fn dock_reorder_at(
        &self,
        dock: Entity<Dock>,
        dragged: &str,
        x: f32,
        y: f32,
    ) -> Result<Option<(Arc<str>, bool)>, FrameworkError> {
        let Some(chrome) = self.assembled_slot::<DockChrome>(dock.stable_id()) else {
            return Ok(None);
        };
        let Some(strip) = self.read(dock, |dock| {
            let visible = visible_branch(&dock.root, &dock.hidden)?;
            strip_containing_item(chrome, visible.as_ref(), dragged)
        })?
        else {
            return Ok(None);
        };
        let options = self
            .read(Entity::<Tabs>::from_stable_id(strip), |tabs| {
                tabs.option_nodes().to_vec()
            })
            .unwrap_or_default();
        for (id, option) in options {
            let Some(bounds) = self.world().component_layout_box(option) else {
                continue;
            };
            if !bounds.contains(x, y) {
                continue;
            }
            let before = x < bounds.x + bounds.width * 0.5;
            return Ok(Some((id, before)));
        }
        Ok(None)
    }

    fn dock_drop_target_at(
        &self,
        dock: Entity<Dock>,
        dragged: &str,
        x: f32,
        y: f32,
    ) -> Result<Option<(Arc<str>, DockDropZone)>, FrameworkError> {
        let leaves = if let Some(chrome) = self.assembled_slot::<DockChrome>(dock.stable_id()) {
            self.read(dock, |dock| {
                let mut leaves = Vec::new();
                if let Some(visible) = visible_branch(&dock.root, &dock.hidden) {
                    collect_drop_leaves(chrome, visible.as_ref(), &mut leaves);
                }
                leaves
            })?
        } else {
            Vec::new()
        };
        for (id, frame) in leaves {
            if id.as_ref() == dragged {
                continue;
            }
            let Some(bounds) = self.world().component_layout_box(frame) else {
                continue;
            };
            if let Some(zone) = drop_zone_at(bounds, x, y) {
                return Ok(Some((id, zone)));
            }
        }
        Ok(None)
    }
}

fn document_of(context: &AppContext, id: StableNodeId) -> Result<DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

fn assemble_branch(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    node: &DockNode,
    old: Option<DockChrome>,
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    match node {
        DockNode::Item { id, content } => assemble_item(
            context,
            document,
            root_id,
            id,
            *content,
            old,
            drop_zone_for(drop_target, std::slice::from_ref(id)),
            locked,
            titles,
            hosts,
            at_root,
        ),
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } => assemble_tabs(
            context,
            document,
            root_id,
            tabs,
            active,
            contents,
            old,
            drop_zone_for(drop_target, tabs),
            locked,
            titles,
            hosts,
            at_root,
        ),
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => assemble_split(
            context,
            document,
            root_id,
            *axis,
            *ratio,
            first,
            second,
            old,
            drop_target,
            locked,
            titles,
            hosts,
            at_root,
        ),
    }
}

fn drop_zone_for(
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    ids: &[Arc<str>],
) -> Option<DockDropZone> {
    drop_target
        .as_ref()
        .filter(|(id, _)| ids.iter().any(|item| item == id))
        .map(|(_, zone)| *zone)
}

fn title_lookup(titles: &[(Arc<str>, Arc<str>)], id: &str) -> Arc<str> {
    titles
        .iter()
        .find(|(key, _)| key.as_ref() == id)
        .map(|(_, title)| Arc::clone(title))
        .unwrap_or_else(|| Arc::from(id))
}

fn assemble_item(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    id: &Arc<str>,
    content: Option<StableNodeId>,
    old: Option<DockChrome>,
    zone: Option<DockDropZone>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Item {
            frame,
            title,
            overlay,
        }) => Some((frame, title, overlay)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _)) = reused {
        if frame != root_id {
            context.update_component(
                Entity::<DockItemFrame>::from_stable_id(frame),
                |frame, _| {
                    frame.id = Arc::clone(id);
                },
            )?;
        }
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockItemFrame { id: Arc::clone(id) })?
            .stable_id()
    };
    let label = title_lookup(titles, id);
    let title = if let Some((_, title, _)) = reused {
        let entity = Entity::<DockTitle>::from_stable_id(title);
        context.update_component(entity, |title, _| {
            title.label = Arc::clone(&label);
            title.locked = locked;
        })?;
        title
    } else {
        context
            .create_detached_component(document, DockTitle { label, locked })?
            .stable_id()
    };
    let overlay = ensure_overlay(
        context,
        document,
        reused.and_then(|(_, _, overlay)| overlay),
        zone,
    )?;
    let mut ordered = vec![title];
    if let Some(content) = content {
        ordered.push(content);
    }
    if let Some(overlay) = overlay {
        ordered.push(overlay);
    }
    reconcile_ids(context, frame, &ordered)?;
    Ok(DockChrome::Item {
        frame,
        title,
        overlay,
    })
}

fn assemble_tabs(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    tabs: &[Arc<str>],
    active: &Arc<str>,
    contents: &[(Arc<str>, Option<StableNodeId>)],
    old: Option<DockChrome>,
    zone: Option<DockDropZone>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Tabs {
            frame,
            strip,
            overlay,
        }) => Some((frame, strip, overlay)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _)) = reused {
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockTabsFrame)?
            .stable_id()
    };
    if frame != root_id && reused.is_some() {
        context.reproject_component(Entity::<DockTabsFrame>::from_stable_id(frame))?;
    }
    let active = effective_active(tabs, active);
    let options = tabs
        .iter()
        .map(|id| TabOption::new(Arc::clone(id), title_lookup(titles, id)).draggable(!locked))
        .collect::<Vec<_>>();
    let strip = if let Some((_, strip, _)) = reused {
        context.update_component(Entity::<Tabs>::from_stable_id(strip), |strip, _| {
            strip.options = options.clone();
            strip.selected = Some(Arc::clone(&active));
            strip.focus = Some(Arc::clone(&active));
            strip.fill = true;
            strip.size = ControlSize::Small;
        })?;
        strip
    } else {
        context
            .create_detached_component(
                document,
                Tabs::new(Arc::clone(&active))
                    .options(options)
                    .fill(true)
                    .size(ControlSize::Small),
            )?
            .stable_id()
    };
    let overlay = ensure_overlay(
        context,
        document,
        reused.and_then(|(_, _, overlay)| overlay),
        zone,
    )?;
    let mut ordered = vec![strip];
    for (_, content) in contents {
        if let Some(content) = *content {
            ordered.push(content);
        }
    }
    if let Some(overlay) = overlay {
        ordered.push(overlay);
    }
    reconcile_ids(context, frame, &ordered)?;
    Ok(DockChrome::Tabs {
        frame,
        strip,
        overlay,
    })
}

fn assemble_split(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    axis: DockAxis,
    ratio: f32,
    first: &DockNode,
    second: &DockNode,
    old: Option<DockChrome>,
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Split {
            frame,
            handle,
            indicator,
            first,
            second,
        }) => Some((frame, handle, indicator, first, second)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _, _, _)) = reused {
        if frame != root_id {
            context.update_component(
                Entity::<DockSplitFrame>::from_stable_id(frame),
                |split, _| {
                    split.axis = axis;
                },
            )?;
        }
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockSplitFrame { axis })?
            .stable_id()
    };
    let handle = if let Some((_, handle, _, _, _)) = reused {
        context.update_component(Entity::<DockHandle>::from_stable_id(handle), |handle, _| {
            handle.axis = axis;
            handle.locked = locked;
        })?;
        handle
    } else {
        context
            .create_detached_component(document, DockHandle { axis, locked })?
            .stable_id()
    };
    let indicator = if let Some((_, _, indicator, _, _)) = reused {
        context.update_component(
            Entity::<DockHandleMark>::from_stable_id(indicator),
            |mark, _| {
                mark.axis = axis;
            },
        )?;
        indicator
    } else {
        context
            .create_detached_component(document, DockHandleMark { axis })?
            .stable_id()
    };
    reconcile_ids(context, handle, &[indicator])?;
    let (old_first, old_second) = match reused {
        Some((_, _, _, first, second)) => (Some(*first), Some(*second)),
        None => (None, None),
    };
    let first_chrome = assemble_branch(
        context,
        document,
        root_id,
        first,
        old_first,
        drop_target,
        locked,
        titles,
        hosts,
        false,
    )?;
    let second_chrome = assemble_branch(
        context,
        document,
        root_id,
        second,
        old_second,
        drop_target,
        locked,
        titles,
        hosts,
        false,
    )?;
    apply_grow_now(context, first_chrome.frame(), clamp_ratio(ratio))?;
    apply_grow_now(context, second_chrome.frame(), 1.0 - clamp_ratio(ratio))?;
    reconcile_ids(
        context,
        frame,
        &[first_chrome.frame(), handle, second_chrome.frame()],
    )?;
    Ok(DockChrome::Split {
        frame,
        handle,
        indicator,
        first: Box::new(first_chrome),
        second: Box::new(second_chrome),
    })
}

fn apply_grow_now(
    context: &mut AppContext,
    id: StableNodeId,
    grow: f32,
) -> Result<(), FrameworkError> {
    let Some(current) = context.world().node_style(id).cloned() else {
        return Ok(());
    };
    if current.layout.flex_grow == Some(grow) {
        return Ok(());
    }
    let mut style = current;
    Arc::make_mut(&mut style.layout).flex_grow = Some(grow);
    let mut mutations = MutationQueue::new();
    mutations.set_style(id, style);
    context.commit_mutations(mutations)?;
    Ok(())
}

fn ensure_overlay(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
    zone: Option<DockDropZone>,
) -> Result<Option<StableNodeId>, FrameworkError> {
    match (existing, zone) {
        (Some(id), Some(zone)) => {
            context.update_component(
                Entity::<DockDropOverlay>::from_stable_id(id),
                |overlay, _| {
                    overlay.zone = zone;
                    overlay.visible = true;
                },
            )?;
            Ok(Some(id))
        }
        (Some(id), None) => {
            context.update_component(
                Entity::<DockDropOverlay>::from_stable_id(id),
                |overlay, _| {
                    overlay.visible = false;
                },
            )?;
            Ok(Some(id))
        }
        (None, Some(zone)) => Ok(Some(
            context
                .create_detached_component(
                    document,
                    DockDropOverlay {
                        zone,
                        visible: true,
                    },
                )?
                .stable_id(),
        )),
        (None, None) => Ok(None),
    }
}

fn branch_children(chrome: &DockChrome, node: &DockNode) -> Vec<StableNodeId> {
    match (chrome, node) {
        (DockChrome::Item { title, overlay, .. }, DockNode::Item { content, .. }) => {
            let mut children = vec![*title];
            if let Some(content) = content {
                children.push(*content);
            }
            if let Some(overlay) = overlay {
                children.push(*overlay);
            }
            children
        }
        (DockChrome::Tabs { strip, overlay, .. }, DockNode::Tabs { contents, .. }) => {
            let mut children = vec![*strip];
            for (_, content) in contents {
                if let Some(content) = content {
                    children.push(*content);
                }
            }
            if let Some(overlay) = overlay {
                children.push(*overlay);
            }
            children
        }
        (
            DockChrome::Split {
                handle,
                first,
                second,
                ..
            },
            _,
        ) => {
            vec![first.frame(), *handle, second.frame()]
        }
        _ => Vec::new(),
    }
}

fn park_hosts(
    context: &mut AppContext,
    chrome: &DockChrome,
    hosts: &HashSet<StableNodeId>,
) -> Result<(), FrameworkError> {
    let mut mutations = MutationQueue::new();
    collect_host_parks(context, chrome.frame(), hosts, &mut mutations);
    if !mutations.is_empty() {
        context.commit_mutations(mutations)?;
    }
    Ok(())
}

fn collect_host_parks(
    context: &AppContext,
    id: StableNodeId,
    hosts: &HashSet<StableNodeId>,
    mutations: &mut MutationQueue,
) {
    let Some(node) = context.world().node(id) else {
        return;
    };
    for child in node.children {
        if hosts.contains(&child) {
            mutations.park_subtree(child);
        } else {
            collect_host_parks(context, child, hosts, mutations);
        }
    }
}

fn dispose_chrome(
    context: &mut AppContext,
    chrome: DockChrome,
    root_id: StableNodeId,
    hosts: &HashSet<StableNodeId>,
) -> Result<(), FrameworkError> {
    park_hosts(context, &chrome, hosts)?;
    match chrome {
        DockChrome::Item {
            frame,
            title,
            overlay,
        } => {
            remove_if_present::<DockTitle>(context, title)?;
            if let Some(overlay) = overlay {
                remove_if_present::<DockDropOverlay>(context, overlay)?;
            }
            if frame != root_id {
                remove_if_present::<DockItemFrame>(context, frame)?;
            }
        }
        DockChrome::Tabs {
            frame,
            strip,
            overlay,
        } => {
            remove_if_present::<Tabs>(context, strip)?;
            if let Some(overlay) = overlay {
                remove_if_present::<DockDropOverlay>(context, overlay)?;
            }
            if frame != root_id {
                remove_if_present::<DockTabsFrame>(context, frame)?;
            }
        }
        DockChrome::Split {
            frame,
            handle,
            indicator,
            first,
            second,
        } => {
            dispose_chrome(context, *first, root_id, hosts)?;
            dispose_chrome(context, *second, root_id, hosts)?;
            remove_if_present::<DockHandleMark>(context, indicator)?;
            remove_if_present::<DockHandle>(context, handle)?;
            if frame != root_id {
                remove_if_present::<DockSplitFrame>(context, frame)?;
            }
        }
    }
    Ok(())
}

fn remove_if_present<C: ComponentView>(
    context: &mut AppContext,
    id: StableNodeId,
) -> Result<(), FrameworkError> {
    if context.world().contains(id) {
        let _ = context.remove_view(Entity::<C>::from_stable_id(id))?;
    }
    Ok(())
}

fn reconcile_children<C: ComponentView>(
    context: &mut AppContext,
    parent: Entity<C>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    reconcile_ids(context, parent.stable_id(), ordered)
}

fn reconcile_ids(
    context: &mut AppContext,
    parent: StableNodeId,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    context.reconcile_children(parent, ordered)
}

static DOCK_SPLIT_DRAG_OPS: crate::framework::ActiveDragOps = crate::framework::ActiveDragOps {
    pre_gesture: false,
    cancel_on_touch_scroll: false,
    update: |cx, doc, _, ptr, x, y, _| cx.update_dock_split_resize(doc, ptr, x, y),
    end: |cx, doc, _, ptr, _, _, _, cancel| cx.end_dock_split_resize(doc, ptr, cancel),
};

static DOCK_ITEM_DRAG_OPS: crate::framework::ActiveDragOps = crate::framework::ActiveDragOps {
    pre_gesture: false,
    cancel_on_touch_scroll: false,
    update: |cx, doc, _, ptr, x, y, _| cx.update_dock_item_drag(doc, ptr, x, y),
    end: |cx, doc, _, ptr, x, y, _, cancel| cx.end_dock_item_drag(doc, ptr, x, y, cancel),
};

/// The router reaches dock handles and drags only once a dock exists.
pub(crate) static DOCK_CAPABILITIES: crate::framework::DockCapabilities =
    crate::framework::DockCapabilities {
        handle_near_hit: AppContext::dock_handle_near_hit,
        tab_strip_near_hit: AppContext::dock_tab_strip_near_hit,
        is_handle: AppContext::is_dock_handle,
        is_item_source: AppContext::is_dock_item_source,
        begin_split: AppContext::begin_dock_split_resize,
        begin_item: AppContext::begin_dock_item_drag,
        adjust_focused_split: AppContext::adjust_focused_dock_split,
    };

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nana_ui_core::{
        FlexDirection, LengthSpec, OverflowSpec, PositionSpec, SemanticColorRole, ViewStateStore,
    };

    use super::super::component::{DOCK_TITLE_BAR_HEIGHT, DockChrome};
    use super::super::model::{
        DEFAULT_FLOATING_HEIGHT, DEFAULT_FLOATING_WIDTH, DEFAULT_FLOATING_X, DEFAULT_FLOATING_Y,
        insert_dock_item, reorder_dock_tab,
    };
    use super::super::*;
    use crate::tabs::Tabs;
    use crate::{
        AppContext, DocumentId, Entity, InteractionState, LayoutBox, MAX_SPLIT_DEPTH,
        MutationQueue, NodeKind, StableNodeId, Text,
    };

    #[test]
    fn dock_workspace_layout_json_uses_historical_layout_fields() {
        let mut workspace = DockWorkspace::new(DockNode::split(
            DockAxis::Horizontal,
            0.25,
            DockNode::tabs(
                ["scenes", "sources"],
                "scenes",
                [("scenes", None), ("sources", None)],
            ),
            DockNode::item("editor", Some(StableNodeId::new(7).expect("content id"))),
        ));
        workspace.hidden.push(Arc::from("controls"));
        workspace.floating.push(DockFloatingSurface {
            id: Arc::from("1"),
            root: DockNode::item("mixer", None),
            x: 40.0,
            y: 50.0,
            width: 360.0,
            height: 280.0,
        });
        let json = workspace.layout_json().expect("workspace serializes");
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"kind\":\"split\""));
        assert!(json.contains("\"axis\":\"horizontal\""));
        assert!(json.contains("\"kind\":\"tabs\""));
        assert!(json.contains("\"surface\":1"));
        assert!(json.contains("\"bounds\""));
        assert!(json.contains("\"hidden\":[\"controls\"]"));
        assert!(!json.contains("content"));
        assert!(!json.contains("primary"));
        assert!(!json.contains("next_surface"));

        let restored = DockWorkspace::from_layout_json(&json).expect("workspace restores");
        assert!(restored.main.contains("editor"));
        assert!(restored.main.contains("scenes"));
        match &restored.main {
            DockNode::Split { first, .. } => match first.as_ref() {
                DockNode::Tabs { contents, .. } => {
                    assert!(contents.iter().all(|(_, content)| content.is_none()));
                }
                other => panic!("expected tabs, got {other:?}"),
            },
            other => panic!("expected split, got {other:?}"),
        }
        match &restored.main {
            DockNode::Split { second, .. } => match second.as_ref() {
                DockNode::Item { content, .. } => assert_eq!(*content, None),
                other => panic!("expected item, got {other:?}"),
            },
            other => panic!("expected split, got {other:?}"),
        }
        assert_eq!(
            restored
                .hidden
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["controls"]
        );
        assert_eq!(restored.floating.len(), 1);
        assert_eq!(restored.floating[0].id.as_ref(), "1");
        assert_eq!(restored.floating[0].x, 40.0);
        assert_eq!(restored.floating[0].width, 360.0);
        let again = restored.layout_json().expect("second serialize");
        assert_eq!(json, again);
    }

    #[test]
    fn dock_workspace_roundtrip_through_store() {
        let store = ViewStateStore::new(
            nana_ui_core::memory_store(),
            nana_ui_core::RestorationPath::root(),
        );
        let mut workspace = DockWorkspace::new(DockNode::split(
            DockAxis::Horizontal,
            0.25,
            DockNode::tabs(
                ["scenes", "sources"],
                "scenes",
                [("scenes", None), ("sources", None)],
            ),
            DockNode::item("editor", None),
        ));
        workspace.hidden.push(Arc::from("controls"));
        workspace
            .save_to_store(&store, "gallery")
            .expect("dock saves");
        let mut restored = DockWorkspace::new(DockNode::item("placeholder", None));
        assert!(
            restored
                .restore_from_store(&store, "gallery")
                .expect("dock restores")
        );
        assert_eq!(
            restored.layout_json().expect("restored serializes"),
            workspace.layout_json().expect("original serializes")
        );
        assert!(
            !restored
                .restore_from_store(&store, "missing")
                .expect("missing key")
        );
    }

    #[test]
    fn dock_workspace_layout_json_parses_historical_adapter_document() {
        let json = r#"{"version":1,"main":{"kind":"tabs","tabs":["a","b"],"active":"b"},"floating":[{"surface":2,"root":{"kind":"item","id":"c"},"bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0},"monitor":"m"}],"hidden":["d"],"locked":true}"#;
        let mut workspace = DockWorkspace::new(DockNode::item("keep", None)).primary("keep");
        workspace
            .restore_layout_json(json)
            .expect("historical json restores");
        assert_eq!(workspace.primary.as_deref(), Some("keep"));
        match &workspace.main {
            DockNode::Tabs { tabs, active, .. } => {
                assert_eq!(
                    tabs.iter().map(|id| id.as_ref()).collect::<Vec<_>>(),
                    ["a", "b"]
                );
                assert_eq!(active.as_ref(), "b");
            }
            other => panic!("expected tabs, got {other:?}"),
        }
        assert_eq!(workspace.floating[0].id.as_ref(), "2");
        assert_eq!(workspace.floating[0].y, 2.0);
        assert_eq!(workspace.hidden[0].as_ref(), "d");
        let encoded = workspace.layout_json().expect("product re-encode");
        assert!(encoded.contains("\"locked\":true"));
        assert!(encoded.contains("\"monitor\":\"m\""));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    #[test]
    fn dock_workspace_from_layout_json_clamps_split_ratio() {
        let json = r#"{"version":1,"main":{"kind":"split","axis":"vertical","ratio":4.0,"first":{"kind":"item","id":"top"},"second":{"kind":"item","id":"bottom"}}}"#;
        let workspace = DockWorkspace::from_layout_json(json).expect("loads");
        assert_eq!(workspace.main.split_ratio_at(&[]), Some(MAX_SPLIT_RATIO));
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn body(context: &mut AppContext, label: &str) -> StableNodeId {
        context
            .create_component(document(), Text::new(label))
            .unwrap()
            .stable_id()
    }

    fn descendants(context: &AppContext, root: StableNodeId) -> Vec<StableNodeId> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            ids.push(id);
            if let Some(node) = context.world().node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        ids
    }

    fn find_tag(context: &AppContext, root: StableNodeId, tag: &str) -> Vec<StableNodeId> {
        descendants(context, root)
            .into_iter()
            .filter(|id| {
                context.world().node(*id).is_some_and(|node| {
                    matches!(
                        &node.kind,
                        NodeKind::Element { tag: current } if current == tag
                    )
                })
            })
            .collect()
    }

    fn visible_overlay(context: &AppContext, root: StableNodeId) -> Option<StableNodeId> {
        find_tag(context, root, "dock-drop-overlay")
            .into_iter()
            .find(|id| {
                context
                    .world()
                    .node_style(*id)
                    .is_some_and(|style| !style.layout.hidden)
            })
    }

    /// `depth` splits down to the item `leaf`, each with an item beside it.
    fn split_chain(depth: usize) -> DockNode {
        let mut node = DockNode::item("leaf", None);
        for n in 0..depth {
            node = DockNode::split(
                DockAxis::Horizontal,
                0.5,
                DockNode::item(format!("beside-{n}"), None),
                node,
            );
        }
        node
    }

    #[test]
    fn a_drop_past_the_split_depth_is_refused() {
        let mut full = split_chain(MAX_SPLIT_DEPTH);
        assert_eq!(full.split_depth(), MAX_SPLIT_DEPTH);
        let item = DockNode::item("dropped", None);
        assert!(!insert_dock_item(
            &mut full,
            "leaf",
            item.clone(),
            DockDropZone::Right
        ));
        assert_eq!(full.split_depth(), MAX_SPLIT_DEPTH);
        // A tab adds no split, and a shallower leaf still splits.
        assert!(insert_dock_item(
            &mut full,
            "leaf",
            item.clone(),
            DockDropZone::Tab
        ));
        let mut room = split_chain(MAX_SPLIT_DEPTH - 1);
        assert!(insert_dock_item(
            &mut room,
            "leaf",
            item,
            DockDropZone::Right
        ));
        assert_eq!(room.split_depth(), MAX_SPLIT_DEPTH);
    }

    #[test]
    fn dock_split_ratio_clamps_and_preserves_child_order() {
        let low = DockNode::split(
            DockAxis::Horizontal,
            -2.0,
            DockNode::item("left", None),
            DockNode::item("right", None),
        );
        assert_eq!(low.split_ratio(), Some(MIN_SPLIT_RATIO));
        assert_eq!(
            low.flatten()
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["left", "right"]
        );

        let high = DockNode::split(
            DockAxis::Vertical,
            4.0,
            DockNode::item("top", None),
            DockNode::item("bottom", None),
        );
        assert_eq!(high.split_ratio(), Some(MAX_SPLIT_RATIO));

        let nan = DockNode::split(
            DockAxis::Horizontal,
            f32::NAN,
            DockNode::item("a", None),
            DockNode::item("b", None),
        );
        assert_eq!(nan.split_ratio(), Some(0.5));

        let mut context = AppContext::new();
        let first = body(&mut context, "first");
        let second = body(&mut context, "second");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    1.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let chrome = context
            .assembled_slot::<DockChrome>(dock.stable_id())
            .cloned()
            .unwrap();
        let (first_frame, handle, second_frame) = chrome.split_children().unwrap();
        assert_eq!(
            context.world().node(dock.stable_id()).unwrap().children,
            [first_frame, handle, second_frame]
        );
        assert_eq!(
            context
                .world()
                .node_style(first_frame)
                .unwrap()
                .layout
                .flex_grow,
            Some(MAX_SPLIT_RATIO)
        );
        assert_eq!(
            context
                .world()
                .node_style(second_frame)
                .unwrap()
                .layout
                .flex_grow,
            Some(1.0 - MAX_SPLIT_RATIO)
        );
        assert_eq!(
            context
                .world()
                .node_style(dock.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Row)
        );
    }

    #[test]
    fn dock_tabs_hide_inactive_body() {
        let mut context = AppContext::new();
        let code = body(&mut context, "code");
        let preview = body(&mut context, "preview");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::tabs(
                    ["code", "preview"],
                    "code",
                    [("code", Some(code)), ("preview", Some(preview))],
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert!(!context.world().node_style(code).unwrap().layout.hidden);
        assert!(context.world().node_style(preview).unwrap().layout.hidden);
        assert_eq!(
            context.world().node(code).unwrap().parent,
            Some(dock.stable_id())
        );
        assert_eq!(
            context.world().node(preview).unwrap().parent,
            Some(dock.stable_id())
        );

        context
            .update_component(dock, |dock, _| {
                if let DockNode::Tabs { active, .. } = &mut dock.root {
                    *active = Arc::from("preview");
                }
            })
            .unwrap();
        assert!(context.world().node_style(code).unwrap().layout.hidden);
        assert!(!context.world().node_style(preview).unwrap().layout.hidden);
        assert!(context.world().contains(code));
        assert!(context.world().contains(preview));
    }

    #[test]
    fn dock_item_projects_content_slot() {
        let mut context = AppContext::new();
        let content = body(&mut context, "inspector-body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::item("inspector", Some(content)))
                    .title("inspector", "Inspector"),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let title = find_tag(&context, dock.stable_id(), "dock-title")[0];
        assert_eq!(context.world().text(title), Some("Inspector"));
        assert_eq!(
            context.world().node(content).unwrap().parent,
            Some(dock.stable_id())
        );
        assert_eq!(
            context.world().node(dock.stable_id()).unwrap().children[0],
            title
        );
        assert!(
            context
                .world()
                .node(dock.stable_id())
                .unwrap()
                .children
                .contains(&content)
        );
        let style = context.world().node_style(dock.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.layout.height, Some(LengthSpec::Fill));
        assert_eq!(style.layout.overflow_y, OverflowSpec::Hidden);
        assert_eq!(
            context
                .world()
                .accessibility(dock.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("inspector")
        );
    }

    #[test]
    fn dock_drop_overlay_present_only_when_drop_target_set() {
        let mut context = AppContext::new();
        let content = body(&mut context, "pane");
        let dock = context
            .create_component(document(), Dock::new(DockNode::item("pane", Some(content))))
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert!(visible_overlay(&context, dock.stable_id()).is_none());

        context
            .update_component(dock, |dock, _| {
                dock.drop_target = Some((Arc::from("pane"), DockDropZone::Left));
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let overlay = visible_overlay(&context, dock.stable_id()).expect("overlay");
        let style = context.world().node_style(overlay).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::AccentSoft));
        assert_eq!(style.layout.width, Some(LengthSpec::Percent(50.0)));
        assert_eq!(style.layout.position, PositionSpec::Absolute);
        assert_eq!(
            context.world().node(overlay).unwrap().parent,
            Some(dock.stable_id())
        );
        assert!(
            context
                .world()
                .node(dock.stable_id())
                .unwrap()
                .children
                .contains(&content)
        );

        context
            .update_component(dock, |dock, _| {
                dock.drop_target = None;
            })
            .unwrap();
        assert!(visible_overlay(&context, dock.stable_id()).is_none());
    }

    #[test]
    fn dock_locked_omits_handle_pointer() {
        let mut context = AppContext::new();
        let first = body(&mut context, "a");
        let second = body(&mut context, "b");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Vertical,
                    0.4,
                    DockNode::item("a", Some(first)),
                    DockNode::item("b", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = find_tag(&context, dock.stable_id(), "dock-handle")[0];
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );

        context
            .update_component(dock, |dock, _| {
                dock.locked = true;
            })
            .unwrap();
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let mark = find_tag(&context, dock.stable_id(), "dock-handle-mark")[0];
        assert_eq!(
            context.world().node_style(mark).unwrap().background,
            Some(SemanticColorRole::BorderStrong)
        );
    }

    #[test]
    fn dock_panel_border_and_child_slot() {
        let mut context = AppContext::new();
        let child = body(&mut context, "fields");
        let panel = context
            .create_component(document(), DockPanel::new().padding(8.0).content(child))
            .unwrap();
        let style = context.world().node_style(panel.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
        assert_eq!(style.layout.border_width, Some(nana_ui_core::HAIRLINE));
        assert_eq!(style.layout.border_radius, Some(0.0));
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(style.layout.height, Some(LengthSpec::Shrink));
        assert_eq!(style.layout.padding_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(
            context.world().node(panel.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "dock-panel".into(),
            }
        );
        assert_eq!(
            context.world().node(child).unwrap().parent,
            Some(panel.stable_id())
        );
    }

    #[test]
    fn dock_idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let content = body(&mut context, "body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::item("pane", Some(content)))
                    .drop_target("pane", DockDropZone::Tab),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let panel_child = body(&mut context, "panel-body");
        let panel = context
            .create_component(document(), DockPanel::new().content(panel_child))
            .unwrap();
        let _ = context.take_system_work();
        context.reproject_component(dock).unwrap();
        context.reproject_component(panel).unwrap();
        assert!(context.take_system_work().is_empty());
    }

    #[test]
    fn float_item_removes_branch_from_main_and_returns_spec() {
        let mut dock = Dock::new(DockNode::split(
            DockAxis::Horizontal,
            0.4,
            DockNode::item("inspector", None),
            DockNode::item("console", None),
        ));

        let surface = dock.float_item("console").expect("float console");
        assert_eq!(surface.id.as_ref(), "console");
        assert_eq!(surface.root, DockNode::item("console", None));
        assert_eq!(
            (surface.x, surface.y, surface.width, surface.height),
            (
                DEFAULT_FLOATING_X,
                DEFAULT_FLOATING_Y,
                DEFAULT_FLOATING_WIDTH,
                DEFAULT_FLOATING_HEIGHT
            )
        );
        assert!(!dock.contains("console"));
        assert_eq!(dock.flatten(), vec![Arc::from("inspector")]);
        assert!(dock.float_item("missing").is_none());
        assert!(dock.float_item("inspector").is_none());
        assert!(dock.contains("inspector"));
    }

    #[test]
    fn float_item_from_tabs_collapses_remaining_item() {
        let mut dock = Dock::new(DockNode::tabs(
            ["code", "preview"],
            "preview",
            [("code", None), ("preview", None)],
        ));
        let surface = dock.float_item("preview").expect("float preview");
        assert_eq!(surface.root, DockNode::item("preview", None));
        assert_eq!(dock.root, DockNode::item("code", None));
    }

    #[test]
    fn reassembled_dock_omits_floated_id() {
        let mut context = AppContext::new();
        let inspector = body(&mut context, "inspector-body");
        let console = body(&mut context, "console-body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(inspector)),
                    DockNode::item("console", Some(console)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();

        let event = context
            .float_dock_item(dock, "console")
            .unwrap()
            .expect("floated");
        let DockWorkspaceEvent::OpenFloating(surface) = event else {
            panic!("open floating");
        };
        assert_eq!(surface.id.as_ref(), "console");
        assert_eq!(surface.root, DockNode::item("console", Some(console)));
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("inspector")]
        );

        context.assemble_dock(dock).unwrap();
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("inspector")]
        );
        assert!(!context.read(dock, |dock| dock.contains("console")).unwrap());
        assert_eq!(
            context.world().node(inspector).unwrap().parent,
            Some(dock.stable_id())
        );
        assert!(!descendants(&context, dock.stable_id()).contains(&console));
    }

    #[test]
    fn workspace_float_item_tracks_a_new_surface() {
        let mut workspace = DockWorkspace::new(DockNode::split(
            DockAxis::Vertical,
            0.5,
            DockNode::item("editor", None),
            DockNode::item("terminal", None),
        ));
        let event = workspace.float_item("terminal").expect("float terminal");
        let DockWorkspaceEvent::OpenFloating(surface) = &event else {
            panic!("open floating");
        };
        assert_eq!(surface.id.as_ref(), "1");
        assert_eq!(surface.window_key(), 1);
        assert!(!workspace.main.contains("terminal"));
        assert_eq!(workspace.floating.len(), 1);
        assert_eq!(workspace.surfaces().len(), 2);
        assert_eq!(workspace.surfaces()[0].id.as_ref(), MAIN_SURFACE_ID);
        assert_eq!(workspace.surfaces()[0].bounds, None);
        assert_eq!(
            workspace.surfaces()[1].bounds,
            Some((
                DEFAULT_FLOATING_X,
                DEFAULT_FLOATING_Y,
                DEFAULT_FLOATING_WIDTH,
                DEFAULT_FLOATING_HEIGHT
            ))
        );

        workspace.apply(DockWorkspaceEvent::MoveFloating {
            id: Arc::from("1"),
            x: 80.0,
            y: 90.0,
            width: 360.0,
            height: 280.0,
        });
        assert_eq!(workspace.floating[0].x, 80.0);
        workspace.apply(DockWorkspaceEvent::CloseFloating(Arc::from("1")));
        assert!(workspace.floating.is_empty());
        assert_eq!(dock_surface_window_key(MAIN_SURFACE_ID), 0);
        assert_ne!(dock_surface_window_key("inspector"), 0);
    }

    #[test]
    fn hide_omits_item_from_assemble_and_last_cannot_hide() {
        let mut context = AppContext::new();
        let inspector = body(&mut context, "inspector-body");
        let console = body(&mut context, "console-body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(inspector)),
                    DockNode::item("console", Some(console)),
                ))
                .primary("inspector"),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("inspector"), Arc::from("console")]
        );

        context
            .update_component(dock, |dock, _| {
                assert!(!dock.hide("inspector"));
                assert!(dock.hide("console"));
                assert!(!dock.hide("console"));
                assert!(!dock.hide("inspector"));
            })
            .unwrap();
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("inspector")]
        );
        assert!(context.read(dock, |dock| dock.contains("console")).unwrap());
        assert!(
            !context
                .read(dock, |dock| dock.is_visible("console"))
                .unwrap()
        );
        context.assemble_dock(dock).unwrap();
        assert_eq!(find_tag(&context, dock.stable_id(), "dock-title").len(), 1);
        assert!(!descendants(&context, dock.stable_id()).contains(&console));
        assert_eq!(
            context.world().node(inspector).unwrap().parent,
            Some(dock.stable_id())
        );

        context
            .update_component(dock, |dock, _| {
                assert!(dock.show("console"));
                assert!(!dock.show("console"));
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("inspector"), Arc::from("console")]
        );
        assert_eq!(find_tag(&context, dock.stable_id(), "dock-title").len(), 2);
        assert!(descendants(&context, dock.stable_id()).contains(&console));

        let mut only = Dock::new(DockNode::item("only", None));
        assert!(!only.hide("only"));
        assert!(only.is_visible("only"));
    }

    #[test]
    fn begin_update_end_split_drag_changes_ratio() {
        let mut context = AppContext::new();
        let first = body(&mut context, "first");
        let second = body(&mut context, "second");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let chrome = context
            .assembled_slot::<DockChrome>(dock.stable_id())
            .cloned()
            .unwrap();
        let (first_frame, handle, second_frame) = chrome.split_children().unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            first_frame,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 156.8,
                height: 200.0,
            },
        );
        layout.write_layout(
            handle,
            LayoutBox {
                x: 156.8,
                y: 0.0,
                width: DOCK_DIVIDER_HIT_SIZE,
                height: 200.0,
            },
        );
        layout.write_layout(
            second_frame,
            LayoutBox {
                x: 164.8,
                y: 0.0,
                width: 235.2,
                height: 200.0,
            },
        );
        context.commit_mutations(layout).unwrap();

        assert!(
            context
                .begin_dock_split_resize(document(), 1, handle, 160.0, 20.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_split_resize(document(), 1, 200.0, 20.0)
                .unwrap()
        );
        assert!(context.end_dock_split_resize(document(), 1, false).unwrap());
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - clamp_ratio(0.4 + 40.0 / (400.0 - DOCK_DIVIDER_HIT_SIZE))).abs() < 0.001);
        assert_eq!(
            ratio,
            dock_split_ratio_from_pointer(0.4, 160.0, 200.0, 400.0 - DOCK_DIVIDER_HIT_SIZE)
        );
        assert!(context.world().pointer_capture(document(), 1).is_none());
    }

    #[test]
    fn split_ratio_helpers_are_the_live_formula() {
        let available = 400.0 - DOCK_DIVIDER_HIT_SIZE;
        assert!(
            (dock_split_ratio_from_pointer(0.4, 160.0, 200.0, available)
                - clamp_ratio(0.4 + 40.0 / available))
            .abs()
                < f32::EPSILON
        );
        assert!((dock_nudge_split_ratio(0.4, 1.0) - clamp_ratio(0.45)).abs() < 1e-6);
        let (first, second) = dock_split_child_lengths(0.4, 400.0);
        assert!((first - available * 0.4).abs() < f32::EPSILON);
        assert!((second - (available - first)).abs() < f32::EPSILON);
        let mut workspace = DockWorkspace::new(DockNode::split(
            DockAxis::Horizontal,
            0.4,
            DockNode::item("inspector", None),
            DockNode::item("console", None),
        ));
        assert!(workspace.set_split_ratio(MAIN_SURFACE_ID, &[], 0.7));
        assert_eq!(workspace.main.split_ratio_at(&[]), Some(clamp_ratio(0.7)));
        assert!(!workspace.set_split_ratio(MAIN_SURFACE_ID, &[], 0.7));
    }

    #[test]
    fn drop_onto_left_and_tab_mutates_tree() {
        let mut context = AppContext::new();
        let first = body(&mut context, "first");
        let second = body(&mut context, "second");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let chrome = context
            .assembled_slot::<DockChrome>(dock.stable_id())
            .cloned()
            .unwrap();
        let (first_frame, _handle, second_frame) = chrome.split_children().unwrap();
        let titles = find_tag(&context, dock.stable_id(), "dock-title");
        let inspector_title = titles[0];
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            first_frame,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 196.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            second_frame,
            LayoutBox {
                x: 204.0,
                y: 0.0,
                width: 196.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            inspector_title,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 196.0,
                height: DOCK_TITLE_BAR_HEIGHT,
            },
        );
        context.commit_mutations(layout).unwrap();

        assert!(
            context
                .begin_dock_item_drag(document(), 1, inspector_title, 20.0, 10.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_item_drag(document(), 1, 302.0, 100.0)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.drop_target.clone()).unwrap(),
            Some((Arc::from("console"), DockDropZone::Tab))
        );
        assert!(
            context
                .end_dock_item_drag(document(), 1, 302.0, 100.0, false)
                .unwrap()
        );
        assert!(matches!(
            context.read(dock, |dock| dock.root.clone()).unwrap(),
            DockNode::Tabs { .. }
        ));
        assert_eq!(
            context.read(dock, |dock| dock.flatten()).unwrap(),
            vec![Arc::from("console"), Arc::from("inspector")]
        );

        context
            .update_component(dock, |dock, _| {
                dock.root = DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                );
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let chrome = context
            .assembled_slot::<DockChrome>(dock.stable_id())
            .cloned()
            .unwrap();
        let (first_frame, _handle, second_frame) = chrome.split_children().unwrap();
        let titles = find_tag(&context, dock.stable_id(), "dock-title");
        let console_title = titles[1];
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            first_frame,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 196.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            second_frame,
            LayoutBox {
                x: 204.0,
                y: 0.0,
                width: 196.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            console_title,
            LayoutBox {
                x: 204.0,
                y: 0.0,
                width: 196.0,
                height: DOCK_TITLE_BAR_HEIGHT,
            },
        );
        context.commit_mutations(layout).unwrap();

        assert!(
            context
                .begin_dock_item_drag(document(), 2, console_title, 220.0, 10.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_item_drag(document(), 2, 20.0, 100.0)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.drop_target.clone()).unwrap(),
            Some((Arc::from("inspector"), DockDropZone::Left))
        );
        assert!(
            context
                .end_dock_item_drag(document(), 2, 20.0, 100.0, false)
                .unwrap()
        );
        match context.read(dock, |dock| dock.root.clone()).unwrap() {
            DockNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(axis, DockAxis::Horizontal);
                assert_eq!(first.flatten()[0].as_ref(), "console");
                assert_eq!(second.flatten()[0].as_ref(), "inspector");
            }
            other => panic!("expected split, got {other:?}"),
        }
    }

    #[test]
    fn workspace_hide_show_and_primary() {
        let mut workspace = DockWorkspace::new(DockNode::split(
            DockAxis::Horizontal,
            0.4,
            DockNode::item("gallery.primary", None),
            DockNode::item("assets", None),
        ))
        .primary("gallery.primary");
        assert!(!workspace.hide("gallery.primary"));
        assert!(workspace.hide("assets"));
        assert!(!workspace.is_visible("assets"));
        assert_eq!(
            workspace.surfaces()[0].root.flatten(),
            vec![Arc::from("gallery.primary")]
        );
        assert!(workspace.show("assets"));
        assert!(workspace.is_visible("assets"));
        assert_eq!(workspace.surfaces()[0].root.flatten().len(), 2);
    }

    fn tab_option_nodes(
        context: &AppContext,
        dock: crate::Entity<Dock>,
    ) -> Vec<(Arc<str>, StableNodeId)> {
        let chrome = context
            .assembled_slot::<DockChrome>(dock.stable_id())
            .cloned()
            .unwrap();
        let strip = match chrome {
            DockChrome::Tabs { strip, .. } => strip,
            _ => panic!("expected tabs chrome"),
        };
        context
            .read(Entity::<Tabs>::from_stable_id(strip), |tabs| {
                tabs.option_nodes().to_vec()
            })
            .unwrap()
    }

    fn layout_two_tab_strip(
        context: &mut AppContext,
        dock: crate::Entity<Dock>,
        first: StableNodeId,
        second: StableNodeId,
    ) {
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            first,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: DOCK_TITLE_BAR_HEIGHT,
            },
        );
        layout.write_layout(
            second,
            LayoutBox {
                x: 80.0,
                y: 0.0,
                width: 80.0,
                height: DOCK_TITLE_BAR_HEIGHT,
            },
        );
        context.commit_mutations(layout).unwrap();
    }

    fn two_tab_dock(
        context: &mut AppContext,
        locked: bool,
    ) -> (crate::Entity<Dock>, StableNodeId, StableNodeId) {
        let first_body = body(context, "first-body");
        let second_body = body(context, "second-body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::tabs(
                    ["first", "second"],
                    "first",
                    [("first", Some(first_body)), ("second", Some(second_body))],
                ))
                .locked(locked),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let options = tab_option_nodes(context, dock);
        assert_eq!(options.len(), 2);
        (dock, options[0].1, options[1].1)
    }

    #[test]
    fn reorder_dock_tab_moves_before_or_after_sibling() {
        let mut root = DockNode::tabs(
            ["first", "second"],
            "first",
            [("first", None), ("second", None)],
        );
        assert!(reorder_dock_tab(&mut root, "first", "second", false));
        assert_eq!(
            root.flatten(),
            vec![Arc::from("second"), Arc::from("first")]
        );
        assert!(reorder_dock_tab(&mut root, "first", "second", true));
        assert_eq!(
            root.flatten(),
            vec![Arc::from("first"), Arc::from("second")]
        );
        assert!(!reorder_dock_tab(&mut root, "first", "second", true));
        assert!(!reorder_dock_tab(&mut root, "first", "first", false));
    }

    #[test]
    fn drag_first_tab_onto_second_reorders_by_before_after() {
        let mut context = AppContext::new();
        let (dock, first_tab, second_tab) = two_tab_dock(&mut context, false);
        layout_two_tab_strip(&mut context, dock, first_tab, second_tab);

        assert!(
            context
                .begin_dock_item_drag(document(), 1, first_tab, 20.0, 10.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_item_drag(document(), 1, 140.0, 10.0)
                .unwrap()
        );
        assert_eq!(
            context
                .read(dock, |dock| {
                    dock.item_drag
                        .as_ref()
                        .and_then(|drag| drag.reorder.clone())
                })
                .unwrap(),
            Some((Arc::from("second"), false))
        );
        assert!(
            context
                .read(dock, |dock| dock.drop_target.clone())
                .unwrap()
                .is_none()
        );
        assert!(
            context
                .end_dock_item_drag(document(), 1, 140.0, 10.0, false)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.root.flatten()).unwrap(),
            vec![Arc::from("second"), Arc::from("first")]
        );

        context
            .update_component(dock, |dock, _| {
                dock.root = DockNode::tabs(
                    ["first", "second"],
                    "first",
                    [("first", None), ("second", None)],
                );
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let options = tab_option_nodes(&context, dock);
        layout_two_tab_strip(&mut context, dock, options[0].1, options[1].1);
        assert!(
            context
                .begin_dock_item_drag(document(), 2, options[0].1, 20.0, 10.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_item_drag(document(), 2, 100.0, 10.0)
                .unwrap()
        );
        assert_eq!(
            context
                .read(dock, |dock| {
                    dock.item_drag
                        .as_ref()
                        .and_then(|drag| drag.reorder.clone())
                })
                .unwrap(),
            Some((Arc::from("second"), true))
        );
        assert!(
            context
                .end_dock_item_drag(document(), 2, 100.0, 10.0, false)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.root.flatten()).unwrap(),
            vec![Arc::from("first"), Arc::from("second")]
        );
    }

    #[test]
    fn dock_tab_reorder_is_noop_when_dropping_on_self() {
        let mut context = AppContext::new();
        let (dock, first_tab, second_tab) = two_tab_dock(&mut context, false);
        layout_two_tab_strip(&mut context, dock, first_tab, second_tab);

        assert!(
            context
                .begin_dock_item_drag(document(), 1, first_tab, 20.0, 10.0)
                .unwrap()
        );
        assert!(
            context
                .update_dock_item_drag(document(), 1, 40.0, 10.0)
                .unwrap()
        );
        assert_eq!(
            context
                .read(dock, |dock| {
                    dock.item_drag
                        .as_ref()
                        .and_then(|drag| drag.reorder.clone())
                })
                .unwrap()
                .map(|(id, _)| id),
            Some(Arc::from("first"))
        );
        assert!(
            context
                .end_dock_item_drag(document(), 1, 40.0, 10.0, false)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.root.flatten()).unwrap(),
            vec![Arc::from("first"), Arc::from("second")]
        );
        assert!(matches!(
            context.read(dock, |dock| dock.root.clone()).unwrap(),
            DockNode::Tabs { .. }
        ));
    }

    #[test]
    fn locked_dock_does_not_reorder() {
        let mut locked = Dock::new(DockNode::tabs(
            ["first", "second"],
            "first",
            [("first", None), ("second", None)],
        ))
        .locked(true);
        assert!(!locked.reorder_tab("first", "second", false));
        assert_eq!(
            locked.flatten(),
            vec![Arc::from("first"), Arc::from("second")]
        );

        let mut context = AppContext::new();
        let (dock, first_tab, second_tab) = two_tab_dock(&mut context, true);
        layout_two_tab_strip(&mut context, dock, first_tab, second_tab);
        assert!(
            !context
                .begin_dock_item_drag(document(), 1, first_tab, 20.0, 10.0)
                .unwrap()
        );
        assert_eq!(
            context.read(dock, |dock| dock.root.flatten()).unwrap(),
            vec![Arc::from("first"), Arc::from("second")]
        );
    }

    #[test]
    fn focused_dock_handle_adjust_changes_ratio_and_clamps() {
        let mut context = AppContext::new();
        let first = body(&mut context, "first");
        let second = body(&mut context, "second");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = find_tag(&context, dock.stable_id(), "dock-handle")[0];
        assert!(context.focus_node(document(), handle).unwrap());
        assert!(
            !context.focus_node(document(), handle).unwrap(),
            "focus_node reports whether focus changed, not whether the node is focused"
        );

        assert!(context.adjust_focused_dock_split(document(), 1.0).unwrap());
        let ratio = context
            .read(dock, |dock| dock.root.split_ratio())
            .unwrap()
            .unwrap();
        assert!((ratio - (0.4 + DOCK_SPLIT_KEYBOARD_STEP)).abs() < f32::EPSILON);

        context
            .update_component(dock, |dock, _| {
                if let DockNode::Split { ratio, .. } = &mut dock.root {
                    *ratio = 0.94;
                }
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        if context.world().focused(document()) != Some(handle) {
            assert!(context.focus_node(document(), handle).unwrap());
        }
        assert!(context.adjust_focused_dock_split(document(), 1.0).unwrap());
        assert_eq!(
            context
                .read(dock, |dock| dock.root.split_ratio())
                .unwrap()
                .unwrap(),
            MAX_SPLIT_RATIO
        );
        assert!(!context.adjust_focused_dock_split(document(), 1.0).unwrap());

        context
            .update_component(dock, |dock, _| {
                if let DockNode::Split { ratio, .. } = &mut dock.root {
                    *ratio = 0.06;
                }
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        if context.world().focused(document()) != Some(handle) {
            assert!(context.focus_node(document(), handle).unwrap());
        }
        assert!(context.adjust_focused_dock_split(document(), -1.0).unwrap());
        assert_eq!(
            context
                .read(dock, |dock| dock.root.split_ratio())
                .unwrap()
                .unwrap(),
            MIN_SPLIT_RATIO
        );
        assert!(!context.adjust_focused_dock_split(document(), -1.0).unwrap());

        context
            .update_component(dock, |dock, _| {
                dock.locked = true;
                if let DockNode::Split { ratio, .. } = &mut dock.root {
                    *ratio = 0.4;
                }
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert!(!context.adjust_focused_dock_split(document(), 1.0).unwrap());
        assert_eq!(
            context
                .read(dock, |dock| dock.root.split_ratio())
                .unwrap()
                .unwrap(),
            0.4
        );
    }

    #[test]
    fn eight_pane_keyboard_split_resize_repeats_after_focus() {
        let mut context = AppContext::new();
        let mut contents = [StableNodeId::new(1).unwrap(); 8];
        for (index, slot) in contents.iter_mut().enumerate() {
            *slot = body(&mut context, &format!("pane {index}"));
        }
        fn item(index: usize, content: StableNodeId) -> DockNode {
            DockNode::item(format!("pane-{index}"), Some(content))
        }
        fn split(axis: DockAxis, first: DockNode, second: DockNode) -> DockNode {
            DockNode::split(axis, 0.5, first, second)
        }
        let root = split(
            DockAxis::Horizontal,
            split(
                DockAxis::Vertical,
                split(
                    DockAxis::Horizontal,
                    item(0, contents[0]),
                    item(1, contents[1]),
                ),
                split(
                    DockAxis::Horizontal,
                    item(2, contents[2]),
                    item(3, contents[3]),
                ),
            ),
            split(
                DockAxis::Vertical,
                split(
                    DockAxis::Horizontal,
                    item(4, contents[4]),
                    item(5, contents[5]),
                ),
                split(
                    DockAxis::Horizontal,
                    item(6, contents[6]),
                    item(7, contents[7]),
                ),
            ),
        );
        let dock = context
            .create_component(document(), Dock::new(root))
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert_eq!(context.read(dock, |dock| dock.flatten().len()).unwrap(), 8);
        let _ = context.layout_document(document(), crate::LayoutViewport::new(1_280.0, 800.0));

        let handle = context
            .world()
            .document_order(document())
            .into_iter()
            .find(|&id| {
                context.is_dock_handle(id)
                    && context
                        .world()
                        .interaction(id)
                        .is_some_and(|interaction| interaction.focusable)
            })
            .expect("assembled dock must expose a focusable split handle");
        assert!(context.focus_node(document(), handle).unwrap());
        assert!(!context.focus_node(document(), handle).unwrap());

        for iteration in 0usize..8 {
            let direction = if iteration.is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            if context.world().focused(document()) != Some(handle) {
                assert!(context.focus_node(document(), handle).unwrap());
            }
            assert!(
                context
                    .adjust_focused_dock_split(document(), direction)
                    .unwrap()
            );
            let _ = context.layout_document(document(), crate::LayoutViewport::new(1_280.0, 800.0));
        }
    }
}
