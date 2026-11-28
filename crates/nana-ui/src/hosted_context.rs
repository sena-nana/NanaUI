//! The hosted device and its window surfaces.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::gpu_adapter::{
    AdapterCapabilitySummary, GpuAdapterChoice, GpuAdapterInfo, GpuAdapterSelection,
    automatic_adapter_with_capabilities, pick_adapter_with_capabilities,
};
use crate::{ResolvedSurfaceProfile, SurfacePresentationPolicy};
use nana_gpu::{__framework, GpuBackend, GpuContext, GpuDeviceLost, GpuTextureFormat};

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

/// Whether a window in this process may be created for a platform compositor.
///
/// The question has to be answered before any window exists —
/// `WS_EX_NOREDIRECTIONBITMAP` is a creation-time flag winit owns and nothing
/// can set durably afterwards. Proving the answer means creating a DX12
/// instance and enumerating its adapters, which creates a device on every
/// adapter: hundreds of milliseconds that would block the event thread before
/// the first window. So the answer is optimistic. The device request makes
/// the same enumeration off the event thread, and a composed target it cannot
/// build falls back to the plain path like any other composed failure.
pub(crate) struct GpuBootstrap {
    /// Whether a window in this process can present through a platform
    /// compositor. It answers only that: what a client's alpha then turns out
    /// to be is the surface's answer once it negotiates, and the plain path is
    /// not automatically opaque — a Vulkan surface negotiates `PreMultiplied`
    /// on it.
    composition: bool,
}

impl GpuBootstrap {
    /// A bootstrap that was not asked about composition.
    pub(crate) const fn plain() -> Self {
        Self { composition: false }
    }

    /// Answers composition capability without touching the GPU.
    ///
    /// `host_backend` is the backend of a GPU context an embedder already
    /// built: composition is only on the table when that context is DX12.
    /// Otherwise it is on the table wherever `WGPU_BACKEND` leaves DX12 to
    /// select; the device request proves it.
    pub(crate) fn probe(host_backend: Option<wgpu::Backend>) -> Self {
        #[cfg(target_os = "windows")]
        {
            let composition = match host_backend {
                Some(backend) => backend == wgpu::Backend::Dx12,
                None => wgpu::Backends::from_env()
                    .unwrap_or_default()
                    .contains(wgpu::Backends::DX12),
            };
            Self { composition }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = host_backend;
            Self::plain()
        }
    }

    pub(crate) const fn composition_available(&self) -> bool {
        self.composition
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
    profile: ResolvedSurfaceProfile,
    /// Most recent advisory HDR/luminance metadata for this surface's display.
    ///
    /// Surface capabilities decide whether an HDR profile may be configured;
    /// this value only tunes the presentation transform.  It is refreshed on
    /// window/display events, rather than queried on every frame.
    hdr_info: wgpu::DisplayHdrInfo,
    configuration: wgpu::SurfaceConfiguration,
    want_transparent: bool,
    /// Whether this surface spent its one alpha-mode rebuild. Never cleared:
    /// `rebind` reads the alpha mode off the incoming surface instead.
    alpha_recreate_attempted: bool,
    /// Live-resize present mode resolved from this surface's capabilities.
    /// Present-mode support is fixed per surface and adapter, so it is
    /// resolved at surface creation instead of re-queried every live frame.
    live_present_mode: wgpu::PresentMode,
    /// The window's program keeps presenting continuously (`FrameDemand::Continuous`
    /// or `RuntimeProgram::continuous_presentation`).
    /// On DX12 such a surface runs at DXGI maximum frame latency 2; see
    /// [`steady_frame_latency`].
    continuous_redraw: bool,
    /// Monotonic counter bumped whenever the underlying swapchain is
    /// configured or recreated, so per-backbuffer damage history can reset.
    configure_generation: u64,
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

    pub const fn format(&self) -> GpuTextureFormat {
        __framework::format_from_wgpu(self.format)
    }

    /// The concrete colour profile negotiated for this surface.
    pub const fn profile(&self) -> ResolvedSurfaceProfile {
        self.profile
    }

    pub(crate) fn surface_info(
        &self,
        generation: nana_gpu::DeviceGeneration,
        display: Option<nana_ui_platform::DisplayInfo>,
    ) -> Option<crate::WindowSurfaceInfo> {
        if self.needs_recovery || self.needs_reconfigure || self.needs_target_commit {
            return None;
        }
        let config = &self.configuration;
        Some(crate::WindowSurfaceInfo {
            extent: [config.width, config.height],
            format: __framework::format_from_wgpu(config.format),
            color_space: self.profile.scene_profile().color_space,
            present_mode: crate::SurfacePresentMode::from_wgpu(config.present_mode),
            alpha_mode: crate::SurfaceAlphaMode::from_wgpu(config.alpha_mode),
            desired_maximum_frame_latency: config.desired_maximum_frame_latency,
            device_generation: generation,
            display,
        })
    }

    /// Return the latest advisory display HDR metadata for an interop host.
    /// The returned value is a snapshot; call this after a display/window
    /// event rather than polling it on every frame.
    #[cfg(feature = "wgpu-interop")]
    pub fn display_hdr_info(&self) -> wgpu::DisplayHdrInfo {
        self.hdr_info.clone()
    }

    /// Cached multiplier above SDR reference white that the display can drive.
    /// Active HDR surfaces without live platform metadata fall back to the
    /// conservative default HDR budget (`16.0`), while SDR surfaces use `1.0`.
    /// This is advisory and must never be used as the sole HDR capability test.
    pub(crate) fn tone_map_headroom(&self) -> f32 {
        resolve_tone_map_headroom(self.profile, &self.hdr_info)
    }

    /// Cached SDR reference white in nits.  wgpu only reports it on platforms
    /// with absolute luminance metadata; the 80-nit fallback matches the
    /// scRGB convention used by the presentation pipeline and wgpu's HDR
    /// reference example.
    pub(crate) fn reference_white_nits(&self) -> f32 {
        self.hdr_info
            .luminance
            .and_then(|luminance| luminance.sdr_white_nits)
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(80.0)
    }

    /// Snapshot the event-cached presentation parameters for the Scene
    /// painter. Unknown metadata uses the conservative, finite defaults; the
    /// surface profile itself remains the capability authority.
    pub(crate) fn presentation_parameters(&self) -> crate::ScenePresentationParameters {
        crate::ScenePresentationParameters::new(
            self.tone_map_headroom(),
            self.reference_white_nits(),
        )
        .with_extended_linear_white(self.extended_linear_white())
        .normalized()
    }

    /// SDR white in this surface's scRGB units. Windows composes scRGB with
    /// 1.0 at 80 nits and lets the user move SDR white (the "SDR content
    /// brightness" slider); writing white as 1.0 there shows the UI at a
    /// fraction of every other window's brightness. macOS EDR already means
    /// SDR white by 1.0.
    fn extended_linear_white(&self) -> f32 {
        if cfg!(windows) {
            self.reference_white_nits() / 80.0
        } else {
            1.0
        }
    }

    /// Re-query advisory display metadata after a display/configuration event.
    /// Returns `true` only when the cached value changed, allowing the host to
    /// request one redraw without rebuilding application state or pipelines.
    pub(crate) fn refresh_hdr_info(&mut self, gpu: &GpuContext) -> bool {
        let next = self.surface.display_hdr_info(__framework::adapter(gpu));
        if next == self.hdr_info {
            return false;
        }
        self.hdr_info = next;
        crate::host_diagnostics::record_surface_hdr(self.profile, &self.hdr_info);
        true
    }

    /// Re-query surface colour capabilities and advisory display metadata after
    /// an embedder's display/HDR notification. Returns `true` when either the
    /// resolved surface profile or presentation parameters changed.
    #[cfg(feature = "wgpu-interop")]
    pub fn refresh_display_hdr_info(&mut self, graphics: &HostedGpuShared) -> bool {
        let profile_changed = self.re_resolve_profile(&graphics.gpu);
        let hdr_changed = self.refresh_hdr_info(&graphics.gpu);
        profile_changed || hdr_changed
    }

    /// Resolve the profile again against what the surface offers now (on
    /// another display, or after the alpha mode changed) and reconfigure
    /// when it moved. Whether it did.
    pub(crate) fn re_resolve_profile(&mut self, gpu: &GpuContext) -> bool {
        let capabilities = self.surface.get_capabilities(__framework::adapter(gpu));
        let Some(next) = crate::surface_profile::resolve_surface_profile_with_format_support(
            self.profile.requested,
            &capabilities,
            self.want_transparent,
            |format| gpu.supports_render_format(__framework::format_from_wgpu(format)),
        ) else {
            return false;
        };
        if next.fallback.is_some() {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::FORMAT_FALLBACK);
        }
        if !surface_profile_changed(self.profile, next) {
            return false;
        }
        self.profile = next;
        self.format = next.format;
        self.configuration.format = next.format;
        self.configuration.color_space = next.color_space;
        crate::host_diagnostics::record_surface_hdr(self.profile, &self.hdr_info);
        self.reconfigure(gpu);
        true
    }

    /// Alpha composition mode selected from the native surface capabilities.
    pub const fn alpha_mode(&self) -> crate::SurfaceAlphaMode {
        crate::SurfaceAlphaMode::from_wgpu(self.configuration.alpha_mode)
    }

