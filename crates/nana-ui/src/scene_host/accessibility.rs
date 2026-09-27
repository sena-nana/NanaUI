//! Scene host accessibility coordination.

use super::*;

/// One unpresented batch can stay incremental. Multiple batches may contain
/// intermediate parent removal/reparenting, so publish the final tree instead
/// of discarding a batch or guessing how to combine their tombstones.
#[derive(Default)]
pub(super) enum PendingAccessibility {
    #[default]
    None,
    Delta(nana_ui_runtime::AccessibilityDelta),
    Snapshot,
}

impl PendingAccessibility {
    pub(super) fn stage(&mut self, delta: nana_ui_runtime::AccessibilityDelta) {
        if delta.updated.is_empty() && delta.removed.is_empty() {
            return;
        }
        *self = match self {
            Self::None => Self::Delta(delta),
            Self::Delta(_) | Self::Snapshot => Self::Snapshot,
        };
    }

    #[cfg(not(target_os = "android"))]
    fn take_for_publication(&mut self) -> (Option<AccessibilityUpdate>, bool) {
        match std::mem::take(self) {
            Self::None => (None, false),
            Self::Delta(delta) => (Some(AccessibilityUpdate::Delta(delta)), false),
            Self::Snapshot => (None, true),
        }
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod pending_tests {
    use super::*;
    use nana_ui_runtime::{
        AccessibilityDelta, AccessibilityState, DocumentId, LayoutBox, MutationQueue, NodeKind,
        UiWorld,
    };

    fn commit(world: &mut UiWorld, queue: MutationQueue) -> AccessibilityDelta {
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.project_accessibility_delta(&work)
    }

    #[test]
    fn first_publication_rebuilds_the_base_missing_from_a_partial_delta() {
        let document = DocumentId::new(1).unwrap();
        let id = |value| StableNodeId::new(value).unwrap();
        let mut world = UiWorld::new();
        let mut initial = MutationQueue::new();
        for value in [1, 2] {
            initial.create(id(value), document, NodeKind::Element { tag: "div".into() });
            initial.write_layout(
                id(value),
                LayoutBox {
                    width: 100.0,
                    height: 20.0,
                    ..Default::default()
                },
            );
        }
        initial.insert(id(1), id(2), None);
        commit(&mut world, initial); // Application already consumed initial work.
        let mut edit = MutationQueue::new();
        edit.set_accessibility(
            id(2),
            AccessibilityState {
                label: Some("Latest".into()),
                ..Default::default()
            },
        );
        let delta = AccessibilityUpdate::Delta(commit(&mut world, edit));
        let expected = world.project_accessibility(document);
        assert_eq!(expected.len(), 2);
        for from_program in [false, true] {
            let snapshots = std::cell::Cell::new(0);
            let update = next_accessibility_update(
                (!from_program).then(|| delta.clone()),
                from_program.then(|| delta.clone()),
                false,
                None,
                Some(world.generation()),
                || {
                    snapshots.set(snapshots.get() + 1);
                    world.project_accessibility(document)
                },
            );
            assert_eq!(
                update,
                Some(AccessibilityUpdate::Full {
                    generation: Some(world.generation()),
                    nodes: expected.clone()
                })
            );
            assert_eq!(snapshots.get(), 1);
        }
    }

    #[test]
    fn first_publication_accepts_an_explicit_complete_source_without_reprojection() {
        let full = AccessibilityUpdate::Full {
            generation: Some(3),
            nodes: Vec::new(),
        };
        assert_eq!(
            next_accessibility_update(None, Some(full.clone()), false, None, Some(3), || panic!(
                "explicit full source already contains the base tree"
            )),
            Some(full)
        );
    }

    #[test]
    fn retries_publish_all_committed_changes_from_one_final_snapshot() {
        let document = DocumentId::new(1).unwrap();
        let id = |value| StableNodeId::new(value).unwrap();
        let create = |queue: &mut MutationQueue, value| {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
            queue.write_layout(
                id(value),
                LayoutBox {
                    width: 100.0,
                    height: 20.0,
                    ..Default::default()
                },
            );
            if value != 1 {
                queue.insert(id(1), id(value), None);
            }
        };
        let mut world = UiWorld::new();
        let mut initial = MutationQueue::new();
        create(&mut initial, 1);
        create(&mut initial, 2);
        commit(&mut world, initial);
        let published_generation = world.generation();

        let mut pending = PendingAccessibility::default();
        let mut added = MutationQueue::new();
        create(&mut added, 3);
        pending.stage(commit(&mut world, added));
        // Surface acquisition/encoding did not complete, so nothing is drained.
        let mut edited = MutationQueue::new();
        edited.set_accessibility(
            id(2),
            AccessibilityState {
                label: Some("Latest".into()),
                ..Default::default()
            },
        );
        pending.stage(commit(&mut world, edited));
        pending.stage(AccessibilityDelta {
            generation: world.generation(),
            updated: vec![],
            removed: vec![],
        });

        let (queued, resnapshot) = pending.take_for_publication();
        let snapshots = std::cell::Cell::new(0);
        let Some(AccessibilityUpdate::Full { generation, nodes }) = next_accessibility_update(
            queued,
            None,
            resnapshot,
            Some(published_generation),
            Some(world.generation()),
            || {
                snapshots.set(snapshots.get() + 1);
                world.project_accessibility(document)
            },
        ) else {
            panic!("both unpresented transactions must reach the native tree")
        };
        assert_eq!(snapshots.get(), 1);
        assert_eq!(generation, Some(world.generation()));
        assert!(nodes.iter().any(|node| node.id == id(3)));
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.id == id(2))
                .unwrap()
                .label
                .as_deref(),
            Some("Latest")
        );
        assert_eq!(nodes, world.project_accessibility(document));
        assert_eq!(pending.take_for_publication(), (None, false));
    }

