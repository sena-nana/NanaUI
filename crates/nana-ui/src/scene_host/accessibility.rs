//! Scene host accessibility coordination.

use super::*;

impl<Program: RuntimeProgram> SceneReady<Program> {
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
        let ime_changed = self
            .program
            .write_document(id, |document| {
                let document_id = document.document();
                RuntimeInputAdapter::default()
                    .dispatch_ime(document.context_mut(), document_id, &event)
                    .map(|disposition| {
                        disposition.prevent_default && !matches!(event, ImeEvent::Enabled)
                    })
            })
            .transpose()
            .unwrap_or_else(|error| {
                // Drop this IME event instead of panicking; the program sees
                // the failure through host_failure.
                self.program.host_failure(HostFailure::ImeDispatch {
                    window: id,
                    error: error.to_string(),
                });
                Some(false)
            })
            .unwrap_or(false);
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
        if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .map_or_else(Vec::new, HostedAccessibility::take_actions)
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .map_or_else(Vec::new, HostedAccessibility::take_actions)
        }
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn synchronize_accessibility(&mut self, id: WindowId) {
        let scale_factor = self.scale_factor(id);
        let has_adapter = if id == WindowId::PRIMARY {
            self.accessibility.is_some()
        } else {
            self.auxiliary
                .get(&id)
                .is_some_and(|host| host.accessibility.is_some())
        };
        if !has_adapter {
            return;
        }
        let scale_factor_changed = if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .is_some_and(|accessibility| accessibility.scale_factor_changed(scale_factor))
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .is_some_and(|accessibility| accessibility.scale_factor_changed(scale_factor))
        };
        let projector_generation = if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .and_then(HostedAccessibility::retained_generation)
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .and_then(HostedAccessibility::retained_generation)
        };
        let pending = self.accessibility_pending_mut(id).take();
        let program = self.program.take_accessibility_update(id);
        let world_generation = accessibility_world_generation(&mut self.program, id);
        let Some(update) = next_accessibility_update(
            pending,
            program,
            scale_factor_changed,
            projector_generation,
            world_generation,
            || accessibility_snapshot(&mut self.program, id),
        ) else {
            return;
        };
        if id == WindowId::PRIMARY {
            if let Some(accessibility) = self.accessibility.as_mut() {
                accessibility.synchronize(update, scale_factor);
            }
        } else if let Some(accessibility) = self
            .auxiliary
            .get_mut(&id)
            .and_then(|host| host.accessibility.as_mut())
        {
            accessibility.synchronize(update, scale_factor);
        }
    }
    pub(super) fn accessibility_pending_mut(
        &mut self,
        id: WindowId,
    ) -> &mut Option<AccessibilityUpdate> {
        if id == WindowId::PRIMARY {
            &mut self.accessibility_pending
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            &mut host.accessibility_pending
        } else {
            &mut self.accessibility_pending
        }
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn accessibility_mut(&mut self, id: WindowId) -> Option<&mut HostedAccessibility> {
        if id == WindowId::PRIMARY {
            self.accessibility.as_mut()
        } else {
            self.auxiliary
                .get_mut(&id)
                .and_then(|host| host.accessibility.as_mut())
        }
    }
}