    pub(crate) const fn wgpu_alpha_mode(&self) -> wgpu::CompositeAlphaMode {
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

    /// Resize and reconfigure this surface from the current native window size.
    ///
    /// This is the low-level surface operation. Embedding hosts should normally
    /// call [`HostedGpuShared::resize_surface`] so the surface is always paired
    /// with the device that owns it. The method remains public for advanced
    /// hosts that retain the surface and device as separate parts via
    /// [`HostedGpuContext::into_parts`].
    #[doc(hidden)]
    pub fn resize(&mut self, gpu: &GpuContext) {
        let size = self.window.surface_size();
        if !surface_size_changed(
            (self.configuration.width, self.configuration.height),
            (size.width, size.height),
        ) {
            return;
        }
        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.reconfigure(gpu);
    }

    /// Apply size and live-resize present policy, then configure at most once.
    ///
    /// This is the low-level surface operation. Embedding hosts should normally
    /// call [`HostedGpuShared::prepare_surface_frame`] so the surface is always
    /// paired with the device that owns it. The method remains public for
    /// advanced hosts that retain the surface and device as separate parts via
    /// [`HostedGpuContext::into_parts`].
    #[doc(hidden)]
    pub fn prepare_frame(&mut self, gpu: &GpuContext, live: bool) {
        let size = self.window.surface_size();
        let latency =
            steady_frame_latency(live, self.continuous_redraw, gpu.capabilities().backend());
        let mut changed = self.apply_live_resize_policy(latency);
        if surface_size_changed(
            (self.configuration.width, self.configuration.height),
            (size.width, size.height),
        ) {
            self.configuration.width = size.width;
            self.configuration.height = size.height;
            changed = true;
        }
        if changed {
            self.reconfigure(gpu);
        }
    }

    /// Whether the window's program keeps redrawing. Takes effect at the next
    /// [`Self::prepare_frame`]; a change reconfigures the swapchain once.
    pub(crate) fn set_continuous_redraw(&mut self, continuous: bool) {
        self.continuous_redraw = continuous;
    }

    fn apply_live_resize_policy(&mut self, frame_latency: u32) -> bool {
        let Some((present_mode, desired_maximum_frame_latency)) = live_resize_policy_change(
            self.configuration.present_mode,
            self.configuration.desired_maximum_frame_latency,
            self.live_present_mode,
            frame_latency,
        ) else {
            return false;
        };
        self.configuration.present_mode = present_mode;
        self.configuration.desired_maximum_frame_latency = desired_maximum_frame_latency;
        self.configuration.width > 0 && self.configuration.height > 0
    }

    pub(crate) const fn configure_generation(&self) -> u64 {
        self.configure_generation
    }

    fn reconfigure(&mut self, gpu: &GpuContext) {
        self.surface
            .configure(__framework::device(gpu), &self.configuration);
        self.configure_generation = self.configure_generation.wrapping_add(1);
        self.needs_target_commit = true;
        self.needs_reconfigure = false;
    }

    fn apply_alpha_mode(
        &mut self,
        instance: &wgpu::Instance,
        gpu: &GpuContext,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        let capabilities = self.surface.get_capabilities(__framework::adapter(gpu));
        if self.target.mode() == HostedSurfaceMode::Window
            && alpha_mode_needs_surface_recreate(
                want_transparent,
                &capabilities.alpha_modes,
                self.alpha_recreate_attempted,
            )
        {
            self.alpha_recreate_attempted = true;
            return self.recover_with_alpha(instance, gpu, want_transparent);
        }
        let alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            want_transparent,
        )?;
        let transparency_changed = self.want_transparent != want_transparent;
        self.want_transparent = want_transparent;
        if self.configuration.alpha_mode != alpha_mode {
            self.configuration.alpha_mode = alpha_mode;
            self.reconfigure(gpu);
        }
        // A transparent surface wants a format with alpha bits: a 10-bit PQ
        // swapchain keeps only two.
        if transparency_changed {
            self.re_resolve_profile(gpu);
        }
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
        gpu: &GpuContext,
    ) -> Result<(), HostedGpuError> {
        let capabilities = surface.get_capabilities(__framework::adapter(gpu));
        let profile = crate::surface_profile::resolve_surface_profile_with_format_support(
            // Keep the caller's wide-gamut/HDR request across a device
            // migration. Resolving Auto here silently downgraded an existing
            // profile to sRGB after recovery, even when the new surface still
            // advertised the requested colour space.
            self.profile.requested,
            &capabilities,
            self.want_transparent,
            |format| gpu.supports_render_format(__framework::format_from_wgpu(format)),
        )
        .ok_or_else(|| {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE);
            HostedGpuError::SurfaceHasNoFormats
        })?;
        if profile.fallback.is_some() {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::FORMAT_FALLBACK);
        }
        let format = profile.format;
        let alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            self.want_transparent,
        )?;
        let hdr_info = surface.display_hdr_info(__framework::adapter(gpu));
        self.surface = surface;
        let size = self.window.surface_size();
        self.format = format;
        self.profile = profile;
        self.hdr_info = hdr_info;
        crate::host_diagnostics::record_surface_hdr(self.profile, &self.hdr_info);
        self.live_present_mode = preferred_live_present_mode(&capabilities.present_modes);
        self.configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: profile.color_space,
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
        self.reconfigure(gpu);
        Ok(())
    }

    fn recover(
        &mut self,
        instance: &wgpu::Instance,
        gpu: &GpuContext,
    ) -> Result<(), HostedGpuError> {
        self.recover_with_alpha(instance, gpu, self.want_transparent)
    }

    fn recover_with_alpha(
        &mut self,
        instance: &wgpu::Instance,
        gpu: &GpuContext,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        let surface = self.target.create_surface(instance, self.window.clone())?;
        let capabilities = surface.get_capabilities(__framework::adapter(gpu));
        let profile = crate::surface_profile::resolve_surface_profile_with_format_support(
            self.profile.requested,
            &capabilities,
            want_transparent,
            |format| gpu.supports_render_format(__framework::format_from_wgpu(format)),
        )
        .ok_or_else(|| {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE);
            HostedGpuError::SurfaceHasNoFormats
        })?;
        if profile.fallback.is_some() {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::FORMAT_FALLBACK);
        }
        let hdr_info = surface.display_hdr_info(__framework::adapter(gpu));
        self.configuration.alpha_mode = surface_alpha(
            self.target.mode(),
            &capabilities.alpha_modes,
            want_transparent,
        )?;
        self.live_present_mode = preferred_live_present_mode(&capabilities.present_modes);
        self.configuration.present_mode = self.live_present_mode;
        self.format = profile.format;
        self.profile = profile;
        self.hdr_info = hdr_info;
        crate::host_diagnostics::record_surface_hdr(self.profile, &self.hdr_info);
        self.configuration.format = profile.format;
        self.configuration.color_space = profile.color_space;
        self.surface = surface;
        self.reconfigure(gpu);
        self.commit_target()?;
        self.want_transparent = want_transparent;
        self.needs_recovery = false;
        Ok(())
    }

    fn acquire_frame(
        &mut self,
        instance: &wgpu::Instance,
        gpu: &GpuContext,
    ) -> Result<SurfaceFrame, HostedGpuError> {
        if !self.is_drawable() {
            return Ok(SurfaceFrame::Skipped);
        }
        if self.needs_recovery {
            self.recover(instance, gpu)?;
        }
        if self.needs_reconfigure {
            self.reconfigure(gpu);
        }
        self.commit_target()?;
        let result = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Ok(SurfaceFrame::Ready(frame)),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.needs_reconfigure = true;
                Ok(SurfaceFrame::Ready(frame))
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::SURFACE_OUTDATED);
                // A stale swapchain is the common resize race. Reconfigure and
                // retry once in the same frame so a resize step does not drop
                // its redraw to the next event-loop iteration.
                self.reconfigure(gpu);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => Ok(SurfaceFrame::Ready(frame)),
                    wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        self.needs_reconfigure = true;
                        Ok(SurfaceFrame::Ready(frame))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        Err(HostedGpuError::SurfaceValidation)
                    }
                    _ => Ok(SurfaceFrame::Retry),
                }
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::SURFACE_LOST);
                nana_diagnostics::event!(nana_diagnostics::framework::gpu::SURFACE_LOST_EVENT);
                self.recover(instance, gpu)?;
                Ok(SurfaceFrame::Retry)
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::SURFACE_TIMEOUT);
                Ok(SurfaceFrame::Skipped)
            }
            wgpu::CurrentSurfaceTexture::Occluded => Ok(SurfaceFrame::Skipped),
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

/// The hosted device without ownership of any native window.
#[derive(Clone)]
pub struct HostedGpuShared {
    instance: wgpu::Instance,
    gpu: GpuContext,
    /// The host has started recovering from this device's loss.
    loss_handled: Arc<AtomicBool>,
    /// This device's loss was reported to diagnostics.
    loss_reported: Arc<AtomicBool>,
    /// The adapter the program asked for, asked for again by a replacement
    /// device.
    choice: GpuAdapterChoice,
    /// The process's backend policy, which a replacement device follows too.
    backend_policy: crate::GpuBackendPolicy,
    /// What was found and chosen when this device was created.
    selection: Arc<GpuAdapterSelection>,
}

impl std::ops::Deref for HostedGpuContext {
    type Target = HostedGpuShared;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

impl HostedGpuContext {
    pub fn into_parts(self) -> (HostedGpuShared, HostedGpuSurface) {
        (self.shared, self.primary)
    }

    #[cfg(feature = "wgpu-interop")]
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

    #[cfg(feature = "wgpu-interop")]
    pub async fn new_with_surface_mode(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        mode: HostedSurfaceMode,
    ) -> Result<Self, HostedGpuError> {
        Self::new_with_surface_mode_policy(
            window,
            required_features,
            want_transparent,
            mode,
            SurfacePresentationPolicy::Auto,
        )
        .await
    }

