//! A window's content painted a second time into an offscreen output, for a
//! consumer outside the window (a Spout sender, a recorder, a preview).
//!
//! The output reuses the window's own `UiScene`: the host paints it into the
//! output target inside the frame it already records for the window, after the
//! window's paint and before the submit. Nothing is laid out twice. A window
//! that is hidden, minimised or occluded keeps its output going when
//! [`WindowOutputConfig::while_hidden`] is set: the host flushes the document
//! and paints only the output on a GPU-only tick.
//!
//! An unchanged presentation records and submits nothing. The output is BGRA8
//! with sRGB-encoded colour and premultiplied (gamma-space) alpha by default.
//! With [`WindowOutputExport::Native`] on Windows, every frame is also copied
//! into a DX12 shared texture another D3D device can open
//! ([`nana_gpu::NativeExportPool`]); no pixel is read back to the CPU.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use nana_gpu::{
    FrameContext, GpuContext, GpuError, GpuSubmission, GpuTexture, GpuTextureDescriptor,
    GpuTextureFormat, GpuTextureUsages,
};
use nana_ui_platform::{WindowGeometry, WindowId};
use nana_ui_scene::UiScene;

use super::external::{ExternalFramePlan, ExternalPrepare, ExternalSurface, ExternalSurfaceConfig};
use crate::{
    AlphaEncoding, HostTextureRegistry, RenderTargetId, SceneGpuRendererRegistry,
    ScenePaintViewport, ScenePresentationProfile, SceneWgpuPainter,
};

/// Every output is this format: `DXGI_FORMAT_B8G8R8A8_UNORM` holding
/// sRGB-encoded colour.
pub const WINDOW_OUTPUT_FORMAT: GpuTextureFormat = GpuTextureFormat::BGRA8_UNORM;

/// The cadence of a hidden window's output when no `max_fps` is given.
pub const WINDOW_OUTPUT_HIDDEN_FPS: NonZeroU32 = match NonZeroU32::new(30) {
    Some(fps) => fps,
    None => unreachable!(),
};

/// A consumer that keeps the previous native frame this long has stalled;
/// the pool is retired and a fresh one opened.
const NATIVE_STALL: Duration = Duration::from_secs(2);

/// Output size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowOutputExtent {
    /// The window's own physical size; follows every resize.
    MatchWindow,
    /// A fixed size, independent of the window. The window's content is
    /// fitted into it with [`WindowOutputConfig::fit`].
    Fixed { width: u32, height: u32 },
    /// The window's physical size times this factor.
    Scale(f32),
}

/// How the window's content is placed in an output of another aspect ratio.
/// The scene is never laid out again for the output and the painter scales
/// uniformly, so the content is scaled as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowOutputFit {
    /// The whole window, centred; the rest of the output is transparent.
    #[default]
    Contain,
    /// The output is filled, centred; what overflows is cut off.
    Cover,
}

/// How alpha is stored in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowOutputAlpha {
    /// Colour multiplied by alpha in its encoded (sRGB) space: what a
    /// compositor blends, and OBS's "Premultiplied" Spout setting.
    #[default]
    Premultiplied,
    /// Colour divided back out of alpha by one extra pass.
    Straight,
}

/// Where frames go besides the same-device texture every output has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowOutputExport {
    /// [`WindowOutputFrame::texture`] only: a texture on the host's device.
    #[default]
    SameDevice,
    /// Also a DX12 shared texture and fence token
    /// ([`WindowOutputFrame::take_native`]). Needs Windows, a DX12 device and
    /// the `native-export` feature; otherwise the output reports
    /// [`WindowOutputStatus::NativeUnavailable`] and keeps producing
    /// same-device frames.
    Native,
}

/// What a window's output should be. Returned from
/// [`crate::RuntimeProgram::window_output`]; read every frame, so a change takes
/// effect on the next one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowOutputConfig {
    pub extent: WindowOutputExtent,
    pub fit: WindowOutputFit,
    pub alpha: WindowOutputAlpha,
    /// Keep producing frames while the window is hidden, minimised or
    /// occluded (but laid out at its last size).
    pub while_hidden: bool,
    /// Upper bound on produced frames per second. `None` follows the window
    /// (and [`WINDOW_OUTPUT_HIDDEN_FPS`] while it is hidden).
    pub max_fps: Option<NonZeroU32>,
    pub export: WindowOutputExport,
}

