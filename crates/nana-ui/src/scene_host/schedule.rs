//! Scene host schedule coordination.

use super::*;
use crate::runtime_host::FrameDemand;
use std::num::NonZeroU32;

/// Compositor overlay present cadence. Display refresh still caps vsync.
const COMPOSITOR_PRESENT_HZ: NonZeroU32 = match NonZeroU32::new(120) {
    Some(hz) => hz,
    None => unreachable!(),
};

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
    pub(super) fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.drain_host_work(event_loop);
        // Windows' native move/resize loop dispatches proxy messages but never
        // reaches winit's outer AboutToWait. Serve all windows and hidden GPU
        // outputs with the same demand/deadline policy during that loop.
        #[cfg(target_os = "windows")]
        if !self.shutting_down
            && !event_loop.exiting()
            && self
                .window_contexts
                .values()
                .any(|host| host.size_move.is_active())
        {
            self.about_to_wait(event_loop);
        }
    }

    pub(super) fn drain_host_work(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.host_work.take_pending();
        self.host_work_deadline = None;
        self.drain_window_requests(event_loop);
        self.apply_pending_icons();
        self.process_startup_requests(event_loop);
        self.complete_file_dialogs(event_loop);
        self.complete_color_samplers(event_loop);
        self.drain_all_color_pick_requests(event_loop);
        self.drain_host_messages(event_loop);
        self.deliver_pen_leaves(event_loop);
        self.drain_browser_events(event_loop);
        self.drain_web_surface_events(event_loop);
        self.drain_notification_events(event_loop);
        // Views' futures whose wakers fired: what they write lands in the
        // next frame of the windows that read it.
        if nana_ui_runtime::view::poll_tasks() > 0 {
            self.request_reactive_redraws(None);
        }
    }
    pub(super) fn drain_host_messages(&mut self, event_loop: &dyn ActiveEventLoop) {
        let remaining = drain_host_batch(
            || {
                if self.shutting_down || event_loop.exiting() {
                    return false;
                }
                // Startup messages go first: they were returned before
                // anything else could be sent.
                let Some(message) = self
                    .startup_messages
                    .pop_front()
                    .or_else(|| self.messages.try_recv().ok())
                else {
                    return false;
                };
                self.process_message(event_loop, message);
                !self.shutting_down && !event_loop.exiting()
            },
            Instant::now,
        );
        if remaining {
            self.host_work.wake();
        } else {
            self.startup_messages_applied();
        }
    }
    pub(super) fn drain_notification_events(&mut self, event_loop: &dyn ActiveEventLoop) {
        for event in self.notifications.take_events() {
            if self.shutting_down || event_loop.exiting() {
                return;
            }
            let update = self.program.notification_event(event, &self.context());
            self.apply_update(event_loop, update, None);
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
    /// Deliver a system reduce-motion change recorded by the window subclass.
    fn sync_reduced_motion(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !nana_window::take_reduced_motion_change() {
            return;
        }
        let Some(reduced) = nana_window::system_reduced_motion() else {
            return;
        };
        if reduced == self.reduced_motion {
            return;
        }
        self.reduced_motion = reduced;
        for id in self.known_window_ids() {
            self.sync_document_reduced_motion(id);
            let update = self.program.window_event(
                WindowEvent::ReducedMotionChanged { id, reduced },
                &self.context_for(id),
            );
            self.apply_update(event_loop, update, None);
        }
    }

    /// Report the system reduce-motion preference to `id`'s document, so its
    /// view-layer motion follows it ([`nana_ui_runtime::AppContext::reduced_motion`]).
    pub(super) fn sync_document_reduced_motion(&mut self, id: WindowId) {
        let reduced = self.reduced_motion;
        let _ = self.program.write_document(id, |document| {
            document.context_mut().set_system_reduced_motion(reduced);
        });
    }

    /// Deliver a system high-contrast change recorded by the window subclass.
    fn sync_high_contrast(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !nana_window::take_high_contrast_change() {
            return;
        }
        let Some(high_contrast) = nana_window::system_high_contrast() else {
            return;
        };
        if high_contrast == self.high_contrast {
            return;
        }
        self.high_contrast = high_contrast;
        for id in self.known_window_ids() {
            self.apply_high_contrast(id);
            let update = self.program.window_event(
                WindowEvent::HighContrastChanged { id, high_contrast },
                &self.context_for(id),
            );
            self.apply_update(event_loop, update, None);
        }
    }

    /// Deliver a system reduce-transparency change (macOS display options).
    fn sync_reduced_transparency(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !nana_window::take_reduced_transparency_change() {
            return;
        }
        let Some(reduced) = nana_window::system_reduced_transparency() else {
            return;
        };
        if reduced == self.reduced_transparency {
            return;
        }
        self.reduced_transparency = reduced;
        for id in self.known_window_ids() {
            let update = self.program.window_event(
                WindowEvent::ReducedTransparencyChanged { id, reduced },
                &self.context_for(id),
            );
            self.apply_update(event_loop, update, None);
        }
    }

    /// Deliver a system text-size change (Windows `TextScaleFactorChanged`).
    fn sync_text_scale(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !nana_window::take_text_scale_change() {
            return;
        }
        let Some(scale) = nana_window::system_text_scale() else {
            return;
        };
        if self.text_scale == Some(scale) {
            return;
        }
        self.text_scale = Some(scale);
        for id in self.known_window_ids() {
            let update = self.program.window_event(
                WindowEvent::TextScaleChanged { id, scale },
                &self.context_for(id),
            );
            self.apply_update(event_loop, update, None);
        }
    }

    /// Hand the window's document the system high-contrast setting: while it
    /// is on, the installed theme paints with the high-contrast palette.
    pub(super) fn apply_high_contrast(&mut self, id: WindowId) {
        let high_contrast = self.high_contrast;
        let changed = self
            .program
            .write_document(id, |document| {
                document.context_mut().set_high_contrast(high_contrast)
            })
            .unwrap_or(false);
        if changed {
            self.request_redraw(id);
        }
    }

    pub(super) fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Pointer moves and wheel deltas merged through this turn route once.
        self.drain_deferred_input(event_loop);
        let now = Instant::now();
        if self
            .host_work_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.drain_host_work(event_loop);
        }
        #[cfg(target_os = "macos")]
        {
            self.unpin_idle_present_transactions();
            self.commit_finished_live_resizes(event_loop);
        }
        let surface_retry_due = self
            .window_contexts
            .values()
            .any(|host| host.surface_retry.is_some_and(|deadline| now >= deadline));
        if surface_retry_due {
            // A device-loss callback may have been waiting on the last in-flight
            // submission when the window was suspended. Poll before local retry.
            self.graphics.gpu().poll();
        }
        if self.graphics.take_device_lost()
            || self.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            self.recover_device(event_loop);
        }
        if surface_retry_due {
            self.retry_surfaces(now);
        }
        let startup_latch = self.poll_startup_latch(event_loop);
        self.sample_passthrough_forward(event_loop);
        self.sync_reduced_motion(event_loop);
        self.sync_high_contrast(event_loop);
        self.sync_reduced_transparency(event_loop);
        self.sync_text_scale(event_loop);
        self.sync_display_topology(event_loop);
        let now = Instant::now().max(now);
        if self.next_wakeup().is_some_and(|deadline| now >= deadline) {
            self.wake(event_loop, now);
        }
        let frame_deadline = self.schedule_presentations(event_loop, now);
        // Last: routing and the frames above recorded what the IME should
        // show (a frame moves the caret it anchors to).
        self.apply_window_ime();
        // Taken here, after everything above has had its chance to enqueue
        // work: a signal raised on the host thread deliberately skips the
        // proxy wake-up, so this deadline is the only thing that brings the
        // loop back for it. Taken before `retry_surfaces` and
        // `schedule_presentations`, whatever they signalled stayed pending
        // with nothing scheduled, and waited for an unrelated event.
        if self.host_work.take_pending() && !self.shutting_down {
            self.host_work_deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(1));
        }
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
            startup_latch,
        ]
        .into_iter()
        .flatten()
        .min();
        self.wake_deadline = next_wakeup;
        if !self.embedded {
            event_loop
                .set_control_flow(next_wakeup.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
            // winit on Windows waits on the control flow before it sees an exit
            // asked for during this turn, and with no window left nothing wakes it.
            if event_loop.exiting() {
                event_loop.set_control_flow(ControlFlow::Poll);
            }
        }
    }
    pub(super) fn can_present(&self, id: WindowId) -> bool {
        let Some(host) = self.window_contexts.get(&id) else {
            return false;
        };
        host.surface_retry.is_none()
            // An underlay is covered by its parent by design, and what it
            // draws is still read — by capture software — while covered.
            && (!self.occluded.contains(&id)
                || self.startup_presentation_held(id)
                || host.settings.role == WindowRole::Underlay)
            && self.window(id).is_some_and(|window| {
                window.is_visible() != Some(false) && window.is_minimized() != Some(true)
            })
    }

    fn schedule_presentations(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        now: Instant,
    ) -> Option<Instant> {
        let mut changed =
            std::mem::take(&mut *self.texture_redraws.lock().expect("texture redraws"));
        // The startup painter arrived: the frames held for it are due.
        if self.startup_painter.is_some() && self.take_startup_painter(false) {
            changed.extend(self.known_window_ids());
        }
        if self
            .material_redraws
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            changed.extend(self.known_window_ids());
        }
        for id in changed {
            if self.can_present(id) && !self.startup_holds(id) {
                self.request_redraw(id);
            }
        }
        let mut next = None;
        for id in self.known_window_ids() {
            // `redraw` and `tick_hidden_gpu` below can close another window
            // still in this snapshot, or exit the process. The id was copied
            // out before that, so indexing it panics once it is gone.
            if event_loop.exiting() {
                break;
            }
            let Some(retrying) = self
                .window_contexts
                .get(&id)
                .map(|host| host.surface_retry.is_some())
            else {
                self.frame_schedules.remove(&id);
                continue;
            };
            if self.render_suspended || retrying {
                self.frame_schedules.remove(&id);
                continue;
            }
            let demand = self.window_frame_demand(id);
            let drawable = drawable_surface(self.geometry_of(id).physical_size);
            let due = self.frame_schedules.entry(id).or_default().due(demand, now);
            // A primary window held under its splash gets no periodic frames:
            // nothing it drew could be seen. Explicit redraws still flush it,
            // and the takeover request draws the frame that ends the hold.
            let presents = self.can_present(id) && !self.startup_holds(id);
            let tick = frame_tick(presents, due, drawable);
            let (deadline, armed_at) = match tick {
                FrameTick::Present => {
                    let watchdog = self
                        .frame_schedules
                        .entry(id)
                        .or_default()
                        .note_requested(demand, now);
                    if now >= watchdog && cfg!(target_os = "windows") {
                        // A whole period went by and the requested paint never
                        // came. A swap chain presents without one, so serve
                        // the frame here rather than wait for the next
                        // unrelated message.
                        nana_diagnostics::metric!(
                            nana_diagnostics::framework::host::FRAMES_SERVED_WITHOUT_PAINT
                        );
                        self.redraw(event_loop, id);
                        if !self.window_contexts.contains_key(&id) {
                            continue;
                        }
                        let armed_at = Instant::now();
                        let demand = self.window_frame_demand(id);
                        let schedule = self.frame_schedules.entry(id).or_default();
                        (schedule.arm(demand, armed_at), armed_at)
                    } else {
                        let schedule = self.frame_schedules.entry(id).or_default();
                        // Where frames must come from the paint callback, a
                        // request that went unanswered is simply made again.
                        let watchdog = if now >= watchdog {
                            schedule.renew_request(now);
                            schedule.note_requested(demand, now)
                        } else {
                            watchdog
                        };
                        let deadline = schedule.arm(demand, now);
                        self.request_redraw(id);
                        // The due frame's deadline is behind us, so only the
                        // paint just requested would bring the loop back for
                        // it. Wait for the grace period as well, in case the
                        // platform withholds that paint.
                        (
                            Some(deadline.filter(|at| *at > now).unwrap_or(watchdog)),
                            now,
                        )
                    }
                }
                FrameTick::GpuOnly => {
                    let served = self.tick_hidden_gpu(event_loop, id);
                    let armed_at = Instant::now();
                    let demand = self.window_frame_demand(id);
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
        let mut drained = 0usize;
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
            // Depth is what was waiting when the drain began; later rounds
            // are messages the updates themselves sent.
            if drained == 0 {
                drained = queued.len();
            }
            for boxed in queued {
                let Ok(message) = boxed.downcast::<Program::Message>() else {
                    continue;
                };
                update = update.merge(self.program.update(*message, &self.context_for(id)));
            }
        }
        if drained > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::host::MESSAGE_QUEUE_DEPTH,
                drained
            );
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
        // The program may start a reveal or a motion from a message or its
        // own wake, between frames; that is stamped with the clock too.
        let now = Instant::now().max(now);
        for id in self.known_window_ids() {
            self.sync_compositor_clock_at(id, now);
        }
        let mut update = self.drain_all_program_messages();
        update = update.merge(self.program.wake(now, &self.context()));
        // Re-read after syncing the compositor clock and running program hooks:
        // pending lifecycle events report `animation_now` as their deadline, so
        // checking against an older `now` would leave them permanently undrained.
        let animation_now = Instant::now().max(now);
        for id in self.known_window_ids() {
            let due = self
                .program
                .read_document(id, |document| {
                    animation_due(self.animation_clock, document, animation_now)
                })
                .unwrap_or(false);
            if !due {
                continue;
            }
            let frame = self.program.write_document(id, |document| {
                self.animation_clock
                    .wake(document.context_mut(), animation_now)
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
                    self.program
                        .report_host_failure(HostFailure::AnimationFrame {
                            window: id,
                            error: error.to_string(),
                        });
                }
            }
            if cpu_wake_redraw(true, had_samples) {
                update = update.merge(RuntimeProgramUpdate::redraw(id));
            }
        }
        update = update.merge(self.drain_all_program_messages());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }

    pub(super) fn window_frame_demand(&mut self, id: WindowId) -> FrameDemand {
        let program = self.program.frame_demand(id);
        // A sliced layout left subtrees out of view to place: each frame's
        // flush continues it, so frames keep coming until it is done.
        let compositor = self
            .program
            .read_document(id, |document| {
                document.compositor_needs_tick() || document.layout_pending()
            })
            .unwrap_or(false);
        let can_present = self.can_present(id);
        let max_rate = self.program.max_frame_rate(id);
        merge_frame_demand(
            window_present_demand(program, compositor, can_present, max_rate),
            self.window_output_demand(id, can_present),
        )
    }

    /// Bring the document's clock to this frame before it is flushed. A
    /// reveal or motion the flush starts is stamped with that clock, so it
    /// must be this frame's even when nothing was live: stamped with the
    /// instant the compositor last ran, the next frame's clock would be an
    /// idle period ahead of it and the motion would skip that much.
    /// Setting the clock marks nothing dirty; a resting scene keeps the time
    /// it was last presented at.
    pub(super) fn sync_compositor_clock(&mut self, id: WindowId) {
        self.sync_compositor_clock_at(id, Instant::now());
    }

    pub(super) fn sync_compositor_clock_at(&mut self, id: WindowId, at: Instant) {
        let now = self.animation_clock.runtime_time(at);
        // The cadence compositor frames present at is the document's frame
        // interval while it plays (Issue #300).
        let interval = presentation_display_interval(self.program.max_frame_rate(id));
        let _ = self.program.write_document(id, |document| {
            document
                .context_mut()
                .set_presentation_display_interval(interval);
            document.sync_presentation_clock(now);
        });
    }

    pub(super) fn window_surface_generation(&self, id: WindowId) -> u64 {
        self.window_contexts
            .get(&id)
            .map_or(0, |host| host.surface_generation)
    }

    pub(super) fn bump_window_surface_generation(&mut self, id: WindowId) -> Option<u64> {
        let doc_generation = self
            .program
            .read_document(id, |document| document.scene().surface_generation())
            .unwrap_or(0);
        let host = self.window_contexts.get_mut(&id)?;
        let next = advance_surface_generation(&mut host.surface_generation, doc_generation);
        let _ = self.program.write_document(id, |document| {
            document.set_surface_generation(next);
        });
        Some(next)
    }

    pub(super) fn bump_all_surface_generations(&mut self) {
        for id in self.known_window_ids() {
            self.bump_window_surface_generation(id);
        }
    }
}