    /// Create a hosted context whose primary surface requests an explicit
    /// presentation policy. PQ/HLG are selected only when the surface
    /// advertises the exact format/colour-space pair; otherwise resolution
    /// falls back deterministically to the extended-linear or SDR profile.
    #[cfg(feature = "wgpu-interop")]
    pub async fn new_with_surface_policy(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        policy: SurfacePresentationPolicy,
    ) -> Result<Self, HostedGpuError> {
        Self::new_with_surface_mode_policy(
            window,
            required_features,
            want_transparent,
            HostedSurfaceMode::Window,
            policy,
        )
        .await
    }

    /// Variant of [`Self::new_with_surface_mode`] that also controls the
    /// colour-presentation policy of the primary surface.
    #[cfg(feature = "wgpu-interop")]
    pub async fn new_with_surface_mode_policy(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        mode: HostedSurfaceMode,
        policy: SurfacePresentationPolicy,
    ) -> Result<Self, HostedGpuError> {
        let (pending, request) = PendingPrimarySurface::begin_with_policy(
            window,
            required_features,
            want_transparent,
            mode,
            policy,
            GpuAdapterChoice::Automatic,
            crate::GpuBackendPolicy::Plain,
        )?;
        pending.finish(request.acquire().await?)
    }

    /// Rebuild GPU resources while retaining the primary native visual tree.
    /// Move this context onto a new device, rebinding the primary surface in
    /// place (see [`HostedGpuSurface::rebind`]). Other surfaces follow with
    /// [`HostedGpuShared::recreate_surface`]. An error leaves the context
    /// unchanged.
    #[cfg(feature = "wgpu-interop")]
    pub async fn recreate(
        &mut self,
        required_features: wgpu::Features,
    ) -> Result<(), HostedGpuError> {
        let AcquiredDevice {
            shared, surface, ..
        } = DeviceRequest::new(
            self.primary.window.clone(),
            required_features,
            &self.primary.target,
            self.shared.choice.clone(),
            self.shared.backend_policy,
        )?
        .acquire()
        .await?;
        self.primary.rebind(surface, &shared.gpu)?;
        self.shared = shared;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    pub fn windows_composition(&self) -> Option<&crate::WindowsComposition> {
        self.primary.windows_composition()
    }

    pub const fn format(&self) -> GpuTextureFormat {
        self.primary.format()
    }

    /// Alpha composition mode used by the primary native surface.
    pub const fn alpha_mode(&self) -> crate::SurfaceAlphaMode {
        self.primary.alpha_mode()
    }

    pub fn physical_size(&self) -> (u32, u32) {
        self.primary.physical_size()
    }

    pub fn resize(&mut self) {
        self.primary.resize(&self.shared.gpu);
    }

    pub fn prepare_frame(&mut self, live: bool) {
        self.primary.prepare_frame(&self.shared.gpu, live);
    }

    pub fn reconfigure(&mut self) {
        self.primary.reconfigure(&self.shared.gpu);
    }

    pub fn recover_surface(&mut self) -> Result<(), HostedGpuError> {
        self.primary
            .recover(&self.shared.instance, &self.shared.gpu)
    }

    pub fn apply_alpha_mode(&mut self, want_transparent: bool) -> Result<(), HostedGpuError> {
        self.primary
            .apply_alpha_mode(&self.shared.instance, &self.shared.gpu, want_transparent)
    }

    /// Refresh the primary surface's negotiated profile and cached display HDR
    /// metadata after an embedder receives a display/system-HDR notification.
    #[cfg(feature = "wgpu-interop")]
    pub fn refresh_display_hdr_info(&mut self) -> bool {
        let profile_changed = self.primary.re_resolve_profile(&self.shared.gpu);
        let hdr_changed = self.primary.refresh_hdr_info(&self.shared.gpu);
        profile_changed || hdr_changed
    }

    pub fn is_drawable(&self) -> bool {
        self.primary.is_drawable()
    }

    #[cfg(feature = "wgpu-interop")]
    pub fn acquire_frame(&mut self) -> Result<HostedSurfaceFrame, HostedGpuError> {
        self.primary
            .acquire_frame(&self.shared.instance, &self.shared.gpu)
            .map(Into::into)
    }

    /// Abandon an acquired primary frame after encoding fails. Drop all views
    /// and unfinished encoders referencing it before calling this method.
    /// The next acquisition recreates only this surface: on DX12, dropping a
    /// frame does not restore the consumed frame-latency waitable signal.
    #[cfg(feature = "wgpu-interop")]
    pub fn discard_frame(&mut self, frame: wgpu::SurfaceTexture) {
        drop(frame);
        self.primary.needs_recovery = true;
    }
}

impl HostedGpuShared {
    /// Adopt an existing host device (see `GpuContext::from_wgpu`) and the
    /// instance its surfaces are created on, without requesting another
    /// adapter or device. Device-loss notification and replacement remain the
    /// embedding host's responsibility.
    #[cfg(feature = "wgpu-interop")]
    pub fn from_device(instance: wgpu::Instance, gpu: GpuContext) -> Self {
        Self::adopt(instance, gpu)
    }

    fn adopt(instance: wgpu::Instance, gpu: GpuContext) -> Self {
        // An adopted device was not chosen here: the selection only names it.
        let selection = GpuAdapterSelection {
            adapters: vec![GpuAdapterInfo::from_wgpu(__framework::adapter_info(&gpu))],
            active: Some(0),
            requested_missing: false,
        };
        Self {
            instance,
            gpu,
            loss_handled: Arc::new(AtomicBool::new(false)),
            loss_reported: Arc::new(AtomicBool::new(false)),
            choice: GpuAdapterChoice::Automatic,
            backend_policy: crate::GpuBackendPolicy::Plain,
            selection: Arc::new(selection),
        }
    }

    /// A replacement device seeded by `surface`, which is rebound onto it,
    /// on the adapter `choice` asks for.
    pub(crate) async fn rebuild_for_surface(
        surface: &mut HostedGpuSurface,
        choice: GpuAdapterChoice,
        backend_policy: crate::GpuBackendPolicy,
    ) -> Result<Self, HostedGpuError> {
        let AcquiredDevice {
            shared,
            surface: raw,
            ..
        } = DeviceRequest::new(
            surface.window.clone(),
            wgpu::Features::empty(),
            &surface.target,
            choice,
            backend_policy,
        )?
        .acquire()
        .await?;
        surface.rebind(raw, &shared.gpu)?;
        Ok(shared)
    }