impl Default for WindowOutputConfig {
    /// 1920×1080, contained, premultiplied, kept going while hidden, on the
    /// same device.
    fn default() -> Self {
        Self {
            extent: WindowOutputExtent::Fixed {
                width: 1920,
                height: 1080,
            },
            fit: WindowOutputFit::Contain,
            alpha: WindowOutputAlpha::Premultiplied,
            while_hidden: true,
            max_fps: None,
            export: WindowOutputExport::SameDevice,
        }
    }
}

impl WindowOutputConfig {
    pub const fn with_extent(mut self, extent: WindowOutputExtent) -> Self {
        self.extent = extent;
        self
    }

    pub const fn with_fit(mut self, fit: WindowOutputFit) -> Self {
        self.fit = fit;
        self
    }

    pub const fn with_alpha(mut self, alpha: WindowOutputAlpha) -> Self {
        self.alpha = alpha;
        self
    }

    pub const fn while_hidden(mut self, enabled: bool) -> Self {
        self.while_hidden = enabled;
        self
    }

    pub const fn with_max_fps(mut self, fps: Option<NonZeroU32>) -> Self {
        self.max_fps = fps;
        self
    }

    pub const fn with_export(mut self, export: WindowOutputExport) -> Self {
        self.export = export;
        self
    }

    /// The output size for a window of `geometry`, `None` while that is 0.
    pub fn resolve_extent(&self, geometry: &WindowGeometry) -> Option<[u32; 2]> {
        let (width, height) = geometry.physical_size;
        let extent = match self.extent {
            WindowOutputExtent::MatchWindow => [width, height],
            WindowOutputExtent::Fixed { width, height } => [width, height],
            WindowOutputExtent::Scale(scale) => {
                if !(scale.is_finite() && scale > 0.0) || width == 0 || height == 0 {
                    return None;
                }
                [
                    ((width as f32 * scale).round() as u32).max(1),
                    ((height as f32 * scale).round() as u32).max(1),
                ]
            }
        };
        (!extent.contains(&0)).then_some(extent)
    }

    /// Where the window's scene lands in an output of `extent`.
    pub fn viewport(
        &self,
        geometry: &WindowGeometry,
        extent: [u32; 2],
        clear_color: [f32; 4],
    ) -> Option<ScenePaintViewport> {
        let (logical_width, logical_height) = geometry.logical_size;
        if !(logical_width > 0.0 && logical_height > 0.0) || extent.contains(&0) {
            return None;
        }
        let [width, height] = [extent[0] as f32, extent[1] as f32];
        let scale_x = width / logical_width;
        let scale_y = height / logical_height;
        let scale = match self.fit {
            WindowOutputFit::Contain => scale_x.min(scale_y),
            WindowOutputFit::Cover => scale_x.max(scale_y),
        };
        let logical = [width / scale, height / scale];
        Some(ScenePaintViewport {
            logical_size: logical,
            physical_size: extent,
            scale_factor: scale,
            scene_origin: [0.0, 0.0],
            target_origin: [
                (logical[0] - logical_width) * 0.5,
                (logical[1] - logical_height) * 0.5,
            ],
            clear_color,
            clear: true,
        })
    }
}

/// One produced output frame, handed to
/// [`crate::RuntimeProgram::window_output_frame`] right after the host submitted
/// it.
///
/// [`Self::texture`] is on the host's device. Work submitted to that device
/// from now on sees the frame (queue order), so copy or sample it in GPU work
/// you submit before this callback returns, or keep the texture and use it
/// before two more output frames are produced: the output recycles it after
/// that. Nothing here waits for the GPU.
#[derive(Debug)]
pub struct WindowOutputFrame {
    window: WindowId,
    texture: GpuTexture,
    content_revision: u64,
    resource_generation: u64,
    alpha: WindowOutputAlpha,
    #[cfg(all(windows, feature = "native-export"))]
    native: std::cell::RefCell<Option<nana_gpu::NativeFrameToken>>,
}

