//! AppContext lifecycle operations.

use super::*;

impl AppContext {
    /// Atomically attach and order a ListItem's typed direct-child slots.
    /// Every existing child must be named exactly once; arbitrary nested or
    /// duplicate slot identities are rejected before retained state changes.
    pub fn set_list_item_slots(
        &mut self,
        item: Entity<ListItem>,
        slots: ListItemSlots,
    ) -> Result<bool, FrameworkError> {
        self.read(item, |_| ())?;
        let ordered = [slots.leading, slots.content, slots.trailing]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        if unique.len() != ordered.len() {
            return Err(FrameworkError::InvalidListItemSlots {
                item: item.id,
                slot: None,
            });
        }
        let item_node = self
            .world
            .node(item.id)
            .ok_or(FrameworkError::MissingView(item.id))?;
        if item_node
            .children
            .iter()
            .any(|child| !unique.contains(child))
        {
            return Err(FrameworkError::InvalidListItemSlots {
                item: item.id,
                slot: item_node
                    .children
                    .iter()
                    .find(|child| !unique.contains(child))
                    .copied(),
            });
        }
        for &slot in &ordered {
            let Some(node) = self.world.node(slot) else {
                return Err(FrameworkError::InvalidListItemSlots {
                    item: item.id,
                    slot: Some(slot),
                });
            };
            if node.document != item_node.document
                || node.parent.is_some_and(|parent| parent != item.id)
            {
                return Err(FrameworkError::InvalidListItemSlots {
                    item: item.id,
                    slot: Some(slot),
                });
            }
        }
        let changed =
            item_node.children != ordered || self.read(item, |item| item.slots != slots)?;
        if !changed {
            return Ok(false);
        }
        let item_id = item.id;
        self.update_component(item, |item, cx| {
            item.slots = slots;
            for slot in &ordered {
                cx.mutations().insert(item_id, *slot, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach an EmptyState's application-owned action.
    /// Intrinsic icon and message content remain fields of EmptyState.
    pub fn set_empty_state_action(
        &mut self,
        empty: Entity<EmptyState>,
        action: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(empty, |empty| empty.action)?;
        let owned_current = self.validate_feedback_action(empty.id, current, action)?;
        let ordered = action.into_iter().collect::<Vec<_>>();
        let changed = current != action
            || self
                .world
                .node(empty.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = empty.id;
        self.update_component(empty, |empty, cx| {
            empty.action = action;
            if let Some(current) = owned_current
                && Some(current) != action
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(action) = action {
                cx.mutations().insert(parent, action, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach a FormField's application-owned control.
    pub fn set_form_field_control(
        &mut self,
        field: Entity<FormField>,
        control: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(field, |field| field.control)?;
        let owned_current = self.validate_feedback_action(field.id, current, control)?;
        let ordered = control.into_iter().collect::<Vec<_>>();
        let changed = current != control
            || self
                .world
                .node(field.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = field.id;
        self.update_component(field, |field, cx| {
            field.control = control;
            if let Some(current) = owned_current
                && Some(current) != control
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(control) = control {
                cx.mutations().insert(parent, control, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically replaces a modal's application-owned direct children.
    /// Removed children remain alive and parked so their view state and handlers
    /// can be remounted by identity.
    pub fn set_modal_slots<C: ModalSurface>(
        &mut self,
        modal: Entity<C>,
        slots: ModalSlots,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(modal, |modal| modal.slots().clone())?;
        let current_order = current.ordered();
        let ordered = slots.ordered();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        if unique.len() != ordered.len() {
            return Err(FrameworkError::InvalidModalSlots {
                parent: modal.id,
                slot: None,
            });
        }
        let parent_node = self
            .world
            .node(modal.id)
            .ok_or(FrameworkError::MissingView(modal.id))?;
        // The view field is public API, but never trusted as ownership proof.
        // A builder-declared first mount is the only field/tree mismatch allowed.
        if parent_node.children != current_order
            && !(parent_node.children.is_empty() && current_order == ordered)
        {
            return Err(FrameworkError::InvalidModalSlots {
                parent: modal.id,
                slot: parent_node
                    .children
                    .first()
                    .copied()
                    .or(current_order.first().copied()),
            });
        }
        for slot in &ordered {
            let Some(node) = self.world.node(*slot) else {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: modal.id,
                    slot: Some(*slot),
                });
            };
            if *slot == modal.id
                || node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != modal.id)
                || (node.parent.is_none()
                    && self.world.mount_state(*slot) != Some(crate::MountState::Parked))
            {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: modal.id,
                    slot: Some(*slot),
                });
            }
        }
        if current == slots && parent_node.children == ordered {
            return Ok(false);
        }
        let removed = parent_node
            .children
            .iter()
            .copied()
            .filter(|child| !unique.contains(child))
            .collect::<Vec<_>>();
        let parent = modal.id;
        self.update_component(modal, |modal, cx| {
            *modal.slots_mut() = slots;
            for child in removed {
                cx.mutations().park_subtree(child);
            }
            for child in ordered {
                cx.mutations().insert(parent, child, None);
            }
        })?;
        Ok(true)
    }

    pub fn set_confirm_slots(
        &mut self,
        confirm: Entity<crate::ConfirmDialog>,
        slots: crate::ConfirmSlots,
    ) -> Result<bool, FrameworkError> {
        let modal_slots = slots.modal_slots();
        let current = self.read(confirm, |confirm| confirm.slots().clone())?;
        let ordered = modal_slots.ordered();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        let parent_node = self
            .world
            .node(confirm.id)
            .ok_or(FrameworkError::MissingView(confirm.id))?;
        if unique.len() != ordered.len()
            || (parent_node.children != current.ordered()
                && !(parent_node.children.is_empty() && current.ordered() == ordered))
        {
            return Err(FrameworkError::InvalidModalSlots {
                parent: confirm.id,
                slot: None,
            });
        }
        for slot in &ordered {
            let node = self
                .world
                .node(*slot)
                .ok_or(FrameworkError::InvalidModalSlots {
                    parent: confirm.id,
                    slot: Some(*slot),
                })?;
            if node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != confirm.id)
                || (node.parent.is_none()
                    && self.world.mount_state(*slot) != Some(crate::MountState::Parked))
            {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: confirm.id,
                    slot: Some(*slot),
                });
            }
        }
        let typed_changed =
            self.read(confirm, |confirm| confirm.confirm_slots() != Some(&slots))?;
        if current == modal_slots && parent_node.children == ordered && !typed_changed {
            return Ok(false);
        }
        let removed = parent_node
            .children
            .iter()
            .copied()
            .filter(|child| !unique.contains(child))
            .collect::<Vec<_>>();
        let parent = confirm.id;
        self.update_component(confirm, |confirm, cx| {
            *confirm.slots_mut() = modal_slots;
            confirm.set_confirm_slots_state(slots);
            for child in removed {
                cx.mutations().park_subtree(child);
            }
            for child in ordered {
                cx.mutations().insert(parent, child, None);
            }
        })?;
        Ok(true)
    }

    pub fn set_confirm_state(
        &mut self,
        confirm: Entity<crate::ConfirmDialog>,
        busy: bool,
        danger: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(confirm, |confirm| {
            confirm.busy == busy && confirm.danger == danger
        })? {
            return Ok(false);
        }
        let node = self
            .world
            .node(confirm.id)
            .ok_or(FrameworkError::MissingView(confirm.id))?;
        let document = node.document;
        let action_roots = self.read(confirm, |confirm| {
            confirm
                .slots()
                .close_action
                .into_iter()
                .chain(confirm.slots().actions.iter().copied())
                .collect::<HashSet<_>>()
        })?;
        let release_focus = busy
            && self.world.focused(document).is_some_and(|id| {
                action_roots
                    .iter()
                    .any(|root| self.overlay_descendant(*root, id))
            });
        let root = confirm.id;
        self.update_component(confirm, |confirm, cx| {
            confirm.busy = busy;
            confirm.danger = danger;
            if release_focus {
                cx.mutations().request_focus(document, Some(root));
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach a LabeledValue's optional action child.
    /// The child retains its own activation handler; the summary never becomes
    /// an implicit action target.
    pub fn set_labeled_value_action(
        &mut self,
        summary: Entity<LabeledValue>,
        action: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(summary, |summary| summary.action)?;
        let owned_current = self.validate_feedback_action(summary.id, current, action)?;
        let ordered = action.into_iter().collect::<Vec<_>>();
        let changed = current != action
            || self
                .world
                .node(summary.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = summary.id;
        self.update_component(summary, |summary, cx| {
            summary.action = action;
            if let Some(current) = owned_current
                && Some(current) != action
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(action) = action {
                cx.mutations().insert(parent, action, None);
            }
        })?;
        Ok(true)
    }

    pub(super) fn validate_feedback_action(
        &self,
        parent: StableNodeId,
        current: Option<StableNodeId>,
        action: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        let parent_node = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?;
        if parent_node.children.len() > 1 {
            return Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: parent_node.children.get(1).copied(),
            });
        }
        let owned_current = parent_node.children.first().copied();
        match (current, owned_current) {
            (None, None) => {}
            (Some(declared), Some(owned)) if declared == owned => {}
            // A builder may declare one detached action before its first
            // explicit mount. Only the same requested identity may complete
            // that declaration; it is never treated as an owned child to park.
            (Some(declared), None) if action == Some(declared) => {}
            (declared, owned) => {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: declared.or(owned),
                });
            }
        }
        if let Some(owned) = owned_current {
            let node = self
                .world
                .node(owned)
                .ok_or(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(owned),
                })?;
            if node.document != parent_node.document || node.parent != Some(parent) {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(owned),
                });
            }
        }
        if let Some(slot) = action {
            let Some(node) = self.world.node(slot) else {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(slot),
                });
            };
            if slot == parent
                || node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != parent)
            {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(slot),
                });
            }
        }
        Ok(owned_current)
    }

    pub(super) fn sync_component_lifecycle(
        &mut self,
        id: StableNodeId,
    ) -> Result<(), FrameworkError> {
        if self.views.get(&id).is_some_and(|view| view.is::<Tabs>()) {
            self.sync_tabs_options(Entity::from_stable_id(id))?;
        }
        self.sync_sidebar_section_body_port(id);
        let tooltip = self
            .views
            .get(&id)
            .and_then(|view| view.downcast_ref::<IconButton>())
            .and_then(|button| (!button.disabled).then(|| button.tooltip.clone()).flatten());
        match (tooltip, self.component_lifecycle.tooltips.get(&id).copied()) {
            (Some(configured), None) => {
                let document = self
                    .world
                    .node(id)
                    .ok_or(FrameworkError::MissingView(id))?
                    .document;
                let overlay = self.allocate_id();
                let tooltip =
                    Tooltip::with_config(Arc::clone(&configured.label), configured.config);
                let mut mutations = MutationQueue::new();
                mutations.create(overlay, document, tooltip.node_kind());
                tooltip.project(overlay, &self.world, &mut mutations);
                mutations.insert(id, overlay, None);
                mutations.set_overlay_host(id, crate::OverlayHostState::default());
                self.world.commit(mutations)?;
                self.views.insert(overlay, Box::new(tooltip));
                self.component_lifecycle.tooltips.insert(
                    id,
                    TooltipLifecycle {
                        overlay,
                        show_at: None,
                        open: false,
                    },
                );
            }
            (Some(configured), Some(existing)) => {
                self.update_component(
                    Entity::<Tooltip>::from_stable_id(existing.overlay),
                    |tooltip, _| {
                        if tooltip.label != configured.label || tooltip.config != configured.config
                        {
                            *tooltip = Tooltip::with_config(
                                Arc::clone(&configured.label),
                                configured.config,
                            );
                        }
                    },
                )?;
            }
            (None, Some(existing)) => {
                let mut mutations = MutationQueue::new();
                mutations.set_overlay_host(id, crate::OverlayHostState::default());
                mutations.despawn_subtree(existing.overlay);
                self.world.commit(mutations)?;
                self.views.remove(&existing.overlay);
                self.component_lifecycle.tooltips.remove(&id);
            }
            (None, None) => {}
        }

        let desired_loading = self.views.get(&id).and_then(|view| {
            if view
                .downcast_ref::<Button>()
                .is_some_and(|button| button.loading)
            {
                Some(LoadingComponent::Button)
            } else if view
                .downcast_ref::<Switch>()
                .is_some_and(|switch| switch.loading)
            {
                Some(LoadingComponent::Switch)
            } else if view
                .downcast_ref::<crate::Card>()
                .is_some_and(|card| card.loading)
            {
                Some(LoadingComponent::Card)
            } else {
                None
            }
        });
        match desired_loading {
            Some(kind) => {
                self.component_lifecycle.loading.insert(id, kind);
                if self.world.is_mounted(id)
                    && self.component_lifecycle.next_loading_frame.is_none()
                {
                    self.component_lifecycle.next_loading_frame =
                        Some(self.component_lifecycle.now);
                }
            }
            None => {
                self.component_lifecycle.loading.remove(&id);
                if !self
                    .component_lifecycle
                    .loading
                    .keys()
                    .any(|target| self.world.is_mounted(*target))
                {
                    self.component_lifecycle.next_loading_frame = None;
                }
            }
        }

        // 工作区折叠/展开过渡由运行时帧循环接管：模型带未回收过渡的
        // Workspace 登记进帧调度，结算后撤销，宿主无需自行驱动。
        if self
            .views
            .get(&id)
            .is_some_and(|view| view.is::<Workspace>())
        {
            let transitioning = self
                .views
                .get(&id)
                .and_then(|view| view.downcast_ref::<Workspace>())
                .is_some_and(|workspace| workspace.model.has_active_transitions());
            if transitioning {
                self.component_lifecycle
                    .workspace_transitions
                    .insert(id, ());
                if self.world.is_mounted(id)
                    && self.component_lifecycle.next_workspace_frame.is_none()
                {
                    self.component_lifecycle.next_workspace_frame =
                        Some(self.component_lifecycle.now);
                }
            } else {
                self.component_lifecycle.workspace_transitions.remove(&id);
                if self.component_lifecycle.workspace_transitions.is_empty() {
                    self.component_lifecycle.next_workspace_frame = None;
                }
            }
        }
        Ok(())
    }

    pub(super) fn sync_sidebar_section_body_port(&mut self, id: StableNodeId) {
        let Some(body) = self
            .views
            .get(&id)
            .and_then(|view| view.downcast_ref::<SidebarSection>())
            .and_then(|section| section.body)
        else {
            return;
        };
        let Some(style) = self.world.node_style(body).cloned() else {
            return;
        };
        let Some(list) = self
            .views
            .get_mut(&body)
            .and_then(|view| view.downcast_mut::<List>())
        else {
            return;
        };
        if list.style != style {
            list.style = style;
        }
    }

    pub(super) fn retained_subtree(&self, root: StableNodeId) -> Vec<StableNodeId> {
        let mut subtree = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.world.node(id) else {
                continue;
            };
            stack.extend(node.children.iter().rev().copied());
            subtree.push(id);
        }
        subtree
    }

    pub(super) fn suspend_component_lifecycle(&mut self, id: StableNodeId) {
        if let Some(button) = self
            .views
            .get_mut(&id)
            .and_then(|view| view.downcast_mut::<IconButton>())
        {
            button.tooltip_open = false;
        }
        if let Some(tooltip) = self.component_lifecycle.tooltips.get_mut(&id) {
            tooltip.show_at = None;
            tooltip.open = false;
        }
        if !self
            .component_lifecycle
            .loading
            .keys()
            .any(|target| self.world.is_mounted(*target))
        {
            self.component_lifecycle.next_loading_frame = None;
        }
    }

    pub(super) fn resume_component_lifecycle(
        &mut self,
        id: StableNodeId,
    ) -> Result<(), FrameworkError> {
        if self.world.is_mounted(id)
            && self.component_lifecycle.loading.contains_key(&id)
            && self.component_lifecycle.next_loading_frame.is_none()
        {
            self.component_lifecycle.next_loading_frame = Some(self.component_lifecycle.now);
        }
        // Parking cancels a skeleton's pulse timeline; remounting restarts it.
        // Projections only start timelines for pending or mounted nodes, and a
        // remount may never project again on its own.
        if self.world.is_mounted(id)
            && self
                .views
                .get(&id)
                .is_some_and(|view| view.is::<crate::Skeleton>())
            && let Some(spec) = crate::Skeleton::pulse_animation(id, self.component_lifecycle.now)
            && !self.world.animation_is_active(spec.id)
        {
            let mut mutations = MutationQueue::new();
            mutations.start_animation_with_playback(spec, crate::Skeleton::PULSE_PLAYBACK);
            self.world.commit(mutations)?;
        }
        Ok(())
    }

    pub(super) fn enter_tooltip(
        &mut self,
        target: StableNodeId,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        let Some(button) = self
            .views
            .get(&target)
            .and_then(|view| view.downcast_ref::<IconButton>())
        else {
            return Ok(());
        };
        let Some(configured) = button.tooltip.as_ref() else {
            return Ok(());
        };
        let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) else {
            return Ok(());
        };
        lifecycle.show_at = now.checked_add(Duration::from_millis(configured.config.delay_ms));
        if lifecycle.show_at == Some(now) {
            self.open_tooltip(target)?;
        }
        Ok(())
    }

