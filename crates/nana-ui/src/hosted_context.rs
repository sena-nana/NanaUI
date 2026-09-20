//! Shared WGPU context for NanaUI hosted applications.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

static NEXT_DEVICE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Native presentation mechanism, selected before creating the window surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostedSurfaceMode {
    #[default]
    Window,
    #[cfg(target_os = "windows")]
    WindowsComposition,
}

#[derive(Clone)]
enum HostedSurfaceTarget {
    Window,
    #[cfg(target_os = "windows")]
    WindowsComposition(crate::WindowsComposition),
}

impl HostedSurfaceTarget {
    fn new(
        mode: HostedSurfaceMode,
        _window: Arc<dyn winit::window::Window>,
    ) -> Result<Self, HostedGpuError> {
        match mode {
            HostedSurfaceMode::Window => Ok(Self::Window),
            #[cfg(target_os = "windows")]
            HostedSurfaceMode::WindowsComposition => crate::WindowsComposition::new(_window)
                .map(Self::WindowsComposition)
                .map_err(|e| HostedGpuError::SurfaceCreation(e.to_string())),
        }
    }

    fn mode(&self) -> HostedSurfaceMode {
        match self {
            Self::Window => HostedSurfaceMode::Window,
            #[cfg(target_os = "windows")]
            Self::WindowsComposition(_) => HostedSurfaceMode::WindowsComposition,
        }
    }

    fn create_surface(
        &self,
        instance: &wgpu::Instance,
        window: Arc<dyn winit::window::Window>,
    ) -> Result<wgpu::Surface<'static>, HostedGpuError> {
        let result = match self {
            Self::Window => instance.create_surface(window),
            #[cfg(target_os = "windows")]
            Self::WindowsComposition(composition) => {
                // The tree retains the COM visual and HWND; WGPU adds its own visual reference.
                unsafe {
                    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(
                        composition.ui_visual(),
                    ))
                }
            }
        };
        result.map_err(|e| HostedGpuError::SurfaceCreation(e.to_string()))
    }

    fn commit(&self) -> Result<(), HostedGpuError> {
        match self {
            Self::Window => Ok(()),
            #[cfg(target_os = "windows")]
            // The swapchain WGPU just bound to the UI visual is staged on the
            // composition device, not on the tree, so this commit is not
            // optional the way a frame's is.
            Self::WindowsComposition(composition) => composition
                .commit_external()
                .map_err(|e| HostedGpuError::SurfaceCreation(e.to_string())),
        }
    }
}

/// One GPU bootstrap: the instance a capability was probed on, kept so the real
/// device request does not initialise the backend a second time.
///
/// The composition question has to be answered before any window exists —
/// `WS_EX_NOREDIRECTIONBITMAP` is a creation-time flag winit owns and nothing
/// can set durably afterwards — but answering it means creating an instance and
/// enumerating adapters, which is most of what selecting the real device does.
/// Doing both on one instance is what keeps the probe and the device from
/// disagreeing, and from paying for DX12 initialisation twice.
pub(crate) struct GpuBootstrap {
    /// Whether a window in this process can present through a platform
    /// compositor. It answers only that: what a client's alpha then turns out
    /// to be is the surface's answer once it negotiates, and the plain path is
    /// not automatically opaque — a Vulkan surface negotiates `PreMultiplied`
    /// on it.
    composition: bool,
    /// The instance the probe proved that adapter on, for the device request to
    /// reuse. `None` when nothing was probed, or when an embedder's device is
    /// the one that will be used and brought its own instance.
    instance: Option<wgpu::Instance>,
}

impl GpuBootstrap {
    /// A bootstrap that was not asked about composition, and so probed nothing.
    pub(crate) const fn plain() -> Self {
        Self {
            composition: false,
            instance: None,
        }
    }

    /// Probes composition capability once, keeping the instance it probed on.
    ///
    /// `host_backend` is the backend of a GPU context an embedder already built.
    /// There is nothing to narrow there — the device exists — so no instance is
    /// created, and composition is only on the table when that context is
    /// already DX12.
    pub(crate) fn probe(host_backend: Option<wgpu::Backend>) -> Self {
        if let Some(backend) = host_backend {
            let _ = backend;
            #[cfg(target_os = "windows")]
            return Self {
                composition: backend == wgpu::Backend::Dx12,
                instance: None,
            };
            #[cfg(not(target_os = "windows"))]
            return Self::plain();
        }
        #[cfg(target_os = "windows")]
        {
            // `WGPU_BACKEND` still wins: a run pinned to another backend has no
            // composition available to it.
            let backends = wgpu::Backends::from_env().unwrap_or_default() & wgpu::Backends::DX12;
            if backends.is_empty() {
                return Self::plain();
            }
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            if pollster::block_on(instance.enumerate_adapters(backends)).is_empty() {
                return Self::plain();
            }
            Self {
                composition: true,
                // Kept: selecting the device is the other half of what this
                // enumeration already did.
                instance: Some(instance),
            }
        }
        #[cfg(not(target_os = "windows"))]
        Self::plain()
    }

    pub(crate) const fn composition_available(&self) -> bool {
        self.composition
    }