impl WindowOutputFrame {
    pub fn window(&self) -> WindowId {
        self.window
    }

    pub fn texture(&self) -> &GpuTexture {
        &self.texture
    }

    pub fn extent(&self) -> [u32; 2] {
        let (width, height) = self.texture.size();
        [width, height]
    }

    pub fn format(&self) -> GpuTextureFormat {
        WINDOW_OUTPUT_FORMAT
    }

    pub fn alpha(&self) -> WindowOutputAlpha {
        self.alpha
    }

    /// Increases with every produced frame of this output.
    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }

    /// Changes when the output's textures are recreated (resize, device).
    pub fn resource_generation(&self) -> u64 {
        self.resource_generation
    }

    /// The DX12 shared-texture token of this frame, once. Accept its release
    /// ([`nana_gpu::NativeFrameToken::accept_release`]) and signal it after
    /// the read; a token left here or dropped unaccepted is released by the
    /// host.
    #[cfg(all(windows, feature = "native-export"))]
    pub fn take_native(&self) -> Option<nana_gpu::NativeFrameToken> {
        self.native.borrow_mut().take()
    }
}

/// A change in an output's state, reported once per transition to
/// [`crate::RuntimeProgram::window_output_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowOutputStatus {
    /// Frames are produced at `extent`; `native` says whether they carry a
    /// native token.
    Active { extent: [u32; 2], native: bool },
    /// Native export was asked for and is not available; frames continue on
    /// the same device only.
    NativeUnavailable { reason: String },
    /// The consumer still held the previous native frame, so frames are
    /// produced without a token until it releases it.
    NativeDeferred,
    /// The native textures of `pool_generation` were retired (resize, device
    /// switch, a consumer that stopped releasing). The next token names a new
    /// pool; reopen its handles.
    NativeRetired { pool_generation: u64 },
    /// A frame could not be produced; the window itself is unaffected.
    Failed { reason: String },
    /// The program withdrew the output, or its window closed.
    Stopped,
}

/// Mixed into an output's painter target ids, so they never meet a window's.
const OUTPUT_TARGET_NAMESPACE: u64 = 0xA8A8_0000_0000_0000;

fn target_namespace(window: WindowId) -> u64 {
    OUTPUT_TARGET_NAMESPACE ^ window.0.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17)
}

/// The output's presentation profile: sRGB into a UNORM target, so the
/// painter's final pass writes encoded bytes.
pub(crate) fn output_profile() -> ScenePresentationProfile {
    ScenePresentationProfile::sdr(WINDOW_OUTPUT_FORMAT)
}

/// What the host should do with an output this frame.
pub(crate) enum OutputStep {
    Idle,
    Record {
        plan: ExternalFramePlan,
        viewport: ScenePaintViewport,
    },
}

/// Recorded into a frame, waiting for its submission.
pub(crate) struct RecordedOutput {
    plan: ExternalFramePlan,
    texture: GpuTexture,
    #[cfg(all(windows, feature = "native-export"))]
    native: Option<nana_gpu::StagedNativeFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reported {
    Active { extent: [u32; 2], native: bool },
    NativeUnavailable,
    NativeDeferred,
    Failed,
}

/// The host's state for one window's output.
pub(crate) struct WindowOutputState {
    window: WindowId,
    config: Option<WindowOutputConfig>,
    surface: Option<ExternalSurface>,
    straight: Option<StraightAlpha>,
    #[cfg(all(windows, feature = "native-export"))]
    native: Option<nana_gpu::NativeExportPool>,
    /// Why native export is off, once found out for this device.
    native_unavailable: Option<String>,
    last_frame: Option<Instant>,
    /// A frame was skipped for `max_fps`; produce it at this instant.
    throttled_until: Option<Instant>,
    /// Bumped for every frame whose content may change without the scene's
    /// projection changing (custom renderers, compositor animation).
    volatile: u64,
    reported: Option<Reported>,
    statuses: Vec<WindowOutputStatus>,
    /// Painter targets of retired surfaces, for the host to remove.
    retired_targets: Vec<RenderTargetId>,
}

impl WindowOutputState {
    pub(crate) fn new(window: WindowId) -> Self {
        Self {
            window,
            config: None,
            surface: None,
            straight: None,
            #[cfg(all(windows, feature = "native-export"))]
            native: None,
            native_unavailable: None,
            last_frame: None,
            throttled_until: None,
            volatile: 0,
            reported: None,
            statuses: Vec::new(),
            retired_targets: Vec::new(),
        }
    }

