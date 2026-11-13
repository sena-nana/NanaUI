//! A window's retained presentation supplied to another consumer (a Spout
//! sender, a recorder, a preview).
//!
//! The shared presentation driver paints the window's own `UiScene` once per
//! compatible pixel contract in the host's one frame. Compatible windows sample
//! that canonical output; differing contracts paint separately. A window
//! that is hidden, minimised or occluded keeps its output going when
//! [`WindowOutputConfig::while_hidden`] is set: the host flushes the document
//! and paints only the output on a GPU-only tick.
//!
//! An unchanged presentation records and submits nothing. The output is BGRA8
//! with sRGB-encoded colour and premultiplied (gamma-space) alpha by default.
//! With [`WindowOutputExport::Native`] on Windows, every frame is also copied
//! into a DX12 shared texture another D3D device can open
//! ([`nana_gpu::NativeExportPool`]); no pixel is read back to the CPU.

#[cfg(test)]
#[path = "window_output/production_tests.rs"]
mod production_tests;

use super::driver::frame_driver::{DirectPresentation, PreparedPresentation};
use super::driver::frame_driver::{
    PresentationFrameDriver, RecordedRetained, RetainedPresentation,
};
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use nana_gpu::{
    FrameContext, GpuContext, GpuError, GpuSubmission, GpuTexture, GpuTextureDescriptor,
    GpuTextureFormat, GpuTextureUsages,
};
use nana_ui_platform::{WindowGeometry, WindowId};
use nana_ui_scene::UiScene;

use super::external::{ExternalFramePlan, ExternalSurface, ExternalSurfaceConfig};
use super::window_trace::{self, WindowTrace};
#[cfg(test)]
use crate::SceneWgpuPainter;
use crate::{
    AlphaEncoding, HostTextureRegistry, RenderTargetId, SceneGpuRendererRegistry,
    ScenePaintViewport, ScenePresentationProfile,
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
#[cfg(all(windows, feature = "native-export"))]
const NATIVE_STALL: Duration = Duration::from_secs(2);
/// How soon an output held back for its consumer is tried again.
const NATIVE_RETRY: Duration = Duration::from_millis(4);

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
    /// Publish transparent content once, then stop producing frames while
    /// retaining this output's resources. Resume with `false`; withdraw the
    /// config to release resources. Hosts must bound the suspension lifetime.
    pub suspended: bool,
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
            suspended: false,
        }
    }
}

impl WindowOutputConfig {
    pub const fn with_suspended(mut self, suspended: bool) -> Self {
        self.suspended = suspended;
        self
    }
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
/// you submit before this callback returns. For later or asynchronous use,
/// retain [`Self::lease`] and attach it to each consuming GPU frame. The
/// producer cannot recycle a leased slot. Nothing here waits for the GPU.
#[derive(Debug)]
pub struct WindowOutputFrame {
    window: WindowId,
    texture: GpuTexture,
    content_revision: u64,
    resource_generation: u64,
    alpha: WindowOutputAlpha,
    lease: super::ExternalFrame,
    traces: [Option<super::OutputTraceSnapshot>; 3],
    #[cfg(all(windows, feature = "native-export"))]
    native: std::cell::RefCell<Option<nana_gpu::NativeFrameToken>>,
    resend: std::cell::Cell<bool>,
}

impl WindowOutputFrame {
    /// The consumer could not take this frame (a Spout receiver held its
    /// lock, say). Produce the output again soon even if the scene stays the
    /// same, so the last picture of an animation is not lost.
    pub fn request_resend(&self) {
        self.resend.set(true);
    }

    pub(crate) fn resend_requested(&self) -> bool {
        self.resend.get()
    }

    pub fn source_id(&self) -> super::OutputSurfaceId {
        self.lease.provenance().source()
    }