    /// The probe's instance, for the device request to reuse. Taken, not
    /// cloned: there is one probed instance per process and the device
    /// selection is its one consumer.
    pub(crate) fn take_instance(&mut self) -> Option<wgpu::Instance> {
        self.instance.take()
    }
}

fn surface_alpha(
    mode: HostedSurfaceMode,
    modes: &[wgpu::CompositeAlphaMode],
    transparent: bool,
) -> Result<wgpu::CompositeAlphaMode, HostedGpuError> {
    match mode {
        HostedSurfaceMode::Window => Ok(preferred_alpha_mode(modes, transparent)),
        #[cfg(target_os = "windows")]
        HostedSurfaceMode::WindowsComposition => {
            if modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                Ok(wgpu::CompositeAlphaMode::PreMultiplied)
            } else {
                Err(HostedGpuError::SurfaceCreation(
                    "DirectComposition requires premultiplied alpha".into(),
                ))
            }
        }
    }
}

/// Cloneable access to the host's only device and queue pair.
#[derive(Clone)]
pub struct HostedGpuResources {
    generation: u64,
    adapter: wgpu::Adapter,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    // Shared so per-frame context clones do not copy the adapter's strings.
    adapter_info: Arc<wgpu::AdapterInfo>,
    /// Serializes off-thread `Queue` work against `Surface::configure`.
    /// wgpu waits for GPU idle before recreating a configured swapchain; a
    /// concurrent submit from another thread panics with `GpuWaitTimeout`.
    submit: Arc<RwLock<()>>,
}

impl HostedGpuResources {
    /// Wrap an application-created adapter/device/queue as NanaUI's single
    /// hosted GPU context. This does not request or duplicate any resource.
    pub fn from_existing(
        adapter: wgpu::Adapter,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Self {
        Self::from_parts(
            NEXT_DEVICE_GENERATION.fetch_add(1, Ordering::Relaxed),
            adapter,
            device,
            queue,
        )
    }

    fn from_parts(
        generation: u64,
        adapter: wgpu::Adapter,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Self {
        let adapter_info = Arc::new(adapter.get_info());
        Self {
            generation,
            adapter,
            device,
            queue,
            adapter_info,
            submit: Arc::new(RwLock::new(())),
        }
    }

    /// Monotonic identity of this host GPU context. Clones share the same
    /// generation; successful device recreation gets a fresh generation.
    /// Applications can fence their device-dependent caches in `rebuild_gpu`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// Off-thread producers hold a read lock around `queue.submit` /
    /// `write_texture` / `write_buffer`, and drop it before `poll(Wait)`.
    /// The UI thread must not take a read lock: it configures on this thread
    /// and `RwLock` is not reentrant.
    pub fn submit_lock(&self) -> Arc<RwLock<()>> {
        Arc::clone(&self.submit)
    }

    fn lock_reconfigure(&self) -> RwLockWriteGuard<'_, ()> {
        self.submit
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// One window and surface attached to a shared hosted GPU context.
pub struct HostedGpuSurface {
    surface: wgpu::Surface<'static>,
    target: HostedSurfaceTarget,
    window: Arc<dyn winit::window::Window>,
    needs_target_commit: bool,
    needs_recovery: bool,
    /// A suboptimal frame is still presented; the surface cannot be configured
    /// while that frame is alive, so reconfiguration waits for the next acquire.
    needs_reconfigure: bool,
    format: wgpu::TextureFormat,
    configuration: wgpu::SurfaceConfiguration,
    want_transparent: bool,
    /// Whether this surface spent its one alpha-mode rebuild. Never cleared:
    /// `rebind` reads the alpha mode off the incoming surface instead.
    alpha_recreate_attempted: bool,
    /// Live-resize present mode resolved from this surface's capabilities.
    /// Present-mode support is fixed per surface and adapter, so it is
    /// resolved at surface creation instead of re-queried every live frame.
    live_present_mode: wgpu::PresentMode,
}

impl HostedGpuSurface {
    #[cfg(target_os = "windows")]
    pub fn windows_composition(&self) -> Option<&crate::WindowsComposition> {
        match &self.target {
            HostedSurfaceTarget::WindowsComposition(composition) => Some(composition),
            HostedSurfaceTarget::Window => None,
        }
    }

    fn commit_target(&mut self) -> Result<(), HostedGpuError> {
        if self.needs_target_commit {
            self.target.commit()?;
            self.needs_target_commit = false;
        }
        Ok(())
    }

    pub(crate) fn window(&self) -> &Arc<dyn winit::window::Window> {
        &self.window
    }

    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Alpha composition mode selected from the native surface capabilities.
    pub const fn alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.configuration.alpha_mode
    }

    pub fn physical_size(&self) -> (u32, u32) {
        let size = self.window.surface_size();
        (size.width, size.height)
    }

    pub fn is_drawable(&self) -> bool {
        let size = self.window.surface_size();
        size.width > 0 && size.height > 0
    }

    pub fn resize(&mut self, resources: &HostedGpuResources) {
        let size = self.window.surface_size();
        if !surface_size_changed(
            (self.configuration.width, self.configuration.height),
            (size.width, size.height),
        ) {
            return;
        }
        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.reconfigure(resources);
    }

    /// Apply size and live-resize present policy, then configure at most once.
    pub fn prepare_frame(&mut self, resources: &HostedGpuResources, live: bool) {
        let size = self.window.surface_size();
        let mut changed = self.apply_live_resize_policy(live);
        if surface_size_changed(
            (self.configuration.width, self.configuration.height),
            (size.width, size.height),
        ) {
            self.configuration.width = size.width;
            self.configuration.height = size.height;
            changed = true;
        }
        if changed {
            self.reconfigure(resources);
        }
    }

    fn apply_live_resize_policy(&mut self, live: bool) -> bool {
        let Some((present_mode, desired_maximum_frame_latency)) = live_resize_policy_change(
            self.configuration.present_mode,
            self.configuration.desired_maximum_frame_latency,
            self.live_present_mode,
            live,
        ) else {
            return false;
        };
        self.configuration.present_mode = present_mode;
        self.configuration.desired_maximum_frame_latency = desired_maximum_frame_latency;
        self.configuration.width > 0 && self.configuration.height > 0
    }

    fn reconfigure(&mut self, resources: &HostedGpuResources) {
        let _gate = resources.lock_reconfigure();
        self.surface
            .configure(resources.device(), &self.configuration);
        self.needs_target_commit = true;
        self.needs_reconfigure = false;
    }

    fn apply_alpha_mode(
        &mut self,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        resources: &HostedGpuResources,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        let capabilities = self.surface.get_capabilities(adapter);
        if self.target.mode() == HostedSurfaceMode::Window
            && alpha_mode_needs_surface_recreate(
                want_transparent,
                &capabilities.alpha_modes,
                self.alpha_recreate_attempted,
            )
        {
            self.alpha_recreate_attempted = true;
            return self.recover_with_alpha(instance, adapter, resources, want_transparent);
        }
        let alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            want_transparent,
        )?;
        if self.configuration.alpha_mode != alpha_mode {
            self.configuration.alpha_mode = alpha_mode;
            self.reconfigure(resources);
        }
        self.want_transparent = want_transparent;
        Ok(())
    }

    /// Move this surface onto another device, in place.
    ///
    /// DXGI allows one swap chain per HWND and wgpu creates it at `configure`,
    /// so the old surface is dropped before the new one is configured; the
    /// other order fails with `Invalid surface`. Every fallible step runs
    /// before the swap, so an error leaves this surface untouched. The target
    /// commit is left to the next `acquire_frame`.
    fn rebind(
        &mut self,
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        resources: &HostedGpuResources,
    ) -> Result<(), HostedGpuError> {
        let capabilities = surface.get_capabilities(adapter);
        let format = preferred_surface_format(&capabilities.formats)
            .ok_or(HostedGpuError::SurfaceHasNoFormats)?;
        let alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            self.want_transparent,
        )?;
        self.surface = surface;
        let size = self.window.surface_size();
        self.format = format;
        self.live_present_mode = preferred_live_present_mode(&capabilities.present_modes);
        self.configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Srgb,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: self.live_present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        self.needs_recovery = false;
        // `alpha_recreate_attempted` survives: the alpha mode above already
        // comes from the incoming surface, and re-granting it would re-arm the
        // rebuild on every `retry_surfaces` pass.
        self.reconfigure(resources);
        Ok(())
    }