    /// The device every surface of this host renders with.
    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }
    /// The adapters that could present the window when this device was
    /// created, and which one it runs on.
    pub fn adapter_selection(&self) -> Arc<GpuAdapterSelection> {
        Arc::clone(&self.selection)
    }
    pub(crate) fn adapter_choice(&self) -> &GpuAdapterChoice {
        &self.choice
    }
    pub(crate) fn adapter_info(&self) -> &wgpu::AdapterInfo {
        __framework::adapter_info(&self.gpu)
    }
    /// Synchronize a secondary surface with its native size and presentation
    /// policy for the next frame.
    ///
    /// Use this method for surfaces created with [`Self::create_surface`],
    /// [`Self::create_surface_with_policy`], or
    /// [`Self::create_surface_with_mode`]. It keeps the GPU owner and surface
    /// together and is the preferred alternative to calling the hidden
    /// [`HostedGpuSurface::prepare_frame`] operation directly.
    pub fn prepare_surface_frame(&self, surface: &mut HostedGpuSurface, live: bool) {
        surface.prepare_frame(&self.gpu, live);
    }
    pub fn apply_surface_alpha_mode(
        &self,
        surface: &mut HostedGpuSurface,
        want_transparent: bool,
    ) -> Result<(), HostedGpuError> {
        surface.apply_alpha_mode(&self.instance, &self.gpu, want_transparent)
    }
    /// `true` once per lost device: the first call after the loss, so the host
    /// starts recovery once. Loss itself is [`GpuContext::is_lost`].
    pub fn take_device_lost(&self) -> bool {
        self.gpu.is_lost() && !self.loss_handled.swap(true, Ordering::AcqRel)
    }
    /// Loss handling found nothing to recover with (no window to seed the
    /// replacement device); the next check starts it again.
    pub(crate) fn rearm_device_loss(&self) {
        self.loss_handled.store(false, Ordering::Release);
    }
    pub(crate) fn is_device_lost(&self) -> bool {
        self.gpu.is_lost()
    }
    /// The loss report, once per lost device. Also counts as handling the loss.
    pub fn take_device_lost_report(&self) -> Option<GpuDeviceLost> {
        if !self.gpu.is_lost() {
            return None;
        }
        self.loss_handled.store(true, Ordering::Release);
        if self.loss_reported.swap(true, Ordering::AcqRel) {
            return None;
        }
        self.gpu.lost_report()
    }
    pub fn create_surface(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        self.create_surface_with_policy(window, want_transparent, SurfacePresentationPolicy::Auto)
    }
    pub fn create_surface_with_policy(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        policy: SurfacePresentationPolicy,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        let target = HostedSurfaceTarget::new(HostedSurfaceMode::Window, window.clone())?;
        self.create_surface_with_target_policy(window, want_transparent, target, policy)
    }
    pub fn create_surface_with_mode(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        mode: HostedSurfaceMode,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        self.create_surface_with_mode_policy(
            window,
            want_transparent,
            mode,
            SurfacePresentationPolicy::Auto,
        )
    }
    /// Create a secondary surface with an explicit colour-presentation policy
    /// and native surface mode.
    pub fn create_surface_with_mode_policy(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        mode: HostedSurfaceMode,
        policy: SurfacePresentationPolicy,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        let target = HostedSurfaceTarget::new(mode, window.clone())?;
        self.create_surface_with_target_policy(window, want_transparent, target, policy)
    }
    /// Rebind a surface onto these GPU resources in place, keeping its window
    /// or composition target. See [`HostedGpuSurface::rebind`].
    pub fn recreate_surface(&self, surface: &mut HostedGpuSurface) -> Result<(), HostedGpuError> {
        #[cfg(target_os = "windows")]
        if surface.target.mode() == HostedSurfaceMode::WindowsComposition
            && self.adapter_info().backend != wgpu::Backend::Dx12
        {
            return Err(HostedGpuError::Adapter(
                "DirectComposition requires the shared DX12 device".into(),
            ));
        }
        let raw = surface
            .target
            .create_surface(&self.instance, surface.window.clone())?;
        surface.rebind(raw, &self.gpu)
    }
    fn create_surface_with_target_policy(
        &self,
        window: Arc<dyn winit::window::Window>,
        want_transparent: bool,
        target: HostedSurfaceTarget,
        policy: SurfacePresentationPolicy,
    ) -> Result<HostedGpuSurface, HostedGpuError> {
        #[cfg(target_os = "windows")]
        if target.mode() == HostedSurfaceMode::WindowsComposition
            && self.adapter_info().backend != wgpu::Backend::Dx12
        {
            return Err(HostedGpuError::Adapter(
                "DirectComposition requires the shared DX12 device".into(),
            ));
        }
        let surface = target.create_surface(&self.instance, window.clone())?;
        let capabilities = surface.get_capabilities(__framework::adapter(&self.gpu));
        configure_surface(
            window,
            surface,
            &capabilities,
            &self.gpu,
            want_transparent,
            target,
            policy,
        )
    }
    /// Resize a secondary surface from its current native window size.
    ///
    /// This is the preferred facade for surfaces owned by this shared GPU
    /// context. Call it before acquiring a frame when a host handles resize
    /// events outside the standard scene host.
    pub fn resize_surface(&self, surface: &mut HostedGpuSurface) {
        surface.resize(&self.gpu);
    }
    pub(crate) fn acquire(
        &self,
        surface: &mut HostedGpuSurface,
    ) -> Result<SurfaceFrame, HostedGpuError> {
        surface.acquire_frame(&self.instance, &self.gpu)
    }
    #[cfg(feature = "wgpu-interop")]
    pub fn acquire_surface_frame(
        &self,
        surface: &mut HostedGpuSurface,
    ) -> Result<HostedSurfaceFrame, HostedGpuError> {
        self.acquire(surface).map(Into::into)
    }
    pub(crate) fn present_frame(&self, frame: wgpu::SurfaceTexture) {
        __framework::queue(&self.gpu).present(frame);
    }

    pub(crate) fn present_frame_with_damage(
        &self,
        frame: wgpu::SurfaceTexture,
        dirty_rects: &[[u32; 4]],
    ) {
        __framework::queue(&self.gpu).present_with_damage(frame, dirty_rects);
    }

    #[cfg(feature = "wgpu-interop")]
    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.present_frame(frame);
    }
    /// Apply a reconfiguration deferred by a suboptimal frame once that frame
    /// has been presented, without waiting for another redraw.
    pub(crate) fn apply_pending_reconfigure(&self, surface: &mut HostedGpuSurface) {
        if surface.needs_reconfigure {
            surface.reconfigure(&self.gpu);
        }
    }
    pub(crate) fn abandon_frame(
        &self,
        surface: &mut HostedGpuSurface,
        frame: wgpu::SurfaceTexture,
    ) {
        drop(frame);
        surface.needs_recovery = true;
    }
    #[cfg(feature = "wgpu-interop")]
    pub fn discard_surface_frame(
        &self,
        surface: &mut HostedGpuSurface,
        frame: wgpu::SurfaceTexture,
    ) {
        self.abandon_frame(surface, frame);
    }
}

fn configure_surface(
    window: Arc<dyn winit::window::Window>,
    surface: wgpu::Surface<'static>,
    capabilities: &wgpu::SurfaceCapabilities,
    gpu: &GpuContext,
    want_transparent: bool,
    target: HostedSurfaceTarget,
    policy: SurfacePresentationPolicy,
) -> Result<HostedGpuSurface, HostedGpuError> {
    let profile = crate::surface_profile::resolve_surface_profile_with_format_support(
        policy,
        capabilities,
        want_transparent,
        |format| gpu.supports_render_format(__framework::format_from_wgpu(format)),
    )
    .ok_or_else(|| {
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE);
        HostedGpuError::SurfaceHasNoFormats
    })?;
    if profile.fallback.is_some() {
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::FORMAT_FALLBACK);
    }
    let hdr_info = surface.display_hdr_info(__framework::adapter(gpu));
    let format = profile.format;
    let size = window.surface_size();
    let configuration = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: profile.color_space,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: preferred_live_present_mode(&capabilities.present_modes),
        alpha_mode: surface_alpha(target.mode(), &capabilities.alpha_modes, want_transparent)?,
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
    };
    surface.configure(__framework::device(gpu), &configuration);
    target.commit()?;
    crate::host_diagnostics::record_surface_hdr(profile, &hdr_info);
    Ok(HostedGpuSurface {
        window,
        surface,
        target,
        needs_target_commit: false,
        needs_recovery: false,
        needs_reconfigure: false,
        format,
        profile,
        hdr_info,
        configuration,
        want_transparent,
        alpha_recreate_attempted: false,
        live_present_mode: preferred_live_present_mode(&capabilities.present_modes),
        continuous_redraw: false,
        configure_generation: 1,
    })
}

/// The half of creating a device that has to happen on the window's thread:
/// the instance and the window's surface. What it hands off is `Send`, so the
/// adapter and device request — the part that can take seconds — can run on
/// another thread while the window's event loop keeps turning.
pub(crate) struct DeviceRequest {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    required_features: wgpu::Features,
    /// The backends `instance` was created for, to enumerate.
    backends: wgpu::Backends,
    choice: GpuAdapterChoice,
    backend_policy: crate::GpuBackendPolicy,
}

/// A device and the surface it was chosen for, not yet configured.
pub(crate) struct AcquiredDevice {
    shared: HostedGpuShared,
    surface: wgpu::Surface<'static>,
    capabilities: wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
}

impl AcquiredDevice {
    pub(crate) fn gpu(&self) -> &GpuContext {
        &self.shared.gpu
    }

    pub(crate) fn scene_profile(&self) -> crate::ScenePresentationProfile {
        let profile = crate::surface_profile::resolve_surface_profile_with_format_support(
            SurfacePresentationPolicy::Auto,
            &self.capabilities,
            false,
            |format| {
                self.shared
                    .gpu
                    .supports_render_format(__framework::format_from_wgpu(format))
            },
        )
        .map(|profile| profile.scene_profile())
        .unwrap_or_else(|| {
            crate::ScenePresentationProfile::sdr(__framework::format_from_wgpu(self.format))
        });
        crate::scene_paint::negotiate_presentation_for_gpu(&self.shared.gpu, profile)
            .unwrap_or(profile)
    }
}

/// A primary window whose surface waits for its device; see [`DeviceRequest`].
pub(crate) struct PendingPrimarySurface {
    window: Arc<dyn winit::window::Window>,
    target: HostedSurfaceTarget,
    want_transparent: bool,
    policy: SurfacePresentationPolicy,
}

impl PendingPrimarySurface {
    /// Creates the surface target and the surface on the window's thread.
    pub(crate) fn begin_with_policy(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        want_transparent: bool,
        mode: HostedSurfaceMode,
        policy: SurfacePresentationPolicy,
        choice: GpuAdapterChoice,
        backend_policy: crate::GpuBackendPolicy,
    ) -> Result<(Self, DeviceRequest), HostedGpuError> {
        let target = HostedSurfaceTarget::new(mode, window.clone())?;
        let request = DeviceRequest::new(
            window.clone(),
            required_features,
            &target,
            choice,
            backend_policy,
        )?;
        Ok((
            Self {
                window,
                target,
                want_transparent,
                policy,
            },
            request,
        ))
    }

    /// Configures the surface on the device, back on the window's thread.
    pub(crate) fn finish(self, device: AcquiredDevice) -> Result<HostedGpuContext, HostedGpuError> {
        let primary = configure_surface(
            self.window,
            device.surface,
            &device.capabilities,
            &device.shared.gpu,
            self.want_transparent,
            self.target,
            self.policy,
        )?;
        Ok(HostedGpuContext {
            shared: device.shared,
            primary,
        })
    }
}

impl DeviceRequest {
    fn new(
        window: Arc<dyn winit::window::Window>,
        required_features: wgpu::Features,
        target: &HostedSurfaceTarget,
        choice: GpuAdapterChoice,
        backend_policy: crate::GpuBackendPolicy,
    ) -> Result<Self, HostedGpuError> {
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
        let instance = wgpu::Instance::new(instance_descriptor(backends));
        let surface = target.create_surface(&instance, window)?;
        Ok(Self {
            instance,
            surface,
            required_features,
            backends,
            choice,
            backend_policy,
        })
    }