    /// Owner-attributed opt-in observations; all entries are None by default.
    pub const fn output_traces(&self) -> [Option<super::OutputTraceSnapshot>; 3] {
        self.traces
    }
    /// Retain this exact output revision across asynchronous same-device GPU
    /// consumption. Merely cloning `texture()` does not reserve a producer slot.
    pub fn lease(&self) -> WindowOutputLease {
        WindowOutputLease {
            texture: self.texture.clone(),
            frame: self.lease.clone(),
            alpha: self.alpha,
        }
    }
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

/// A persistent output reference, independent of the window callback's cadence.
#[derive(Debug, Clone)]
pub struct WindowOutputLease {
    texture: GpuTexture,
    frame: super::ExternalFrame,
    alpha: WindowOutputAlpha,
}

impl WindowOutputLease {
    pub fn texture(&self) -> Result<&GpuTexture, super::ExternalSurfaceError> {
        self.frame.validate()?;
        Ok(&self.texture)
    }
    pub fn alpha(&self) -> WindowOutputAlpha {
        self.alpha
    }
    pub fn content_revision(&self) -> u64 {
        self.frame.content_revision()
    }
    pub fn resource_generation(&self) -> u64 {
        self.frame.resource_generation()
    }
    /// Attach sampling lifetime to the frame before submitting it. Discard
    /// releases the reference; submit retains it until actual GPU completion.
    pub fn keep_alive_until_complete(
        &self,
        frame: &mut FrameContext,
    ) -> Result<(), super::ExternalSurfaceError> {
        self.frame.validate()?;
        if frame.generation() != self.texture.generation() {
            return Err(super::ExternalSurfaceError::DeviceMismatch {
                expected: self.texture.generation(),
                found: frame.generation(),
            });
        }
        frame.keep_alive_until_complete(self.clone());
        Ok(())
    }
}

/// A change in an output's state, reported once per transition to
/// [`crate::RuntimeProgram::window_output_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowOutputStatus {
    /// Structural capability negotiation, including explicit reasons why
    /// different rasterizations cannot share. Not emitted per frame.
    Planned { plan: super::OutputPlan },
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

/// The host holds a frame slot before planning the output, so the other
/// frames in flight each hold an output slot until they complete, and the
/// published frame holds one more. Fewer slots defer the output whenever the
/// GPU runs that far behind, as WARP does.
fn output_slots(gpu: &GpuContext) -> usize {
    (gpu.policy().frame_slot_count() + 1).min(super::external::MAX_SLOTS)
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
    Active {
        extent: [u32; 2],
        native: bool,
    },
    NativeUnavailable,
    #[cfg(all(windows, feature = "native-export"))]
    NativeDeferred,
    Failed,
}

/// The host's state for one window's output.
pub(crate) struct WindowOutputState {
    window: WindowId,
    config: Option<WindowOutputConfig>,
    blank_scene: UiScene,
    suspend_pending: bool,
    suspend_at: Instant,
    surface: Option<ExternalSurface>,
    straight: Vec<StraightAlpha>,
    planner: super::RenderTargetPlanner,
    transfer: Option<super::transfer::OutputTransfer>,
    work: nana_ui_core::OutputWorkObservation,
    #[cfg(all(windows, feature = "native-export"))]
    native: Option<nana_gpu::NativeExportPool>,
    #[cfg(all(windows, feature = "native-export"))]
    retired_native: Vec<nana_gpu::NativeExportRetirement>,
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
    trace: Option<Box<WindowTrace>>,
}

impl WindowOutputState {
    pub(crate) fn set_output_trace_enabled(&mut self, enabled: bool) {
        if enabled == self.trace.is_some() {
            return;
        }
        self.trace = enabled.then(Box::default);
        self.refresh_trace_source(0, super::OutputRecreateReason::None);
    }

    pub(crate) fn output_traces(&self) -> [Option<super::OutputTraceSnapshot>; 3] {
        self.trace
            .as_ref()
            .map_or([None; 3], |trace| trace.snapshots)
    }

    fn refresh_trace_source(&mut self, recreated: usize, reason: super::OutputRecreateReason) {
        if let (Some(trace), Some(surface)) = (&mut self.trace, &self.surface) {
            trace.ensure_source(
                surface.id().get(),
                surface.gpu().generation(),
                surface.resource_generation(),
                surface.config().extent,
                surface.config().presentation.target_format,
                recreated,
                reason,
            );
            #[cfg(all(windows, feature = "native-export"))]
            if let Some(pool) = &self.native
                && let (Some(generation), Some(extent)) = (pool.pool_generation(), pool.extent())
            {
                trace.native_target(generation, extent, nana_gpu::NATIVE_EXPORT_FORMAT);
            }
        }
    }

    fn trace_work(&mut self, owner: usize, work: nana_ui_core::OutputWorkObservation) {
        if owner != window_trace::PRODUCER {
            super::trace::consumer_metrics(work, None);
        }
        if let Some(trace) = &mut self.trace {
            trace.observe(owner, work);
        }
    }