    fn recover(
        &mut self,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        resources: &HostedGpuResources,
    ) -> Result<(), HostedGpuError> {
        self.recover_with_alpha(instance, adapter, resources, self.want_transparent)
    }

    fn recover_with_alpha(
        &mut self,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        resources: &HostedGpuResources,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        let surface = self.target.create_surface(instance, self.window.clone())?;
        let capabilities = surface.get_capabilities(adapter);
        if !capabilities.formats.contains(&self.format) {
            return Err(HostedGpuError::SurfaceFormatChanged {
                expected: self.format,
            });
        }
        self.configuration.alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            want_transparent,
        )?;
        self.live_present_mode = preferred_live_present_mode(&capabilities.present_modes);
        self.configuration.present_mode = self.live_present_mode;
        self.surface = surface;
        self.reconfigure(resources);
        self.commit_target()?;
        self.want_transparent = want_transparent;
        self.needs_recovery = false;
        Ok(())
    }

    fn acquire_frame(
        &mut self,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        resources: &HostedGpuResources,
    ) -> Result<HostedSurfaceFrame, HostedGpuError> {
        if !self.is_drawable() {
            return Ok(HostedSurfaceFrame::Skipped);
        }
        if self.needs_recovery {
            self.recover(instance, adapter, resources)?;
        }
        if self.needs_reconfigure {
            self.reconfigure(resources);
        }
        self.commit_target()?;
        let result = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Ok(HostedSurfaceFrame::Ready(frame)),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.needs_reconfigure = true;
                Ok(HostedSurfaceFrame::Ready(frame))
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                // A stale swapchain is the common resize race. Reconfigure and
                // retry once in the same frame so a resize step does not drop
                // its redraw to the next event-loop iteration.
                self.reconfigure(resources);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => {
                        Ok(HostedSurfaceFrame::Ready(frame))
                    }
                    wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        self.needs_reconfigure = true;
                        Ok(HostedSurfaceFrame::Ready(frame))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        Err(HostedGpuError::SurfaceValidation)
                    }
                    _ => Ok(HostedSurfaceFrame::Retry),
                }
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recover(instance, adapter, resources)?;
                Ok(HostedSurfaceFrame::Retry)
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(HostedSurfaceFrame::Skipped)
            }
            wgpu::CurrentSurfaceTexture::Validation => Err(HostedGpuError::SurfaceValidation),
        };
        self.commit_target()?;
        result
    }
}