    /// The demand this output adds to its window: its hidden cadence, or the
    /// instant a throttled frame is due.
    pub(crate) fn frame_demand(&self, can_present: bool) -> crate::FrameDemand {
        let Some(config) = self.config else {
            return crate::FrameDemand::OnDemand;
        };
        if !can_present && config.while_hidden {
            return crate::FrameDemand::Continuous(
                config.max_fps.unwrap_or(WINDOW_OUTPUT_HIDDEN_FPS),
            );
        }
        self.throttled_until
            .map_or(crate::FrameDemand::OnDemand, crate::FrameDemand::At)
    }

    /// Apply the program's answer for this frame. Withdrawing the output
    /// releases everything and reports `Stopped`.
    pub(crate) fn set_config(&mut self, config: Option<WindowOutputConfig>) {
        if config.is_none() && self.config.is_some() {
            self.release();
            self.statuses.push(WindowOutputStatus::Stopped);
            self.reported = None;
        }
        if let (Some(old), Some(new)) = (self.config, config)
            && (old.alpha != new.alpha || old.export != new.export)
        {
            // Different passes and pools: start over on the next frame.
            self.release();
        }
        self.config = config;
    }

    /// The device changed: every resource was made on the old one.
    pub(crate) fn device_replaced(&mut self) {
        self.release();
        self.native_unavailable = None;
    }

    fn release(&mut self) {
        if let Some(surface) = self.surface.take() {
            self.retired_targets.extend(surface.target_ids());
        }
        self.straight = None;
        self.retire_native();
        self.throttled_until = None;
    }

    fn retire_native(&mut self) {
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(mut pool) = self.native.take() {
            if let Some(pool_generation) = pool.pool_generation() {
                self.statuses
                    .push(WindowOutputStatus::NativeRetired { pool_generation });
            }
            pool.retire();
        }
    }

    pub(crate) fn take_statuses(&mut self) -> Vec<WindowOutputStatus> {
        std::mem::take(&mut self.statuses)
    }

    pub(crate) fn take_retired_targets(&mut self) -> Vec<RenderTargetId> {
        std::mem::take(&mut self.retired_targets)
    }

    /// Every painter target this output holds, for teardown.
    pub(crate) fn target_ids(&self) -> Vec<RenderTargetId> {
        self.surface
            .iter()
            .flat_map(ExternalSurface::target_ids)
            .chain(self.retired_targets.iter().copied())
            .collect()
    }

    fn report(&mut self, reported: Reported, status: WindowOutputStatus) {
        if self.reported != Some(reported) {
            self.reported = Some(reported);
            self.statuses.push(status);
        }
    }

    fn fail(&mut self, reason: String) {
        if self.reported != Some(Reported::Failed) {
            self.reported = Some(Reported::Failed);
            self.statuses.push(WindowOutputStatus::Failed { reason });
        }
    }

