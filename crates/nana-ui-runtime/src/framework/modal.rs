//! AppContext modal operations.

use super::*;

impl AppContext {
    /// Activate one direct overlay child and move focus into its retained
    /// subtree in the same Runtime transaction.
    pub fn activate_overlay<O: View>(
        &mut self,
        host: Entity<OverlayHost>,
        overlay: Entity<O>,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        self.read(overlay, |_| ())?;
        let overlay_node = self
            .world
            .node(overlay.id)
            .ok_or(FrameworkError::MissingView(overlay.id))?;
        if overlay_node.parent != Some(host.id) {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: host.id,
                child: overlay.id,
            });
        }
        let previous =
            self.world
                .overlay_host(host.id)
                .ok_or(FrameworkError::InvalidComponentHierarchy {
                    parent: host.id,
                    child: overlay.id,
                })?;
        if previous.active == Some(overlay.id) && !self.world.surface_closed(overlay.id) {
            return Ok(false);
        }
        if self.runtime_overlay_kind(overlay.id).is_none() {
            return Err(FrameworkError::ViewType(overlay.id));
        }
        let activation_focus = self.modal_initial_focus(overlay_node.document, overlay.id)?;
        self.validate_modal_slots_for_activation(overlay.id)?;
        let previous_focus = self.world.focused(overlay_node.document);
        let restore_focus = previous.restore_focus.or(previous_focus);
        let next = crate::OverlayHostState {
            active: Some(overlay.id),
            restore_focus,
        };
        let activation_token = self
            .component_lifecycle
            .next_overlay_activation_token
            .checked_add(1)
            .ok_or(FrameworkError::OverlayActivationTokenExhausted(host.id))?;
        self.update_overlay_host(host, next, overlay_node.document, None, activation_focus)?;
        let Some(final_active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        let final_validation = self
            .world
            .is_overlay_reachable(final_active)
            .then_some(())
            .ok_or(FrameworkError::InvalidComponentValue(final_active))
            .and_then(|_| {
                self.world
                    .node(final_active)
                    .ok_or(FrameworkError::MissingView(final_active))
            })
            .and_then(|node| {
                if node.parent == Some(host.id) && node.document == overlay_node.document {
                    Ok(node)
                } else {
                    Err(FrameworkError::InvalidComponentHierarchy {
                        parent: host.id,
                        child: final_active,
                    })
                }
            })
            .and_then(|_| {
                self.runtime_overlay_kind(final_active)
                    .ok_or(FrameworkError::ViewType(final_active))
            })
            .and_then(|kind| {
                self.validate_modal_slots_for_activation(final_active)?;
                Ok(kind)
            });
        let final_kind = match final_validation {
            Ok(kind) => kind,
            Err(error) => {
                let rollback_active = previous.active.filter(|active| {
                    self.world.node(*active).is_some_and(|node| {
                        node.parent == Some(host.id)
                            && node.document == overlay_node.document
                            && self.world.is_mounted(*active)
                    })
                });
                let rollback_state = crate::OverlayHostState {
                    active: rollback_active,
                    restore_focus: rollback_active.and(previous.restore_focus),
                };
                let rollback_focus = previous_focus.filter(|focus| {
                    self.sequential_focus_candidate(overlay_node.document, *focus)
                        && rollback_active
                            .is_none_or(|root| self.overlay_reachable_within(root, *focus))
                });
                let mut rollback = MutationQueue::new();
                rollback.set_overlay_host(host.id, rollback_state);
                rollback.set_interaction(host.id, self.overlay_host_interaction(rollback_active));
                rollback.request_focus(overlay_node.document, rollback_focus);
                self.commit_mutations(rollback)?;
                return Err(error);
            }
        };
        if !final_kind.blocks_pointer() {
            return Ok(true);
        }
        self.component_lifecycle.next_overlay_activation_token = activation_token;
        self.component_lifecycle
            .overlay_activation_tokens
            .insert(host.id, activation_token);
        let mut motion = MutationQueue::new();
        motion.set_surface_open(final_active, true, final_kind == RuntimeOverlayKind::Menu);
        self.commit_mutations(motion)?;
        self.prepare_blocking_overlay_activation(overlay_node.document, final_active);
        Ok(true)
    }

    pub fn dismiss_overlay(&mut self, host: Entity<OverlayHost>) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let previous = self
            .world
            .overlay_host(host.id)
            .ok_or(FrameworkError::MissingView(host.id))?;
        if previous.active.is_none()
            || previous
                .active
                .is_some_and(|root| self.world.surface_closed(root))
        {
            return Ok(false);
        }
        let document = self
            .world
            .node(host.id)
            .ok_or(FrameworkError::MissingView(host.id))?
            .document;
        let root = previous.active.expect("active overlay checked above");
        if matches!(
            self.runtime_overlay_kind(root),
            Some(RuntimeOverlayKind::Dialog | RuntimeOverlayKind::Menu)
        ) {
            let mut mutations = MutationQueue::new();
            mutations.set_surface_open(
                root,
                false,
                self.runtime_overlay_kind(root) == Some(RuntimeOverlayKind::Menu),
            );
            self.commit_mutations(mutations)?;
            self.update_component(host, |_, cx| {
                cx.emit(crate::OverlayClosing { root });
            })?;
        } else {
            self.update_overlay_host(
                host,
                crate::OverlayHostState::default(),
                document,
                previous.restore_focus,
                None,
            )?;
        }
        Ok(true)
    }

    pub fn dismiss_dialog(
        &mut self,
        host: Entity<OverlayHost>,
        trigger: nana_ui_core::DialogCloseTrigger,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let Some(active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        if self.runtime_overlay_kind(active) != Some(RuntimeOverlayKind::Dialog) {
            return Err(FrameworkError::ViewType(active));
        }
        self.request_dialog_close(host, trigger)
    }

    pub fn request_dialog_close(
        &mut self,
        host: Entity<OverlayHost>,
        trigger: nana_ui_core::DialogCloseTrigger,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let Some(active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        if !self.dialog_allows(active, trigger) {
            return Ok(false);
        }
        self.dismiss_overlay(host)
    }

    pub(super) fn update_overlay_host(
        &mut self,
        host: Entity<OverlayHost>,
        next: crate::OverlayHostState,
        document: DocumentId,
        dismiss_restore: Option<StableNodeId>,
        activation_focus: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let focus = activation_focus
            .or_else(|| {
                next.active
                    .and_then(|active| self.first_overlay_focusable(document, active))
            })
            .or_else(|| {
                next.restore_focus
                    .or(dismiss_restore)
                    .filter(|id| self.overlay_focus_candidate(document, *id))
            });
        let host_id = host.id;
        let interaction = self.overlay_host_interaction(next.active);
        self.update_component(host, |_host, cx| {
            cx.mutations().set_overlay_host(host_id, next);
            cx.mutations().set_interaction(host_id, interaction);
            cx.mutations().request_focus(document, focus);
            cx.emit(OverlayChanged {
                active: next.active,
            });
        })
    }

    /// A host stretches across its whole region, so it may only take the
    /// pointer while a modal overlay is up; a passive one such as a toast has
    /// to leave the workspace underneath usable. Projection cannot decide this
    /// because it reads the world before the new activation is committed.
    pub(super) fn overlay_host_interaction(
        &self,
        active: Option<StableNodeId>,
    ) -> crate::InteractionState {
        let blocks_pointer = active
            .and_then(|id| self.world.accessibility(id))
            .and_then(|accessibility| overlay_kind_for_role(accessibility.role))
            .is_some_and(RuntimeOverlayKind::blocks_pointer);
        crate::InteractionState {
            pointer_events: blocks_pointer,
            focusable: false,
        }
    }
}

impl AppContext {
    pub(super) fn finish_surface_exit(&mut self, root: StableNodeId) -> Result<(), FrameworkError> {
        if !self.world.surface_closed(root) {
            return Ok(());
        }
        let host = self
            .world
            .node(root)
            .and_then(|node| node.parent)
            .filter(|host| {
                self.world
                    .overlay_host(*host)
                    .is_some_and(|state| state.active == Some(root))
            });
        if let Some(host) = host {
            self.update_component(Entity::<OverlayHost>::from_stable_id(host), |_, cx| {
                cx.mutations()
                    .set_overlay_host(host, crate::OverlayHostState::default());
                cx.emit(OverlayChanged { active: None });
            })?;
        } else if self
            .views
            .get(&root)
            .is_some_and(|view| view.is::<crate::ActionMenu>())
        {
            self.update_component(Entity::<crate::ActionMenu>::from_stable_id(root), |_, _| {})?;
        } else if self
            .views
            .get(&root)
            .is_some_and(|view| view.is::<crate::Popover>())
        {
            self.update_component(Entity::<crate::Popover>::from_stable_id(root), |_, _| {})?;
        } else if self
            .views
            .get(&root)
            .is_some_and(|view| view.is::<crate::AnchoredActionMenu>())
        {
            self.update_component(
                Entity::<crate::AnchoredActionMenu>::from_stable_id(root),
                |_, _| {},
            )?;
        } else if self
            .views
            .get(&root)
            .is_some_and(|view| view.is::<crate::ContextMenu>())
        {
            self.update_component(
                Entity::<crate::ContextMenu>::from_stable_id(root),
                |_, _| {},
            )?;
        }
        Ok(())
    }
}

impl AppContext {
    pub(super) fn prepare_surface_closing(&self, mutations: &mut MutationQueue) {
        let targets = mutations
            .as_slice()
            .iter()
            .filter_map(|mutation| match mutation {
                crate::UiMutation::SetSurfaceOpen { id, open, .. } => Some((*id, *open)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        for (root, open) in targets {
            if open {
                continue;
            }
            let Some(node) = self.world.node(root) else {
                continue;
            };
            let Some(host) = node.parent else {
                continue;
            };
            let Some(state) = self
                .world
                .overlay_host(host)
                .filter(|state| state.active == Some(root))
            else {
                continue;
            };
            mutations.set_interaction(
                host,
                crate::InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
            );
            if self
                .world
                .focused(node.document)
                .is_some_and(|focus| self.overlay_descendant(root, focus))
            {
                let restore = state
                    .restore_focus
                    .filter(|focus| self.overlay_focus_candidate(node.document, *focus));
                mutations.request_focus(node.document, restore);
            }
        }
    }
}