/// GPU bootstrap for advanced native hosts. `into_parts` separates the first
/// surface from the shared device; the Scene host manages every surface equally.
pub struct HostedGpuContext {
    shared: HostedGpuShared,
    primary: HostedGpuSurface,
}

/// Shared device resources without ownership of any native window.
#[derive(Clone)]
pub struct HostedGpuShared {
    instance: wgpu::Instance,
    resources: HostedGpuResources,
    device_lost: Arc<AtomicBool>,
    device_lost_report: Arc<Mutex<Option<HostedDeviceLost>>>,
}

impl std::ops::Deref for HostedGpuContext {
    type Target = HostedGpuShared;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedDeviceLost {
    pub reason: String,
    pub message: String,
}

impl HostedGpuContext {
    pub fn into_parts(self) -> (HostedGpuShared, HostedGpuSurface) {
        (self.shared, self.primary)
    }

    pub async fn new(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
    ) -> Result<Self, HostedGpuError> {
        Self::new_with_surface_mode(
            window,
            required_features,
            want_transparent,
            HostedSurfaceMode::Window,
        )
        .await
    }

    pub async fn new_with_surface_mode(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        mode: HostedSurfaceMode,
    ) -> Result<Self, HostedGpuError> {
        let target = HostedSurfaceTarget::new(mode, window.clone())?;
        Self::new_with_target(window, required_features, want_transparent, target).await
    }

    /// Rebuild GPU resources while retaining the primary native visual tree.
    /// Move this context onto a new device, rebinding the primary surface in
    /// place (see [`HostedGpuSurface::rebind`]). Other surfaces follow with
    /// [`HostedGpuShared::recreate_surface`]. An error leaves the context
    /// unchanged.
    pub async fn recreate(
        &mut self,
        required_features: wgpu::Features,
    ) -> Result<(), HostedGpuError> {
        let (shared, surface, _, _) = Self::acquire_device(
            self.primary.window.clone(),
            required_features,
            &self.primary.target,
        )
        .await?;
        self.primary
            .rebind(surface, shared.resources.adapter(), &shared.resources)?;
        self.shared = shared;
        Ok(())
    }

    async fn new_with_target(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        target: HostedSurfaceTarget,
    ) -> Result<Self, HostedGpuError> {
        Self::new_with_target_on(window, required_features, want_transparent, target, None).await
    }

    /// As `new_with_target`, reusing `instance` when the caller already built
    /// one to answer a capability question. See [`GpuBootstrap`].
    pub(crate) async fn new_with_bootstrap(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        mode: HostedSurfaceMode,
        instance: Option<wgpu::Instance>,
    ) -> Result<Self, HostedGpuError> {
        let target = HostedSurfaceTarget::new(mode, window.clone())?;
        Self::new_with_target_on(
            window,
            required_features,
            want_transparent,
            target,
            instance,
        )
        .await
    }

    async fn new_with_target_on(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        target: HostedSurfaceTarget,
        instance: Option<wgpu::Instance>,
    ) -> Result<Self, HostedGpuError> {
        let (shared, surface, capabilities, format) =
            Self::acquire_device_on(window.clone(), required_features, &target, instance).await?;
        let primary = configure_surface(
            window,
            surface,
            format,
            &capabilities,
            &shared.resources,
            want_transparent,
            target,
        )?;
        Ok(Self { shared, primary })
    }

    /// A new device for `target`, plus its not-yet-configured surface.
    async fn acquire_device(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        target: &HostedSurfaceTarget,
    ) -> Result<
        (
            HostedGpuShared,
            wgpu::Surface<'static>,
            wgpu::SurfaceCapabilities,
            wgpu::TextureFormat,
        ),
        HostedGpuError,
    > {
        Self::acquire_device_on(window, required_features, target, None).await
    }