    /// Chooses the adapter for the surface and requests the device. Runs on
    /// any thread.
    pub(crate) async fn acquire(self) -> Result<AcquiredDevice, HostedGpuError> {
        let Self {
            instance,
            surface,
            required_features,
            backends,
            choice,
            backend_policy,
        } = self;
        let adapters = instance.enumerate_adapters(backends).await;
        let compatible = on_preferred_backend(
            adapters
                .into_iter()
                .filter(|adapter| adapter.is_surface_supported(&surface))
                .collect(),
            preferred_backend(backend_policy),
            |adapter: &wgpu::Adapter| adapter.get_info().backend,
        );
        let infos: Vec<GpuAdapterInfo> = compatible
            .iter()
            .map(|adapter| GpuAdapterInfo::from_wgpu(&adapter.get_info()))
            .collect();
        let summaries: Vec<AdapterCapabilitySummary> = compatible
            .iter()
            .map(|adapter| adapter_capability_summary(adapter, &surface))
            .collect();
        let env_name = std::env::var("WGPU_ADAPTER_NAME").ok();
        let power = wgpu::PowerPreference::from_env().unwrap_or_default();
        let (picked, mut requested_missing) =
            pick_adapter_with_capabilities(&infos, &summaries, &choice, env_name.as_deref(), power);
        // An adapter the program asked for that cannot make a device is
        // passed over for the automatic one, not a failed start.
        let mut chosen = None;
        if let (Some(index), GpuAdapterChoice::Matching(_)) = (picked, &choice)
            && !requested_missing
        {
            match request_shared_device(&compatible[index], required_features).await {
                Ok(device) => chosen = Some((index, device)),
                Err(_) => requested_missing = true,
            }
        }
        let (active, adapter, (device, queue)) = match chosen {
            Some((index, device)) => (Some(index), compatible[index].clone(), device),
            None => {
                let index = if requested_missing {
                    automatic_adapter_with_capabilities(
                        &infos,
                        &summaries,
                        env_name.as_deref(),
                        power,
                    )
                } else {
                    picked
                };
                let adapter = match index {
                    Some(index) => compatible[index].clone(),
                    None => {
                        if !compatible.is_empty() {
                            nana_diagnostics::event!(
                                nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE
                            );
                            return Err(HostedGpuError::Adapter(
                                "no surface-compatible GPU adapter supports a renderable surface format"
                                    .into(),
                            ));
                        }
                        let adapter = wgpu::util::initialize_adapter_from_env_or_default(
                            &instance,
                            Some(&surface),
                        )
                        .await
                        .map_err(|error| {
                            nana_diagnostics::event!(
                                nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE
                            );
                            HostedGpuError::Adapter(error.to_string())
                        })?;
                        if !adapter_capability_summary(&adapter, &surface)
                            .has_renderable_surface_format
                        {
                            nana_diagnostics::event!(
                                nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE
                            );
                            return Err(HostedGpuError::Adapter(
                                "selected GPU adapter has no renderable surface format".into(),
                            ));
                        }
                        adapter
                    }
                };
                let device = request_shared_device(&adapter, required_features)
                    .await
                    .map_err(|error| HostedGpuError::Device(error.to_string()))?;
                let info = GpuAdapterInfo::from_wgpu(&adapter.get_info());
                let active = index.or_else(|| infos.iter().position(|known| *known == info));
                (active, adapter, device)
            }
        };
        let capabilities = surface.get_capabilities(&adapter);
        let format = preferred_surface_format_for_adapter(&adapter, &capabilities.formats)
            .ok_or_else(|| {
                nana_diagnostics::event!(nana_diagnostics::framework::gpu::ADAPTER_INCOMPATIBLE);
                HostedGpuError::SurfaceHasNoFormats
            })?;
        let gpu = __framework::adopt_tracking_loss(adapter, device, queue);
        let mut shared = HostedGpuShared::adopt(instance, gpu);
        shared.choice = choice;
        shared.backend_policy = backend_policy;
        shared.selection = Arc::new(GpuAdapterSelection {
            adapters: infos,
            active,
            requested_missing,
        });
        Ok(AcquiredDevice {
            shared,
            surface,
            capabilities,
            format,
        })
    }
}

/// The backend `policy` has adapter selection prefer. On Windows a
/// composition-capable process runs on DX12, the backend DirectComposition
/// visuals and native video input share, even before any window is composed.
fn preferred_backend(policy: crate::GpuBackendPolicy) -> Option<wgpu::Backend> {
    (cfg!(target_os = "windows") && policy.wants_composition()).then_some(wgpu::Backend::Dx12)
}

/// `adapters` narrowed to those on `preferred` when any of them is; all of
/// them otherwise, so a machine without that backend still gets a device.
fn on_preferred_backend<A>(
    adapters: Vec<A>,
    preferred: Option<wgpu::Backend>,
    backend: impl Fn(&A) -> wgpu::Backend,
) -> Vec<A> {
    match preferred {
        Some(preferred) if adapters.iter().any(|adapter| backend(adapter) == preferred) => adapters
            .into_iter()
            .filter(|adapter| backend(adapter) == preferred)
            .collect(),
        _ => adapters,
    }
}

/// The one shared device on `adapter`.
async fn request_shared_device(
    adapter: &wgpu::Adapter,
    required_features: wgpu::Features,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("NanaUI hosted shared device"),
            required_features: shared_device_features(adapter.features(), required_features)
                | nana_gpu::__framework::precompiled_stage_features(adapter),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        })
        .await
}

/// What acquiring a surface texture produced.
pub(crate) enum SurfaceFrame {
    Ready(wgpu::SurfaceTexture),
    Retry,
    Skipped,
}

/// [`SurfaceFrame`] for hosts that drive a surface themselves.
#[cfg(feature = "wgpu-interop")]
pub enum HostedSurfaceFrame {
    Ready(wgpu::SurfaceTexture),
    Retry,
    Skipped,
}

