//! Scene host schedule coordination.

use super::*;

impl<Program: RuntimeProgram> SceneReady<Program> {
    pub(super) fn process_message(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        message: Program::Message,
    ) {
        self.bind_after_present.insert(WindowId::PRIMARY);
        let update = self.program.update(message, &self.context());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }
    pub(super) fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let now = Instant::now();
        #[cfg(target_os = "macos")]
        self.unpin_idle_present_transactions();
        if self.graphics.take_device_lost()
            || self.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            self.recover_device(event_loop);
        }
        if self.next_wakeup().is_some_and(|deadline| now >= deadline) {
            self.wake(event_loop, now);
        }
        let next_wakeup = [self.next_gpu_retry, self.next_wakeup()]
            .into_iter()
            .flatten()
            .min();
        event_loop.set_control_flow(next_wakeup.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
    pub(super) fn animation_deadline(&mut self) -> Option<Instant> {
        self.known_window_ids()
            .into_iter()
            .filter_map(|id| {
                self.program
                    .read_document(id, |document| {
                        self.animation_clock.next_wakeup(document.context())
                    })
                    .flatten()
            })
            .min()
    }
    pub(super) fn next_wakeup(&mut self) -> Option<Instant> {
        match (self.animation_deadline(), self.program.next_wakeup()) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        }
    }
    pub(super) fn drain_program_messages(&mut self, id: WindowId) -> RuntimeProgramUpdate {
        let mut update = RuntimeProgramUpdate::default();
        for _ in 0..MAX_PROGRAM_DISPATCHES {
            let queued = self
                .program
                .write_document(id, |document| {
                    document.context_mut().take_program_messages()
                })
                .unwrap_or_default();
            if queued.is_empty() {
                break;
            }
            self.bind_after_present.insert(id);
            for boxed in queued {
                let Ok(message) = boxed.downcast::<Program::Message>() else {
                    continue;
                };
                update = update.merge(self.program.update(*message, &self.context_for(id)));
            }
        }
        update
    }
    pub(super) fn drain_all_program_messages(&mut self) -> RuntimeProgramUpdate {
        let mut update = RuntimeProgramUpdate::default();
        for id in self.known_window_ids() {
            update = update.merge(self.drain_program_messages(id));
        }
        update
    }
    pub(super) fn wake(&mut self, event_loop: &dyn ActiveEventLoop, now: Instant) {
        let mut update = self.drain_all_program_messages();
        update = update.merge(self.program.wake(now, &self.context()));
        for id in self.known_window_ids() {
            let frame = self.program.write_document(id, |document| {
                self.animation_clock.wake(document.context_mut(), now)
            });
            let Some(frame) = frame else {
                continue;
            };
            let had_samples = frame.has_updates();
            match self
                .program
                .animation_frame(id, frame, &self.context_for(id))
            {
                Ok(frame_update) => update = update.merge(frame_update),
                Err(error) => {
                    self.program.host_failure(HostFailure::AnimationFrame {
                        window: id,
                        error: error.to_string(),
                    });
                }
            }
            if had_samples {
                update = update.merge(RuntimeProgramUpdate::redraw(id));
            }
        }
        update = update.merge(self.drain_all_program_messages());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }
}