    /// `instance` is a bootstrap's, when one already narrowed the backends and
    /// enumerated adapters to answer a capability question. Building a second
    /// instance for the same backend is initialisation done twice, and leaves
    /// the probe and the device free to disagree.
    async fn acquire_device_on(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        target: &HostedSurfaceTarget,
        instance: Option<wgpu::Instance>,
    ) -> Result<
        (
            HostedGpuShared,
            wgpu::Surface<'static>,
            wgpu::SurfaceCapabilities,
            wgpu::TextureFormat,
        ),
        HostedGpuError,
    > {
        #[allow(unused_mut)]
        let mut backends = wgpu::Backends::from_env().unwrap_or_default();
        #[cfg(target_os = "windows")]
        if target.mode() == HostedSurfaceMode::WindowsComposition {
            backends &= wgpu::Backends::DX12;
            if backends.is_empty() {
                return Err(HostedGpuError::Adapter(
                    "DirectComposition requires the DX12 backend".into(),
                ));
            }
        }
        let instance = instance.unwrap_or_else(|| {
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            })
        });
        let surface = target.create_surface(&instance, window.clone())?;
        let adapter = wgpu::util::initialize_adapter_from_env_or_default(&instance, Some(&surface))
            .await
            .map_err(|error| HostedGpuError::Adapter(error.to_string()))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = preferred_surface_format(&capabilities.formats)
            .ok_or(HostedGpuError::SurfaceHasNoFormats)?;
        let required_features = adapter.features() & required_features;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("NanaUI hosted shared device"),
                required_features,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .map_err(|error| HostedGpuError::Device(error.to_string()))?;
        let device_lost = Arc::new(AtomicBool::new(false));
        let device_lost_callback = Arc::clone(&device_lost);
        let device_lost_report = Arc::new(Mutex::new(None));
        let device_lost_report_callback = Arc::clone(&device_lost_report);
        device.set_device_lost_callback(move |reason, message| {
            device_lost_callback.store(true, Ordering::Release);
            if let Ok(mut report) = device_lost_report_callback.lock() {
                *report = Some(HostedDeviceLost {
                    reason: format!("{reason:?}"),
                    message,
                });
            }
        });
        let resources = HostedGpuResources::from_parts(
            NEXT_DEVICE_GENERATION.fetch_add(1, Ordering::Relaxed),
            adapter,
            Arc::new(device),
            Arc::new(queue),
        );
        let shared = HostedGpuShared {
            instance,
            resources,
            device_lost,
            device_lost_report,
        };
        Ok((shared, surface, capabilities, format))
    }

    #[cfg(target_os = "windows")]
    pub fn windows_composition(&self) -> Option<&crate::WindowsComposition> {
        self.primary.windows_composition()
    }

    pub const fn format(&self) -> wgpu::TextureFormat {
        self.primary.format()
    }

    /// Alpha composition mode used by the primary native surface.
    pub const fn alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.primary.alpha_mode()
    }

    pub fn physical_size(&self) -> (u32, u32) {
        self.primary.physical_size()
    }

    pub fn resize(&mut self) {
        self.primary.resize(&self.shared.resources);
    }

    pub fn prepare_frame(&mut self, live: bool) {
        self.primary.prepare_frame(&self.shared.resources, live);
    }

    pub fn reconfigure(&mut self) {
        self.primary.reconfigure(&self.shared.resources);
    }

    pub fn recover_surface(&mut self) -> Result<(), HostedGpuError> {
        self.primary.recover(
            &self.shared.instance,
            self.shared.resources.adapter(),
            &self.shared.resources,
        )
    }

    pub fn apply_alpha_mode(&mut self, want_transparent: bool) -> Result<(), HostedGpuError> {
        self.primary.apply_alpha_mode(
            &self.shared.instance,
            self.shared.resources.adapter(),
            &self.shared.resources,
            want_transparent,
        )
    }

    pub fn is_drawable(&self) -> bool {
        self.primary.is_drawable()
    }

    pub fn acquire_frame(&mut self) -> Result<HostedSurfaceFrame, HostedGpuError> {
        self.primary.acquire_frame(
            &self.shared.instance,
            self.shared.resources.adapter(),
            &self.shared.resources,
        )
    }

    /// Abandon an acquired primary frame after encoding fails. Drop all views
    /// and unfinished encoders referencing it before calling this method.
    /// The next acquisition recreates only this surface: on DX12, dropping a
    /// frame does not restore the consumed frame-latency waitable signal.
    pub fn discard_frame(&mut self, frame: wgpu::SurfaceTexture) {
        drop(frame);
        self.primary.needs_recovery = true;
    }
}