    /// Decide whether this frame produces an output frame. `volatile` says
    /// the content may change without the scene's projection changing.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin(
        &mut self,
        gpu: &GpuContext,
        geometry: &WindowGeometry,
        scene: &UiScene,
        host_revision: u64,
        volatile: bool,
        host_textures: Option<&HostTextureRegistry>,
        clear_color: [f32; 4],
        now: Instant,
    ) -> OutputStep {
        let Some(config) = self.config else {
            return OutputStep::Idle;
        };
        let Some(extent) = config.resolve_extent(geometry) else {
            return OutputStep::Idle;
        };
        let Some(viewport) = config.viewport(geometry, extent, clear_color) else {
            return OutputStep::Idle;
        };
        if let Err(reason) = self.ensure_surface(gpu, extent) {
            self.fail(reason);
            return OutputStep::Idle;
        }
        if volatile {
            self.volatile = self.volatile.wrapping_add(1);
        }
        let revision = host_revision ^ self.volatile.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let surface = self.surface.as_mut().expect("surface was just ensured");
        let plan = match surface.prepare(scene, revision, viewport, host_textures) {
            Ok(ExternalPrepare::Record(plan)) => plan,
            Ok(ExternalPrepare::Reused { .. }) => return OutputStep::Idle,
            Ok(ExternalPrepare::Deferred) => {
                // Every slot is in flight: come back for this content soon
                // rather than leave the output a frame behind.
                self.throttled_until = Some(now + Duration::from_millis(4));
                return OutputStep::Idle;
            }
            Err(error) => {
                self.fail(error.to_string());
                return OutputStep::Idle;
            }
        };
        if let (Some(fps), Some(last)) = (config.max_fps, self.last_frame) {
            let next = last + Duration::from_secs_f64(1.0 / f64::from(fps.get()));
            if now < next {
                self.throttled_until = Some(next);
                return OutputStep::Idle;
            }
        }
        self.throttled_until = None;
        OutputStep::Record { plan, viewport }
    }

    fn ensure_surface(&mut self, gpu: &GpuContext, extent: [u32; 2]) -> Result<(), String> {
        let config = ExternalSurfaceConfig::with_presentation(extent, output_profile())
            .with_copy_source(true);
        let current = self
            .surface
            .as_ref()
            .is_some_and(|surface| surface.config() == config && surface.gpu().same_device(gpu));
        if current {
            return Ok(());
        }
        let mut surface =
            ExternalSurface::with_host_painter(gpu, config, target_namespace(self.window))
                .map_err(|error| error.to_string())?;
        surface.set_alpha_encoding(AlphaEncoding::Gamma);
        if let Some(old) = self.surface.replace(surface) {
            self.retired_targets.extend(old.target_ids());
        }
        // New extent, new device or first frame: conversion target and
        // native textures follow the surface.
        self.straight = None;
        self.retire_native();
        Ok(())
    }

    /// Paint the scene into the output inside `frame`, then the conversion
    /// pass and the native copy when configured.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        step: OutputStep,
        frame: &mut FrameContext,
        painter: &mut SceneWgpuPainter,
        scene: &UiScene,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Option<RecordedOutput> {
        let OutputStep::Record { plan, viewport } = step else {
            return None;
        };
        let config = self.config?;
        let surface = self.surface.as_mut()?;
        // An output is composited by someone else: grayscale text, the alpha
        // encoding compositors blend, and plain SDR parameters.
        painter.set_subpixel_text(None);
        painter.set_presentation_parameters(Default::default());
        if let Err(error) = surface.record(
            &plan,
            frame,
            Some(painter),
            scene,
            viewport,
            host_textures,
            gpu_renderers,
        ) {
            self.fail(error.to_string());
            return None;
        }
        let mut texture = match surface.plan_texture(&plan) {
            Ok(texture) => texture.clone(),
            Err(error) => {
                self.fail(error.to_string());
                return None;
            }
        };
        if config.alpha == WindowOutputAlpha::Straight {
            let gpu = frame.gpu().clone();
            let extent = surface.config().extent;
            let straight = match self.straight.take() {
                Some(straight) if straight.fits(&gpu, extent) => straight,
                _ => match StraightAlpha::new(&gpu, extent) {
                    Ok(straight) => straight,
                    Err(error) => {
                        self.fail(error.to_string());
                        return None;
                    }
                },
            };
            straight.record(frame, &texture);
            texture = straight.target.clone();
            self.straight = Some(straight);
        }
        #[cfg(all(windows, feature = "native-export"))]
        let native = if config.export == WindowOutputExport::Native {
            self.stage_native(frame, &texture)
        } else {
            None
        };
        #[cfg(not(all(windows, feature = "native-export")))]
        if config.export == WindowOutputExport::Native && self.native_unavailable.is_none() {
            let reason = "native export needs Windows and the native-export feature".to_string();
            self.native_unavailable = Some(reason.clone());
            self.report(
                Reported::NativeUnavailable,
                WindowOutputStatus::NativeUnavailable { reason },
            );
        }
        Some(RecordedOutput {
            plan,
            texture,
            #[cfg(all(windows, feature = "native-export"))]
            native,
        })
    }