    /// The host observes frame acquisition. It is outside consumer passes;
    /// only the owner of the physical submit receives that observation.
    pub(crate) fn trace_host_submission(
        &mut self,
        direct_ui: bool,
        waits: Option<super::OutputTraceWait>,
    ) {
        if let Some(trace) = &mut self.trace {
            trace.direct_owns_submission = direct_ui;
            trace.host_waits = waits;
            trace.restore_host_waits();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_presentation(
        &mut self,
        step: Option<OutputStep>,
        direct: Option<DirectPresentation>,
        scene: &UiScene,
        frame: &mut FrameContext,
        gpu: &GpuContext,
        painters: &mut impl super::driver::frame_driver::PresentationPainters,
        target: Option<&nana_gpu::GpuRenderTarget>,
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
        fetch_host: Option<nana_ui_platform::SharedFetchHost>,
    ) -> Result<
        (
            Option<RecordedOutput>,
            super::driver::frame_driver::RecordedPresentation,
        ),
        super::ExternalSurfaceError,
    > {
        let prepared = match step {
            Some(OutputStep::Record { plan, viewport }) => PreparedPresentation::retained(
                plan,
                RetainedPresentation {
                    revision: 0,
                    viewport,
                    alpha: AlphaEncoding::Gamma,
                    parameters: direct.map_or_else(Default::default, |d| d.parameters),
                },
            ),
            _ => PresentationFrameDriver::prepare(scene, None, None, None, textures)?,
        }
        .with_direct(
            direct,
            (!self.config.is_some_and(|c| c.suspended)).then(|| self.planner.plan()),
        );
        let retained_scene = self
            .config
            .is_some_and(|c| c.suspended)
            .then_some(&self.blank_scene);
        let mut encoded = PresentationFrameDriver::encode(
            prepared,
            scene,
            retained_scene,
            frame,
            gpu,
            painters,
            self.surface.as_mut(),
            target,
            &mut self.transfer,
            textures,
            renderers,
            fetch_host,
        )?;
        if let Some(trace) = &mut self.trace {
            trace.direct_owns_submission = encoded.direct_ui;
            if let (Some(snapshot), Some(direct)) =
                (&mut trace.snapshots[window_trace::WINDOW], direct)
            {
                snapshot.extent = direct.viewport.physical_size;
                snapshot.format = direct.profile.target_format;
                snapshot.path = if encoded.shared {
                    super::OutputPath::GpuTransfer
                } else {
                    super::OutputPath::DirectSurface
                };
            }
        }
        if let Some(error) = encoded.retained_error.take() {
            self.fail(error.to_string());
        }
        if encoded.shared {
            self.work.extra_passes += 1;
            self.work.gpu_convert_passes += 1;
            self.trace_work(
                window_trace::WINDOW,
                nana_ui_core::OutputWorkObservation {
                    extra_passes: 1,
                    gpu_convert_passes: 1,
                    ..Default::default()
                },
            );
        }
        let conversions = self.work.gpu_convert_passes;
        let output = encoded
            .retained
            .take()
            .and_then(|recorded| self.postprocess(recorded, frame));
        let conversions = self.work.gpu_convert_passes - conversions;
        if conversions != 0 {
            super::driver::frame_driver::accumulate_gpu_work(
                &mut encoded.gpu_work,
                Some(nana_ui_core::GpuWorkObservation {
                    draw_batches: conversions,
                    draw_calls: conversions,
                    ..Default::default()
                }),
            );
        }
        Ok((output, encoded))
    }

    pub(crate) fn new(window: WindowId) -> Self {
        Self {
            window,
            config: None,
            blank_scene: UiScene::new(),
            suspend_pending: false,
            suspend_at: Instant::now(),
            surface: None,
            straight: Vec::new(),
            planner: super::RenderTargetPlanner::new(),
            transfer: None,
            work: Default::default(),
            #[cfg(all(windows, feature = "native-export"))]
            native: None,
            #[cfg(all(windows, feature = "native-export"))]
            retired_native: Vec::new(),
            native_unavailable: None,
            last_frame: None,
            throttled_until: None,
            volatile: 0,
            reported: None,
            statuses: Vec::new(),
            retired_targets: Vec::new(),
            trace: None,
        }
    }

    /// The demand this output adds to its window: its hidden cadence, or the
    /// instant a throttled frame is due.
    pub(crate) fn frame_demand(&self, can_present: bool) -> crate::FrameDemand {
        let Some(config) = self.config else {
            return crate::FrameDemand::OnDemand;
        };
        if config.suspended {
            return if self.suspend_pending {
                crate::FrameDemand::At(self.throttled_until.unwrap_or(self.suspend_at))
            } else {
                crate::FrameDemand::OnDemand
            };
        }
        if !can_present && config.while_hidden {
            return crate::FrameDemand::Continuous(
                config.max_fps.unwrap_or(WINDOW_OUTPUT_HIDDEN_FPS),
            );
        }
        self.throttled_until
            .map_or(crate::FrameDemand::OnDemand, crate::FrameDemand::At)
    }

    pub(crate) fn defer_frame(&mut self, now: Instant) {
        if let Some(trace) = &mut self.trace
            && let Some(producer) = &mut trace.snapshots[window_trace::PRODUCER]
        {
            producer.deferral_reason = super::trace::OutputTraceDeferral::FrameSlots;
        }
        self.work.deferred += 1;
        self.throttled_until = Some(now + Duration::from_millis(4));
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_DEFERRED);
        self.trace_work(
            window_trace::PRODUCER,
            nana_ui_core::OutputWorkObservation {
                deferred: 1,
                ..Default::default()
            },
        );
    }

    /// Apply the program's answer for this frame. Withdrawing the output
    /// releases everything and reports `Stopped`.
    pub(crate) fn set_config(&mut self, config: Option<WindowOutputConfig>) {
        if self.config != config {
            self.suspend_pending = config.is_some_and(|c| c.suspended);
            self.suspend_at = Instant::now();
            if self.config.map(|c| c.suspended) != config.map(|c| c.suspended) {
                self.volatile = self.volatile.wrapping_add(1);
                self.throttled_until = None;
                self.last_frame = None;
                self.reported = None;
            }
        }
        if config.is_none() && self.config.is_some() {
            self.release();
            self.statuses.push(WindowOutputStatus::Stopped);
            self.reported = None;
        }
        if let (Some(old), Some(new)) = (self.config, config)
            && (old.alpha != new.alpha || old.export != new.export)
        {
            // Canonical storage is unchanged. Keep its generation and leases;
            // only conversion/transport resources change at this boundary.
            self.straight.clear();
            self.retire_native();
            self.volatile = self.volatile.wrapping_add(1);
        }
        self.config = config;
        if config.is_none_or(|config| config.export != WindowOutputExport::Native) {
            if let Some(trace) = &mut self.trace {
                trace.clear_native_target();
            }
            self.refresh_trace_source(0, super::OutputRecreateReason::None);
        }
    }

    /// The device changed: every resource was made on the old one.
    pub(crate) fn device_replaced(&mut self) {
        self.release();
        self.native_unavailable = None;
    }

    /// Record this output again soon, scene changed or not: the last frame
    /// never reached its consumer.
    pub(crate) fn resend(&mut self, now: Instant) {
        if self.config.is_none() {
            return;
        }
        self.volatile = self.volatile.wrapping_add(1);
        let at = now + NATIVE_RETRY;
        self.throttled_until = Some(self.throttled_until.map_or(at, |due| due.min(at)));
    }

    fn release(&mut self) {
        if let Some(surface) = self.surface.take() {
            self.retired_targets.extend(surface.target_ids());
        }
        self.straight.clear();
        self.transfer = None;
        self.retire_native();
        self.throttled_until = None;
        self.suspend_pending = self.config.is_some_and(|c| c.suspended);
        self.suspend_at = Instant::now();
    }

    fn retire_native(&mut self) {
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(mut pool) = self.native.take() {
            self.retired_native.retain(|ticket| !ticket.is_released());
            if let Some(ticket) = pool.retirement_ticket() {
                self.retired_native.push(ticket);
            }
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
        self.trace_failed();
        if self.reported != Some(Reported::Failed) {
            self.reported = Some(Reported::Failed);
            self.statuses.push(WindowOutputStatus::Failed { reason });
        }
    }

    pub(crate) fn trace_failed(&mut self) {
        if let Some(trace) = &mut self.trace {
            for snapshot in trace.snapshots.iter_mut().flatten() {
                snapshot.state = super::trace::OutputTraceState::Failed;
                snapshot.waits = None;
            }
        }
    }

    /// Observe a host deferral before `begin`, without reusing prior frame work.
    pub(crate) fn trace_deferred(&mut self, reason: super::OutputTraceDeferral) {
        if let Some(trace) = &mut self.trace {
            trace.host_waits = None;
            trace.observe(
                window_trace::PRODUCER,
                nana_ui_core::OutputWorkObservation {
                    deferred: 1,
                    ..Default::default()
                },
            );
            if let Some(producer) = &mut trace.snapshots[window_trace::PRODUCER] {
                producer.state = super::OutputTraceState::Deferred;
                producer.deferral_reason = reason;
                producer.waits = None;
            }
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
        self.work = Default::default();
        if let Some(trace) = &mut self.trace {
            trace.direct_owns_submission = false;
            trace.host_waits = None;
        }
        let Some(config) = self.config else {
            return OutputStep::Idle;
        };
        if config.suspended && !self.suspend_pending {
            return OutputStep::Idle;
        }
        // The consumer still reads the last frame, so this one could not be
        // exported: hold it back and record the then-current scene shortly
        // instead of rendering a frame only to drop it. A consumer stalled
        // past NATIVE_STALL falls through, and staging gives it a new pool.
        #[cfg(all(windows, feature = "native-export"))]
        if config.export == WindowOutputExport::Native
            && self
                .native
                .as_ref()
                .and_then(nana_gpu::NativeExportPool::consumer_wait)
                .is_some_and(|waiting| waiting < NATIVE_STALL)
        {
            self.work.deferred += 1;
            self.throttled_until = Some(now + NATIVE_RETRY);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_DEFERRED);
            if let Some(trace) = &mut self.trace
                && let Some(consumer) = &mut trace.snapshots[window_trace::TRANSPORT]
            {
                consumer.deferral_reason = super::trace::OutputTraceDeferral::NativeRelease;
            }
            return OutputStep::Idle;
        }
        let host_textures = if config.suspended {
            None
        } else {
            host_textures
        };
        let clear_color = if config.suspended {
            [0.; 4]
        } else {
            clear_color
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
        let recreates = self.work.target_recreates;
        if (volatile && !config.suspended) || self.suspend_pending {
            self.volatile = self.volatile.wrapping_add(1);
        }
        let revision = (if config.suspended { 0 } else { host_revision })
            ^ self.volatile.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let scene = if config.suspended {
            &self.blank_scene
        } else {
            scene
        };
        let surface = self.surface.as_mut().expect("surface was just ensured");
        let prepared = PresentationFrameDriver::prepare(
            scene,
            None,
            Some((
                surface,
                RetainedPresentation {
                    revision,
                    viewport,
                    alpha: AlphaEncoding::Gamma,
                    parameters: Default::default(),
                },
            )),
            None,
            host_textures,
        );
        self.work = surface.last_work();
        self.work.target_recreates += recreates;
        let observed = nana_ui_core::OutputWorkObservation {
            idle_reuse_frames: self.work.idle_reuse_frames,
            deferred: self.work.deferred,
            ..Default::default()
        };
        if self.work.deferred != 0
            && let Some(trace) = &mut self.trace
            && let Some(producer) = &mut trace.snapshots[window_trace::PRODUCER]
        {
            producer.deferral_reason = super::trace::OutputTraceDeferral::SourceSlots;
        }
        self.trace_work(window_trace::PRODUCER, observed);
        let plan = match prepared {
            Ok(prepared) => {
                if let Some(crate::ExternalRenderOutcome::Reused {
                    resource_generation,
                    content_revision,
                }) = prepared.outcome
                    && let Some(trace) = &mut self.trace
                {
                    trace.versions(resource_generation, content_revision);
                }
                if matches!(
                    prepared.outcome,
                    Some(crate::ExternalRenderOutcome::Deferred)
                ) {
                    self.throttled_until = Some(now + Duration::from_millis(4));
                    return OutputStep::Idle;
                }
                let Some((plan, _)) = prepared.into_retained() else {
                    return OutputStep::Idle;
                };
                plan
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
        let before = self.work.target_recreates;
        let reason = match &self.surface {
            None => super::OutputRecreateReason::Created,
            Some(surface) if !surface.gpu().same_device(gpu) => super::OutputRecreateReason::Device,
            Some(surface) if surface.config().extent != extent => {
                super::OutputRecreateReason::Extent
            }
            Some(_) => super::OutputRecreateReason::Requirements,
        };
        let config = ExternalSurfaceConfig::with_presentation(extent, output_profile())
            .with_copy_source(true)
            .with_slots(output_slots(gpu));
        let current = self
            .surface
            .as_ref()
            .is_some_and(|surface| surface.config() == config && surface.gpu().same_device(gpu));
        if current {
            return Ok(());
        }
        if let Some(surface) = self.surface.as_mut() {
            let old_targets: Vec<_> = surface.target_ids().collect();
            if !surface.gpu().same_device(gpu) {
                surface
                    .replace_gpu(gpu)
                    .map_err(|error| error.to_string())?;
                self.work.target_recreates += surface.last_work().target_recreates;
            }
            surface
                .resize(gpu, config)
                .map_err(|error| error.to_string())?;
            self.work.target_recreates += surface.last_work().target_recreates;
            self.retired_targets.extend(old_targets);
        } else {
            let mut surface =
                ExternalSurface::with_host_painter(gpu, config, target_namespace(self.window))
                    .map_err(|error| error.to_string())?;
            surface.set_alpha_encoding(AlphaEncoding::Gamma);
            self.surface = Some(surface);
            self.work.target_recreates += config.slots;
            nana_diagnostics::metric!(
                nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES,
                config.slots as u64
            );
        }
        // New extent, new device or first frame: conversion target and
        // native textures follow the surface.
        self.straight.clear();
        self.retire_native();
        self.refresh_trace_source(self.work.target_recreates - before, reason);
        Ok(())
    }

    /// Paint the scene into the output inside `frame`, then the conversion
    /// pass and the native copy when configured.
    #[cfg(test)]
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
        let surface = self.surface.as_mut()?;
        let scene = if self.config.is_some_and(|c| c.suspended) {
            &self.blank_scene
        } else {
            scene
        };
        let recorded = match PresentationFrameDriver::encode_retained(
            surface,
            plan,
            RetainedPresentation {
                revision: 0,
                viewport,
                alpha: AlphaEncoding::Gamma,
                parameters: Default::default(),
            },
            painter,
            scene,
            frame,
            host_textures,
            gpu_renderers,
        ) {
            Ok(recorded) => recorded,
            Err(error) => {
                self.fail(error.to_string());
                return None;
            }
        };
        self.postprocess(recorded, frame)
    }

    /// Consumer transport work only. Scene encoding and target selection are
    /// owned by the shared presentation driver.
    pub(crate) fn postprocess(
        &mut self,
        recorded: RecordedRetained,
        frame: &mut FrameContext,
    ) -> Option<RecordedOutput> {
        self.work.ui_renders += 1;
        self.trace_work(
            window_trace::PRODUCER,
            nana_ui_core::OutputWorkObservation {
                ui_renders: 1,
                ..Default::default()
            },
        );
        let config = self.config?;
        let surface = self.surface.as_ref()?;
        let RecordedRetained { plan, mut texture } = recorded;
        if config.alpha == WindowOutputAlpha::Straight {
            let gpu = frame.gpu().clone();
            let extent = surface.config().extent;
            // One conversion target per canonical slot. The output lease
            // reserves both through asynchronous consumers, never one shared
            // conversion texture that a later frame could overwrite.
            let slot = plan.slot_index();
            while self.straight.len() <= slot {
                match StraightAlpha::new(&gpu, extent) {
                    Ok(target) => {
                        self.straight.push(target);
                        self.work.target_recreates += 1;
                        if let Some(trace) = &mut self.trace {
                            trace.observe(
                                window_trace::TRANSPORT,
                                nana_ui_core::OutputWorkObservation {
                                    target_recreates: 1,
                                    ..Default::default()
                                },
                            );
                            trace.reason(
                                window_trace::TRANSPORT,
                                super::OutputRecreateReason::Created,
                            );
                        }
                        nana_diagnostics::metric!(
                            nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES
                        );
                    }
                    Err(error) => {
                        self.fail(error.to_string());
                        return None;
                    }
                }
            }
            let straight = &self.straight[slot];
            straight.record(frame, &texture);
            texture = straight.target.clone();
            self.work.extra_passes += 1;
            self.work.gpu_convert_passes += 1;
            if let Some(trace) = &mut self.trace {
                trace.transfer(window_trace::TRANSPORT);
            }
            self.trace_work(
                window_trace::TRANSPORT,
                nana_ui_core::OutputWorkObservation {
                    extra_passes: 1,
                    gpu_convert_passes: 1,
                    ..Default::default()
                },
            );
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_EXTRA_PASSES);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_GPU_CONVERT_PASSES);
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

    /// Negotiate the actual Window + external consumer pair. A normal window
    /// without an output never constructs or calls this planner.
    pub(crate) fn can_share_window(
        &mut self,
        gpu: &GpuContext,
        geometry: &WindowGeometry,
        profile: ScenePresentationProfile,
        viewport: ScenePaintViewport,
        opaque: bool,
    ) -> bool {
        use super::*;
        let Some(config) = self.config else {
            return false;
        };
        if config.suspended {
            return false;
        }
        let Some(extent) = config.resolve_extent(geometry) else {
            return false;
        };
        let Some(output_viewport) = config.viewport(geometry, extent, viewport.clear_color) else {
            return false;
        };
        let viewport_key = |v: ScenePaintViewport| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            for value in v
                .logical_size
                .into_iter()
                .chain([v.scale_factor])
                .chain(v.scene_origin)
                .chain(v.target_origin)
            {
                value.to_bits().hash(&mut hash);
            }
            hash.finish()
        };
        let requirements = RenderTargetRequirements {
            presentation: PresentationId(self.window.0),
            viewport_revision: viewport_key(output_viewport),
            extent,
            profile: output_profile(),
            alpha: OutputAlpha::GammaPremultiplied,
            text_aa: OutputTextAa::Grayscale,
            device_generation: gpu.generation(),
            ownership: OutputOwnership::SameDevice,
            persistent: true,
        };
        let text_aa = if opaque {
            match crate::scene_paint::SubpixelOrder::system() {
                Some(crate::scene_paint::SubpixelOrder::Rgb) => OutputTextAa::SubpixelRgb,
                Some(crate::scene_paint::SubpixelOrder::Bgr) => OutputTextAa::SubpixelBgr,
                None => OutputTextAa::Grayscale,
            }
        } else {
            OutputTextAa::Grayscale
        };
        let consumers = [
            OutputConsumer {
                id: OutputConsumerId(0),
                kind: OutputConsumerKind::External,
                requirements,
                capabilities: PresenterCapabilities {
                    sampled: true,
                    ..Default::default()
                },
            },
            OutputConsumer {
                id: OutputConsumerId(1),
                kind: OutputConsumerKind::Window,
                requirements: RenderTargetRequirements {
                    extent: viewport.physical_size,
                    profile,
                    viewport_revision: viewport_key(viewport),
                    alpha: if opaque {
                        OutputAlpha::LinearPremultiplied
                    } else {
                        OutputAlpha::GammaPremultiplied
                    },
                    text_aa,
                    persistent: false,
                    ..requirements
                },
                capabilities: PresenterCapabilities {
                    direct_render: true,
                    copy_destination: true,
                    ..Default::default()
                },
            },
        ];
        if self.planner.update(&consumers) {
            self.work.plan_rebuilds += 1;
            self.statuses.push(WindowOutputStatus::Planned {
                plan: self.planner.plan().clone(),
            });
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_PLAN_REBUILDS);
        }
        self.planner.plan().groups.len() == 1 && self.planner.plan().render_count() == 1
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
            self.retired_native.retain(|ticket| !ticket.is_released());
            if self.retired_native.len() >= 2 {
                self.report(Reported::NativeDeferred, WindowOutputStatus::NativeDeferred);
                return None;
            }
            let (width, height) = texture.size();
            match nana_gpu::NativeExportPool::new(frame.gpu(), [width, height]) {
                Ok(pool) => {
                    if let Some(trace) = &mut self.trace
                        && let (Some(generation), Some(extent)) =
                            (pool.pool_generation(), pool.extent())
                    {
                        trace.native_target(generation, extent, nana_gpu::NATIVE_EXPORT_FORMAT);
                    }
                    self.native = Some(pool);
                    self.work.target_recreates += nana_gpu::NATIVE_EXPORT_SLOTS;
                    if let Some(trace) = &mut self.trace {
                        trace.observe(
                            window_trace::TRANSPORT,
                            nana_ui_core::OutputWorkObservation {
                                target_recreates: nana_gpu::NATIVE_EXPORT_SLOTS,
                                ..Default::default()
                            },
                        );
                        trace.reason(
                            window_trace::TRANSPORT,
                            super::OutputRecreateReason::Created,
                        );
                    }
                    nana_diagnostics::metric!(
                        nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES,
                        nana_gpu::NATIVE_EXPORT_SLOTS as u64
                    );
                }
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
            Ok(nana_gpu::NativeExportOutcome::Staged(staged)) => {
                self.work.gpu_copies += 1;
                let (width, height) = texture.size();
                let bytes = width as usize * height as usize * 4;
                self.work.gpu_copy_bytes += bytes;
                if let Some(trace) = &mut self.trace {
                    trace.transfer(window_trace::TRANSPORT);
                }
                self.trace_work(
                    window_trace::TRANSPORT,
                    nana_ui_core::OutputWorkObservation {
                        gpu_copies: 1,
                        gpu_copy_bytes: bytes,
                        ..Default::default()
                    },
                );
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_GPU_COPIES);
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::gpu::OUTPUT_GPU_COPY_BYTES,
                    bytes as u64
                );
                Some(staged)
            }
            Ok(nana_gpu::NativeExportOutcome::Deferred(
                nana_gpu::NativeExportDeferral::ConsumerBusy { waiting },
            )) => {
                if let Some(trace) = &mut self.trace
                    && let Some(consumer) = &mut trace.snapshots[window_trace::TRANSPORT]
                {
                    consumer.deferral_reason = super::trace::OutputTraceDeferral::NativeRelease;
                }
                self.trace_work(
                    window_trace::TRANSPORT,
                    nana_ui_core::OutputWorkObservation {
                        deferred: 1,
                        ..Default::default()
                    },
                );
                if waiting >= NATIVE_STALL {
                    // The consumer stopped releasing: give it a fresh pool
                    // rather than defer forever.
                    self.retire_native();
                } else {
                    self.report(Reported::NativeDeferred, WindowOutputStatus::NativeDeferred);
                }
                // This frame is dropped; an unchanged scene would otherwise
                // never be exported again.
                self.resend(Instant::now());
                None
            }
            Ok(nana_gpu::NativeExportOutcome::Deferred(
                nana_gpu::NativeExportDeferral::ProducerBusy,
            )) => {
                if let Some(trace) = &mut self.trace
                    && let Some(consumer) = &mut trace.snapshots[window_trace::TRANSPORT]
                {
                    consumer.deferral_reason = super::trace::OutputTraceDeferral::NativeReservation;
                }
                self.trace_work(
                    window_trace::TRANSPORT,
                    nana_ui_core::OutputWorkObservation {
                        deferred: 1,
                        ..Default::default()
                    },
                );
                self.report(Reported::NativeDeferred, WindowOutputStatus::NativeDeferred);
                self.resend(Instant::now());
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
        let completed = match PresentationFrameDriver::finish(
            surface,
            RecordedRetained {
                plan: recorded.plan,
                texture: recorded.texture.clone(),
            },
            submission,
            true,
        ) {
            Ok(completed) => completed,
            Err(error) => {
                self.fail(error.to_string());
                return None;
            }
        };
        let crate::ExternalRenderOutcome::Submitted {
            resource_generation,
            content_revision,
            ..
        } = completed.outcome
        else {
            return None;
        };
        let lease = completed
            .lease
            .expect("presenter requested its consumer lease");
        self.last_frame = Some(now);
        let alpha = self.config.map(|config| config.alpha).unwrap_or_default();
        self.work.content_revisions += 1;
        self.work.submissions += 1;
        if let Some(trace) = &mut self.trace {
            trace.versions(resource_generation, content_revision);
            trace.observe(
                window_trace::PRODUCER,
                nana_ui_core::OutputWorkObservation {
                    content_revisions: 1,
                    submissions: usize::from(!trace.direct_owns_submission),
                    ..Default::default()
                },
            );
        }
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
        if self.config.is_some_and(|c| c.suspended)
            && (has_native
                || self.native_unavailable.is_some()
                || self
                    .config
                    .is_some_and(|c| c.export != WindowOutputExport::Native))
        {
            self.suspend_pending = false;
            self.throttled_until = None;
        }
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
        if let Some(trace) = &mut self.trace {
            trace.restore_host_waits();
        }
        Some(WindowOutputFrame {
            window: self.window,
            texture: recorded.texture,
            content_revision,
            resource_generation,
            alpha,
            lease,
            traces: self.output_traces(),
            #[cfg(all(windows, feature = "native-export"))]
            native: std::cell::RefCell::new(native),
            resend: std::cell::Cell::new(false),
        })
    }

    /// Work counters of the last output operation (CPU readbacks stay 0).
    pub(crate) fn last_work(&self) -> nana_ui_core::OutputWorkObservation {
        self.work
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
        let shader = crate::scene_paint::wgsl::Program::new(
            "nana-ui.window-output.straight",
            STRAIGHT_ALPHA_WGSL,
        );
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
                module: &shader.stage(device, "vs").module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader.stage(device, "fs").module,
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

    #[test]
    fn compatible_window_uses_one_cached_plan_and_preserves_opaque_quality() {
        let gpu = crate::test_gpu::context();
        let window = geometry((64.0, 48.0), 1.0);
        let mut state = WindowOutputState::new(WindowId(8));
        let config = WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow);
        state.set_config(Some(config));
        let viewport = config.viewport(&window, [64, 48], [0.0; 4]).unwrap();
        let profile = ScenePresentationProfile::sdr(GpuTextureFormat::RGBA8_UNORM_SRGB);
        assert!(state.can_share_window(&gpu, &window, profile, viewport, false));
        let rebuilds = state.planner.rebuild_count();
        for _ in 0..240 {
            assert!(state.can_share_window(&gpu, &window, profile, viewport, false));
        }
        assert_eq!(state.planner.rebuild_count(), rebuilds);
        assert!(!state.can_share_window(&gpu, &window, profile, viewport, true));
        assert!(
            state
                .planner
                .plan()
                .incompatibilities
                .iter()
                .any(|difference| difference
                    .reasons
                    .contains(&super::super::OutputIncompatibilityReason::Alpha))
        );
    }

    #[test]
    fn window_resize_advances_generation_and_rejects_old_leases() {
        let gpu = crate::test_gpu::context();
        let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, output_profile());
        let mut state = WindowOutputState::new(WindowId(9));
        state.set_config(Some(
            WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow),
        ));
        let scene = UiScene::new();
        let first = record_one(
            &mut state,
            &mut painter,
            &geometry((64.0, 48.0), 1.0),
            &scene,
        )
        .unwrap();
        let old = first.lease();
        let second = record_one(
            &mut state,
            &mut painter,
            &geometry((80.0, 48.0), 1.0),
            &scene,
        )
        .unwrap();
        assert!(second.resource_generation() > first.resource_generation());
        assert!(matches!(
            old.texture(),
            Err(super::super::ExternalSurfaceError::StaleFrame)
        ));
        assert_eq!(second.extent(), [80, 48]);
    }