impl HostedGpuShared {
    /// Adopt an existing host device without requesting another adapter/device.
    /// Device-loss notification and replacement remain the embedding host's responsibility.
    pub fn from_device(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Self {
        Self {
            instance,
            resources: HostedGpuResources::from_parts(
                NEXT_DEVICE_GENERATION.fetch_add(1, Ordering::Relaxed),
                adapter,
                device,
                queue,
            ),
            device_lost: Arc::new(AtomicBool::new(false)),
            device_lost_report: Arc::new(Mutex::new(None)),
        }
    }

    /// A replacement device seeded by `surface`, which is rebound onto it.
    pub(crate) async fn rebuild_for_surface(
        surface: &mut HostedGpuSurface,
    ) -> Result<Self, HostedGpuError> {
        let (shared, raw, _, _) = HostedGpuContext::acquire_device(
            surface.window.clone(),
            wgpu::Features::empty(),
            &surface.target,
        )
        .await?;
        surface.rebind(raw, shared.resources.adapter(), &shared.resources)?;
        Ok(shared)
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        self.resources.adapter()
    }
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        self.resources.adapter_info()
    }
    pub fn resources(&self) -> HostedGpuResources {
        self.resources.clone()
    }
    pub fn prepare_surface_frame(&self, surface: &mut HostedGpuSurface, live: bool) {
        surface.prepare_frame(&self.resources, live);
    }
    pub fn apply_surface_alpha_mode(
        &self,
        surface: &mut HostedGpuSurface,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        surface.apply_alpha_mode(
            &self.instance,
            self.resources.adapter(),
            &self.resources,
            want_transparent,
        )
    }
    pub fn take_device_lost(&self) -> bool {
        self.device_lost.swap(false, Ordering::AcqRel)
    }
    pub(crate) fn is_device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Acquire)
    }
    pub fn take_device_lost_report(&self) -> Option<HostedDeviceLost> {
        self.device_lost.store(false, Ordering::Release);
        self.device_lost_report.lock().ok()?.take()
    }
    pub fn create_surface(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        self.create_surface_with_mode(window, want_transparent, HostedSurfaceMode::Window)
    }
    pub fn create_surface_with_mode(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        mode: HostedSurfaceMode,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        let target = HostedSurfaceTarget::new(mode, window.clone())?;
        self.create_surface_with_target(window, want_transparent, target)
    }
    /// Rebind a surface onto these GPU resources in place, keeping its window
    /// or composition target. See [`HostedGpuSurface::rebind`].
    pub fn recreate_surface(&self, surface: &mut HostedGpuSurface) -> Result<(), HostedGpuError> {
        #[cfg(target_os = "windows")]
        if surface.target.mode() == HostedSurfaceMode::WindowsComposition
            && self.resources.adapter_info().backend != wgpu::Backend::Dx12
        {
            return Err(HostedGpuError::Adapter(
                "DirectComposition requires the shared DX12 device".into(),
            ));
        }
        let raw = surface
            .target
            .create_surface(&self.instance, surface.window.clone())?;
        surface.rebind(raw, self.resources.adapter(), &self.resources)
    }
    fn create_surface_with_target(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        target: HostedSurfaceTarget,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        #[cfg(target_os = "windows")]
        if target.mode() == HostedSurfaceMode::WindowsComposition
            && self.resources.adapter_info().backend != wgpu::Backend::Dx12
        {
            return Err(HostedGpuError::Adapter(
                "DirectComposition requires the shared DX12 device".into(),
            ));
        }
        let surface = target.create_surface(&self.instance, window.clone())?;
        let capabilities = surface.get_capabilities(self.resources.adapter());
        let format = preferred_surface_format(&capabilities.formats)
            .ok_or(HostedGpuError::SurfaceHasNoFormats)?;
        configure_surface(
            window,
            surface,
            format,
            &capabilities,
            &self.resources,
            want_transparent,
            target,
        )
    }
    pub fn resize_surface(&self, surface: &mut HostedGpuSurface) {
        surface.resize(&self.resources);
    }
    pub fn acquire_surface_frame(
        &self,
        surface: &mut HostedGpuSurface,
    ) -> Result<HostedSurfaceFrame, HostedGpuError> {
        surface.acquire_frame(&self.instance, self.resources.adapter(), &self.resources)
    }
    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.resources.queue().present(frame);
    }
    /// Apply a reconfiguration deferred by a suboptimal frame once that frame
    /// has been presented, without waiting for another redraw.
    pub(crate) fn apply_pending_reconfigure(&self, surface: &mut HostedGpuSurface) {
        if surface.needs_reconfigure {
            surface.reconfigure(&self.resources);
        }
    }
    pub fn discard_surface_frame(
        &self,
        surface: &mut HostedGpuSurface,
        frame: wgpu::SurfaceTexture,
    ) {
        drop(frame);
        surface.needs_recovery = true;
    }
}

fn configure_surface(
    window: Arc<dyn winit::window::Window>,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    capabilities: &wgpu::SurfaceCapabilities,
    resources: &HostedGpuResources,
    want_transparent: bool,
    target: HostedSurfaceTarget,
) -> Result<HostedGpuSurface, HostedGpuError> {
    let size = window.surface_size();
    let configuration = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Srgb,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: preferred_live_present_mode(&capabilities.present_modes),
        alpha_mode: surface_alpha(target.mode(), &capabilities.alpha_modes, want_transparent)?,
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
    };
    let _gate = resources.lock_reconfigure();
    surface.configure(resources.device(), &configuration);
    target.commit()?;
    Ok(HostedGpuSurface {
        window,
        surface,
        target,
        needs_target_commit: false,
        needs_recovery: false,
        needs_reconfigure: false,
        format,
        configuration,
        want_transparent,
        alpha_recreate_attempted: false,
        live_present_mode: preferred_live_present_mode(&capabilities.present_modes),
    })
}

pub enum HostedSurfaceFrame {
    Ready(wgpu::SurfaceTexture),
    Retry,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedGpuError {
    SurfaceCreation(String),
    Adapter(String),
    Device(String),
    SurfaceHasNoFormats,
    SurfaceFormatChanged { expected: wgpu::TextureFormat },
    SurfaceValidation,
}

impl fmt::Display for HostedGpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceCreation(message) => {
                write!(formatter, "failed to create surface: {message}")
            }
            Self::Adapter(message) => write!(formatter, "failed to select GPU adapter: {message}"),
            Self::Device(message) => write!(formatter, "failed to create GPU device: {message}"),
            Self::SurfaceHasNoFormats => formatter.write_str("surface has no texture formats"),
            Self::SurfaceFormatChanged { expected } => write!(
                formatter,
                "recovered surface does not support renderer format {expected:?}"
            ),
            Self::SurfaceValidation => formatter.write_str("surface acquisition failed validation"),
        }
    }
}

