//! Adapter for an existing winit event loop (`EmbeddedRuntime`).

use super::*;

/// Adapter for an existing winit event loop. Call from the host's window thread.
/// This adapter never creates or exits an event loop and shares the supplied GPU.
pub struct EmbeddedRuntime<Program: RuntimeProgram> {
    manager: WindowManager<Program>,
}

impl<Program: RuntimeProgram> EmbeddedRuntime<Program> {
    pub fn new(
        event_loop: &dyn ActiveEventLoop,
        proxy: EventLoopProxy,
        graphics: crate::HostedGpuShared,
        settings: WindowDescriptor,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        window_factory::initialize(
            event_loop,
            proxy,
            tx,
            rx,
            settings,
            Arc::new(Mutex::new(None)),
            graphics,
        )
        .map(|manager| Self { manager })
    }

    pub fn windows(&self) -> &crate::WindowService {
        &self.manager.windows
    }

    /// Forward the embedding host's device-loss notification on the window thread
    /// before forwarding further window events. This does not exit the host loop
    /// or install/replace the host's device callback.
    pub fn notify_device_lost(&mut self) {
        // Embedded devices never raise the hosted device-lost flag; the
        // embedder tells us here instead. Recording it on the context lets
        // everything holding it (producer threads) see the loss, and writes
        // the `gpu.device_lost` fault and snapshot (once per device); taking
        // the report right away marks it handled.
        let graphics = &self.manager.graphics;
        nana_gpu::__framework::mark_lost(
            graphics.gpu(),
            nana_gpu::GpuDeviceLost {
                reason: nana_gpu::GpuLossReason::Unknown,
                message: "reported by the embedding host".into(),
            },
        );
        let _ = graphics.take_device_lost_report();
        self.manager.render_suspended = true;
        self.manager.next_gpu_retry = None;
    }

    pub fn needs_gpu_replacement(&self) -> bool {
        self.manager.render_suspended
    }

    /// Called by the embedding host after it replaces its device.
    ///
    /// Always adopts `graphics`: surfaces are rebound in place, and DXGI's one
    /// swap chain per HWND leaves no old surface to fall back to. Windows whose
    /// rebind failed recover individually; the error is returned only when
    /// every window failed.
    pub fn replace_gpu(&mut self, graphics: crate::HostedGpuShared) -> Result<(), String> {
        let outcomes: Vec<_> = self
            .manager
            .window_contexts
            .iter_mut()
            .map(|(&id, host)| (id, graphics.recreate_surface(&mut host.surface)))
            .collect();
        let all_failed = outcomes.iter().all(|(_, outcome)| outcome.is_err());
        let error = outcomes
            .first()
            .and_then(|(_, outcome)| outcome.as_ref().err())
            .filter(|_| all_failed)
            .map(ToString::to_string);
        self.manager.switch_gpu(graphics, outcomes);
        error.map_or(Ok(()), Err)
    }

    /// Displays connected now.
    pub fn displays(&self, event_loop: &dyn ActiveEventLoop) -> Vec<nana_ui_platform::DisplayInfo> {
        display::display_infos(event_loop)
    }

    /// Create a window synchronously on the host's window thread.
    /// Resolves exactly like `WindowService::create_window`, without a queue round trip.
    pub fn create_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        descriptor: WindowDescriptor,
    ) -> Result<crate::WindowHandle, crate::WindowError> {
        crate::window_service::validate_descriptor(&descriptor)?;
        self.manager.create_service_window(event_loop, descriptor)
    }

    pub fn is_empty(&self) -> bool {
        self.manager.window_contexts.is_empty()
    }

    /// Returns false for windows owned by another component of the host.
    pub fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        native_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) -> bool {
        let Some(id) = self.manager.window_ids.get(&native_id).copied() else {
            return false;
        };
        self.manager.handle_window_event(event_loop, id, event);
        true
    }

    pub fn wake(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.manager.proxy_wake_up(event_loop);
    }

    /// Refresh cached display HDR/headroom metadata after the embedding host
    /// receives a system display-parameter notification. This is an
    /// event-driven update: it only requests redraws for surfaces whose
    /// metadata changed and never rebuilds application or Scene state.
    pub fn refresh_display_hdr_info(&mut self) -> bool {
        let ids: Vec<_> = self.manager.window_contexts.keys().copied().collect();
        // Every window refreshes; none is skipped once one changed.
        let mut changed = false;
        for id in ids {
            changed |= self.manager.refresh_surface_hdr_info(id);
        }
        changed
    }

    /// Returns NanaUI's next deadline for the host to merge with its own timers.
    pub fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) -> Option<Instant> {
        self.manager.about_to_wait(event_loop);
        self.manager.wake_deadline
    }
}