    #[test]
    fn idle_retries_preserve_a_single_delta_and_targets_remain_independent() {
        let delta = |generation| AccessibilityDelta {
            generation,
            updated: vec![],
            removed: vec![StableNodeId::new(generation).unwrap()],
        };
        let mut first = PendingAccessibility::default();
        let mut second = PendingAccessibility::default();
        first.stage(delta(1));
        second.stage(delta(1));
        for generation in 2..=100 {
            first.stage(delta(generation));
            second.stage(AccessibilityDelta {
                generation: 1,
                updated: vec![],
                removed: vec![],
            });
        }
        assert_eq!(first.take_for_publication(), (None, true));
        assert_eq!(
            second.take_for_publication(),
            (Some(AccessibilityUpdate::Delta(delta(1))), false)
        );
        first.stage(delta(101));
        assert_eq!(
            first.take_for_publication(),
            (Some(AccessibilityUpdate::Delta(delta(101))), false)
        );
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn handle_ime(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: ImeEvent,
    ) {
        let window_event = WindowEvent::Ime {
            id,
            event: event.clone(),
        };
        let now = self.animation_clock.runtime_time(std::time::Instant::now());
        let source = nana_ui_platform::InputSourceId(id.0);
        let generation =
            nana_ui_platform::EndpointGeneration(*self.input_generations.entry(id).or_insert(1));
        let sequence = self.input_mut(id).next_canonical_sequence();
        let document = self
            .program
            .read_document(id, |document| document.document());
        let mut ime_changed = false;
        if let Some(document) = document {
            self.input_router
                .ensure_attached(source, generation, document);
            let metadata = nana_ui_platform::InputMetadata {
                source,
                device: nana_ui_platform::DeviceId(0),
                generation,
                sequence,
                timestamp: nana_ui_platform::InputTimestamp(
                    now.as_nanos().min(u128::from(u64::MAX)) as u64,
                ),
            };
            if let Some(canonical) = nana_ui_platform::lower_ime_event(&event, metadata) {
                if let Some(disposition) = self.route_native_lifecycle_event(id, canonical, now) {
                    ime_changed = disposition.prevent_default && !matches!(event, ImeEvent::Enabled)
                }
                // Native IME is still applied through the existing winit
                // request path below; clipboard and cursor requests are
                // fulfilled and correlated instead of being silently dropped.
                let _ = self.drain_host_service_requests(id);
            }
        }
        let modal_blocks_ime = self
            .program
            .read_document(id, |document| {
                document
                    .context()
                    .has_blocking_runtime_overlay(document.document())
            })
            .unwrap_or(false);
        // Runtime already applied IME. Still notify the program so Vue can emit
        // JS events; programs must not re-apply the same IME to Runtime.
        let mut update =
            gated_runtime_window_update(!should_deliver_program_ime(modal_blocks_ime), || {
                self.program
                    .window_event(window_event, &self.context_for(id))
            });
        if ime_changed {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
        self.apply_ime_request(id);
    }

    /// Fulfil native-window HostService requests outside the input route.
    /// IME remains applied by the established native IME state machine, while
    /// Cursor requests perform the direct native icon side effect here.
    /// Unsupported capabilities are drained explicitly so they cannot fill the
    /// bounded queue forever. Clipboard responses are applied only after the
    /// Router's generation/document/focus checks pass.
    pub(super) fn drain_host_service_requests(&mut self, id: WindowId) -> bool {
        let requests = self
            .program
            .write_document(id, |document| {
                self.input_router
                    .take_host_service_requests(document.context(), usize::MAX)
            })
            .unwrap_or_default();
        let mut applied = false;
        let mut failure = None;
        let mut clipboard = None;
        for request in requests {
            let outcome = if request.capability() == HostCapability::Clipboard {
                let host = clipboard
                    .get_or_insert_with(|| ClipboardHostServices::new(default_shared_clipboard()));
                nana_ui_platform::HostServices::request(host, request.clone())
            } else if let HostServiceRequest::Cursor { cursor, .. } = &request {
                if self.apply_host_cursor(id, cursor) {
                    nana_ui_platform::HostServiceOutcome::Success
                } else {
                    nana_ui_platform::HostServiceOutcome::Unsupported
                }
            } else if matches!(&request, HostServiceRequest::NativeTextInput { .. }) {
                // Text content and caret geometry remain owned by the
                // established IME request state machine. This capability
                // intent only wakes that native application path.
                self.apply_ime_request(id);
                nana_ui_platform::HostServiceOutcome::Success
            } else {
                nana_ui_platform::HostServiceOutcome::Unsupported
            };
            let response = HostServiceResponse { request, outcome };
            let result = self.program.write_document(id, |document| {
                self.input_router
                    .apply_host_service_response(document.context_mut(), response)
            });
            match result {
                Some(Ok(changed)) => applied |= changed,
                Some(Err(error)) => failure = Some(error.to_string()),
                None => {}
            }
        }
        if let Some(error) = failure {
            self.program
                .report_host_failure(HostFailure::InputDispatch { window: id, error });
        }
        applied
    }

    pub(super) fn apply_ime_request(&mut self, id: WindowId) {
        let request = self
            .program
            .read_document(id, |document| resolved_scene_ime_request(Some(document)))
            .unwrap_or_else(|| resolved_scene_ime_request(None));
        let surrounding = self
            .program
            .read_document(id, runtime_ime_surrounding)
            .flatten();
        let previous = self.ime.get(&id);
        if previous
            .is_some_and(|applied| applied.request == request && applied.surrounding == surrounding)
        {
            return;
        }
        let Some(window) = self.window(id).cloned() else {
            return;
        };
        // Follow the focused editable field, not NSWindow key status.
        // Gating on has_focus() disables IME while the SCIM candidate panel is
        // key, and also races automation that activates then types immediately.
        let ime_text = surrounding.as_ref().and_then(|snapshot| {
            ImeSurroundingText::new(snapshot.text.clone(), snapshot.cursor, snapshot.anchor).ok()
        });
        apply_text_input_request(
            window.as_ref(),
            ime_apply(
                previous.map(|applied| &applied.request),
                previous.is_some_and(|applied| applied.surrounding.is_some()),
                request,
                ime_text,
            ),
        );
        self.ime.insert(
            id,
            AppliedIme {
                request,
                surrounding,
            },
        );
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn take_accessibility_actions(
        &self,
        id: WindowId,
    ) -> Vec<nana_ui_runtime::AccessibilityActionRequest> {
        self.window_contexts
            .get(&id)
            .and_then(|host| host.accessibility.as_ref())
            .map_or_else(Vec::new, HostedAccessibility::take_actions)
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn synchronize_accessibility(&mut self, id: WindowId) {
        let scale_factor = self.scale_factor(id);
        let has_adapter = self
            .window_contexts
            .get(&id)
            .is_some_and(|host| host.accessibility.is_some());
        if !has_adapter {
            return;
        }
        let scale_factor_changed = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.accessibility.as_ref())
            .is_some_and(|accessibility| accessibility.scale_factor_changed(scale_factor));
        let projector_generation = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.accessibility.as_ref())
            .and_then(HostedAccessibility::retained_generation);
        let Some(pending) = self.accessibility_pending_mut(id) else {
            return;
        };
        let (pending, resnapshot) = pending.take_for_publication();
        let program = self.program.take_accessibility_update(id);
        let world_generation = accessibility_world_generation(&mut self.program, id);
        let Some(update) = next_accessibility_update(
            pending,
            program,
            scale_factor_changed || resnapshot,
            projector_generation,
            world_generation,
            || accessibility_snapshot(&mut self.program, id),
        ) else {
            return;
        };
        if let Some(accessibility) = self
            .window_contexts
            .get_mut(&id)
            .and_then(|host| host.accessibility.as_mut())
        {
            accessibility.synchronize(update, scale_factor);
        }
    }
    pub(super) fn accessibility_pending_mut(
        &mut self,
        id: WindowId,
    ) -> Option<&mut PendingAccessibility> {
        self.window_contexts
            .get_mut(&id)
            .map(|host| &mut host.accessibility_pending)
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn accessibility_mut(&mut self, id: WindowId) -> Option<&mut HostedAccessibility> {
        self.window_contexts
            .get_mut(&id)
            .and_then(|host| host.accessibility.as_mut())
    }
}