impl std::error::Error for HostedGpuError {}

/// Failure starting or running the Nana Scene host event loop.
#[derive(Debug)]
pub enum HostedRunError {
    EventLoop(winit::error::EventLoopError),
    Startup(String),
}

impl fmt::Display for HostedRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(formatter, "hosted event loop failed: {error}"),
            Self::Startup(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for HostedRunError {}

fn preferred_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

pub(crate) fn preferred_alpha_mode(
    modes: &[wgpu::CompositeAlphaMode],
    want_transparent: bool,
) -> wgpu::CompositeAlphaMode {
    if want_transparent {
        [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Auto,
        ]
        .into_iter()
        .find(|mode| modes.contains(mode))
        .or_else(|| modes.first().copied())
        .unwrap_or(wgpu::CompositeAlphaMode::Auto)
    } else if modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto)
    }
}

fn surface_size_changed(configured: (u32, u32), inner: (u32, u32)) -> bool {
    inner.0 > 0 && inner.1 > 0 && configured != inner
}

fn live_resize_frame_latency(live: bool) -> u32 {
    if live { 2 } else { 1 }
}

/// Present configuration a surface must move to for the next frame's live
/// state, or `None` when the current configuration already matches. Steady
/// frames use the surface's preferred unblocked mode (`Mailbox`, then
/// `Immediate`, then `AutoVsync`). Entering live keeps that mode and adds one
/// extra frame of queue headroom; leaving live restores latency 1. Resolving
/// this once per frame keeps steady frames reconfigure-free.
fn live_resize_policy_change(
    current_present_mode: wgpu::PresentMode,
    current_frame_latency: u32,
    live_present_mode: wgpu::PresentMode,
    live: bool,
) -> Option<(wgpu::PresentMode, u32)> {
    let present_mode = live_present_mode;
    let frame_latency = live_resize_frame_latency(live);
    if present_mode == current_present_mode && frame_latency == current_frame_latency {
        None
    } else {
        Some((present_mode, frame_latency))
    }
}

fn preferred_live_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    [
        wgpu::PresentMode::Mailbox,
        wgpu::PresentMode::Immediate,
        wgpu::PresentMode::AutoVsync,
    ]
    .into_iter()
    .find(|mode| modes.contains(mode))
    .unwrap_or(wgpu::PresentMode::AutoVsync)
}

fn advertised_transparent_alpha(modes: &[wgpu::CompositeAlphaMode]) -> bool {
    modes.iter().any(|mode| {
        matches!(
            *mode,
            wgpu::CompositeAlphaMode::PreMultiplied
                | wgpu::CompositeAlphaMode::PostMultiplied
                | wgpu::CompositeAlphaMode::Auto
        )
    })
}

/// Recreate when transparency is requested but the surface advertises no alpha
/// mode that composites it: HWND/DWM flags applied after the first
/// `create_surface` need a new DXGI swapchain before Pre/Post/Auto appear.
///
/// Spent once per surface. A backend that reports alpha modes from the target
/// kind rather than from the window never widens the set, so a second rebuild
/// is pure cost; on Windows DX12, where every HWND surface advertises `Opaque`,
/// it also fails `CreateSwapChainForHwnd`. The host reports a fallback instead.
pub(crate) fn alpha_mode_needs_surface_recreate(
    want_transparent: bool,
    advertised: &[wgpu::CompositeAlphaMode],
    already_attempted: bool,
) -> bool {
    want_transparent && !already_attempted && !advertised_transparent_alpha(advertised)
}

#[cfg(test)]
mod tests {
    use super::{
        alpha_mode_needs_surface_recreate, live_resize_frame_latency, live_resize_policy_change,
        preferred_alpha_mode, preferred_live_present_mode, preferred_surface_format,
        surface_size_changed,
    };
    use std::sync::{Arc, RwLock};