    pub(super) fn leave_tooltip(&mut self, target: StableNodeId) -> Result<(), FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) else {
            return Ok(());
        };
        lifecycle.show_at = None;
        if !lifecycle.open {
            return Ok(());
        }
        lifecycle.open = false;
        self.update_component(
            Entity::<IconButton>::from_stable_id(target),
            |button, cx| {
                button.tooltip_open = false;
                cx.mutations()
                    .set_overlay_host(target, crate::OverlayHostState::default());
            },
        )?;
        Ok(())
    }

    pub(super) fn open_tooltip(&mut self, target: StableNodeId) -> Result<bool, FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get(&target).copied() else {
            return Ok(false);
        };
        if lifecycle.open {
            return Ok(false);
        }
        self.position_tooltip(target, lifecycle.overlay)?;
        if let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) {
            lifecycle.show_at = None;
            lifecycle.open = true;
        }
        self.update_component(
            Entity::<IconButton>::from_stable_id(target),
            |button, cx| {
                button.tooltip_open = true;
                cx.mutations().set_overlay_host(
                    target,
                    crate::OverlayHostState {
                        active: Some(lifecycle.overlay),
                        restore_focus: None,
                    },
                );
            },
        )?;
        Ok(true)
    }

    pub(super) fn position_open_tooltips(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let targets = self
            .component_lifecycle
            .tooltips
            .iter()
            .filter_map(|(&target, tooltip)| {
                (tooltip.open
                    && self
                        .world
                        .node(target)
                        .is_some_and(|node| node.document == document))
                .then_some((target, tooltip.overlay))
            })
            .collect::<Vec<_>>();
        for (target, overlay) in targets {
            self.position_tooltip(target, overlay)?;
        }
        Ok(())
    }

    pub(super) fn pointer_location_on(&self, target: StableNodeId) -> Option<(f32, f32)> {
        let document = self.world.node(target)?.document;
        self.component_lifecycle.pointer_positions.iter().find_map(
            |(&(owner, pointer_id), &position)| {
                (owner == document && self.world.pointer_hover(owner, pointer_id) == Some(target))
                    .then_some(position)
            },
        )
    }

    pub(super) fn reposition_follow_cursor_tooltip(
        &mut self,
        target: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get(&target).copied() else {
            return Ok(());
        };
        if !lifecycle.open {
            return Ok(());
        }
        let follows = self
            .read(
                Entity::<Tooltip>::from_stable_id(lifecycle.overlay),
                |tooltip| tooltip.config.placement == TooltipPlacement::FollowCursor,
            )
            .unwrap_or(false);
        if follows {
            self.position_tooltip(target, lifecycle.overlay)?;
        }
        Ok(())
    }

    pub(super) fn position_tooltip(
        &mut self,
        target: StableNodeId,
        overlay: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let anchor = self
            .world
            .layout_box(target)
            .ok_or(FrameworkError::MissingView(target))?;
        let document = self
            .world
            .node(target)
            .ok_or(FrameworkError::MissingView(target))?
            .document;
        let Some(viewport) = self.component_lifecycle.viewports.get(&document).copied() else {
            return Ok(());
        };
        let metrics = self.world.text_metrics(overlay).unwrap_or_default();
        let (config, mut style) = self
            .read(Entity::<Tooltip>::from_stable_id(overlay), |tooltip| {
                (tooltip.config, tooltip.style.clone())
            })?;
        let padding_x = TooltipConfig::PADDING_X;
        let padding_y = TooltipConfig::PADDING_Y;
        let desired_width = (metrics.width + padding_x * 2.0 + 2.0)
            .min(config.max_width)
            .max(0.0);
        let height = (metrics.height + padding_y * 2.0 + 2.0).max(0.0);
        let padding = config.viewport_padding.max(0.0);
        let left_available = (anchor.x - config.gap - padding).max(0.0);
        let right_available =
            (viewport.width - padding - (anchor.x + anchor.width + config.gap)).max(0.0);
        let cursor = self.pointer_location_on(target).unwrap_or((
            anchor.x + anchor.width / 2.0,
            anchor.y + anchor.height / 2.0,
        ));
        let (width, horizontal_side) = match config.placement {
            TooltipPlacement::Left => {
                let side = if left_available >= desired_width
                    || (left_available >= right_available && right_available < desired_width)
                {
                    TooltipPlacement::Left
                } else {
                    TooltipPlacement::Right
                };
                let available = if side == TooltipPlacement::Left {
                    left_available
                } else {
                    right_available
                };
                (desired_width.min(available), Some(side))
            }
            TooltipPlacement::Right => {
                let side = if right_available >= desired_width
                    || (right_available >= left_available && left_available < desired_width)
                {
                    TooltipPlacement::Right
                } else {
                    TooltipPlacement::Left
                };
                let available = if side == TooltipPlacement::Left {
                    left_available
                } else {
                    right_available
                };
                (desired_width.min(available), Some(side))
            }
            TooltipPlacement::Top | TooltipPlacement::Bottom | TooltipPlacement::FollowCursor => {
                (desired_width, None)
            }
        };
        let top = (
            anchor.x + (anchor.width - width) / 2.0,
            anchor.y - config.gap - height,
        );
        let right = (
            anchor.x + anchor.width + config.gap,
            anchor.y + (anchor.height - height) / 2.0,
        );
        let bottom = (
            anchor.x + (anchor.width - width) / 2.0,
            anchor.y + anchor.height + config.gap,
        );
        let left = (
            anchor.x - config.gap - width,
            anchor.y + (anchor.height - height) / 2.0,
        );
        let follow_above = (cursor.0, cursor.1 - height - config.gap);
        let follow_below = (cursor.0, cursor.1 + config.gap);
        let fits = |(x, y): (f32, f32)| {
            x >= padding
                && y >= padding
                && x + width <= viewport.width - padding
                && y + height <= viewport.height - padding
        };
        let preferred = match config.placement {
            TooltipPlacement::Top => top,
            TooltipPlacement::Right => right,
            TooltipPlacement::Bottom => bottom,
            TooltipPlacement::Left => left,
            TooltipPlacement::FollowCursor => follow_above,
        };
        let opposite = match config.placement {
            TooltipPlacement::Top => bottom,
            TooltipPlacement::Right => left,
            TooltipPlacement::Bottom => top,
            TooltipPlacement::Left => right,
            TooltipPlacement::FollowCursor => follow_below,
        };
        let (x, y) = if let Some(side) = horizontal_side {
            match side {
                TooltipPlacement::Left => left,
                TooltipPlacement::Right => right,
                TooltipPlacement::Top
                | TooltipPlacement::Bottom
                | TooltipPlacement::FollowCursor => unreachable!(),
            }
        } else if fits(preferred) || !fits(opposite) {
            preferred
        } else {
            opposite
        };
        let max_x = (viewport.width - padding - width).max(padding);
        let max_y = (viewport.height - padding - height).max(padding);
        let layout = Arc::make_mut(&mut style.layout);
        layout.offset_left = Some(LengthSpec::Px(x.clamp(padding, max_x)));
        layout.offset_top = Some(LengthSpec::Px(y.clamp(padding, max_y)));
        layout.width = Some(LengthSpec::Px(width));
        self.update_component(Entity::<Tooltip>::from_stable_id(overlay), |tooltip, _| {
            tooltip.style = style;
        })?;
        Ok(())
    }
}
