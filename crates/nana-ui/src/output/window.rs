//! Window presenter adapter for the direct-surface output path.
//!
//! This adapter is deliberately a thin capability boundary. It never creates
//! an intermediate texture and it does not run target negotiation while a
//! frame is being encoded. Hosts refresh its requirements after resize,
//! surface rebind, or profile/device changes, then pass the descriptor to
//! [`super::RenderTargetPlanner`].

#[cfg(feature = "hosted")]
use crate::hosted_context::{HostedGpuShared, HostedGpuSurface};
#[cfg(feature = "hosted")]
use crate::{AlphaEncoding, PresenterCapabilities, RenderTargetRequirements, SurfaceAlphaMode};
#[cfg(feature = "hosted")]
use nana_gpu::GpuTextureUsages;

/// A borrowed native-window presenter. The window surface remains owned by
/// the host; this value only exposes its direct target capabilities and the
/// lifecycle operations that are already surface-local.
#[cfg(feature = "hosted")]
pub struct WindowPresenter<'a> {
    shared: &'a HostedGpuShared,
    surface: &'a mut HostedGpuSurface,
    requirements: RenderTargetRequirements,
}

#[cfg(feature = "hosted")]
impl<'a> WindowPresenter<'a> {
    /// Create a presenter and snapshot the current target requirements.
    /// Reconfiguration is explicit through [`Self::refresh_requirements`].
    pub fn new(shared: &'a HostedGpuShared, surface: &'a mut HostedGpuSurface) -> Self {
        let requirements = target_requirements(surface);
        Self {
            shared,
            surface,
            requirements,
        }
    }

    /// The cached descriptor passed to the target planner. Calling this has no
    /// surface or GPU work.
    pub const fn requirements(&self) -> RenderTargetRequirements {
        self.requirements
    }

    /// Direct window surfaces are intentionally the only path this adapter
    /// advertises. Persistent/exportable output belongs to ExternalSurface.
    pub const fn capabilities(&self) -> PresenterCapabilities {
        PresenterCapabilities {
            direct_surface: true,
            same_device_sample: false,
            gpu_copy: false,
            native_share: false,
            cpu_fallback: false,
            allow_cpu_fallback: false,
        }
    }

    /// Refresh the planner descriptor after a structural surface change.
    /// Hosts should call this after resize, profile renegotiation, or device
    /// recreation, rather than every frame.
    pub fn refresh_requirements(&mut self) {
        self.requirements = target_requirements(self.surface);
    }

    /// Apply resize/live-resize policy to the existing surface. This keeps the
    /// direct path and does not allocate a retained target.
    pub fn prepare_frame(&mut self, live_resize: bool) {
        self.shared.prepare_surface_frame(self.surface, live_resize);
        // `prepare_surface_frame` may reconfigure the surface after a native
        // resize. Keep the cached planner descriptor aligned with that
        // boundary; this is only a metadata snapshot, not GPU work.
        self.refresh_requirements();
    }

    /// Reconfigure the existing native surface after a host-side structural
    /// change. The target descriptor should be refreshed afterwards.
    pub fn reconfigure(&mut self) {
        self.shared.resize_surface(self.surface);
        self.refresh_requirements();
    }

    /// Access the host's GPU context used by this presenter.
    pub fn gpu(&self) -> &nana_gpu::GpuContext {
        self.shared.gpu()
    }

    /// Access the borrowed surface for the host's existing acquire/present
    /// loop. No alternate frame path is introduced here.
    pub fn surface(&mut self) -> &mut HostedGpuSurface {
        self.surface
    }
}

#[cfg(feature = "hosted")]
fn target_requirements(surface: &HostedGpuSurface) -> RenderTargetRequirements {
    let profile = surface.profile().scene_profile();
    let alpha = match surface.alpha_mode() {
        SurfaceAlphaMode::Opaque => AlphaEncoding::Linear,
        SurfaceAlphaMode::Auto
        | SurfaceAlphaMode::PreMultiplied
        | SurfaceAlphaMode::PostMultiplied
        | SurfaceAlphaMode::Inherit => AlphaEncoding::Gamma,
    };
    let (width, height) = surface.physical_size();
    RenderTargetRequirements::new([width, height], profile.target_format)
        .with_color_space(profile.color_space)
        .with_alpha(alpha)
        .with_usage(GpuTextureUsages::RENDER_TARGET)
}