pub(super) fn advance_surface_generation(window_generation: &mut u64, doc_generation: u64) -> u64 {
    let next = (*window_generation).max(doc_generation).wrapping_add(1);
    *window_generation = next;
    next
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameTick {
    None,
    Present,
    GpuOnly,
}

fn animation_due(
    clock: RuntimeAnimationClock,
    document: &nana_ui_scene::RuntimeDocument,
    now: Instant,
) -> bool {
    clock
        .next_wakeup(document.context())
        .is_some_and(|deadline| deadline <= now)
}

pub(super) fn drawable_surface(physical_size: (u32, u32)) -> bool {
    physical_size.0 > 0 && physical_size.1 > 0
}

/// The geometry a window's output lays out and draws at (Issue #304): the
/// window's own while it has a surface to draw, otherwise the last one that
/// did. A minimised window reports zero size on some platforms; its output
/// keeps the layout it was showing rather than collapsing it, and a fixed
/// output extent is the target's pixels whatever the window's size. Before
/// the window was ever drawable this stays zero and the output draws nothing.
pub(super) fn output_geometry(
    current: nana_ui_platform::WindowGeometry,
    last_drawable: nana_ui_platform::WindowGeometry,
) -> nana_ui_platform::WindowGeometry {
    if drawable_surface(current.physical_size) {
        current
    } else {
        last_drawable
    }
}

/// Per-window present demand. Compositor overlay may keep presenting this
/// window without a CPU animation deadline. Hidden/minimized compositor does
/// not drive GpuOnly ticks: resume presents at absolute time. The program's
/// [`RuntimeProgram::max_frame_rate`] holds both cadences down: compositor
/// motion samples absolute time, so it loses frames, not speed.
pub(super) fn window_present_demand(
    program: FrameDemand,
    compositor_needs_tick: bool,
    can_present: bool,
    max_rate: Option<NonZeroU32>,
) -> FrameDemand {
    let compositor = if compositor_needs_tick && can_present {
        FrameDemand::Continuous(COMPOSITOR_PRESENT_HZ)
    } else {
        FrameDemand::OnDemand
    };
    cap_frame_demand(merge_frame_demand(program, compositor), max_rate)
}

/// The interval between compositor presents for a window held to
/// `max_rate`.
pub(super) fn presentation_display_interval(max_rate: Option<NonZeroU32>) -> Duration {
    let hz = max_rate.map_or(COMPOSITOR_PRESENT_HZ, |max| COMPOSITOR_PRESENT_HZ.min(max));
    Duration::from_secs(1) / hz.get()
}

/// A continuous cadence held to `max_rate`. A one-shot deadline is the
/// program's own and is served when due.
pub(super) fn cap_frame_demand(demand: FrameDemand, max_rate: Option<NonZeroU32>) -> FrameDemand {
    match (demand, max_rate) {
        (FrameDemand::Continuous(rate), Some(max_rate)) => {
            FrameDemand::Continuous(rate.min(max_rate))
        }
        (demand, _) => demand,
    }
}

pub(super) fn merge_frame_demand(left: FrameDemand, right: FrameDemand) -> FrameDemand {
    match (left, right) {
        (FrameDemand::OnDemand, other) | (other, FrameDemand::OnDemand) => other,
        (FrameDemand::Continuous(left), FrameDemand::Continuous(right)) => {
            FrameDemand::Continuous(left.max(right))
        }
        (FrameDemand::At(left), FrameDemand::At(right)) => FrameDemand::At(left.min(right)),
        (continuous @ FrameDemand::Continuous(_), FrameDemand::At(_))
        | (FrameDemand::At(_), continuous @ FrameDemand::Continuous(_)) => continuous,
    }
}

/// CPU animation wake is per-window. A due deadline on one document does not
/// mark a static sibling for redraw.
pub(super) fn cpu_wake_redraw(deadline_due: bool, frame_has_updates: bool) -> bool {
    deadline_due && frame_has_updates
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
    fn a_static_window_polled_at_240hz_presents_nothing() {
        let mut schedule = FrameSchedule::default();
        let start = Instant::now();
        let step = Duration::from_nanos(1_000_000_000 / 240);
        let demand = FrameDemand::OnDemand;
        for index in 0u32..240 {
            let now = start + step * index;
            let tick = frame_tick(true, schedule.due(demand, now), true);
            assert_eq!(tick, FrameTick::None);
            let deadline = schedule.arm(demand, now);
            assert_eq!(frame_wait_target(tick, deadline, now), None);
        }
        let one_second = start + Duration::from_secs(1);
        assert_eq!(
            frame_tick(true, schedule.due(demand, one_second), true),
            FrameTick::None
        );
    }

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

    /// Issue #304: a minimised window reporting zero size keeps its output
    /// at the last drawable geometry, and a fixed output extent stays the
    /// target's pixels: the output still has a viewport to draw.
    #[test]
    fn a_zero_sized_window_outputs_at_its_last_drawable_geometry() {
        use crate::output::window_output::{WindowOutputConfig, WindowOutputExtent};
        use nana_ui_platform::WindowGeometry;
        let shown = WindowGeometry {
            physical_size: (1600, 1200),
            logical_size: (800.0, 600.0),
            scale_factor: 2.0,
            ..WindowGeometry::default()
        };
        let minimised = WindowGeometry {
            physical_size: (0, 0),
            logical_size: (0.0, 0.0),
            ..shown
        };
        assert_eq!(super::output_geometry(shown, shown), shown);
        let geometry = super::output_geometry(minimised, shown);
        assert_eq!(geometry, shown);
        let config = WindowOutputConfig::default().with_extent(WindowOutputExtent::Fixed {
            width: 320,
            height: 240,
        });
        let extent = config.resolve_extent(&geometry).expect("target pixels");
        assert_eq!(extent, [320, 240]);
        let viewport = config
            .viewport(&geometry, extent, [0.0; 4])
            .expect("a viewport to draw");
        assert_eq!(viewport.physical_size, [320, 240]);
        assert!((viewport.scale_factor - 0.4).abs() < 1e-6);
        // The window's own zero size has no viewport: that is what stopped it.
        assert!(config.viewport(&minimised, extent, [0.0; 4]).is_none());
        // A window-sized output keeps the size and layout it was showing.
        let matching = WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow);
        let extent = matching.resolve_extent(&geometry).expect("last size");
        assert_eq!(extent, [1600, 1200]);
        let viewport = matching
            .viewport(&geometry, extent, [0.0; 4])
            .expect("a viewport to draw");
        assert_eq!(viewport.logical_size, [800.0, 600.0]);
        assert!(matching.resolve_extent(&minimised).is_none());
        // A window never drawn has no last geometry; its output stays idle.
        let never = super::output_geometry(minimised, WindowGeometry::default());
        assert!(!drawable_surface(never.physical_size));
    }

    fn fps(n: u32) -> FrameDemand {
        FrameDemand::Continuous(std::num::NonZeroU32::new(n).unwrap())
    }

    #[test]
    fn compositor_tick_is_lightweight_present_not_cpu_wake() {
        let demand = super::window_present_demand(FrameDemand::OnDemand, true, true, None);
        assert_eq!(demand, fps(120));
        assert!(!super::cpu_wake_redraw(false, false));
        assert!(!super::cpu_wake_redraw(false, true));
    }

    #[test]
    fn compositor_tick_does_not_force_hidden_gpu_catch_up() {
        let hidden = super::window_present_demand(FrameDemand::OnDemand, true, false, None);
        assert_eq!(hidden, FrameDemand::OnDemand);
        assert_eq!(
            super::window_present_demand(fps(60), true, false, None),
            fps(60),
            "external GPU cadence still ticks when hidden; compositor does not add one"
        );
        let now = Instant::now();
        let schedule = FrameSchedule::default();
        assert_eq!(
            frame_tick(false, schedule.due(hidden, now), true),
            FrameTick::None
        );
    }

    #[test]
    fn static_window_is_not_redrawn_by_a_sibling_cpu_deadline() {
        let animated = super::cpu_wake_redraw(true, true);
        let static_window = super::cpu_wake_redraw(false, false);
        assert!(animated);
        assert!(!static_window);
        assert_eq!(
            super::window_present_demand(FrameDemand::OnDemand, false, true, None),
            FrameDemand::OnDemand
        );
    }

    #[test]
    fn compositor_present_merges_with_program_gpu_cadence() {
        assert_eq!(
            super::window_present_demand(fps(30), true, true, None),
            fps(120)
        );
        assert_eq!(
            super::window_present_demand(fps(240), true, true, None),
            fps(240)
        );
        let later = Instant::now() + Duration::from_millis(50);
        assert_eq!(
            super::window_present_demand(FrameDemand::At(later), true, true, None),
            fps(120)
        );
        assert_eq!(
            super::window_present_demand(FrameDemand::At(later), false, true, None),
            FrameDemand::At(later)
        );
    }

    #[test]
    fn compositor_continuous_skips_missed_ticks_on_resume() {
        let demand = super::window_present_demand(FrameDemand::OnDemand, true, true, None);
        let mut schedule = FrameSchedule::default();
        let start = Instant::now();
        assert!(schedule.due(demand, start));
        let _ = schedule.advance_served(demand, start);
        let resumed = start + Duration::from_secs(2);
        assert!(schedule.due(demand, resumed));
        let next = schedule.advance_served(demand, resumed).expect("cadence");
        assert!(next > resumed);
        assert!(!schedule.due(demand, resumed));
        assert_eq!(
            frame_tick(true, schedule.due(demand, resumed), true),
            FrameTick::None
        );
    }

    fn rate(n: u32) -> Option<std::num::NonZeroU32> {
        std::num::NonZeroU32::new(n)
    }

    #[test]
    fn a_max_frame_rate_holds_the_compositor_cadence_to_the_program() {
        // A glyph reveal or effect alone no longer lifts the window to 120 Hz.
        assert_eq!(
            super::window_present_demand(FrameDemand::OnDemand, true, true, rate(30)),
            fps(30)
        );
        // Nor does it outbid the program's own cadence under the cap.
        assert_eq!(
            super::window_present_demand(fps(30), true, true, rate(30)),
            fps(30)
        );
        assert_eq!(
            super::window_present_demand(fps(240), false, true, rate(60)),
            fps(60),
            "the program's own cadence is held to it too"
        );
        assert_eq!(
            super::window_present_demand(FrameDemand::OnDemand, true, true, rate(240)),
            fps(120),
            "a cap above the compositor cadence does not raise it"
        );
    }

    #[test]
    fn the_presentation_display_interval_follows_the_capped_cadence() {
        assert_eq!(
            super::presentation_display_interval(None),
            Duration::from_secs(1) / 120
        );
        assert_eq!(
            super::presentation_display_interval(rate(60)),
            Duration::from_secs(1) / 60
        );
        assert_eq!(
            super::presentation_display_interval(rate(240)),
            Duration::from_secs(1) / 120
        );
    }

    #[test]
    fn a_max_frame_rate_asks_for_no_frames_of_its_own() {
        let later = Instant::now() + Duration::from_millis(50);
        assert_eq!(
            super::window_present_demand(FrameDemand::OnDemand, false, true, rate(30)),
            FrameDemand::OnDemand
        );
        assert_eq!(
            super::window_present_demand(FrameDemand::At(later), false, true, rate(30)),
            FrameDemand::At(later),
            "a one-shot deadline is served when due"
        );
        assert_eq!(
            super::window_present_demand(FrameDemand::OnDemand, true, false, rate(30)),
            FrameDemand::OnDemand,
            "a hidden window's compositor still drives no ticks"
        );
    }

    /// Frames a window presents in one second when the loop polls it at
    /// 240 Hz and serves every due tick.
    fn presents_in_one_second(demand: FrameDemand) -> u32 {
        let mut schedule = FrameSchedule::default();
        let start = Instant::now();
        let step = Duration::from_nanos(1_000_000_000 / 240);
        let mut presents = 0;
        for index in 0u32..240 {
            let now = start + step * index;
            if frame_tick(true, schedule.due(demand, now), true) == FrameTick::Present {
                presents += 1;
                let _ = schedule.advance_served(demand, now);
            } else {
                let _ = schedule.arm(demand, now);
            }
        }
        presents
    }

    #[test]
    fn a_live_glyph_presentation_presents_at_the_capped_rate() {
        let uncapped = super::window_present_demand(FrameDemand::OnDemand, true, true, None);
        let capped = super::window_present_demand(FrameDemand::OnDemand, true, true, rate(30));
        let uncapped = presents_in_one_second(uncapped);
        let capped = presents_in_one_second(capped);
        assert!(
            (119..=121).contains(&uncapped),
            "uncapped compositor cadence is 120 Hz, got {uncapped}"
        );
        assert!(
            (29..=31).contains(&capped),
            "a 30 Hz cap presents 30 frames a second, got {capped}"
        );
    }

    #[test]
    fn compositor_clock_sync_during_wake_still_drains_pending_animation_events() {
        use nana_ui_runtime::{
            AnimatableProperty, AnimationEventKind, AnimationFillMode, AnimationId,
            AnimationPlayback, AnimationSpec, DocumentId, Easing, MotionInterrupt, MotionTo,
            MotionValue, MutationQueue, Stack,
        };
        use nana_ui_scene::RuntimeDocument;

        let epoch = Instant::now();
        let clock = crate::RuntimeAnimationClock::new(epoch);
        let doc_id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(doc_id);
        let backdrop = document
            .context_mut()
            .create_component(doc_id, Stack::fill_column(0.0))
            .unwrap();
        let target = backdrop.stable_id();
        let anim_id = AnimationId::for_node(target, AnimatableProperty::Opacity);

        let opacity_run = |from: f32, to: f32, start: Duration, duration: Duration| {
            AnimationSpec::new(
                anim_id,
                target,
                start,
                duration,
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(from),
                MotionTo::Value(MotionValue::Scalar(to)),
            )
            .with_interrupt(MotionInterrupt::Replace)
            .with_playback(AnimationPlayback {
                fill_mode: AnimationFillMode::Forwards,
                ..AnimationPlayback::default()
            })
        };

        // Start a 220 ms compositor fade at t = 0 and run its initial start
        // sample so its only remaining deadline is completion at t = 220 ms.
        let mut queue = MutationQueue::new();
        queue.start_animation(opacity_run(
            1.0,
            0.0,
            Duration::ZERO,
            Duration::from_millis(220),
        ));
        document.context_mut().commit_mutations(queue).unwrap();
        let _ = clock.wake(document.context_mut(), epoch);
        assert_eq!(
            clock.next_wakeup(document.context()),
            Some(epoch + Duration::from_millis(220))
        );

        // Cancelling the in-flight track (e.g. unmounting a background section
        // or interrupting a transition) leaves no track deadline, only a
        // pending `Cancelled` event whose deadline tracks `animation_now`.
        let cancel_at = Duration::from_millis(10);
        document.sync_presentation_clock(cancel_at);
        let mut queue = MutationQueue::new();
        queue.stop_animation(anim_id);
        document.context_mut().commit_mutations(queue).unwrap();

        let wait_entered = epoch + cancel_at;
        assert!(super::animation_due(clock, &document, wait_entered));

        // `wake` syncs the compositor clock to wall time before checking due
        // animations; advancing past `wait_entered` moves a pending event's
        // reported deadline with it, so `wake` must evaluate against the
        // post-sync instant rather than the stale `about_to_wait` timestamp.
        let sync_at = wait_entered + Duration::from_micros(5);
        document.sync_presentation_clock(clock.runtime_time(sync_at));
        assert!(!super::animation_due(clock, &document, wait_entered));

        let animation_now = sync_at + Duration::from_micros(2);
        assert!(super::animation_due(clock, &document, animation_now));
        let frame = clock.wake(document.context_mut(), animation_now);
        assert_eq!(frame.events.len(), 1);
        assert_eq!(frame.events[0].kind, AnimationEventKind::Cancelled);
        assert_eq!(
            clock.next_wakeup(document.context()),
            None,
            "once pending events drain, an idle document schedules no wakeup"
        );
    }

    #[test]
    fn dual_window_compositor_wakes_only_active_window_and_recovers_single_surface() {
        use nana_ui_platform::WindowId;
        use nana_ui_runtime::{
            AnimatableProperty, AnimationFillMode, AnimationId, AnimationPlayback, AnimationSpec,
            DocumentId, Easing, LayoutViewport, MeasureTextShaper, MotionTo, MotionValue,
            MutationQueue, Stack,
        };
        use nana_ui_scene::RuntimeDocument;

        let epoch = Instant::now();
        let clock = crate::RuntimeAnimationClock::new(epoch);
        let viewport = LayoutViewport::new(320.0, 240.0);

        let doc1_id = DocumentId::new(1).unwrap();
        let doc2_id = DocumentId::new(2).unwrap();
        let mut doc1 = RuntimeDocument::new(doc1_id);
        let mut doc2 = RuntimeDocument::new(doc2_id);

        let build_node = |doc: &mut RuntimeDocument,
                          doc_id: DocumentId,
                          anim_dur: Duration|
         -> (nana_ui_runtime::StableNodeId, AnimationId) {
            let entity = doc
                .context_mut()
                .create_component(doc_id, Stack::fill_column(0.0))
                .unwrap();
            let node = entity.stable_id();
            let anim_id = AnimationId::for_node(node, AnimatableProperty::Opacity);
            let mut queue = MutationQueue::new();
            queue.start_animation(
                AnimationSpec::new(
                    anim_id,
                    node,
                    Duration::ZERO,
                    anim_dur,
                    Duration::from_millis(16),
                    Easing::Linear,
                )
                .with_property(AnimatableProperty::Opacity)
                .with_range(
                    MotionValue::Scalar(1.0),
                    MotionTo::Value(MotionValue::Scalar(0.3)),
                )
                .with_playback(AnimationPlayback {
                    fill_mode: AnimationFillMode::Forwards,
                    ..AnimationPlayback::default()
                }),
            );
            doc.context_mut().commit_mutations(queue).unwrap();
            (node, anim_id)
        };

        let (node1, anim1) = build_node(&mut doc1, doc1_id, Duration::from_millis(800));
        let (node2, anim2) = build_node(&mut doc2, doc2_id, Duration::from_millis(10));

        // Promote compositor layers on both windows past LAYER_PROMOTE_HOLD (16ms),
        // then advance past doc2's completion + demotion hold so doc2 holds its
        // finished layer idle (`compositor_needs_tick() == false`) while doc1
        // stays actively animating.
        for ms in [0u64, 20, 160] {
            let runtime_now = Duration::from_millis(ms);
            for doc in [&mut doc1, &mut doc2] {
                doc.sync_presentation_clock(runtime_now);
                doc.context_mut().advance_animations(runtime_now);
                doc.flush(viewport, &mut MeasureTextShaper).unwrap();
            }
        }

        assert!(doc1.compositor_needs_tick());
        assert!(!doc2.compositor_needs_tick());
        assert!(doc1.scene().compositor_layer(node1).is_some());
        assert!(doc2.scene().compositor_layer(node2).is_some());
        assert_ne!(doc1.scene().compositor_gpu_motion_ids(node1), (0, 0));

        // Part (a): Drive host frame demand across 10 steps at 120 Hz. Only the
        // active compositor window wakes and redraws; the idle neighbor stays
        // `OnDemand` with 0 redraws.
        let mut sched1 = FrameSchedule::default();
        let mut sched2 = FrameSchedule::default();
        let mut redraws1 = 0u32;
        let mut redraws2 = 0u32;
        let step = Duration::from_secs(1) / 120;
        let sim_start = epoch + Duration::from_millis(160);
        for index in 0u32..10 {
            let now = sim_start + step * index;
            for (doc, sched, redraws) in [
                (&mut doc1, &mut sched1, &mut redraws1),
                (&mut doc2, &mut sched2, &mut redraws2),
            ] {
                doc.sync_presentation_clock(clock.runtime_time(now));
                let demand = super::window_present_demand(
                    FrameDemand::OnDemand,
                    doc.compositor_needs_tick(),
                    true,
                    None,
                );
                let tick = frame_tick(true, sched.due(demand, now), true);
                if tick == FrameTick::Present {
                    *redraws += 1;
                    let _ = sched.advance_served(demand, now);
                } else {
                    let _ = sched.arm(demand, now);
                }
            }
            assert!(!super::animation_due(clock, &doc2, now));
        }
        assert!(
            redraws1 >= 8,
            "active compositor window should redraw at ~120Hz, got {redraws1}"
        );
        assert_eq!(
            redraws2, 0,
            "idle neighbor window must stay OnDemand with 0 redraws"
        );
        assert_eq!(
            super::window_present_demand(
                FrameDemand::OnDemand,
                doc2.compositor_needs_tick(),
                true,
                None
            ),
            FrameDemand::OnDemand
        );

        // Part (b): Single-window surface recovery bumps ONLY window 1's
        // `surface_generation` and preserves valid handles without touching
        // window 2's scene, compositor layer, or presentation epoch.
        let win1 = WindowId::PRIMARY;
        let win2 = WindowId(2);
        let mut surf1 = 10u32;
        let mut surf2 = 20u32;
        let recover_at = sim_start + Duration::from_millis(100);
        let mut retry1 = Some(recover_at);
        let mut retry2 = None;
        let mut gen1 = 0u64;
        let mut gen2 = 0u64;
        let doc2_epoch_before = doc2.scene().presentation_epoch();
        let handle1_before = doc1
            .context()
            .world()
            .motion_descriptors()
            .handle_for(anim1.track_id())
            .expect("live descriptor handle for active window");
        assert!(!handle1_before.is_null());
        let handle2_before = doc2
            .context()
            .world()
            .motion_descriptors()
            .handle_for(anim2.track_id());

        let recovered = {
            let mut windows = [
                (win1, &mut surf1, &mut retry1, &mut gen1, &mut doc1),
                (win2, &mut surf2, &mut retry2, &mut gen2, &mut doc2),
            ];
            super::super::present::recover_window_surfaces(
                &mut windows,
                recover_at,
                |id, surface| {
                    assert_eq!(id, win1, "only suspended window should be rebound");
                    *surface += 1;
                    Ok::<_, ()>(())
                },
            )
        };

        assert_eq!(recovered, vec![win1]);
        assert_eq!(surf1, 11);
        assert_eq!(surf2, 20);
        assert_eq!(gen1, 1);
        assert_eq!(gen2, 0);
        assert_eq!(doc1.scene().surface_generation(), 1);
        assert_eq!(
            doc1.scene()
                .compositor_layer(node1)
                .unwrap()
                .surface_generation,
            1
        );
        assert_eq!(doc2.scene().surface_generation(), 0);
        assert_eq!(
            doc2.scene()
                .compositor_layer(node2)
                .unwrap()
                .surface_generation,
            0
        );
        assert_eq!(doc2.scene().presentation_epoch(), doc2_epoch_before);
        assert_eq!(
            doc2.context()
                .world()
                .motion_descriptors()
                .handle_for(anim2.track_id()),
            handle2_before
        );

        // Recovered window continues evaluating with valid live MotionHandle
        // and non-zero GPU motion IDs at surface_generation == 1.
        let post_recovery = Duration::from_millis(260);
        doc1.sync_presentation_clock(post_recovery);
        doc1.context_mut().advance_animations(post_recovery);
        doc1.flush(viewport, &mut MeasureTextShaper).unwrap();
        let handle1_after = doc1
            .context()
            .world()
            .motion_descriptors()
            .handle_for(anim1.track_id())
            .expect("recovered window retains live descriptor handle");
        assert_eq!(handle1_after, handle1_before);
        assert!(!handle1_after.is_null());
        assert!(
            doc1.context()
                .world()
                .motion_descriptors()
                .evaluate(handle1_after, post_recovery)
                .is_some()
        );
        assert_ne!(doc1.scene().compositor_gpu_motion_ids(node1), (0, 0));
        assert_eq!(
            doc1.scene()
                .compositor_layer(node1)
                .unwrap()
                .surface_generation,
            1
        );

        // Device-wide replacement bumps every window independently.
        let next1 = super::advance_surface_generation(&mut gen1, doc1.scene().surface_generation());
        let next2 = super::advance_surface_generation(&mut gen2, doc2.scene().surface_generation());
        doc1.set_surface_generation(next1);
        doc2.set_surface_generation(next2);
        assert_eq!(doc1.scene().surface_generation(), 2);
        assert_eq!(doc2.scene().surface_generation(), 1);
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