    #[cfg(all(windows, feature = "native-export"))]
    fn stage_native(
        &mut self,
        frame: &mut FrameContext,
        texture: &GpuTexture,
    ) -> Option<nana_gpu::StagedNativeFrame> {
        if self.native_unavailable.is_some() {
            return None;
        }
        if self.native.is_none() {
            let (width, height) = texture.size();
            match nana_gpu::NativeExportPool::new(frame.gpu(), [width, height]) {
                Ok(pool) => self.native = Some(pool),
                Err(error) => {
                    let reason = error.to_string();
                    self.native_unavailable = Some(reason.clone());
                    self.report(
                        Reported::NativeUnavailable,
                        WindowOutputStatus::NativeUnavailable { reason },
                    );
                    return None;
                }
            }
        }
        let pool = self.native.as_mut()?;
        match pool.stage(frame, texture) {
            Ok(nana_gpu::NativeExportOutcome::Staged(staged)) => Some(staged),
            Ok(nana_gpu::NativeExportOutcome::Deferred(
                nana_gpu::NativeExportDeferral::ConsumerBusy { waiting },
            )) => {
                if waiting >= NATIVE_STALL {
                    // The consumer stopped releasing: give it a fresh pool
                    // rather than defer forever.
                    self.retire_native();
                } else {
                    self.report(Reported::NativeDeferred, WindowOutputStatus::NativeDeferred);
                }
                None
            }
            Err(error) => {
                self.retire_native();
                self.fail(error.to_string());
                None
            }
        }
    }

    /// The frame carrying the output was submitted: publish it and build the
    /// frame for the program.
    pub(crate) fn complete(
        &mut self,
        recorded: RecordedOutput,
        submission: &GpuSubmission,
        now: Instant,
    ) -> Option<WindowOutputFrame> {
        let surface = self.surface.as_mut()?;
        let outcome = match surface.bind_submission(recorded.plan, submission) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.fail(error.to_string());
                return None;
            }
        };
        let crate::ExternalRenderOutcome::Submitted {
            resource_generation,
            content_revision,
            ..
        } = outcome
        else {
            return None;
        };
        self.last_frame = Some(now);
        let alpha = self.config.map(|config| config.alpha).unwrap_or_default();
        #[cfg(all(windows, feature = "native-export"))]
        let native = recorded.native.and_then(|staged| {
            let pool = self.native.as_mut()?;
            match pool.finish(staged) {
                Ok(token) => Some(token),
                Err(error) => {
                    self.fail(error.to_string());
                    None
                }
            }
        });
        #[cfg(all(windows, feature = "native-export"))]
        let has_native = native.is_some();
        #[cfg(not(all(windows, feature = "native-export")))]
        let has_native = false;
        let (width, height) = recorded.texture.size();
        let extent = [width, height];
        self.report(
            Reported::Active {
                extent,
                native: has_native,
            },
            WindowOutputStatus::Active {
                extent,
                native: has_native,
            },
        );
        Some(WindowOutputFrame {
            window: self.window,
            texture: recorded.texture,
            content_revision,
            resource_generation,
            alpha,
            #[cfg(all(windows, feature = "native-export"))]
            native: std::cell::RefCell::new(native),
        })
    }

    /// Work counters of the last output operation (CPU readbacks stay 0).
    #[cfg(test)]
    pub(crate) fn last_work(&self) -> nana_ui_core::OutputWorkObservation {
        self.surface
            .as_ref()
            .map(ExternalSurface::last_work)
            .unwrap_or_default()
    }
}