    #[test]
    fn straight_output_uses_distinct_leased_targets_and_defers_when_full() {
        let gpu = crate::test_gpu::context();
        let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, output_profile());
        let mut state = WindowOutputState::new(WindowId(10));
        state.set_config(Some(
            WindowOutputConfig::default()
                .with_extent(WindowOutputExtent::MatchWindow)
                .with_alpha(WindowOutputAlpha::Straight),
        ));
        let window = geometry((64.0, 48.0), 1.0);
        let scene = UiScene::new();
        let slots = output_slots(&gpu);
        let mut held = Vec::new();
        for _ in 0..slots {
            state.volatile += 1;
            let frame = record_one(&mut state, &mut painter, &window, &scene).unwrap();
            let lease = frame.lease();
            for previous in &held {
                let previous: &WindowOutputLease = previous;
                assert!(!previous.texture().unwrap().ptr_eq(lease.texture().unwrap()));
            }
            held.push(lease);
        }
        state.volatile += 1;
        assert!(record_one(&mut state, &mut painter, &window, &scene).is_none());
        assert_eq!(state.straight.len(), slots);
        assert_eq!(state.last_work().ui_renders, 0);
        assert_eq!(state.last_work().submissions, 0);
        assert_eq!(state.last_work().gpu_convert_passes, 0);
    }
}