    #[test]
    fn reconfigure_lock_excludes_concurrent_submit() {
        let lock = Arc::new(RwLock::new(()));
        let (started, waiting) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let writer = Arc::clone(&lock);
        let thread = std::thread::spawn(move || {
            let _write = writer.write().unwrap();
            started.send(()).unwrap();
            released.recv().unwrap();
        });
        waiting
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("reconfigure lock acquired");
        assert!(
            lock.try_read().is_err(),
            "submit must wait while the surface is reconfiguring"
        );
        release.send(()).unwrap();
        thread.join().unwrap();
        assert!(lock.try_read().is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn composition_requires_premultiplied_alpha_even_for_opaque_windows() {
        use super::{HostedSurfaceMode, surface_alpha};
        use wgpu::CompositeAlphaMode::{Opaque, PostMultiplied, PreMultiplied};
        assert_eq!(
            surface_alpha(
                HostedSurfaceMode::WindowsComposition,
                &[Opaque, PreMultiplied],
                false
            )
            .unwrap(),
            PreMultiplied
        );
        assert!(
            surface_alpha(
                HostedSurfaceMode::WindowsComposition,
                &[Opaque, PostMultiplied],
                true
            )
            .is_err()
        );
        assert_eq!(
            surface_alpha(HostedSurfaceMode::Window, &[Opaque, PreMultiplied], false).unwrap(),
            Opaque
        );
    }

    #[test]
    fn surface_preferences_preserve_transparency_and_srgb() {
        assert_eq!(
            preferred_alpha_mode(
                &[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ],
                true,
            ),
            wgpu::CompositeAlphaMode::PostMultiplied
        );
        assert_eq!(
            preferred_alpha_mode(
                &[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ],
                false,
            ),
            wgpu::CompositeAlphaMode::Opaque
        );
        assert_eq!(
            preferred_surface_format(&[
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ]),
            Some(wgpu::TextureFormat::Bgra8UnormSrgb)
        );
    }

    /// A program that asks for composition on a host device that is not DX12
    /// cannot have it: the device is already built, so there is no narrowing
    /// left to do, and the caller has to hear that it is getting the plain
    /// path instead.
    ///
    /// An embedded bootstrap also creates no instance of its own — the
    /// embedder's device is the one that will be used — so the capability
    /// answer costs nothing there.
    #[test]
    fn an_embedded_host_only_offers_composition_on_dx12() {
        use super::GpuBootstrap;

        let mut vulkan = GpuBootstrap::probe(Some(wgpu::Backend::Vulkan));
        assert!(!vulkan.composition_available());
        assert!(vulkan.take_instance().is_none());

        let mut dx12 = GpuBootstrap::probe(Some(wgpu::Backend::Dx12));
        assert_eq!(dx12.composition_available(), cfg!(target_os = "windows"));
        assert!(
            dx12.take_instance().is_none(),
            "an embedder's device brought its own instance"
        );

        // A process that never asked probes nothing at all.
        let mut plain = GpuBootstrap::plain();
        assert!(!plain.composition_available());
        assert!(plain.take_instance().is_none());
    }

    #[test]
    fn an_opaque_only_surface_is_rebuilt_for_alpha_at_most_once() {
        use wgpu::CompositeAlphaMode::{Auto, Opaque, PostMultiplied, PreMultiplied};
        // Windows DX12 reports an HWND surface's alpha modes from the target
        // kind, so the rebuilt surface advertises Opaque again. Only the first
        // attempt can work, and nothing re-grants it.
        assert!(alpha_mode_needs_surface_recreate(true, &[Opaque], false));
        assert!(alpha_mode_needs_surface_recreate(true, &[], false));
        assert!(!alpha_mode_needs_surface_recreate(true, &[Opaque], true));
        assert!(!alpha_mode_needs_surface_recreate(false, &[Opaque], false));
        for offered in [PreMultiplied, PostMultiplied, Auto] {
            assert!(
                !alpha_mode_needs_surface_recreate(true, &[Opaque, offered], false),
                "{offered:?} composites alpha, so the surface is already usable"
            );
        }
    }

    #[test]
    fn identical_drawable_sizes_do_not_need_a_new_swapchain() {
        assert!(!surface_size_changed((1280, 720), (1280, 720)));
        assert!(surface_size_changed((1280, 720), (1281, 720)));
        assert!(!surface_size_changed((1280, 720), (0, 720)));
        assert!(!surface_size_changed((1280, 720), (1280, 0)));
    }

    #[test]
    fn live_resize_prefers_unblocked_present_when_the_surface_advertises_it() {
        assert_eq!(live_resize_frame_latency(true), 2);
        assert_eq!(live_resize_frame_latency(false), 1);
        assert_eq!(
            preferred_live_present_mode(&[
                wgpu::PresentMode::Fifo,
                wgpu::PresentMode::Mailbox,
                wgpu::PresentMode::Immediate,
            ]),
            wgpu::PresentMode::Mailbox
        );
        assert_eq!(
            preferred_live_present_mode(&[wgpu::PresentMode::Fifo, wgpu::PresentMode::Immediate,]),
            wgpu::PresentMode::Immediate
        );
        assert_eq!(
            preferred_live_present_mode(&[wgpu::PresentMode::AutoVsync]),
            wgpu::PresentMode::AutoVsync
        );
    }

    #[test]
    fn live_resize_policy_reconfigures_once_per_session_and_back() {
        use wgpu::PresentMode;

        // Entering live from a stale AutoVsync configuration switches to the
        // cached unblocked mode and extra latency before the first moved frame.
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::Mailbox, true),
            Some((PresentMode::Mailbox, 2))
        );
        // Steady live frames keep the same configuration, so prepare_frame
        // never reconfigures mid-gesture.
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 2, PresentMode::Mailbox, true),
            None
        );
        // Leaving live keeps Mailbox and restores latency 1 exactly once.
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 2, PresentMode::Mailbox, false),
            Some((PresentMode::Mailbox, 1))
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 1, PresentMode::Mailbox, false),
            None
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::Mailbox, false),
            Some((PresentMode::Mailbox, 1))
        );
        // A surface without an unblocked present mode keeps AutoVsync; only
        // the latency changes, once per side of the gesture.
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::AutoVsync, true),
            Some((PresentMode::AutoVsync, 2))
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 2, PresentMode::AutoVsync, false),
            Some((PresentMode::AutoVsync, 1))
        );
    }
}