/// The premultiplied → straight conversion: one fullscreen pass into a
/// target of its own.
struct StraightAlpha {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target: GpuTexture,
}

const STRAIGHT_ALPHA_WGSL: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureLoad(source, vec2<i32>(position.xy), 0);
    if (color.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(min(color.rgb / color.a, vec3<f32>(1.0)), color.a);
}
";

impl StraightAlpha {
    fn new(gpu: &GpuContext, extent: [u32; 2]) -> Result<Self, GpuError> {
        let target = gpu.create_texture(&GpuTextureDescriptor {
            label: Some("nana-ui.window-output.straight"),
            width: extent[0],
            height: extent[1],
            format: WINDOW_OUTPUT_FORMAT,
            usage: GpuTextureUsages::RENDER_TARGET
                | GpuTextureUsages::SAMPLED
                | GpuTextureUsages::COPY_SRC,
        })?;
        let device = nana_gpu::__framework::device(gpu);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.window-output.straight"),
            source: wgpu::ShaderSource::Wgsl(STRAIGHT_ALPHA_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.window-output.straight"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.window-output.straight"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.window-output.straight"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            layout,
            target,
        })
    }

    fn fits(&self, gpu: &GpuContext, extent: [u32; 2]) -> bool {
        let (width, height) = self.target.size();
        [width, height] == extent && self.target.generation() == gpu.generation()
    }

    fn record(&self, frame: &mut FrameContext, source: &GpuTexture) {
        let device = nana_gpu::__framework::device(frame.gpu()).clone();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.window-output.straight"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(nana_gpu::__framework::texture_view(
                    source,
                )),
            }],
        });
        let encoder = nana_gpu::__framework::encoder(frame);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui.window-output.straight"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: nana_gpu::__framework::texture_view(&self.target),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(logical: (f32, f32), scale: f32) -> WindowGeometry {
        WindowGeometry {
            physical_position: None,
            physical_size: (
                (logical.0 * scale).round() as u32,
                (logical.1 * scale).round() as u32,
            ),
            logical_position: None,
            logical_size: logical,
            scale_factor: scale,
            maximized: false,
        }
    }

    #[test]
    fn extents_follow_the_window_or_stay_fixed() {
        let window = geometry((400.0, 300.0), 1.5);
        let config = WindowOutputConfig::default();
        assert_eq!(config.resolve_extent(&window), Some([1920, 1080]));
        assert_eq!(
            config
                .with_extent(WindowOutputExtent::MatchWindow)
                .resolve_extent(&window),
            Some([600, 450])
        );
        assert_eq!(
            config
                .with_extent(WindowOutputExtent::Scale(0.5))
                .resolve_extent(&window),
            Some([300, 225])
        );
        assert_eq!(
            config
                .with_extent(WindowOutputExtent::MatchWindow)
                .resolve_extent(&geometry((0.0, 0.0), 1.0)),
            None
        );
        assert_eq!(
            config
                .with_extent(WindowOutputExtent::Scale(f32::NAN))
                .resolve_extent(&window),
            None
        );
    }

    #[test]
    fn contain_centres_the_whole_window_and_cover_fills_the_output() {
        let window = geometry((400.0, 300.0), 1.0);
        let contain = WindowOutputConfig::default()
            .viewport(&window, [1920, 1080], [0.0; 4])
            .unwrap();
        assert_eq!(contain.scale_factor, 3.6);
        assert_eq!(contain.physical_size, [1920, 1080]);
        assert!((contain.logical_size[1] - 300.0).abs() < 1e-3);
        assert!(contain.target_origin[0] > 0.0 && contain.target_origin[1].abs() < 1e-3);
        let cover = WindowOutputConfig::default()
            .with_fit(WindowOutputFit::Cover)
            .viewport(&window, [1920, 1080], [0.0; 4])
            .unwrap();
        assert_eq!(cover.scale_factor, 4.8);
        assert!(cover.target_origin[1] < 0.0 && cover.target_origin[0].abs() < 1e-3);
    }

    #[test]
    fn hidden_outputs_tick_at_their_cadence_and_throttled_frames_come_due() {
        let mut state = WindowOutputState::new(WindowId(7));
        assert_eq!(state.frame_demand(false), crate::FrameDemand::OnDemand);
        state.set_config(Some(WindowOutputConfig::default()));
        assert_eq!(
            state.frame_demand(false),
            crate::FrameDemand::Continuous(WINDOW_OUTPUT_HIDDEN_FPS)
        );
        assert_eq!(state.frame_demand(true), crate::FrameDemand::OnDemand);
        let at = Instant::now() + Duration::from_millis(5);
        state.throttled_until = Some(at);
        assert_eq!(state.frame_demand(true), crate::FrameDemand::At(at));
        state.set_config(Some(WindowOutputConfig::default().while_hidden(false)));
        assert_eq!(state.frame_demand(false), crate::FrameDemand::At(at));
        state.set_config(None);
        assert_eq!(state.take_statuses(), vec![WindowOutputStatus::Stopped]);
        assert_eq!(state.frame_demand(false), crate::FrameDemand::OnDemand);
    }

    fn record_one(
        state: &mut WindowOutputState,
        painter: &mut SceneWgpuPainter,
        window: &WindowGeometry,
        scene: &UiScene,
    ) -> Option<WindowOutputFrame> {
        let gpu = crate::test_gpu::context();
        let step = state.begin(
            &gpu,
            window,
            scene,
            0,
            false,
            None,
            [0.0; 4],
            Instant::now(),
        );
        if !matches!(step, OutputStep::Record { .. }) {
            return None;
        }
        let mut frame = gpu.begin_frame("window output test");
        let recorded = state
            .record(step, &mut frame, painter, scene, None, None)
            .expect("recorded");
        let submission = frame.submit();
        state.complete(recorded, &submission, Instant::now())
    }

    #[test]
    fn a_static_window_output_records_once_and_never_reads_back() {
        let gpu = crate::test_gpu::context();
        let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, output_profile());
        let window = geometry((64.0, 48.0), 1.0);
        let scene = UiScene::new();
        for alpha in [
            WindowOutputAlpha::Premultiplied,
            WindowOutputAlpha::Straight,
        ] {
            let mut state = WindowOutputState::new(WindowId(3));
            state.set_config(Some(
                WindowOutputConfig::default()
                    .with_extent(WindowOutputExtent::MatchWindow)
                    .with_alpha(alpha),
            ));
            let frame = record_one(&mut state, &mut painter, &window, &scene)
                .expect("the first frame is recorded");
            assert_eq!(frame.extent(), [64, 48]);
            assert_eq!(frame.format(), WINDOW_OUTPUT_FORMAT);
            assert_eq!(frame.alpha(), alpha);
            assert!(frame.texture().usage().contains(GpuTextureUsages::COPY_SRC));
            assert_eq!(state.last_work().cpu_readbacks, 0);
            assert_eq!(
                state.take_statuses(),
                vec![WindowOutputStatus::Active {
                    extent: [64, 48],
                    native: false
                }]
            );
            // Once that frame completed, the same scene records nothing.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                gpu.poll();
                let step = state.begin(
                    &gpu,
                    &window,
                    &scene,
                    0,
                    false,
                    None,
                    [0.0; 4],
                    Instant::now(),
                );
                if matches!(step, OutputStep::Idle) && state.throttled_until.is_none() {
                    break;
                }
                state.throttled_until = None;
                assert!(Instant::now() < deadline, "the output never settled");
                std::thread::yield_now();
            }
            assert_eq!(state.last_work().idle_reuse_frames, 1);
            assert_eq!(state.last_work().cpu_readbacks, 0);
            assert!(state.take_statuses().is_empty());
        }
    }

    #[test]
    fn output_targets_never_share_ids_with_windows() {
        let a = target_namespace(WindowId(1));
        let b = target_namespace(WindowId(2));
        assert_ne!(a, b);
        assert_ne!(a & 0xFFFF_0000_0000_0000, 0);
    }
}