#[cfg(feature = "wgpu-interop")]
impl From<SurfaceFrame> for HostedSurfaceFrame {
    fn from(frame: SurfaceFrame) -> Self {
        match frame {
            SurfaceFrame::Ready(texture) => Self::Ready(texture),
            SurfaceFrame::Retry => Self::Retry,
            SurfaceFrame::Skipped => Self::Skipped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedGpuError {
    SurfaceCreation(String),
    Adapter(String),
    Device(String),
    SurfaceHasNoFormats,
    SurfaceFormatChanged { expected: GpuTextureFormat },
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

/// Optional capabilities shared content producers on the host's device may use
/// when the adapter offers them. Enabling supported optional features does not
/// alter UI pipelines; unsupported features remain off, including after device
/// recovery.
const OPTIONAL_SHARED_DEVICE_FEATURES: wgpu::Features = wgpu::Features::DUAL_SOURCE_BLENDING
    .union(wgpu::Features::TEXTURE_COMPRESSION_BC)
    .union(wgpu::Features::TEXTURE_COMPRESSION_ASTC)
    .union(wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER)
    .union(wgpu::Features::SHADER_F16)
    .union(wgpu::Features::SUBGROUP)
    .union(wgpu::Features::TEXTURE_BINDING_ARRAY)
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
    // The device's shader and pipeline cache, which the host may
    // persist (`nana_gpu::GpuContext::load_pipeline_cache`).
    .union(wgpu::Features::PIPELINE_CACHE);

/// Optional advanced platform capabilities that may be toggled off via
/// `NANA_WGPU_DISABLE_FEATURES` (comma-separated: `rov`, `f16`, `subgroup`,
/// `bindless`, `non_uniform_indexing`, `all`) for A/B testing or driver
/// workarounds without breaking device creation.
const ABLATABLE_ADVANCED_FEATURES: wgpu::Features =
    wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER
        .union(wgpu::Features::SHADER_F16)
        .union(wgpu::Features::SUBGROUP)
        .union(wgpu::Features::TEXTURE_BINDING_ARRAY)
        .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING);

fn parse_disabled_wgpu_features(raw: &str) -> wgpu::Features {
    let mut disabled = wgpu::Features::empty();
    for token in raw.split([',', ';', ' ']) {
        match token.trim().to_ascii_lowercase().as_str() {
            "" => {}
            "rov"
            | "rasterizer_order"
            | "rasterizer-order"
            | "texture_storage_rasterizer_order" => {
                disabled |= wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER;
            }
            "f16" | "shader_f16" | "shader-f16" => {
                disabled |= wgpu::Features::SHADER_F16;
            }
            "subgroup" | "subgroups" => {
                disabled |= wgpu::Features::SUBGROUP;
            }
            "bindless" | "binding_array" | "binding-array" | "texture_binding_array" => {
                disabled |= wgpu::Features::TEXTURE_BINDING_ARRAY
                    | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
            }
            "non_uniform_indexing" | "non-uniform-indexing" => {
                disabled |=
                    wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
            }
            "all" => {
                disabled |= ABLATABLE_ADVANCED_FEATURES;
            }
            _ => {}
        }
    }
    disabled
}

fn disabled_wgpu_features_from_env() -> wgpu::Features {
    std::env::var("NANA_WGPU_DISABLE_FEATURES")
        .ok()
        .map(|raw| parse_disabled_wgpu_features(&raw))
        .unwrap_or_else(wgpu::Features::empty)
}

fn shared_device_features_with_disabled(
    available: wgpu::Features,
    requested: wgpu::Features,
    disabled: wgpu::Features,
) -> wgpu::Features {
    (available & (requested | OPTIONAL_SHARED_DEVICE_FEATURES)) & !disabled
}

/// Shared content producers on the host's device need compressed texture
/// sampling and optional platform capabilities (ROV, f16, subgroups, binding
/// arrays) when the adapter offers them. `NANA_WGPU_DISABLE_FEATURES` can
/// mask off advanced capabilities for A/B testing or driver workarounds.
fn shared_device_features(available: wgpu::Features, requested: wgpu::Features) -> wgpu::Features {
    shared_device_features_with_disabled(available, requested, disabled_wgpu_features_from_env())
}

/// The instance every hosted device is created on: `backends`, and on DX12
/// the shader compiler [`dx12_shader_compiler`] chooses.
fn instance_descriptor(backends: wgpu::Backends) -> wgpu::InstanceDescriptor {
    #[allow(unused_mut)]
    let mut descriptor = wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    #[cfg(target_os = "windows")]
    {
        descriptor.backend_options.dx12.shader_compiler = dx12_shader_compiler(
            wgpu::Dx12Compiler::from_env(),
            std::env::current_exe().ok().as_deref(),
        );
    }
    descriptor
}

/// DX12's shader compiler. Every scene pipeline is compiled at startup, and
/// DXC does that roughly twice as fast as FXC (and unlocks SM 6.x capabilities
/// such as `SHADER_F16` and `SUBGROUP`), but DXC is a DLL pair (`dxcompiler.dll`
/// + `dxil.dll`) the application ships: a `dxcompiler.dll` beside the executable
/// or under its nested `runtime/` directory is used by its full path, so neither
/// `PATH` nor the working directory can substitute another one. Without it the
/// compiler is FXC, which Windows always has. `WGPU_DX12_COMPILER` still decides
/// when it is set.
#[cfg(target_os = "windows")]
fn dx12_shader_compiler(
    from_env: Option<wgpu::Dx12Compiler>,
    exe: Option<&std::path::Path>,
) -> wgpu::Dx12Compiler {
    if let Some(compiler) = from_env {
        return compiler;
    }
    let Some(dir) = exe.and_then(std::path::Path::parent) else {
        return wgpu::Dx12Compiler::Fxc;
    };
    [
        dir.join("dxcompiler.dll"),
        dir.join("runtime").join("dxcompiler.dll"),
    ]
    .into_iter()
    .find(|dll| dll.is_file())
    .and_then(|dll| dll.to_str().map(str::to_owned))
    .map_or(wgpu::Dx12Compiler::Fxc, |dxc_path| {
        wgpu::Dx12Compiler::DynamicDxc { dxc_path }
    })
}

fn adapter_backend(backend: wgpu::Backend) -> GpuBackend {
    match backend {
        wgpu::Backend::Noop => GpuBackend::Noop,
        wgpu::Backend::Vulkan => GpuBackend::Vulkan,
        wgpu::Backend::Metal => GpuBackend::Metal,
        wgpu::Backend::Dx12 => GpuBackend::Dx12,
        wgpu::Backend::Gl => GpuBackend::Gl,
        wgpu::Backend::BrowserWebGpu => GpuBackend::BrowserWebGpu,
    }
}

fn adapter_supports_render_format(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> bool {
    if format.is_depth_stencil_format() {
        return false;
    }
    let features = adapter.features();
    if !features.contains(format.required_features()) {
        return false;
    }
    let downlevel = adapter.get_downlevel_capabilities().flags;
    let use_webgpu_table = !features
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        && downlevel.contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
    let format_features = if use_webgpu_table {
        format.guaranteed_format_features(features)
    } else {
        adapter.get_texture_format_features(format)
    };
    format_features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

fn adapter_capability_summary(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'_>,
) -> AdapterCapabilitySummary {
    let info = adapter.get_info();
    let downlevel = adapter.get_downlevel_capabilities();
    let surface_caps = surface.get_capabilities(adapter);
    let has_renderable_surface_format = surface_caps
        .formats
        .iter()
        .copied()
        .any(|format| adapter_supports_render_format(adapter, format));
    AdapterCapabilitySummary {
        backend: adapter_backend(info.backend),
        supports_view_formats: downlevel.flags.contains(wgpu::DownlevelFlags::VIEW_FORMATS),
        has_renderable_surface_format,
    }
}

fn preferred_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

fn preferred_surface_format_for_adapter(
    adapter: &wgpu::Adapter,
    formats: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    let renderable: Vec<wgpu::TextureFormat> = formats
        .iter()
        .copied()
        .filter(|&format| adapter_supports_render_format(adapter, format))
        .collect();
    let picked = preferred_surface_format(&renderable);
    if picked.is_some() && picked != preferred_surface_format(formats) {
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::FORMAT_FALLBACK);
    }
    picked
}

#[cfg(test)]
pub(crate) use crate::surface_profile::resolve_surface_profile_internal;
pub use crate::surface_profile::surface_profile_changed;

fn resolve_tone_map_headroom(
    profile: ResolvedSurfaceProfile,
    hdr_info: &wgpu::DisplayHdrInfo,
) -> f32 {
    let default_headroom = if profile.is_hdr() {
        crate::ScenePresentationParameters::default().headroom()
    } else {
        1.0
    };
    hdr_info
        .tone_map_headroom()
        .filter(|value| value.is_finite() && *value >= 1.0)
        .unwrap_or(default_headroom)
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

/// Maximum frame latency for the next frame. A live resize takes one extra
/// frame of queue headroom. So does a continuously redrawing DX12 window: at
/// latency 1 its acquire waits out a whole refresh on the
/// frame-latency waitable whenever DWM holds the previous present, which puts
/// a click about 30 ms behind instead of under 10 (`docs/reference/gpu.md` §6).
/// Idle and on-demand windows keep latency 1.
fn steady_frame_latency(live: bool, continuous: bool, backend: GpuBackend) -> u32 {
    if live || (continuous && backend == GpuBackend::Dx12) {
        2
    } else {
        1
    }
}

/// Present configuration a surface must move to for the next frame, given
/// that frame's [`steady_frame_latency`], or `None` when the current
/// configuration already matches. Frames use the surface's preferred
/// unblocked mode (`Mailbox`, then `Immediate`, then `AutoVsync`). Resolving
/// this once per frame keeps steady frames reconfigure-free.
fn live_resize_policy_change(
    current_present_mode: wgpu::PresentMode,
    current_frame_latency: u32,
    live_present_mode: wgpu::PresentMode,
    frame_latency: u32,
) -> Option<(wgpu::PresentMode, u32)> {
    let present_mode = live_present_mode;
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
        alpha_mode_needs_surface_recreate, live_resize_policy_change, preferred_alpha_mode,
        preferred_live_present_mode, preferred_surface_format, resolve_surface_profile_internal,
        steady_frame_latency, surface_profile_changed, surface_size_changed,
    };

    #[test]
    fn shared_device_enables_only_supported_compression_and_requested_features() {
        use wgpu::Features as F;
        let optional = F::DUAL_SOURCE_BLENDING
            | F::TEXTURE_COMPRESSION_BC
            | F::TEXTURE_COMPRESSION_ASTC
            | F::TEXTURE_STORAGE_RASTERIZER_ORDER
            | F::SHADER_F16
            | F::SUBGROUP
            | F::TEXTURE_BINDING_ARRAY
            | F::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | F::PIPELINE_CACHE;
        for available in [
            F::empty(),
            F::TEXTURE_COMPRESSION_BC,
            F::TEXTURE_COMPRESSION_ASTC,
            F::TEXTURE_STORAGE_RASTERIZER_ORDER,
            F::SHADER_F16,
            F::SUBGROUP,
            F::TEXTURE_BINDING_ARRAY,
            F::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
            F::PIPELINE_CACHE,
            optional,
        ] {
            assert_eq!(
                super::shared_device_features_with_disabled(available, F::empty(), F::empty()),
                available
            );
            assert_eq!(
                super::shared_device_features_with_disabled(
                    available | F::TIMESTAMP_QUERY,
                    F::TIMESTAMP_QUERY,
                    F::empty(),
                ),
                available | F::TIMESTAMP_QUERY,
            );
            assert_eq!(
                super::shared_device_features_with_disabled(
                    available,
                    F::TIMESTAMP_QUERY,
                    F::empty(),
                ),
                available
            );
        }
        assert_eq!(
            super::shared_device_features_with_disabled(F::TIMESTAMP_QUERY, F::empty(), F::empty()),
            F::empty()
        );
    }

    #[test]
    fn env_ablation_override_masks_individual_and_all_advanced_features() {
        use wgpu::Features as F;
        let all_optional = super::OPTIONAL_SHARED_DEVICE_FEATURES;

        let disabled = super::parse_disabled_wgpu_features("rov, f16, unknown-token");
        assert_eq!(
            disabled,
            F::TEXTURE_STORAGE_RASTERIZER_ORDER | F::SHADER_F16
        );
        let enabled =
            super::shared_device_features_with_disabled(all_optional, F::empty(), disabled);
        assert!(!enabled.contains(F::TEXTURE_STORAGE_RASTERIZER_ORDER));
        assert!(!enabled.contains(F::SHADER_F16));
        assert!(enabled.contains(F::SUBGROUP));
        assert!(enabled.contains(F::TEXTURE_BINDING_ARRAY));
        assert!(enabled.contains(F::DUAL_SOURCE_BLENDING));

        let bindless_off = super::parse_disabled_wgpu_features("SUBGROUP;bindless");
        assert_eq!(
            bindless_off,
            F::SUBGROUP
                | F::TEXTURE_BINDING_ARRAY
                | F::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
        );

        let all_off = super::parse_disabled_wgpu_features("all");
        let after_all =
            super::shared_device_features_with_disabled(all_optional, F::empty(), all_off);
        assert_eq!(
            after_all,
            F::DUAL_SOURCE_BLENDING
                | F::TEXTURE_COMPRESSION_BC
                | F::TEXTURE_COMPRESSION_ASTC
                | F::PIPELINE_CACHE
        );
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

    #[test]
    fn capability_table_resolves_sdr_p3_and_hdr_with_deterministic_fallbacks() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
                Row {
                    format: F::Bgra8Unorm,
                    color_spaces: S::DISPLAY_P3,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert_eq!(
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::Sdr, &caps, false)
                .unwrap()
                .color_space,
            C::Srgb
        );
        assert_eq!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::WideGamut,
                &caps,
                false
            )
            .unwrap()
            .color_space,
            C::DisplayP3
        );
        assert_eq!(
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::Hdr, &caps, false)
                .unwrap()
                .color_space,
            C::ExtendedSrgb
        );
        let no_hdr = wgpu::SurfaceCapabilities {
            format_capabilities: vec![Row {
                format: F::Bgra8Unorm,
                color_spaces: S::DISPLAY_P3,
            }],
            formats: vec![F::Bgra8Unorm],
            ..caps
        };
        assert_eq!(
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::Hdr, &no_hdr, false)
                .unwrap()
                .color_space,
            C::DisplayP3
        );
        let p3_float = wgpu::SurfaceCapabilities {
            formats: vec![F::Bgra8UnormSrgb, F::Rgba16Float],
            format_capabilities: vec![
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::DISPLAY_P3,
                },
            ],
            ..no_hdr
        };
        let p3 = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::WideGamut,
            &p3_float,
            false,
        )
        .unwrap();
        assert_eq!(p3.format, F::Rgba16Float);
        assert_eq!(p3.color_space, C::DisplayP3);
        assert_eq!(p3.fallback, None);

        let hdr_p3 = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgba16Float],
            format_capabilities: vec![Row {
                format: F::Rgba16Float,
                color_spaces: S::EXTENDED_DISPLAY_P3,
            }],
            ..p3_float
        };
        let hdr_p3 =
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::Hdr, &hdr_p3, false)
                .unwrap();
        assert_eq!(hdr_p3.format, F::Rgba16Float);
        assert_eq!(hdr_p3.color_space, C::ExtendedDisplayP3);
        assert_eq!(
            hdr_p3.fallback,
            Some(crate::SurfaceProfileFallback::CapabilityUnavailable)
        );
    }

    #[test]
    fn explicit_pq_and_hlg_require_matching_surface_capabilities() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm, F::Rgba16Float, F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Rgb10a2Unorm,
                    color_spaces: S::BT2100_PQ | S::BT2100_HLG,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB_LINEAR,
                },
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let pq =
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::HdrPq, &caps, false)
                .unwrap();
        assert_eq!(pq.format, F::Rgb10a2Unorm);
        assert_eq!(pq.color_space, C::Bt2100Pq);
        assert_eq!(pq.fallback, None);

        let hlg = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::HdrHlg,
            &caps,
            false,
        )
        .unwrap();
        assert_eq!(hlg.format, F::Rgb10a2Unorm);
        assert_eq!(hlg.color_space, C::Bt2100Hlg);
        assert_eq!(hlg.fallback, None);

        // PQ and HLG are opt-in; ordinary Hdr remains the scRGB path.
        let hdr =
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::Hdr, &caps, false)
                .unwrap();
        assert_eq!(hdr.format, F::Rgba16Float);
        assert_eq!(hdr.color_space, C::ExtendedSrgbLinear);
    }

    #[test]
    fn explicit_pq_hlg_fallback_is_hdr_first_and_excludes_transparent_output() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let hdr_and_sdr = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm, F::Rgba16Float, F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Rgb10a2Unorm,
                    // HLG is advertised, but this fixture intentionally has
                    // no PQ pair so an explicit PQ request must take the
                    // extended-linear fallback below.
                    color_spaces: S::BT2100_HLG,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB_LINEAR,
                },
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let fallback = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::HdrPq,
            &hdr_and_sdr,
            false,
        )
        .unwrap();
        assert_eq!(fallback.color_space, C::ExtendedSrgbLinear);
        assert_eq!(fallback.format, F::Rgba16Float);
        assert_eq!(
            fallback.fallback,
            Some(crate::SurfaceProfileFallback::CapabilityUnavailable)
        );

        let transparent = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::HdrPq,
            &hdr_and_sdr,
            true,
        )
        .unwrap();
        assert_eq!(transparent.color_space, C::ExtendedSrgbLinear);
        assert_eq!(transparent.format, F::Rgba16Float);

        // A full-alpha fp16 PQ target is still valid for a transparent
        // surface when the provider advertises that exact pair; only the
        // two-alpha-bit 10-bit target above is excluded.
        let fp16_pq = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm, F::Rgba16Float, F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Rgb10a2Unorm,
                    color_spaces: S::BT2100_PQ,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::BT2100_PQ,
                },
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let transparent_fp16 = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::HdrPq,
            &fp16_pq,
            true,
        )
        .unwrap();
        assert_eq!(transparent_fp16.color_space, C::Bt2100Pq);
        assert_eq!(transparent_fp16.format, F::Rgba16Float);
        assert_eq!(transparent_fp16.fallback, None);

        let pq_only = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm],
            format_capabilities: vec![Row {
                format: F::Rgb10a2Unorm,
                color_spaces: S::BT2100_PQ,
            }],
            ..hdr_and_sdr
        };
        assert!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::HdrPq,
                &pq_only,
                true,
            )
            .is_none()
        );
    }

    #[test]
    fn split_capability_rows_for_one_format_are_union_matched() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm],
            format_capabilities: vec![
                Row {
                    format: F::Rgb10a2Unorm,
                    color_spaces: S::BT2100_PQ,
                },
                Row {
                    format: F::Rgb10a2Unorm,
                    color_spaces: S::BT2100_HLG,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert_eq!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::HdrPq,
                &caps,
                false,
            )
            .unwrap()
            .color_space,
            C::Bt2100Pq
        );
        assert_eq!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::HdrHlg,
                &caps,
                false,
            )
            .unwrap()
            .color_space,
            C::Bt2100Hlg
        );
    }

    #[test]
    fn incomplete_capability_table_keeps_advertised_sdr_fallback() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            // Keep a valid default-SDR format in the legacy list even though
            // this synthetic table only contains the explicit HDR row.
            formats: vec![F::Bgra8UnormSrgb, F::Rgba16Float],
            format_capabilities: vec![Row {
                format: F::Rgba16Float,
                // The row is HDR, but only for HLG; this is intentionally
                // incomplete for an explicit PQ request.
                color_spaces: S::BT2100_HLG,
            }],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let resolved =
            resolve_surface_profile_internal(crate::SurfacePresentationPolicy::HdrPq, &caps, false)
                .unwrap();
        assert_eq!(resolved.format, F::Bgra8UnormSrgb);
        assert_eq!(resolved.color_space, C::Srgb);
        assert_eq!(
            resolved.fallback,
            Some(crate::SurfaceProfileFallback::CapabilityUnavailable)
        );
    }

    #[test]
    fn sdr_fallback_does_not_relabel_hdr_format_with_srgb_space() {
        use wgpu::{SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row, TextureFormat as F};
        let malformed = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgba16Float],
            format_capabilities: vec![Row {
                format: F::Rgba16Float,
                color_spaces: S::SRGB,
            }],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::Sdr,
                &malformed,
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn empty_capability_table_never_relabels_hdr_only_formats_as_srgb() {
        use wgpu::{SurfaceColorSpace as C, TextureFormat as F};
        let hdr_only = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgba16Float, F::Rgb10a2Unorm],
            format_capabilities: vec![],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert!(
            resolve_surface_profile_internal(
                crate::SurfacePresentationPolicy::Sdr,
                &hdr_only,
                false,
            )
            .is_none()
        );

        let mut with_sdr = hdr_only;
        with_sdr.formats.insert(0, F::Bgra8UnormSrgb);
        let resolved = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::Sdr,
            &with_sdr,
            false,
        )
        .unwrap();
        assert_eq!(resolved.format, F::Bgra8UnormSrgb);
        assert_eq!(resolved.color_space, C::Srgb);
    }

    #[test]
    fn equal_profiles_do_not_reconfigure() {
        use wgpu::{SurfaceColorSpace as C, TextureFormat as F};
        let p = crate::ResolvedSurfaceProfile {
            requested: crate::SurfacePresentationPolicy::Auto,
            format: F::Bgra8UnormSrgb,
            color_space: C::Srgb,
            fallback: None,
        };
        assert!(!surface_profile_changed(p, p));
        assert!(surface_profile_changed(
            p,
            crate::ResolvedSurfaceProfile {
                color_space: C::DisplayP3,
                ..p
            }
        ));
    }

    /// DXC is the one shipped beside the executable (or under its nested
    /// `runtime/` directory), named by full path; without it the compiler is
    /// FXC, whatever `PATH` holds, and an explicit `WGPU_DX12_COMPILER` wins
    /// over both.
    #[cfg(target_os = "windows")]
    #[test]
    fn dx12_uses_the_dxc_shipped_beside_the_executable() {
        use super::dx12_shader_compiler;

        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nana-dxc-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.exe");
        assert!(matches!(
            dx12_shader_compiler(None, Some(&exe)),
            wgpu::Dx12Compiler::Fxc
        ));

        // Nested `runtime/dxcompiler.dll` is discovered when no top-level DLL exists.
        let runtime_dir = dir.join("runtime");
        std::fs::create_dir_all(&runtime_dir).unwrap();
        let runtime_dll = runtime_dir.join("dxcompiler.dll");
        std::fs::write(&runtime_dll, b"").unwrap();
        match dx12_shader_compiler(None, Some(&exe)) {
            wgpu::Dx12Compiler::DynamicDxc { dxc_path } => {
                assert_eq!(std::path::Path::new(&dxc_path), runtime_dll);
            }
            other => panic!("expected runtime DXC, got {other:?}"),
        }

        // A DLL directly beside the executable takes precedence over `runtime/`.
        let dll = dir.join("dxcompiler.dll");
        std::fs::write(&dll, b"").unwrap();
        match dx12_shader_compiler(None, Some(&exe)) {
            wgpu::Dx12Compiler::DynamicDxc { dxc_path } => {
                assert_eq!(std::path::Path::new(&dxc_path), dll);
            }
            other => panic!("expected the shipped DXC, got {other:?}"),
        }
        assert!(matches!(
            dx12_shader_compiler(Some(wgpu::Dx12Compiler::Fxc), Some(&exe)),
            wgpu::Dx12Compiler::Fxc
        ));
        assert!(matches!(
            dx12_shader_compiler(None, None),
            wgpu::Dx12Compiler::Fxc
        ));
        std::fs::remove_dir_all(&dir).unwrap();
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

        let vulkan = GpuBootstrap::probe(Some(wgpu::Backend::Vulkan));
        assert!(!vulkan.composition_available());

        let dx12 = GpuBootstrap::probe(Some(wgpu::Backend::Dx12));
        assert_eq!(dx12.composition_available(), cfg!(target_os = "windows"));

        // A process that never asked is offered nothing.
        let plain = GpuBootstrap::plain();
        assert!(!plain.composition_available());
    }

    /// A composition-capable process runs on DX12 whenever an adapter there
    /// can present the window, even when the first enumerated one is Vulkan
    /// and no window is composed; without such an adapter it keeps the rest.
    #[test]
    fn a_composition_capable_process_picks_its_adapter_from_dx12() {
        use super::{on_preferred_backend, preferred_backend};
        use crate::GpuBackendPolicy;
        use wgpu::Backend::{Dx12, Vulkan};

        let preferred = preferred_backend(GpuBackendPolicy::CompositionCapable);
        assert_eq!(preferred, cfg!(target_os = "windows").then_some(Dx12));
        assert_eq!(preferred_backend(GpuBackendPolicy::Plain), None);

        let adapters = vec![
            ("dgpu", Vulkan),
            ("igpu", Vulkan),
            ("dgpu", Dx12),
            ("igpu", Dx12),
        ];
        let narrowed = on_preferred_backend(adapters.clone(), Some(Dx12), |a| a.1);
        assert_eq!(narrowed, vec![("dgpu", Dx12), ("igpu", Dx12)]);
        assert_eq!(
            on_preferred_backend(adapters.clone(), None, |a| a.1),
            adapters
        );

        let vulkan_only = vec![("dgpu", Vulkan)];
        assert_eq!(
            on_preferred_backend(vulkan_only.clone(), Some(Dx12), |a| a.1),
            vulkan_only
        );
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
    fn continuous_dx12_and_live_resize_queue_one_more_frame() {
        use nana_gpu::GpuBackend::{Dx12, Metal, Vulkan};

        // Idle and on-demand windows keep latency 1 on every backend.
        for backend in [Dx12, Vulkan, Metal] {
            assert_eq!(steady_frame_latency(false, false, backend), 1);
            assert_eq!(steady_frame_latency(true, false, backend), 2);
        }
        // Only a continuously redrawing DX12 window queues the extra frame.
        assert_eq!(steady_frame_latency(false, true, Dx12), 2);
        assert_eq!(steady_frame_latency(false, true, Vulkan), 1);
        assert_eq!(steady_frame_latency(false, true, Metal), 1);
        // Entering continuous redraw reconfigures once, to latency 2.
        assert_eq!(
            live_resize_policy_change(
                wgpu::PresentMode::Mailbox,
                1,
                wgpu::PresentMode::Mailbox,
                steady_frame_latency(false, true, Dx12),
            ),
            Some((wgpu::PresentMode::Mailbox, 2))
        );
        // A continuous window leaving a live resize stays at 2: no
        // reconfigure when the gesture ends.
        assert_eq!(
            live_resize_policy_change(
                wgpu::PresentMode::Mailbox,
                2,
                wgpu::PresentMode::Mailbox,
                steady_frame_latency(false, true, Dx12),
            ),
            None
        );
    }

    #[test]
    fn live_resize_policy_reconfigures_once_per_session_and_back() {
        use wgpu::PresentMode;

        // Entering live from a stale AutoVsync configuration switches to the
        // cached unblocked mode and extra latency before the first moved frame.
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::Mailbox, 2),
            Some((PresentMode::Mailbox, 2))
        );
        // Steady live frames keep the same configuration, so prepare_frame
        // never reconfigures mid-gesture.
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 2, PresentMode::Mailbox, 2),
            None
        );
        // Leaving live keeps Mailbox and restores latency 1 exactly once.
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 2, PresentMode::Mailbox, 1),
            Some((PresentMode::Mailbox, 1))
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::Mailbox, 1, PresentMode::Mailbox, 1),
            None
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::Mailbox, 1),
            Some((PresentMode::Mailbox, 1))
        );
        // A surface without an unblocked present mode keeps AutoVsync; only
        // the latency changes, once per side of the gesture.
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 1, PresentMode::AutoVsync, 2),
            Some((PresentMode::AutoVsync, 2))
        );
        assert_eq!(
            live_resize_policy_change(PresentMode::AutoVsync, 2, PresentMode::AutoVsync, 1),
            Some((PresentMode::AutoVsync, 1))
        );
    }

    #[test]
    fn transparent_pq_and_hlg_report_transparency_incompatible_and_wide_gamut_fallback_keeps_float_working()
     {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgb10a2Unorm, F::Rgba16Float, F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Rgb10a2Unorm,
                    color_spaces: S::BT2100_PQ | S::BT2100_HLG,
                },
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB_LINEAR,
                },
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PreMultiplied,
            ],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        for policy in [
            crate::SurfacePresentationPolicy::HdrPq,
            crate::SurfacePresentationPolicy::HdrHlg,
        ] {
            let resolved = resolve_surface_profile_internal(policy, &caps, true).unwrap();
            assert_eq!(resolved.color_space, C::ExtendedSrgbLinear);
            assert_eq!(resolved.format, F::Rgba16Float);
            assert_eq!(
                resolved.fallback,
                Some(crate::SurfaceProfileFallback::TransparencyIncompatible)
            );
        }

        let sdr_only = wgpu::SurfaceCapabilities {
            formats: vec![F::Bgra8UnormSrgb],
            format_capabilities: vec![Row {
                format: F::Bgra8UnormSrgb,
                color_spaces: S::SRGB,
            }],
            ..caps
        };
        let wide_fallback = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::WideGamut,
            &sdr_only,
            false,
        )
        .unwrap();
        assert_eq!(wide_fallback.color_space, C::Srgb);
        assert_eq!(
            wide_fallback.fallback,
            Some(crate::SurfaceProfileFallback::CapabilityUnavailable)
        );
        assert_eq!(
            wide_fallback.scene_profile().working_format(),
            nana_gpu::GpuTextureFormat::RGBA16_FLOAT
        );

        let hdr_profile = resolve_surface_profile_internal(
            crate::SurfacePresentationPolicy::Hdr,
            &wgpu::SurfaceCapabilities {
                formats: vec![F::Rgba16Float],
                format_capabilities: vec![Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB_LINEAR,
                }],
                present_modes: vec![wgpu::PresentMode::Fifo],
                alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
                usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
            },
            false,
        )
        .unwrap();
        assert_eq!(
            super::resolve_tone_map_headroom(hdr_profile, &wgpu::DisplayHdrInfo::default()),
            16.0
        );
        assert_eq!(
            super::resolve_tone_map_headroom(wide_fallback, &wgpu::DisplayHdrInfo::default()),
            1.0
        );
    }

    #[test]
    fn surface_profile_negotiation_skips_unrenderable_formats() {
        use wgpu::{
            SurfaceColorSpace as C, SurfaceColorSpaces as S, SurfaceFormatCapabilities as Row,
            TextureFormat as F,
        };
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![F::Rgba16Float, F::Bgra8UnormSrgb],
            format_capabilities: vec![
                Row {
                    format: F::Rgba16Float,
                    color_spaces: S::EXTENDED_SRGB_LINEAR,
                },
                Row {
                    format: F::Bgra8UnormSrgb,
                    color_spaces: S::SRGB,
                },
            ],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let resolved = crate::surface_profile::resolve_surface_profile_with_format_support(
            crate::SurfacePresentationPolicy::Hdr,
            &caps,
            false,
            |format| format != F::Rgba16Float,
        )
        .unwrap();
        assert_eq!(resolved.format, F::Bgra8UnormSrgb);
        assert_eq!(resolved.color_space, C::Srgb);
        assert_eq!(
            resolved.fallback,
            Some(crate::SurfaceProfileFallback::CapabilityUnavailable)
        );
        assert!(
            crate::surface_profile::resolve_surface_profile_with_format_support(
                crate::SurfacePresentationPolicy::Auto,
                &caps,
                false,
                |_| false,
            )
            .is_none()
        );
    }
}
