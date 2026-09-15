//! Scene host schedule coordination.

use super::*;

pub(super) struct HostWorkWake {
    pending: std::sync::atomic::AtomicBool,
    host_thread: std::thread::ThreadId,
    proxy: EventLoopProxy,
}
impl HostWorkWake {
    pub(super) fn new(proxy: EventLoopProxy) -> Self {
        Self {
            pending: std::sync::atomic::AtomicBool::new(false),
            host_thread: std::thread::current().id(),
            proxy,
        }
    }
    pub(super) fn wake(&self) {
        let was_pending = self.pending.swap(true, std::sync::atomic::Ordering::AcqRel);
        // A callback on the host thread is already inside the loop. Re-signalling
        // its native source can starve macOS redraw observers indefinitely.
        if !was_pending && std::thread::current().id() != self.host_thread {
            self.proxy.wake_up();
        }
    }
    fn take_pending(&self) -> bool {
        self.pending
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }
}

/// Yield between finite batches even when callbacks continuously enqueue more work.
/// One slow callback cannot be preempted, but no further callback starts after the deadline.
pub(super) fn drain_host_batch(
    mut work: impl FnMut() -> bool,
    mut now: impl FnMut() -> Instant,
) -> bool {
    let deadline = now() + Duration::from_millis(2);
    for _ in 0..64 {
        if !work() {
            return false;
        }
        if now() >= deadline {
            return true;
        }
    }
    true
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn drain_host_work(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.host_work.take_pending();
        self.host_work_deadline = None;
        self.drain_window_requests(event_loop);
        self.complete_file_dialogs(event_loop);
        self.drain_host_messages(event_loop);
        self.drain_browser_events(event_loop);
    }
    pub(super) fn drain_host_messages(&mut self, event_loop: &dyn ActiveEventLoop) {
        let remaining = drain_host_batch(
            || {
                if self.shutting_down || event_loop.exiting() {
                    return false;
                }
                let Ok(message) = self.messages.try_recv() else {
                    return false;
                };
                self.process_message(event_loop, message);
                !self.shutting_down && !event_loop.exiting()
            },
            Instant::now,
        );
        if remaining {
            self.host_work.wake();
        }
    }
    pub(super) fn process_message(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        message: Program::Message,
    ) {
        if let Some(id) = self.window_contexts.keys().copied().min() {
            self.bind_after_present.insert(id);
        }
        let update = self.program.update(message, &self.context());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }
    pub(super) fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let now = Instant::now();
        if self
            .host_work_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.drain_host_work(event_loop);
        }
        if self.host_work.take_pending() && !self.shutting_down {
            self.host_work_deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(1));
        }
        #[cfg(target_os = "macos")]
        self.unpin_idle_present_transactions();
        let surface_retry_due = self
            .window_contexts
            .values()
            .any(|host| host.surface_retry.is_some_and(|deadline| now >= deadline));
        if surface_retry_due {
            // A device-loss callback may have been waiting on the last in-flight
            // submission when the window was suspended. Poll before local retry.
            let _ = self
                .graphics
                .resources()
                .device()
                .poll(wgpu::PollType::Poll);
        }
        if self.graphics.take_device_lost()
            || self.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            self.recover_device(event_loop);
        }
        if surface_retry_due {
            self.retry_surfaces(now);
        }
        self.sample_passthrough_forward(event_loop);
        if self.next_wakeup().is_some_and(|deadline| now >= deadline) {
            self.wake(event_loop, now);
        }
        let frame_deadline = self.schedule_presentations(event_loop, now);
        let next_wakeup = [
            self.next_gpu_retry,
            (!self.render_suspended)
                .then(|| {
                    self.window_contexts
                        .values()
                        .filter_map(|host| host.surface_retry)
                        .min()
                })
                .flatten(),
            self.next_wakeup(),
            frame_deadline,
            self.host_work_deadline,
            self.passthrough_forward_wakeup(),
        ]
        .into_iter()
        .flatten()
        .min();
        self.wake_deadline = next_wakeup;
        if !self.embedded {
            event_loop
                .set_control_flow(next_wakeup.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
        }
    }
    pub(super) fn can_present(&self, id: WindowId) -> bool {
        self.window_contexts
            .get(&id)
            .is_some_and(|host| host.surface_retry.is_none())
            && !self.occluded.contains(&id)
            && self.window(id).is_some_and(|window| {
                window.is_visible() != Some(false) && window.is_minimized() != Some(true)
            })
    }

    fn schedule_presentations(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        now: Instant,
    ) -> Option<Instant> {
        let changed = std::mem::take(&mut *self.texture_redraws.lock().expect("texture redraws"));
        for id in changed {
            if self.can_present(id) {
                self.request_redraw(id);
            }
        }
        let mut next = None;
        for id in self.known_window_ids() {
            if self.render_suspended || self.window_contexts[&id].surface_retry.is_some() {
                self.frame_schedules.remove(&id);
                continue;
            }
            let demand = self.program.frame_demand(id);
            let drawable = drawable_surface(self.geometry_of(id).physical_size);
            let due = self.frame_schedules.entry(id).or_default().due(demand, now);
            let tick = frame_tick(self.can_present(id), due, drawable);
            let (deadline, armed_at) = match tick {
                FrameTick::Present => {
                    self.request_redraw(id);
                    (
                        self.frame_schedules.entry(id).or_default().arm(demand, now),
                        now,
                    )
                }
                FrameTick::GpuOnly => {
                    let served = self.tick_hidden_gpu(event_loop, id);
                    let armed_at = Instant::now();
                    let demand = self.program.frame_demand(id);
                    let schedule = self.frame_schedules.entry(id).or_default();
                    let deadline = if served {
                        schedule.advance_served(demand, armed_at)
                    } else {
                        schedule.defer(demand, armed_at)
                    };
                    (deadline, armed_at)
                }
                FrameTick::None => (
                    self.frame_schedules.entry(id).or_default().arm(demand, now),
                    now,
                ),
            };
            if let Some(deadline) = frame_wait_target(tick, deadline, armed_at) {
                next = Some(next.map_or(deadline, |old: Instant| old.min(deadline)));
            }
        }
        next
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameTick {
    None,
    Present,
    GpuOnly,
}

pub(super) fn drawable_surface(physical_size: (u32, u32)) -> bool {
    physical_size.0 > 0 && physical_size.1 > 0
}

fn frame_tick(can_present: bool, demand_due: bool, drawable: bool) -> FrameTick {
    if !demand_due {
        FrameTick::None
    } else if can_present && drawable {
        FrameTick::Present
    } else {
        FrameTick::GpuOnly
    }
}

fn frame_wait_target(
    tick: FrameTick,
    deadline: Option<Instant>,
    armed_at: Instant,
) -> Option<Instant> {
    if deadline.is_some_and(|deadline| deadline > armed_at) {
        deadline
    } else if tick == FrameTick::GpuOnly {
        Some(armed_at)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameTick, drawable_surface, frame_tick, frame_wait_target};
    use crate::runtime_host::{FrameDemand, FrameSchedule};
    use std::time::{Duration, Instant};

    #[test]
    fn demand_ticks_only_present_with_a_drawable_visible_surface() {
        let now = Instant::now();
        for (visible, drawable, expected) in [
            (false, false, FrameTick::GpuOnly),
            (false, true, FrameTick::GpuOnly),
            (true, false, FrameTick::GpuOnly),
            (true, true, FrameTick::Present),
        ] {
            assert_eq!(frame_tick(visible, true, drawable), expected);
            let mut schedule = FrameSchedule::default();
            let demand = FrameDemand::OnDemand;
            let tick = frame_tick(visible, schedule.due(demand, now), drawable);
            assert_eq!(tick, FrameTick::None);
            assert_eq!(
                frame_wait_target(tick, schedule.arm(demand, now), now),
                None
            );
        }
    }

    #[test]
    fn zero_size_continuous_ticks_keep_waking_and_resume_presenting() {
        let demand = FrameDemand::Continuous(std::num::NonZeroU32::new(60).unwrap());
        for size in [(0, 100), (200, 0), (0, 0)] {
            for can_present in [false, true] {
                let mut schedule = FrameSchedule::default();
                let mut now = Instant::now();
                for _ in 0..8 {
                    let tick = frame_tick(
                        can_present,
                        schedule.due(demand, now),
                        drawable_surface(size),
                    );
                    assert_eq!(tick, FrameTick::GpuOnly);
                    let deadline = schedule.advance_served(demand, now);
                    let next = frame_wait_target(tick, deadline, now).expect("continuous wakeup");
                    assert!(next > now);
                    assert!(!schedule.due(demand, now));
                    now = next;
                }
                assert_eq!(
                    frame_tick(true, schedule.due(demand, now), true),
                    FrameTick::Present
                );
            }
        }
    }

    #[test]
    fn gpu_only_at_tracks_post_prepare_demand_and_failure_retry() {
        let now = Instant::now();
        let later = now + Duration::from_millis(50);
        let demand = FrameDemand::At(now);
        for (visible, drawable) in [(false, false), (false, true), (true, false)] {
            for (after, expected_deadline, expected_wake) in [
                (FrameDemand::OnDemand, None, now),
                (FrameDemand::At(later), Some(later), later),
                (demand, Some(now), now),
            ] {
                let mut schedule = FrameSchedule::default();
                let tick = frame_tick(visible, schedule.due(demand, now), drawable);
                assert_eq!(tick, FrameTick::GpuOnly);
                let deadline = schedule.advance_served(after, now);
                assert_eq!(deadline, expected_deadline);
                assert_eq!(frame_wait_target(tick, deadline, now), Some(expected_wake));
                // Cleared demand settles to idle; retained At work stays armed.
                assert_eq!(
                    schedule.due(after, expected_wake),
                    expected_deadline.is_some()
                );
            }
            let mut schedule = FrameSchedule::default();
            let tick = frame_tick(visible, schedule.due(demand, now), drawable);
            let retry = frame_wait_target(tick, schedule.defer(demand, now), now).unwrap();
            assert!(retry > now);
            assert!(!schedule.due(demand, now));
            assert!(schedule.due(demand, retry));
        }
    }

    #[test]
    fn gpu_only_waits_at_armed_when_deadline_is_not_after_now() {
        let now = Instant::now();
        assert_eq!(
            frame_wait_target(FrameTick::GpuOnly, Some(now), now),
            Some(now)
        );
        assert_eq!(frame_wait_target(FrameTick::Present, Some(now), now), None);
        let later = now + Duration::from_millis(16);
        assert_eq!(
            frame_wait_target(FrameTick::GpuOnly, Some(later), now),
            Some(later)
        );
    }

    #[test]
    fn zero_size_is_not_a_presentable_surface() {
        assert!(!drawable_surface((0, 0)));
        assert!(!drawable_surface((0, 100)));
        assert!(!drawable_surface((200, 0)));
        assert!(drawable_surface((200, 100)));
    }
}

#[cfg(test)]
mod queue_fairness_tests {
    use super::*;
    #[test]
    fn self_replenishing_producer_yields_without_losing_fifo_order() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(0).unwrap();
        let mut consumed = Vec::new();
        let now = Instant::now();
        assert!(drain_host_batch(
            || {
                let value = rx.try_recv().unwrap();
                consumed.push(value);
                tx.send(value + 1).unwrap();
                true
            },
            || now
        ));
        assert_eq!(consumed, (0..64).collect::<Vec<_>>());
        assert_eq!(rx.try_recv().unwrap(), 64);
    }
    #[test]
    fn slow_callback_yields_before_starting_another() {
        let start = Instant::now();
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let calls = std::cell::Cell::new(0);
        assert!(drain_host_batch(
            || {
                calls.set(calls.get() + 1);
                elapsed.set(Duration::from_millis(3));
                true
            },
            || start + elapsed.get()
        ));
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn empty_or_stopped_queue_does_not_schedule_another_wake() {
        assert!(!drain_host_batch(|| false, Instant::now));
    }
}
