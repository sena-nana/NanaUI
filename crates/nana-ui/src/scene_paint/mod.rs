//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It paints [`UiScene`] through the host GPU.
//! The host owns the [`GpuContext`] and each frame's [`FrameContext`]. HostTexture is sampled in document order
//! inside the current dest (or opacity-group) pass; groups do not open a pass
//! per HostTexture slot.

mod backdrop;
mod buffer_upload;
#[cfg(feature = "charts")]
mod chart;
mod clip;
mod color;
mod dest;
mod host_texture;
mod icon;
mod icon_raster;
mod image_resample;
pub(crate) mod image_url;
mod material;
mod mesh;
mod motion;
#[cfg(target_vendor = "apple")]
mod msl_precompiled;
#[cfg(any(test, not(target_vendor = "apple")))]
mod naga_ir;
mod natural_sizes;
// Only a Windows build loads the stages. The others compile the module for
// its freshness checks, which run wherever the tests do.
#[cfg_attr(not(target_os = "windows"), allow(dead_code, unused_imports))]
mod precompiled;
mod presentation_color;
mod quad;
mod text;

pub use self::material::{MaterialPolicy, set_material_policy};
pub use self::natural_sizes::{ImageNaturalSize, commit_image_natural_sizes};
pub use self::text::SubpixelOrder;
pub use self::text::TextGlyphCounters;
pub(crate) mod url_texture_cache;
mod validate;
pub(crate) mod wgsl;

/// What a painter's image caches hold, beside the glyph atlas that
/// [`TextGlyphCounters`] reports. Bytes are texels as allocated by the
/// painter, without driver padding or alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScenePainterResourceCounters {
    /// Icon atlases: one per render target, the painter's own included, that
    /// has drawn an icon. A target without icons holds none.
    pub icon_atlases: u64,
    pub icon_atlas_bytes: u64,
    /// Uploaded `url(...)` textures, one per URL and source encoding.
    pub url_textures: u64,
    /// Bytes uploaded for them, every mip level.
    pub url_texture_gpu_bytes: u64,
    /// Decoded levels waiting for upload and source bytes kept for refits.
    pub url_texture_cpu_bytes: u64,
    /// Fetches and decodes still running.
    pub url_pending: u64,
    /// Custom-paint path meshes kept on the GPU, node-local, so that frames
    /// which only move or scale them write no vertex.
    pub path_meshes: u64,
    /// Bytes the vertex and index arenas holding them allocate.
    pub path_mesh_bytes: u64,
}

pub(crate) use clip::{covered_span, is_translation_projective, on_grid};

use std::{sync::Arc, time::Instant};

use nana_gpu::{
    __framework, FrameContext, GpuContext, GpuRenderTarget, GpuSubmission, GpuTextureFormat,
    RetainedWrites,
};
use nana_ui_platform::SharedFetchHost;
use nana_ui_scene::{RenderOperation, ScenePrimitiveKind, UiScene};
use url_texture_cache::{UrlTextureCache, egress_of};

use crate::{
    GpuWorkObservation, HostTextureRegistry, PhysicalRect,
    gpu_work::{GpuStageTimings, GpuWorkSink},
    scene_gpu::{
        SceneGpuBatchNode, SceneGpuBatchPassContext, SceneGpuNode, SceneGpuPassContext,
        SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer, SceneGpuRendererRegistry,
        ScenePass,
    },
};

pub use image_url::{
    resolve_background_image_url, resolved_resource_is_allowed, set_background_image_url_base,
};
pub use presentation_color::{
    BT709_TO_BT2020, ScenePresentationParameters, hlg_decode, hlg_encode, hlg_eotf, hlg_oetf,
    linear_sc_rgb_to_bt2020, pq_decode, pq_decode_nits, pq_encode, pq_encode_nits, pq_eotf,
    pq_oetf, tone_map_headroom_rgb,
};
pub use validate::ScenePaintError;
#[allow(dead_code)]
pub type ScenePainterError = ScenePaintError;
use validate::validate_scene;

/// The colour a text run takes in its own space: one that is not plain sRGB.
/// Plain sRGB text goes through the glyph runs' 8-bit sRGB colour, as every
/// resolved theme colour does.
fn explicit_text_color(
    color: Option<nana_ui_core::PaintColor>,
) -> Option<nana_ui_core::PaintColor> {
    color.filter(|color| !matches!(color, nana_ui_core::PaintColor::Srgb { .. }))
}

/// What a text, icon or spinner with no colour of its own paints.
const BLACK: nana_ui_core::PaintColor = nana_ui_core::PaintColor::srgb([0.0, 0.0, 0.0, 1.0]);

/// Final surface encoding for the linear-scRGB Scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScenePresentationColorSpace {
    Srgb,
    DisplayP3,
    ExtendedSrgbLinear,
    ExtendedSrgb,
    ExtendedDisplayP3,
    /// BT.2100 HDR10 signal: BT.2020 primaries with absolute PQ encoding.
    Bt2100Pq,
    /// BT.2100 HLG signal: BT.2020 primaries with relative HLG encoding.
    Bt2100Hlg,
}

/// Host-resolved target profile. Scene primitives are always composed as
/// premultiplied linear scRGB; this describes only the output boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScenePresentationProfile {
    pub target_format: GpuTextureFormat,
    pub color_space: ScenePresentationColorSpace,
    /// Keep a linear fp16 working target when a requested HDR profile fell
    /// back to SDR, so the final pass can still apply its highlight shoulder.
    pub force_float_working: bool,
    /// Compatibility mode for offscreen/test callers that historically pass
    /// a non-sRGB unorm target and expect the linear Scene bytes directly.
    direct_linear: bool,
}

/// Keep the clear used for the linear working target and the clear used for
/// the presentation target identical, even when a public viewport was built
/// from malformed input. RGB remains extended (negative and over-white are
/// valid scRGB values), while alpha is a finite coverage value.
fn normalize_clear_color(mut color: [f32; 4]) -> [f32; 4] {
    for channel in &mut color {
        if !channel.is_finite() {
            *channel = 0.0;
        }
    }
    color[3] = color[3].clamp(0.0, 1.0);
    color
}

fn clear_color_is_extended(color: [f32; 4]) -> bool {
    const EPS: f32 = 1.0e-3;
    if color[3].is_finite() && color[3] <= 0.0 {
        return false;
    }
    color[..3]
        .iter()
        .any(|c| c.is_finite() && (*c < -EPS || *c > 1.0 + EPS))
}

fn scene_uses_float_host_texture(
    scene: &UiScene,
    host_textures: Option<&HostTextureRegistry>,
) -> bool {
    let Some(host_textures) = host_textures else {
        return false;
    };
    let Ok(plan) = scene.frame_plan() else {
        return false;
    };
    plan.custom_nodes.iter().any(|id| {
        scene
            .primitive(*id)
            .and_then(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Custom { node, .. }
                    if node.renderer.as_ref() == "nana.host-texture" =>
                {
                    host_textures.get(node.resource.as_ref())
                }
                _ => None,
            })
            .is_some_and(|binding| binding.texture.format() == GpuTextureFormat::RGBA16_FLOAT)
    })
}

impl ScenePresentationProfile {
    pub(crate) fn effective_for_frame(
        self,
        scene: &UiScene,
        clear_color: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
    ) -> Self {
        if self.is_direct_linear() || self.requires_float_working_target() {
            return self;
        }
        if scene.has_extended_color()
            || clear_color_is_extended(clear_color)
            || scene_uses_float_host_texture(scene, host_textures)
        {
            self.with_force_float_working()
        } else {
            self
        }
    }

    /// Construct a host presentation profile. Scene composition stays in
    /// linear scRGB; the selected colour space is applied exactly once at the
    /// surface boundary. For a native surface, pass the format/colour-space
    /// pair returned by its capability table; offscreen callers may choose an
    /// untyped target when they want the painter to own the transfer function.
    /// BT.2100 PQ/HLG profiles should use an untyped target; the surface
    /// resolver only negotiates those encodings on untyped formats.
    pub const fn new(
        target_format: GpuTextureFormat,
        color_space: ScenePresentationColorSpace,
    ) -> Self {
        Self {
            target_format,
            color_space,
            force_float_working: false,
            direct_linear: false,
        }
    }

    pub const fn sdr(target_format: GpuTextureFormat) -> Self {
        Self {
            target_format,
            color_space: ScenePresentationColorSpace::Srgb,
            force_float_working: false,
            direct_linear: false,
        }
    }

    pub(crate) fn legacy_linear(target_format: GpuTextureFormat) -> Self {
        Self {
            target_format,
            color_space: ScenePresentationColorSpace::Srgb,
            force_float_working: false,
            direct_linear: !target_format.is_srgb(),
        }
    }

    pub(crate) const fn is_direct_linear(self) -> bool {
        self.direct_linear
    }

    pub const fn with_force_float_working(self) -> Self {
        Self {
            force_float_working: true,
            ..self
        }
    }

    pub(crate) fn requires_float_working_target(self) -> bool {
        !self.direct_linear
            && (self.force_float_working
                || !matches!(
                    (self.color_space, self.target_format.is_srgb()),
                    (ScenePresentationColorSpace::Srgb, true)
                ))
    }

    pub fn working_format(self) -> GpuTextureFormat {
        if self.requires_float_working_target() {
            GpuTextureFormat::RGBA16_FLOAT
        } else {
            self.target_format
        }
    }

    /// Surface clear matching the final presentation transfer.  The dynamic
    /// HDR parameters are deliberately supplied separately from the profile:
    /// changing display headroom must not invalidate a painter/pipeline cache
    /// keyed by [`ScenePresentationProfile`].
    #[allow(clippy::excessive_precision)]
    pub(crate) fn surface_clear_with_parameters(
        self,
        color: [f32; 4],
        encoding: AlphaEncoding,
        parameters: ScenePresentationParameters,
    ) -> wgpu::Color {
        let color = normalize_clear_color(color);
        if self.is_direct_linear() {
            return wgpu::Color {
                r: (color[0] * color[3]) as f64,
                g: (color[1] * color[3]) as f64,
                b: (color[2] * color[3]) as f64,
                a: color[3] as f64,
            };
        }
        let alpha = color[3].clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return wgpu::Color::TRANSPARENT;
        }
        let mut rgb = [color[0], color[1], color[2]];
        let sdr_space = matches!(
            self.color_space,
            ScenePresentationColorSpace::Srgb | ScenePresentationColorSpace::DisplayP3
        );
        let headroom = if self.force_float_working || sdr_space {
            1.0
        } else {
            parameters.headroom()
        };
        let gamut_transform = self.force_float_working || sdr_space;
        let encode = |value: f32| {
            let sign = value.signum();
            let absolute = value.abs();
            sign * if absolute <= 0.0031308 {
                absolute * 12.92
            } else {
                1.055 * absolute.powf(1.0 / 2.4) - 0.055
            }
        };
        let decode = |value: f32| {
            let sign = value.signum();
            let absolute = value.abs();
            sign * if absolute <= 0.04045 {
                absolute / 12.92
            } else {
                ((absolute + 0.055) / 1.055).powf(2.4)
            }
        };
        if matches!(self.color_space, ScenePresentationColorSpace::Srgb)
            && !self.force_float_working
            && encoding == AlphaEncoding::Linear
            && self.target_format.is_srgb()
            && !clear_color_is_extended(color)
        {
            return wgpu::Color {
                r: (color[0] * alpha) as f64,
                g: (color[1] * alpha) as f64,
                b: (color[2] * alpha) as f64,
                a: alpha as f64,
            };
        }
        if self.force_float_working || sdr_space {
            // SDR output has no room above white whatever the display reports:
            // scale over-white highlights as a whole to preserve hue.
            rgb = tone_map_headroom_rgb(rgb, 1.0);
        } else if matches!(
            self.color_space,
            ScenePresentationColorSpace::ExtendedSrgbLinear
                | ScenePresentationColorSpace::ExtendedSrgb
                | ScenePresentationColorSpace::ExtendedDisplayP3
                | ScenePresentationColorSpace::Bt2100Pq
                | ScenePresentationColorSpace::Bt2100Hlg
        ) {
            // All HDR profiles share the same scene-linear shoulder.  The
            // extended-scRGB targets keep their linear intermediate, while
            // PQ/HLG then apply their transfer function below.  Keeping this
            // mapping here makes headroom=1 the SDR-limit behaviour for every
            // HDR output contract.
            rgb = tone_map_headroom_rgb(rgb, headroom);
        }
        if matches!(
            self.color_space,
            ScenePresentationColorSpace::DisplayP3 | ScenePresentationColorSpace::ExtendedDisplayP3
        ) {
            rgb = presentation_color::linear_srgb_to_p3(rgb);
        }
        if gamut_transform {
            rgb = presentation_color::gamut_map_rgb(rgb);
        }
        if matches!(
            self.color_space,
            ScenePresentationColorSpace::ExtendedSrgbLinear
        ) {
            let white = parameters.extended_linear_white();
            rgb = rgb.map(|value| value * white);
        }
        let encoded = match self.color_space {
            ScenePresentationColorSpace::ExtendedSrgbLinear => rgb,
            ScenePresentationColorSpace::Bt2100Pq => {
                let rgb = presentation_color::gamut_map_bt2020_rgb(linear_sc_rgb_to_bt2020(rgb));
                rgb.map(|value| pq_encode_nits(value * parameters.reference_white_nits()))
            }
            ScenePresentationColorSpace::Bt2100Hlg => {
                let rgb = presentation_color::gamut_map_bt2020_rgb(linear_sc_rgb_to_bt2020(rgb));
                presentation_color::hlg_encode_display(
                    rgb.map(|value| value * parameters.reference_white_nits() / 1_000.0),
                )
            }
            _ => rgb.map(encode),
        };
        let linear_premult = rgb.map(|value| value * alpha);
        let encoded_premult = encoded.map(|value| value * alpha);
        let output = if matches!(
            self.color_space,
            ScenePresentationColorSpace::ExtendedSrgbLinear
        ) {
            // Extended-linear targets own no transfer function. Keep the
            // tone-mapped scene value linear, including for gamma alpha;
            // shader mode 2 follows this same raw-linear contract.
            linear_premult
        } else if matches!(
            self.color_space,
            ScenePresentationColorSpace::Bt2100Pq | ScenePresentationColorSpace::Bt2100Hlg
        ) {
            // BT.2100 surfaces carry the transfer-encoded signal directly.
            // They are normally untyped RGB10A2/FP16; applying an sRGB
            // attachment transfer to a PQ/HLG code would be incorrect. If a
            // caller constructs an invalid typed-sRGB combination manually,
            // pre-decode the clear so the attachment's OETF still produces
            // the requested BT.2100 code.
            if self.target_format.is_srgb() {
                encoded_premult.map(decode)
            } else {
                encoded_premult
            }
        } else if self.target_format.is_srgb() && encoding == AlphaEncoding::Gamma {
            // A typed sRGB attachment applies its OETF in hardware. Gamma
            // alpha therefore stores the encoded-straight colour multiplied
            // by alpha, and the CPU clear must provide the corresponding
            // linear attachment value.
            encoded_premult.map(decode)
        } else if self.target_format.is_srgb() {
            // Linear alpha stores OETF(linear * alpha) in the typed target;
            // its hardware transfer performs that OETF after the clear.
            linear_premult
        } else if encoding == AlphaEncoding::Gamma {
            encoded_premult
        } else {
            // Unorm P3/extended targets have no hardware transfer. Match the
            // presentation shader's Linear path: encode the premultiplied
            // linear value, rather than encoding straight colour first.
            linear_premult.map(encode)
        };
        wgpu::Color {
            r: output[0] as f64,
            g: output[1] as f64,
            b: output[2] as f64,
            a: alpha as f64,
        }
    }
}

use backdrop::BackdropPipeline;
use clip::{
    FragmentClip, LogicalRect, PaintOrigin, extra_fragment_clips, fragment_clip, intersect_clips,
    intersect_physical, local_rect, mesh_extra_fragment_clips, overlaps_physical, paint_origin,
    paint_transform, physical_bounds, physical_scissor, transformed_aabb,
    transformed_aabb_projective, union_physical,
};
pub(crate) use color::{pack_linear, pack_paint_color};
use dest::{DestPassCounts, DestTarget, GroupSlot, PrefixSplit};
use host_texture::{HostTexturePipeline, PreparedHostTexture};
use icon::{IconPipeline, PreparedIcon};
use mesh::{MeshPipeline, MeshRange, PathRange, StrokeStyle};
use motion::{MotionGpuResources, eval_format_for_gpu};
use quad::QuadPipeline;
use text::{PreparedText, TextPipeline, TextProgram};

#[derive(Clone, Copy, PartialEq)]
pub struct ScenePaintViewport {
    /// Scene dest rect size in logical pixels.
    pub logical_size: [f32; 2],
    /// Full target texture size in physical pixels.
    pub physical_size: [u32; 2],
    pub scale_factor: f32,
    pub scene_origin: [f32; 2],
    /// Logical position of scene (0, 0) on the target.
    pub target_origin: [f32; 2],
    /// Straight linear RGBA used to clear the scene. A theme colour is sRGB
    /// and has to be converted first; the painter premultiplies it for the
    /// internal render target.
    pub clear_color: [f32; 4],
    /// Clear the whole target; otherwise keep existing pixels (`LoadOp::Load`).
    pub clear: bool,
}

/// Paints [`UiScene`]s into the host's targets.
///
/// One painter can serve every presentation profile on its device. What a
/// profile does not change is its core and is shared: the text engine with
/// its shape and raster caches and glyph atlas, the URL images (bucketed by
/// source encoding), the motion resources and every target's retained state,
/// which includes an icon atlas of its own. What a working format changes is
/// a pipeline set, built on first use and kept while the format is painted.
///
/// Targets painted into one frame read the shared glyph atlas and motion
/// buffers when that frame is submitted, so every paint after the first
/// records their uploads in the encoder, behind the passes before it.
/// Shared device/format pipelines, caches, and transient encode scratch for
/// [`SceneWgpuPainter`], separated from `targets` so `paint_recorded` can
/// borrow both disjointly.
struct PainterCore {
    gpu: GpuContext,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Surface format. Scene pipelines use `working_format` for wide/HDR
    /// profiles and write the final encoding only in the presentation pass.
    format: wgpu::TextureFormat,
    working_format: wgpu::TextureFormat,
    configured_presentation: ScenePresentationProfile,
    presentation: ScenePresentationProfile,
    // The active `PipelineSet` is spread over `quads`, `meshes`, the
    // programs inside `icons` and `text`, `host_textures`, `backdrop` and
    // `charts`; the atlases beside those programs are core.
    quads: QuadPipeline,
    motion: MotionGpuResources,
    meshes: MeshPipeline,
    icons: IconPipeline,
    text: TextPipeline,
    host_textures: HostTexturePipeline,
    /// `url(...)` images for quads and HostTexture masks alike: a URL both
    /// use is fetched, decoded, uploaded and retained once per painter.
    /// Shared by every target; target-local bindings follow `image_revision`.
    url_cache: UrlTextureCache,
    backdrop: BackdropPipeline,
    /// Built on the first chart this painter draws.
    #[cfg(feature = "charts")]
    charts: Option<chart::ChartPipeline>,
    /// Built with the other pipeline families, for the dest of this
    /// painter's own profile.
    dest_pipelines: Option<dest::DestPipelines>,
    last_gpu_work: Option<GpuWorkObservation>,
    last_gpu_timings: Option<GpuStageTimings>,
    last_dest_pass_counts: Option<DestPassCounts>,
    /// The frame the last paint was recorded in. A paint into the same one
    /// copies its shared atlas/motion uploads in order, not ahead of the frame.
    last_frame: Option<nana_gpu::FrameId>,
    /// A stroke's points put at its node's origin, reused every stroke.
    placed_points: Vec<[f32; 2]>,
    encode_scratch: EncodeScratch,
    image_revision: u64,
    material_revision: u64,
    /// The host's waker, counting into `materials_ready` first; see
    /// [`SceneWgpuPainter::set_material_waker`].
    material_waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Material pipelines this painter waited for that are ready, and how
    /// many of them it has painted since.
    materials_ready: Arc<std::sync::atomic::AtomicU64>,
    materials_painted: u64,
    /// Egress for `url(...)` loads of the scenes painted next.
    fetch_host: Option<SharedFetchHost>,
    /// True when the most recent encoded `paint` / `paint_target` used partial-
    /// damage incremental rendering (`LoadOp::Load` + scissored commands).
    last_frame_used_partial_damage: bool,
}

impl PainterCore {
    fn new_target_resources(&self) -> TargetResources {
        TargetResources {
            prepared_batch: None,
            quads: self.quads.new_target(&self.device),
            meshes: self.meshes.new_target(&self.device),
            icons: self.icons.new_target(&self.device),
            text: self.text.new_target(&self.device),
            backdrop: self.backdrop.new_target(&self.device),
            #[cfg(feature = "charts")]
            charts: chart::ChartPipelineTarget::default(),
            host_textures: crate::gpu_texture::GpuTextureTarget::default(),
            dest: None,
            painted: None,
            last_encoded_key: None,
            changing_frame_streak: 0,
            painted_demand: Default::default(),
            natural_sizes: Default::default(),
            image_revision: self.image_revision,
            material_revision: self.material_revision,
            bound_fetch_host: None,
        }
    }

    fn adopt_material_waker(&mut self) {
        if let Some(wake) = &self.material_waker {
            self.quads.set_material_waker(Arc::clone(wake));
            self.text.set_material_waker(Arc::clone(wake));
        }
    }

    fn refresh_image_resources(&mut self) -> bool {
        // A material compiled since the last paint: the batch drew natively
        // where it now shades, so neither it nor the painted dest is current.
        let ready = self
            .materials_ready
            .load(std::sync::atomic::Ordering::Acquire);
        let materials = ready != self.materials_painted;
        if materials {
            self.materials_painted = ready;
            self.material_revision = self.material_revision.wrapping_add(1);
            self.image_revision = self.image_revision.wrapping_add(1);
        }
        if !self.url_cache.poll() {
            return materials;
        }
        self.image_revision = self.image_revision.wrapping_add(1);
        true
    }
}

pub struct SceneWgpuPainter {
    core: PainterCore,
    targets: std::collections::HashMap<RenderTargetId, TargetResources>,
    /// The pipeline sets of the working formats not painted right now.
    pipeline_sets: std::collections::HashMap<PipelineSetKey, PipelineSet>,
    /// Targets whose retained GPU state was recorded into a frame that was
    /// later dropped instead of submitted; rebuilt before their next paint.
    /// Keyed by [`RenderTargetId`].
    retained_targets: RetainedWrites,
    /// The same for [`DEFAULT_TARGET_ID`], in a ledger of its own so no host id
    /// can alias it.
    retained_default: RetainedWrites,
    /// Shared atlas/motion uploads copied in a frame's encoder. A dropped
    /// frame invalidates the corresponding CPU upload cache.
    retained_shared: RetainedWrites,
    #[cfg(test)]
    _test_gpu_guard: Option<crate::test_gpu::GpuTestGuard>,
}

/// How a paint stores its premultiplied pixels. Blending is linear either
/// way; a window compositor, though, reads a surface as `enc(color) * alpha`
/// and shows the linear `enc(color * alpha)` too bright wherever alpha is low.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlphaEncoding {
    /// `enc(color * alpha)`: for offscreen readers and opaque surfaces.
    #[default]
    Linear,
    /// `enc(color) * alpha`: for a surface a compositor blends. Opaque pixels
    /// match [`Self::Linear`]; a target that is not sRGB keeps that too.
    Gamma,
}

/// Immutable per-paint options replacing global mutable painter setters.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScenePaintOptions {
    pub alpha: AlphaEncoding,
    pub text_aa: Option<SubpixelOrder>,
    pub damage: Option<PhysicalRect>,
    pub presentation: ScenePresentationParameters,
}

#[derive(Clone, Copy, PartialEq)]
struct PaintedDest {
    instance: u64,
    presentation_epoch: u64,
    /// What [`SceneWgpuPainter::paint`]'s own state was painted for; a target
    /// of [`SceneWgpuPainter::paint_target`] keeps one profile for life.
    presentation: ScenePresentationProfile,
    viewport: ScenePaintViewport,
    size: [u32; 2],
    alpha: AlphaEncoding,
    text_aa: Option<SubpixelOrder>,
}

/// Host-owned identity for a window or offscreen presentation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderTargetId(pub u64);

/// Target id behind [`SceneWgpuPainter::paint`]; tracked in
/// [`SceneWgpuPainter::retained_default`] so no caller target id can alias it.
pub(crate) const DEFAULT_TARGET_ID: RenderTargetId = RenderTargetId(u64::MAX);
const DEFAULT_TARGET: u64 = 0;
const SHARED_ATLAS: u64 = 0;
const SHARED_MOTION: u64 = 1;

struct TargetResources {
    prepared_batch: Option<PreparedBatch>,
    quads: quad::QuadPipelineTarget,
    meshes: mesh::MeshPipelineTarget,
    icons: icon::IconPipelineTarget,
    text: text::TextPipelineTarget,
    backdrop: backdrop::BackdropPipelineTarget,
    #[cfg(feature = "charts")]
    charts: chart::ChartPipelineTarget,
    host_textures: crate::gpu_texture::GpuTextureTarget,
    dest: Option<DestTarget>,
    painted: Option<PaintedDest>,
    last_encoded_key: Option<PaintedDest>,
    changing_frame_streak: u8,
    painted_demand: crate::painted_demand::PaintedChannel,
    natural_sizes: natural_sizes::NaturalSizes,
    image_revision: u64,
    material_revision: u64,
    bound_fetch_host: Option<SharedFetchHost>,
}

#[derive(Default)]
struct EncodeScratch {
    commands: Vec<DrawCommand>,
    group_stack: Vec<nana_ui_scene::OpacityGroup>,
    group_slots: Vec<dest::GroupSlot>,
    group_extents: Vec<GroupExtent>,
    painter_layers: Vec<nana_ui_runtime::StableNodeId>,
    batching: Batching,
    clip_dests: Vec<FragmentClip>,
    drained_commands: Vec<DrawCommand>,
    group_extent_stack: Vec<(u32, GroupExtent)>,
    encode_stack: Vec<GroupFrame>,
    encode_layer_ready: Vec<bool>,
    max_group_depth: usize,
    glyph_then_quad: bool,
}

impl EncodeScratch {
    fn begin_build(&mut self) {
        self.commands.clear();
        self.group_stack.clear();
        self.group_slots.clear();
        self.group_extents.clear();
        self.painter_layers.clear();
        self.batching.clear();
        self.max_group_depth = 0;
        self.glyph_then_quad = false;
    }
}

struct BatchView<'a> {
    commands: &'a [DrawCommand],
    max_group_depth: usize,
    group_slots: &'a [dest::GroupSlot],
    group_extents: &'a [GroupExtent],
    glyph_then_quad: bool,
}

/// What a [`PipelineSet`] is compiled for: the working format, and whether
/// the `url(...)` textures its bindings name are uploaded as sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineSetKey {
    working_format: wgpu::TextureFormat,
    srgb_source: bool,
}

impl PipelineSetKey {
    fn of(presentation: ScenePresentationProfile) -> Self {
        Self {
            working_format: __framework::format_to_wgpu(presentation.working_format()),
            srgb_source: !presentation.is_direct_linear(),
        }
    }
}

/// Every pipeline family compiled for one working format. Pipelines come
/// from the device's registry, so a second set of a format another painter
/// already built compiles nothing.
struct PipelineSet {
    quads: QuadPipeline,
    meshes: MeshPipeline,
    icons: wgpu::RenderPipeline,
    text: TextProgram,
    host_textures: HostTexturePipeline,
    backdrop: BackdropPipeline,
    #[cfg(feature = "charts")]
    charts: Option<chart::ChartPipeline>,
}

struct PreparedBatch {
    key: PaintedDest,
    resources: Vec<TextureBindingKey>,
    renderers: Vec<(usize, u64)>,
    image_revision: u64,
    commands: Vec<DrawCommand>,
    max_group_depth: usize,
    group_slots: Vec<dest::GroupSlot>,
    group_extents: Vec<GroupExtent>,
    /// Read off the document order that built `commands`; it cannot be
    /// recovered from the merged list.
    glyph_then_quad: bool,
    /// Text instances name atlas rectangles. Reusing them after the atlas has
    /// evicted or relocated a glyph would sample whatever now owns that
    /// rectangle, so a changed epoch forces the batch to be rebuilt.
    text_placement_epoch: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct TextureBindingKey {
    identity: u64,
    generation: u64,
    width: u32,
    height: u32,
    alpha: crate::HostTextureAlphaMode,
}

enum DrawCommand {
    Quads {
        range: std::ops::Range<u32>,
        scissor: PhysicalRect,
    },
    Mesh {
        range: MeshRange,
        scissor: PhysicalRect,
    },
    Path {
        range: PathRange,
        scissor: PhysicalRect,
    },
    /// Draw `draw` of the chart in batch slot `slot`.
    #[cfg(feature = "charts")]
    Chart {
        slot: u32,
        draw: u32,
        scissor: PhysicalRect,
    },
    /// A painter layer's mask, applied to the open group before it pops.
    PathMask {
        range: PathRange,
        scissor: PhysicalRect,
        mode: nana_ui_scene::LayerMaskMode,
    },
    Text {
        prepared: PreparedText,
        scissor: PhysicalRect,
    },
    Icon {
        first: usize,
        count: usize,
        scissor: PhysicalRect,
    },
    HostTexture(PreparedHostTexture),
    Custom {
        node: SceneGpuNode,
        renderer: Arc<dyn SceneGpuRenderer>,
        bounds: PhysicalRect,
        clip: PhysicalRect,
    },
    Backdrop {
        index: u32,
    },
    PushGroup {
        layer: usize,
        slot: u32,
    },
    PopGroup,
}

/// Runs `$build` on a scoped thread of its own, or right here when no
/// thread can be started (a target without threads, or none left).
macro_rules! beside {
    ($scope:expr, $name:literal, $build:expr) => {
        match std::thread::Builder::new()
            .name($name.into())
            .spawn_scoped($scope, || $build)
        {
            Ok(handle) => Beside::Thread(handle),
            Err(_) => Beside::Built($build),
        }
    };
}

/// A pipeline family being built by [`beside!`].
enum Beside<'scope, T> {
    Thread(std::thread::ScopedJoinHandle<'scope, T>),
    Built(T),
}

impl<T> Beside<'_, T> {
    /// The family, re-raising a panic from its thread as this thread's own.
    fn join(self) -> T {
        match self {
            Self::Thread(handle) => handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            Self::Built(value) => value,
        }
    }
}

impl SceneWgpuPainter {
    /// A painter for targets of `format` on `gpu`. Every frame it paints must
    /// come from the same device.
    pub fn try_new(gpu: &GpuContext, format: GpuTextureFormat) -> Result<Self, ScenePaintError> {
        Self::try_new_with_presentation(gpu, ScenePresentationProfile::legacy_linear(format))
    }

    pub fn try_new_with_presentation(
        gpu: &GpuContext,
        presentation: ScenePresentationProfile,
    ) -> Result<Self, ScenePaintError> {
        let device = __framework::device(gpu);
        let queue = __framework::queue(gpu);
        let format = __framework::format_to_wgpu(presentation.working_format());
        let policy = gpu.policy();
        // Lazy archive scope: 0 decompressions on warm SharedPipelineRegistry hits,
        // and at most 1 shared decompression across all worker threads when cold.
        #[cfg(target_vendor = "apple")]
        let _msl_scope = msl_precompiled::ArchiveScope::new();
        let precompiled = precompiled::runs_on(gpu, device);
        let (quads, meshes, icons, text, host_textures, backdrop, dest_pipelines) =
            std::thread::scope(|scope| {
                let meshes = beside!(scope, "nana-paint-mesh", {
                    MeshPipeline::try_new_with_policy(device, format, Some(policy), precompiled)
                });
                let icons = beside!(scope, "nana-paint-icon", {
                    IconPipeline::try_new_with_policy(device, format, Some(policy))
                });
                let text = beside!(scope, "nana-paint-text", {
                    (|| -> Result<_, ScenePaintError> {
                        let mut text = TextPipeline::try_new_with_policy(
                            device,
                            queue,
                            format,
                            policy,
                            precompiled,
                        )?;
                        text.try_prewarm_subpixel(device)?;
                        Ok(text)
                    })()
                });
                let dest = beside!(scope, "nana-paint-dest", {
                    dest::DestPipelines::try_new(
                        device,
                        format,
                        __framework::format_to_wgpu(presentation.target_format),
                        presentation,
                        policy,
                    )
                });
                let host_textures = beside!(scope, "nana-paint-host-texture", {
                    HostTexturePipeline::try_new(device, queue, format, policy, gpu)
                });
                let backdrop = beside!(scope, "nana-paint-backdrop", {
                    BackdropPipeline::try_new(device, format, policy)
                });
                let quads =
                    QuadPipeline::try_new_with_policy(device, format, Some(policy), precompiled);
                (
                    quads,
                    meshes.join(),
                    icons.join(),
                    text.join(),
                    host_textures.join(),
                    backdrop.join(),
                    dest.join(),
                )
            });
        let quads = quads?;
        let meshes = meshes?;
        let icons = icons?;
        let text = text?;
        let host_textures = host_textures?;
        let backdrop = backdrop?;
        let dest_pipelines = dest_pipelines?;
        let motion = MotionGpuResources::new_with_policy(
            device,
            quads.motion_layout(),
            Some(policy),
            eval_format_for_gpu(gpu),
        );
        Ok(Self {
            core: PainterCore {
                gpu: gpu.clone(),
                device: device.clone(),
                queue: queue.clone(),
                format,
                working_format: format,
                configured_presentation: presentation,
                presentation,
                quads,
                motion,
                meshes,
                icons,
                text,
                host_textures,
                url_cache: {
                    let mut cache = UrlTextureCache::default();
                    cache.set_srgb_source(!presentation.is_direct_linear());
                    cache
                },
                backdrop,
                #[cfg(feature = "charts")]
                charts: None,
                dest_pipelines: Some(dest_pipelines),
                last_gpu_work: None,
                last_gpu_timings: None,
                last_dest_pass_counts: None,
                last_frame: None,
                placed_points: Vec::new(),
                encode_scratch: EncodeScratch::default(),
                image_revision: 0,
                material_revision: 0,
                material_waker: None,
                materials_ready: Arc::default(),
                materials_painted: 0,
                fetch_host: None,
                last_frame_used_partial_damage: false,
            },
            targets: std::collections::HashMap::new(),
            pipeline_sets: std::collections::HashMap::new(),
            retained_targets: RetainedWrites::new(),
            retained_default: RetainedWrites::new(),
            retained_shared: RetainedWrites::new(),
            #[cfg(test)]
            _test_gpu_guard: None,
        })
    }

    /// A painter for targets of `format` on `gpu`. Every frame it paints must
    /// come from the same device.
    pub fn new(gpu: &GpuContext, format: GpuTextureFormat) -> Self {
        Self::try_new(gpu, format).expect("built-in scene painter pipelines initialize")
    }

    pub fn new_with_presentation(gpu: &GpuContext, presentation: ScenePresentationProfile) -> Self {
        Self::try_new_with_presentation(gpu, presentation)
            .expect("built-in scene painter pipelines initialize")
    }

    /// Format of the surface/target view passed to [`Self::paint`]. Wide and
    /// HDR profiles may use a different internal [`Self::working_format`].
    pub fn format(&self) -> GpuTextureFormat {
        self.core.configured_presentation.target_format
    }

    /// Format used by Scene primitives and custom GPU nodes while composing.
    /// It is `RGBA16_FLOAT` for wide-gamut/HDR profiles and equals
    /// [`Self::format`] on the ordinary SDR path.
    pub fn working_format(&self) -> GpuTextureFormat {
        __framework::format_from_wgpu(self.core.working_format)
    }

    #[allow(clippy::misnamed_getters)]
    pub fn presentation(&self) -> ScenePresentationProfile {
        self.core.configured_presentation
    }

    /// Paint the next targets for `presentation`. A target keeps the profile
    /// it was first painted with; another profile is another target.
    ///
    /// A profile of the working format in use changes only the final
    /// transfer. One of another format swaps that format's [`PipelineSet`]
    /// in, building it on first use; atlases and caches carry over.
    pub(crate) fn try_select_presentation(
        &mut self,
        presentation: ScenePresentationProfile,
    ) -> Result<(), ScenePaintError> {
        self.core.configured_presentation = presentation;
        self.try_apply_presentation(presentation)
    }

    #[cfg(test)]
    pub(crate) fn select_presentation(&mut self, presentation: ScenePresentationProfile) {
        self.try_select_presentation(presentation)
            .expect("select presentation pipelines");
    }

    fn try_apply_presentation(
        &mut self,
        presentation: ScenePresentationProfile,
    ) -> Result<(), ScenePaintError> {
        if presentation == self.core.presentation {
            return Ok(());
        }
        let current = PipelineSetKey::of(self.core.presentation);
        let next = PipelineSetKey::of(presentation);
        if next != current {
            let mut set = match self.pipeline_sets.remove(&next) {
                Some(set) => set,
                None => self.try_build_pipeline_set(next.working_format)?,
            };
            self.swap_pipeline_set(&mut set);
            self.pipeline_sets.insert(current, set);
            self.core.adopt_material_waker();
            self.core.format = next.working_format;
            self.core.working_format = next.working_format;
            self.core.url_cache.set_srgb_source(next.srgb_source);
        }
        self.core.presentation = presentation;
        Ok(())
    }

    fn try_sync_effective_presentation(
        &mut self,
        scene: &UiScene,
        viewport: &ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
    ) -> Result<(), ScenePaintError> {
        let effective = self.core.configured_presentation.effective_for_frame(
            scene,
            viewport.clear_color,
            host_textures,
        );
        self.try_apply_presentation(effective)
    }

    /// The pipeline families for targets of `format`, over this painter's
    /// atlases, compiled side by side as [`Self::try_new_with_presentation`] does.
    fn try_build_pipeline_set(
        &self,
        format: wgpu::TextureFormat,
    ) -> Result<PipelineSet, ScenePaintError> {
        let device = &self.core.device;
        let queue = &self.core.queue;
        let gpu = &self.core.gpu;
        let policy = gpu.policy();
        let atlas_layout = self.core.text.atlas_layout();
        #[cfg(target_vendor = "apple")]
        let _msl_scope = msl_precompiled::ArchiveScope::new();
        let precompiled = precompiled::runs_on(gpu, device);
        let (quads, meshes, icons, text, host_textures, backdrop) = std::thread::scope(|scope| {
            let meshes = beside!(scope, "nana-paint-mesh", {
                MeshPipeline::try_new_with_policy(device, format, Some(policy), precompiled)
            });
            let text = beside!(scope, "nana-paint-text", {
                TextProgram::try_new(device, format, atlas_layout, policy, precompiled)
            });
            let host_textures = beside!(scope, "nana-paint-host-texture", {
                HostTexturePipeline::try_new(device, queue, format, policy, gpu)
            });
            let backdrop = beside!(scope, "nana-paint-backdrop", {
                BackdropPipeline::try_new(device, format, policy)
            });
            let quads =
                QuadPipeline::try_new_with_policy(device, format, Some(policy), precompiled);
            let icons = self
                .core
                .icons
                .try_pipeline_for(device, format, Some(policy));
            (
                quads,
                meshes.join(),
                icons,
                text.join(),
                host_textures.join(),
                backdrop.join(),
            )
        });
        Ok(PipelineSet {
            quads: quads?,
            meshes: meshes?,
            icons: icons?,
            text: text?,
            host_textures: host_textures?,
            backdrop: backdrop?,
            #[cfg(feature = "charts")]
            charts: None,
        })
    }

    /// Exchange the active pipeline families with `set`.
    fn swap_pipeline_set(&mut self, set: &mut PipelineSet) {
        std::mem::swap(&mut self.core.quads, &mut set.quads);
        std::mem::swap(&mut self.core.meshes, &mut set.meshes);
        self.core.icons.swap_pipeline(&mut set.icons);
        self.core.text.swap_program(&mut set.text);
        std::mem::swap(&mut self.core.host_textures, &mut set.host_textures);
        std::mem::swap(&mut self.core.backdrop, &mut set.backdrop);
        #[cfg(feature = "charts")]
        std::mem::swap(&mut self.core.charts, &mut set.charts);
    }

    /// Keep only the pipeline sets some of `presentations` paints with.
    pub(crate) fn retain_presentations(&mut self, presentations: &[ScenePresentationProfile]) {
        self.pipeline_sets.retain(|key, _| {
            presentations
                .iter()
                .any(|presentation| PipelineSetKey::of(*presentation) == *key)
        });
    }

    /// The device this painter records on.
    pub fn gpu(&self) -> &GpuContext {
        &self.core.gpu
    }

    /// Compile skin materials off the painting thread, and call `wake` when
    /// one is ready. Until then the text or surface it shades paints natively;
    /// the next paint after `wake` uses it, and bumps
    /// [`Self::image_revision`] so retained outputs are recorded again.
    ///
    /// Without a waker nothing would paint again for a pipeline compiled in
    /// the background, so a painter without one (offscreen snapshots) compiles
    /// in place, on the frame that first draws the material.
    pub fn set_material_waker(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let ready = Arc::clone(&self.core.materials_ready);
        self.core.material_waker = Some(Arc::new(move || {
            ready.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            wake();
        }));
        self.core.adopt_material_waker();
    }

    /// Wake the owning event loop when an asynchronous URL image completes.
    pub fn set_image_waker(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let keyed: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |_| wake());
        self.set_image_update_waker(keyed);
    }

    /// Wake the owning host with the URL/resource key that completed.
    /// Hosts can use this to redraw only targets that reference the resource.
    pub fn set_image_update_waker(&mut self, wake: Arc<dyn Fn(&str) + Send + Sync>) {
        self.core.url_cache.set_wake(wake);
    }

    /// Policy-gated egress for the `http(s)` `url(...)` images of the scenes
    /// painted next. `None` refuses remote images; `data:`, `file:` and jailed
    /// paths are unaffected.
    ///
    /// A shared painter is given each document's host before painting it, and
    /// caches results per host, so no document sees an image another
    /// document's policy admitted. Identity is the `Arc`: pass clones of one
    /// host, or images are refetched on every change.
    pub fn set_resource_fetch_host(&mut self, host: Option<SharedFetchHost>) {
        self.core.url_cache.set_fetch_host(host.clone());
        self.core.fetch_host = host;
    }

    /// The exclusive owner changed egress for every target of this painter.
    /// Cancel the old requests even if no future batch rebuild will trim them.
    pub(crate) fn retire_resource_fetch_host(&mut self, host: &SharedFetchHost) {
        self.core.url_cache.release_fetch_host(host);
    }

    pub fn has_image_updates(&self) -> bool {
        self.core.url_cache.has_updates()
    }

    /// Offscreen hosts can await completion and paint again without polling pixels.
    pub fn has_pending_images(&self) -> bool {
        self.core.url_cache.has_pending()
    }

    /// Drain completed image resources before deciding whether a retained
    /// target can be reused. Polling only while painting would leave a static
    /// target permanently displaying its pre-decode pixels.
    pub(crate) fn refresh_image_resources(&mut self) -> bool {
        self.core.refresh_image_resources()
    }

    /// Completion generation, used by layered snapshot hosts to repaint earlier layers.
    pub fn image_revision(&self) -> u64 {
        self.core.image_revision
    }

    /// The natural sizes of `url(...)` content images that `target` drew
    /// since the last call (`None` is the target of [`Self::paint`]): an
    /// image it started showing or that decoded at another size, and one it
    /// drew into a box with no area yet. A box whose width or height is auto
    /// takes that size once the document has it.
    ///
    /// Take them after presenting and hand them to the document painted, with
    /// [`commit_image_natural_sizes`]; the built-in host does. Painting never
    /// writes the document. Nothing new costs nothing.
    pub fn take_image_natural_sizes(
        &mut self,
        target: Option<RenderTargetId>,
    ) -> Vec<ImageNaturalSize> {
        let key = target.unwrap_or(DEFAULT_TARGET_ID);
        self.targets
            .get_mut(&key)
            .map(|state| state.natural_sizes.take())
            .unwrap_or_default()
    }

    /// GPU counters from the last `paint` that encoded a real command buffer.
    /// `None` if this painter has not encoded, or the last `paint` was skipped.
    pub fn last_gpu_work(&self) -> Option<GpuWorkObservation> {
        self.core.last_gpu_work
    }

    /// Descriptor upload counters from the last encoded `paint`. Steady
    /// timestamp frames keep `motion_descriptors_uploaded` at zero.
    pub fn last_motion_work(&self) -> nana_ui_core::MotionWorkCounters {
        self.core.motion.last_work()
    }

    /// Batch / upload / encode timings from the last encoded `paint`. Submit is
    /// filled only after the host calls [`Self::record_submit`].
    pub fn last_gpu_timings(&self) -> Option<GpuStageTimings> {
        self.core.last_gpu_timings
    }

    /// Whether the most recent `paint` / `paint_target` rendered only the
    /// requested damage rectangle on top of preserved target contents.
    pub fn last_frame_used_partial_damage(&self) -> bool {
        self.core.last_frame_used_partial_damage
    }

    /// Synchronizes `last_slot_revs` to current `host_textures` damage revisions
    /// and returns the union of mapped physical-pixel damage rectangles when
    /// `target_id` holds a matching cached batch eligible for partial-damage
    /// incremental repaint.
    pub(crate) fn host_texture_damage_for_cached_scene(
        &self,
        target_id: Option<RenderTargetId>,
        scene: &UiScene,
        viewport: ScenePaintViewport,
        presentation: ScenePresentationProfile,
        host_textures: Option<&HostTextureRegistry>,
        last_slot_revs: &mut std::collections::HashMap<nana_ui_runtime::TextureSlotName, u64>,
    ) -> Option<PhysicalRect> {
        let state = self.targets.get(&target_id.unwrap_or(DEFAULT_TARGET_ID))?;
        let batch = state.prepared_batch.as_ref()?;
        let registry = host_textures?;
        let mut eligible = batch.key.instance == scene.instance_id()
            && batch.key.presentation_epoch == scene.presentation_epoch()
            && batch.key.viewport == viewport
            && batch.key.presentation == presentation
            && batch.image_revision == self.core.image_revision
            && batch.text_placement_epoch == self.core.text.placement_epoch()
            && !state.text.order_settle_due()
            && batch.max_group_depth == 0
            && matches!(batch.commands.first(), Some(DrawCommand::HostTexture(_)));
        let mut total_damage = PhysicalRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        for command in &batch.commands {
            match command {
                DrawCommand::Custom { .. }
                | DrawCommand::Backdrop { .. }
                | DrawCommand::PathMask { .. }
                | DrawCommand::PushGroup { .. } => {
                    eligible = false;
                }
                DrawCommand::HostTexture(prepared) => {
                    let prev_rev = last_slot_revs
                        .get(&prepared.slot_name)
                        .copied()
                        .unwrap_or(0);
                    match registry.slot_damage_since(&prepared.slot_name, prev_rev) {
                        Some((current_rev, Some(slot_damage))) => {
                            last_slot_revs.insert(prepared.slot_name.clone(), current_rev);
                            if slot_damage.width > 0 && slot_damage.height > 0 {
                                match prepared.map_source_damage(slot_damage) {
                                    Some(mapped) => {
                                        total_damage =
                                            crate::gpu_view::union_physical(total_damage, mapped);
                                    }
                                    None => eligible = false,
                                }
                            }
                        }
                        Some((current_rev, None)) => {
                            last_slot_revs.insert(prepared.slot_name.clone(), current_rev);
                            eligible = false;
                        }
                        None => {
                            last_slot_revs.remove(&prepared.slot_name);
                            eligible = false;
                        }
                    }
                }
                _ => {}
            }
        }
        eligible.then_some(total_damage)
    }

    /// Records the current `damage_rev` for each `HostTexture` in `target_id`'s
    /// prepared batch after a full-frame paint so subsequent frames can compute
    /// incremental slot damage deltas.
    pub(crate) fn sync_host_texture_revisions(
        &self,
        target_id: Option<RenderTargetId>,
        host_textures: Option<&HostTextureRegistry>,
        last_slot_revs: &mut std::collections::HashMap<nana_ui_runtime::TextureSlotName, u64>,
    ) {
        let Some(registry) = host_textures else {
            last_slot_revs.clear();
            return;
        };
        let Some(batch) = self
            .targets
            .get(&target_id.unwrap_or(DEFAULT_TARGET_ID))
            .and_then(|state| state.prepared_batch.as_ref())
        else {
            return;
        };
        for command in &batch.commands {
            if let DrawCommand::HostTexture(prepared) = command
                && let Some((current_rev, _)) =
                    registry.slot_damage_since(&prepared.slot_name, u64::MAX)
            {
                last_slot_revs.insert(prepared.slot_name.clone(), current_rev);
            }
        }
    }

    /// Text shape-cache counters from the last `paint`: (hits, misses,
    /// evictions). Tests use this to pin the reshaping-skip contract.
    pub fn text_shape_cache_stats(&self) -> (usize, usize, usize) {
        self.core.text.shape_cache_stats()
    }

    /// Native glyph renderer counters: resolve, raster cache, atlas, upload
    /// and draw. Tests pin the cache and atlas contracts through these.
    ///
    /// Summed over every render target this painter serves, so a second
    /// window painted through [`Self::paint_target`] counts too.
    pub fn text_glyph_counters(&self) -> TextGlyphCounters {
        self.core
            .text
            .glyph_counters_across(self.targets.values().map(|state| &state.text))
    }

    /// What the icon atlases and the URL texture cache hold, beside the
    /// glyph atlas of [`Self::text_glyph_counters`]. Summed the same way.
    pub fn resource_counters(&self) -> ScenePainterResourceCounters {
        let (icon_atlases, icon_atlas_bytes) =
            IconPipeline::atlas_counters_across(self.targets.values().map(|state| &state.icons));
        let url = self.core.url_cache.counters();
        let (path_meshes, path_mesh_bytes) = self
            .targets
            .values()
            .map(|state| state.meshes.path_counters())
            .fold((0, 0), |(meshes, bytes), (m, b)| (meshes + m, bytes + b));
        ScenePainterResourceCounters {
            icon_atlases,
            icon_atlas_bytes,
            url_textures: url.textures,
            url_texture_gpu_bytes: url.gpu_bytes,
            url_texture_cpu_bytes: url.cpu_bytes,
            url_pending: url.pending,
            path_meshes,
            path_mesh_bytes,
        }
    }

    /// Render targets with retained state, and pipeline sets kept for
    /// profiles other than the selected one.
    #[cfg(test)]
    pub(crate) fn retained_shape(&self) -> (usize, usize) {
        let named = self
            .targets
            .keys()
            .filter(|id| **id != DEFAULT_TARGET_ID)
            .count();
        (named, self.pipeline_sets.len())
    }

    /// Replace the glyph atlas with one `page_edge` page within
    /// `byte_budget`, for tests that need eviction within a few labels.
    #[cfg(test)]
    pub(crate) fn use_small_text_atlas(&mut self, page_edge: u32, byte_budget: usize) {
        self.core.text = TextPipeline::with_atlas_budget(
            &self.core.device,
            self.core.working_format,
            page_edge,
            byte_budget,
            self.core.gpu.policy(),
        );
        let core = &mut self.core;
        for state in self.targets.values_mut() {
            state.text = core.text.new_target(&core.device);
            state.prepared_batch = None;
            state.painted = None;
        }
    }

    /// Record the submit of the frame this painter last encoded into.
    pub fn record_submit(&mut self, submission: &GpuSubmission) {
        if let Some(timings) = &mut self.core.last_gpu_timings {
            timings.submit = timings.submit.saturating_add(submission.cpu_duration());
        }
    }

    /// Release target-owned textures when a host closes a window or viewport.
    ///
    /// A fetch host no remaining target uses goes with it, cancelling its
    /// image requests now: URL caches only expire entries on frames that
    /// rebuild a batch, which static windows never do.
    pub fn remove_target(&mut self, id: RenderTargetId) {
        let Some(state) = self.targets.remove(&id) else {
            return;
        };
        self.core.url_cache.retire_demand(state.painted_demand.id());
        self.core.text.close_target(state.text);
        let Some(host) = state.bound_fetch_host else {
            return;
        };
        let closed = egress_of(Some(&host));
        if self
            .targets
            .values()
            .any(|state| egress_of(state.bound_fetch_host.as_ref()) == closed)
        {
            return;
        }
        if egress_of(self.core.fetch_host.as_ref()) == closed {
            self.set_resource_fetch_host(None);
        }
        self.core.url_cache.release_fetch_host(&host);
    }

    /// Paint with isolated target state while sharing device pipelines/caches.
    ///
    /// The same `frame` contract as [`Self::paint`]. Targets may share one
    /// frame; a painter records into one frame at a time and the host submits
    /// or drops it before painting into the next, because frames submitted
    /// out of order would land one's glyph-atlas writes under another's passes.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_target(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        frame: &mut FrameContext,
        target: &GpuRenderTarget,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.paint_target_with_options(
            id,
            scene,
            frame,
            target,
            viewport,
            ScenePaintOptions::default(),
            host_textures,
            gpu_renderers,
        )
    }

    /// Paint `scene` into an identified offscreen or multi-window target with explicit per-paint options.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_target_with_options(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        frame: &mut FrameContext,
        target: &GpuRenderTarget,
        viewport: ScenePaintViewport,
        options: ScenePaintOptions,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.begin_frame(Some(id), frame, target)?;
        self.try_sync_effective_presentation(scene, &viewport, host_textures)?;
        let link = crate::gpu_work::FrameLink::of(frame);
        let result = self.paint_recorded(
            id,
            scene,
            __framework::encoder(frame),
            __framework::target_view(target),
            viewport,
            options,
            host_textures,
            gpu_renderers,
            Some(link),
        );
        self.record_ordered_shared_writes(frame);
        if result.is_ok() {
            if let Some(state) = self.targets.get(&id) {
                HostTexturePipeline::keep_alive_until_complete(&state.host_textures, frame);
            }
            frame.record_retained_writes(&self.retained_targets, id.0);
        }
        result
    }

    /// Refuse a frame or target from another device, refuse to record on top
    /// of writes an unsubmitted frame still holds, and rebuild targets whose
    /// retained writes were rolled back by a dropped frame. `None` is the
    /// state behind [`Self::paint`].
    fn begin_frame(
        &mut self,
        target_id: Option<RenderTargetId>,
        frame: &FrameContext,
        target: &GpuRenderTarget,
    ) -> Result<(), ScenePaintError> {
        // A rejected frame must not report a previous target's successful work.
        self.core.last_frame_used_partial_damage = false;
        self.core.last_gpu_work = None;
        self.core.last_gpu_timings = None;
        self.core.last_dest_pass_counts = None;
        let expected = self.core.gpu.generation();
        for found in [frame.generation(), target.generation()] {
            if found != expected {
                return Err(ScenePaintError::DeviceMismatch { expected, found });
            }
        }
        // Before the drain: a frame dropped on another thread in between
        // releases the key and records its rollback in one step, so checking
        // in this order never paints over a rollback the drain missed.
        let in_flight = match target_id {
            Some(id) => self.retained_targets.in_flight(id.0, frame.id()),
            None => self.retained_default.in_flight(DEFAULT_TARGET, frame.id()),
        };
        if in_flight {
            return Err(ScenePaintError::TargetInFlight(target_id));
        }
        if self.retained_shared.has_rolled_back() {
            self.retained_shared.drain_rolled_back(|key| match key {
                SHARED_ATLAS => self.core.text.forget_atlas_placements(),
                SHARED_MOTION => self.core.motion.invalidate_uploads(),
                _ => unreachable!("shared upload ledger key"),
            });
        }
        // Atlas/motion buffers are shared by every target, and this frame's
        // queue writes land ahead of every pass; see `text/upload.rs`.
        let follows = self.core.last_frame == Some(frame.id());
        self.core.last_frame = Some(frame.id());
        self.core.text.set_follows_in_frame(follows);
        self.core.motion.set_follows_in_frame(follows);
        if self.retained_default.has_rolled_back() {
            self.retained_default.drain_rolled_back(|_| {});
            if let Some(state) = self.targets.remove(&DEFAULT_TARGET_ID) {
                self.core.text.close_target(state.text);
            }
        }
        if self.retained_targets.has_rolled_back() {
            let mut rolled_back = Vec::new();
            self.retained_targets
                .drain_rolled_back(|key| rolled_back.push(RenderTargetId(key)));
            for id in rolled_back {
                // Not `remove_target`: the window is still open, so its fetch
                // host and image caches stay.
                if let Some(state) = self.targets.remove(&id) {
                    self.core.text.close_target(state.text);
                }
            }
        }
        Ok(())
    }

    /// Encode `scene` into `frame`, drawing into `target`.
    ///
    /// The painter keeps geometry on the GPU between frames and records part
    /// of each frame's writes to it as copies in the frame's encoder — text
    /// blocks and the text draw order (#224) — then treats them as done. A
    /// frame dropped instead of submitted rolls that back: the next paint of
    /// the target rebuilds it. Painting a target again while an earlier frame
    /// that painted it is still unsubmitted is refused with
    /// [`ScenePaintError::TargetInFlight`], since the two could be submitted
    /// out of order. On `Err` nothing was recorded.
    pub fn paint(
        &mut self,
        scene: &UiScene,
        frame: &mut FrameContext,
        target: &GpuRenderTarget,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.paint_with_options(
            scene,
            frame,
            target,
            viewport,
            ScenePaintOptions::default(),
            host_textures,
            gpu_renderers,
        )
    }

    /// Paint `scene` into the default render target with explicit per-paint options.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_with_options(
        &mut self,
        scene: &UiScene,
        frame: &mut FrameContext,
        target: &GpuRenderTarget,
        viewport: ScenePaintViewport,
        options: ScenePaintOptions,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.begin_frame(None, frame, target)?;
        self.try_sync_effective_presentation(scene, &viewport, host_textures)?;
        let link = crate::gpu_work::FrameLink::of(frame);
        let result = self.paint_recorded(
            DEFAULT_TARGET_ID,
            scene,
            __framework::encoder(frame),
            __framework::target_view(target),
            viewport,
            options,
            host_textures,
            gpu_renderers,
            Some(link),
        );
        self.record_ordered_shared_writes(frame);
        if result.is_ok() {
            if let Some(state) = self.targets.get(&DEFAULT_TARGET_ID) {
                HostTexturePipeline::keep_alive_until_complete(&state.host_textures, frame);
            }
            frame.record_retained_writes(&self.retained_default, DEFAULT_TARGET);
        }
        result
    }

    /// Recorded whether or not the paint succeeded: the copies are in the
    /// frame's encoder either way, and a frame that is dropped loses them.
    fn record_ordered_shared_writes(&mut self, frame: &mut FrameContext) {
        if self.core.text.take_ordered_flush() {
            frame.record_retained_writes(&self.retained_shared, SHARED_ATLAS);
        }
        if self.core.motion.take_ordered_uploads() {
            frame.record_retained_writes(&self.retained_shared, SHARED_MOTION);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_recorded(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        options: ScenePaintOptions,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
        frame: Option<crate::gpu_work::FrameLink>,
    ) -> Result<(), ScenePaintError> {
        let requested_damage = options.damage;
        let presentation_parameters = options.presentation.normalized();
        let core = &mut self.core;
        // A rejected frame must not report a previous target's successful work.
        core.last_frame_used_partial_damage = false;
        core.last_gpu_work = None;
        core.last_gpu_timings = None;
        core.last_dest_pass_counts = None;
        core.refresh_image_resources();
        let target_res = self
            .targets
            .entry(id)
            .or_insert_with(|| core.new_target_resources());
        if target_res.image_revision != core.image_revision {
            target_res.image_revision = core.image_revision;
            target_res.painted = None;
            target_res.last_encoded_key = None;
            target_res.changing_frame_streak = 0;
            target_res.quads.invalidate_image_bindings();
            HostTexturePipeline::invalidate_target_image_bindings(&mut target_res.host_textures);
        }
        if target_res.material_revision != core.material_revision {
            target_res.material_revision = core.material_revision;
            target_res.painted = None;
            target_res.last_encoded_key = None;
            target_res.changing_frame_streak = 0;
            target_res.prepared_batch = None;
        }
        // URL bindings and a cached dest are keyed by URL alone, so ones built
        // through another document's egress must not be reused under this one.
        if egress_of(core.fetch_host.as_ref()) != egress_of(target_res.bound_fetch_host.as_ref()) {
            target_res.bound_fetch_host = core.fetch_host.clone();
            target_res.painted = None;
            target_res.last_encoded_key = None;
            target_res.changing_frame_streak = 0;
            target_res.prepared_batch = None;
            target_res.quads.invalidate_image_bindings();
            HostTexturePipeline::invalidate_target_image_bindings(&mut target_res.host_textures);
        }
        target_res.text.set_subpixel(&core.device, options.text_aa);
        let instance = scene.instance_id();
        // Every rounded corner this frame paints takes the scene's shape. A
        // change of shape is a new scene instance, so a reused batch or dest
        // was painted with this one.
        let corner_exponent = scene.corner_shape().exponent();
        // Also resolves this frame's `PreparedBatch` key, so nothing below
        // walks `FramePlan::custom_nodes` again.
        let resolved = validate_scene(scene, host_textures, gpu_renderers, core.gpu.generation())?;
        if viewport.physical_size[0] == 0 || viewport.physical_size[1] == 0 {
            core.last_gpu_work = None;
            core.last_gpu_timings = None;
            core.last_dest_pass_counts = None;
            target_res.painted = None;
            target_res.last_encoded_key = None;
            target_res.changing_frame_streak = 0;
            // Nothing of this target is visible: withdraw its demand.
            target_res.painted_demand.begin();
            target_res.painted_demand.commit(host_textures);
            core.url_cache.retire_demand(target_res.painted_demand.id());
            return Ok(());
        }

        let scale = if viewport.scale_factor.is_finite() && viewport.scale_factor > 0.0 {
            viewport.scale_factor
        } else {
            1.0
        };
        let dest_physical = [
            (viewport.logical_size[0] * scale).round().max(1.0) as u32,
            (viewport.logical_size[1] * scale).round().max(1.0) as u32,
        ];
        let blit_origin = [
            viewport.target_origin[0] * scale,
            viewport.target_origin[1] * scale,
        ];
        let origin = PaintOrigin::new(paint_origin([0.0, 0.0], viewport.scene_origin), scale);
        let viewport_clip = LogicalRect::viewport([0.0, 0.0], viewport.logical_size);
        let gpu_work = GpuWorkSink::with_frame(&core.gpu, frame);
        let clear_color = normalize_clear_color(viewport.clear_color);
        let clear = wgpu::Color {
            r: (clear_color[0] * clear_color[3]) as f64,
            g: (clear_color[1] * clear_color[3]) as f64,
            b: (clear_color[2] * clear_color[3]) as f64,
            a: clear_color[3] as f64,
        };
        let surface_clear = core.presentation.surface_clear_with_parameters(
            clear_color,
            options.alpha,
            presentation_parameters,
        );
        let painted = PaintedDest {
            instance,
            presentation_epoch: scene.presentation_epoch(),
            presentation: core.presentation,
            viewport,
            size: dest_physical,
            alpha: options.alpha,
            text_aa: options.text_aa,
        };

        // Per-glyph presentation moves with the motion clock, so a frame with
        // a live one is never the dest already painted.
        let motion_live = scene
            .motion_gpu_descriptors()
            .iter()
            .any(|descriptor| descriptor.is_live());
        let glyph_live = scene.glyph_presentation_live();
        let gpu_payload_live = scene.gpu_payload_live();
        let gpu_motion_live = motion_live || glyph_live || gpu_payload_live;
        core.text.write_motion(
            &mut target_res.text,
            &core.queue,
            scene.motion_gpu_now(),
            Some(&gpu_work),
        );
        if target_res.painted == Some(painted)
            && !gpu_motion_live
            && let Some(dest) = target_res.dest.as_mut()
            && dest.has_color()
            && dest.width == dest_physical[0]
            && dest.height == dest_physical[1]
        {
            target_res.last_encoded_key = Some(painted);
            target_res.changing_frame_streak = 0;
            dest.release_prefix();
            if target_res
                .prepared_batch
                .as_ref()
                .is_none_or(|batch| batch.max_group_depth == 0)
            {
                dest.trim_idle_groups(&core.device, 0, 0, false, Some(&gpu_work));
            }
            if !BackdropPipeline::needs_backdrop(&target_res.backdrop) {
                core.backdrop
                    .finish_frame(&mut target_res.backdrop, &core.device, Some(&gpu_work));
            }
            dest.set_presentation_parameters(&core.queue, presentation_parameters, Some(&gpu_work));
            let encode_started = Instant::now();
            let mut dest_passes = DestPassCounts {
                msaa_allocated: dest.msaa_allocated,
                ..DestPassCounts::default()
            };
            dest.blit(
                encoder,
                target,
                blit_origin[0],
                blit_origin[1],
                viewport.physical_size,
                viewport.clear.then_some(surface_clear),
                options.alpha,
                Some(&gpu_work),
                &mut dest_passes,
            );
            core.last_gpu_work = Some(gpu_work.snapshot());
            core.last_gpu_timings = Some(GpuStageTimings {
                batch: std::time::Duration::ZERO,
                gpu_upload: std::time::Duration::ZERO,
                encode: encode_started.elapsed(),
                submit: std::time::Duration::ZERO,
            });
            core.last_dest_pass_counts = Some(dest_passes);
            return Ok(());
        }

        let reused = target_res.prepared_batch.as_ref().is_some_and(|batch| {
            resolved.cacheable
                && batch.key == painted
                && batch.resources == resolved.resources
                && batch.renderers == resolved.renderers
                && batch.image_revision == core.image_revision
                && batch.text_placement_epoch == core.text.placement_epoch()
                // The order is about to give back the gaps text that stopped
                // growing left (#230); only a flush does that.
                && !target_res.text.order_settle_due()
        });
        if reused {
            target_res.text.note_reused_frame();
        }
        let scratch = &mut core.encode_scratch;
        let (batch, gpu_upload) = if reused {
            (std::time::Duration::ZERO, std::time::Duration::ZERO)
        } else {
            let batch_started = Instant::now();
            core.url_cache.begin_frame();
            target_res.natural_sizes.begin_pass();
            target_res.quads.begin_frame(scale);
            target_res.painted_demand.begin();
            target_res.meshes.begin_frame();
            icon::IconPipeline::begin_frame(&mut target_res.icons, dest_physical);
            core.text.begin_frame(&mut target_res.text, dest_physical);
            BackdropPipeline::begin_frame(&mut target_res.backdrop);
            #[cfg(feature = "charts")]
            target_res.charts.begin_frame();
            target_res.quads.set_corner_exponent(corner_exponent);
            target_res.meshes.set_corner_exponent(corner_exponent);
            target_res.text.set_corner_exponent(corner_exponent);
            core.host_textures.set_corner_exponent(corner_exponent);

            scratch.begin_build();
            let mut group_depth = 0usize;
            let mut group_slots = 0u32;
            // A clipped painter layer wholly outside the target, skipped
            // with everything in it: its node and how deep in it we are.
            // Culling cannot drop the pair itself, as its contents are
            // bounded by the layer's clip, not by their own bounds.
            let mut skipped_layer: Option<(nana_ui_runtime::StableNodeId, usize)> = None;
            let whole_dest = PhysicalRect {
                x: 0,
                y: 0,
                width: dest_physical[0],
                height: dest_physical[1],
            };
            // Scissors/text clips round to physical pixels. Include their
            // possible edge coverage even at fractional origins/low DPI.
            let edge_padding = 1.0 / scale;
            let operations = scene
                .visible_operations(nana_ui_scene::SceneRect {
                    x: viewport.scene_origin[0] - edge_padding,
                    y: viewport.scene_origin[1] - edge_padding,
                    width: viewport.logical_size[0] + edge_padding * 2.0,
                    height: viewport.logical_size[1] + edge_padding * 2.0,
                })
                .map_err(|error| ScenePaintError::InvalidRenderGraph(error.to_string()))?;
            for operation in operations.iter() {
                let id = match operation {
                    RenderOperation::PrepareExternal(_) => continue,
                    RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
                };
                let Some(primitive) = scene.draw_primitive(id) else {
                    continue;
                };
                if let Some((node, depth)) = &mut skipped_layer {
                    if *node == primitive.node {
                        match primitive.kind {
                            ScenePrimitiveKind::LayerBegin { .. } => *depth += 1,
                            ScenePrimitiveKind::LayerEnd { .. } => {
                                *depth -= 1;
                                if *depth == 0 {
                                    skipped_layer = None;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    skipped_layer = None;
                }
                let encode = scene.compositor_primitive_encode(
                    primitive.primitive,
                    primitive.transform,
                    primitive.paint_opacity,
                );
                let opacity = encode.opacity;
                let encode_transform = encode.transform;
                target_res
                    .quads
                    .set_motion(encode.motion_ids, encode.transform_origin);
                // A layer never outlives the node that opened it.
                if scratch
                    .painter_layers
                    .last()
                    .is_some_and(|node| *node != primitive.node)
                {
                    scratch.batching.close_all();
                    while scratch.painter_layers.pop().is_some() {
                        group_depth = group_depth.saturating_sub(1);
                        scratch.commands.push(DrawCommand::PopGroup);
                    }
                }
                if sync_opacity_groups(
                    &mut scratch.commands,
                    &mut scratch.group_stack,
                    &mut group_depth,
                    &mut group_slots,
                    &mut scratch.max_group_depth,
                    &mut scratch.group_slots,
                    &scene.primitive_opacity_groups(primitive.primitive),
                    scene,
                    origin,
                    scale,
                ) {
                    scratch.batching.close_all();
                }
                target_res.quads.set_filtering_groups(
                    scratch
                        .group_stack
                        .iter()
                        .filter(|group| !group.filter.is_identity())
                        .count(),
                );
                // Both ends of a painter layer reach here whatever they
                // cover, so no culling below can leave one open.
                match &primitive.kind {
                    ScenePrimitiveKind::LayerBegin {
                        opacity: layer_opacity,
                        blend,
                        clip: layer_clip,
                    } => {
                        let layer_clip = layer_clip.map(|rect| primitive.place(rect));
                        if let Some(rect) = layer_clip
                            && !encode_transform.is_projective()
                            && !rect_meets_target(
                                rect,
                                clip::paint_affine(encode_transform.0, origin),
                                scale,
                                dest_physical,
                            )
                        {
                            skipped_layer = Some((primitive.node, 1));
                            continue;
                        }
                        scratch.batching.close_all();
                        let frag = layer_clip
                            .map(|rect| {
                                clip::local_rect_clip(rect, encode_transform, origin)
                                    .for_physical_pixels(scale)
                            })
                            .unwrap_or(clip::FragmentClip::PASS);
                        let layer = group_depth;
                        let slot = group_slots;
                        group_slots = group_slots.saturating_add(1);
                        group_depth = group_depth.saturating_add(1);
                        scratch.max_group_depth = scratch.max_group_depth.max(group_depth);
                        scratch.group_slots.push(GroupSlot::dest(
                            *layer_opacity,
                            [1.0, 1.0, 1.0],
                            0.0,
                            0.0,
                            blend.gpu_index(),
                            frag,
                        ));
                        scratch
                            .commands
                            .push(DrawCommand::PushGroup { layer, slot });
                        scratch.painter_layers.push(primitive.node);
                        continue;
                    }
                    ScenePrimitiveKind::LayerEnd { mask } => {
                        scratch.batching.close_all();
                        if scratch.painter_layers.pop().is_none() {
                            continue;
                        }
                        if let Some(mask) = mask {
                            let (affine, persp) =
                                paint_transform(encode_transform.0, encode_transform.1, origin);
                            if let Some(range) = target_res.meshes.push_path(
                                &mask.mesh,
                                [
                                    primitive.origin[0] + mask.origin[0],
                                    primitive.origin[1] + mask.origin[1],
                                ],
                                mesh_affine(affine, persp),
                                1.0,
                                clip::FragmentClip::PASS,
                                mask.node_mask.as_ref(),
                            ) {
                                scratch.commands.push(DrawCommand::PathMask {
                                    range,
                                    scissor: whole_dest,
                                    mode: mask.mode,
                                });
                            }
                        }
                        group_depth = group_depth.saturating_sub(1);
                        scratch.commands.push(DrawCommand::PopGroup);
                        continue;
                    }
                    _ => {}
                }
                let Some(clip) = intersect_clips(viewport_clip, &primitive.clips, origin) else {
                    continue;
                };
                let Some(scissor) = physical_scissor(clip, scale, dest_physical) else {
                    continue;
                };
                let clips = match &primitive.kind {
                    ScenePrimitiveKind::Quad { surface, .. } => match &surface.polygon_clip {
                        Some(own) => clip::without_own_polygon(
                            &primitive.clips,
                            primitive.placed_bounds(),
                            primitive.transform,
                            own,
                        ),
                        None => &primitive.clips[..],
                    },
                    _ => &primitive.clips[..],
                };
                let frag_clip = fragment_clip(clips, origin);
                // Quads, icons and host textures test no polygon or ellipse:
                // an innermost one dest-wraps them with its rectangle
                // (`clip_dests_for`), which must not ramp in their own shader
                // as well, or an edge the two share would fade twice.
                let vertex_clip = if frag_clip.polygon_count > 0 {
                    FragmentClip::PASS
                } else {
                    frag_clip
                };
                let (affine, persp) =
                    paint_transform(encode_transform.0, encode_transform.1, origin);
                // The geometry is in its node's coordinates: put it where the
                // node stands first, one add per coordinate, as a box laid out
                // there is. Only then does the transform -- and the grid it
                // snaps a translation to -- apply, so a box that moved paints
                // what one laid out where it now stands does.
                let bounds = local_rect(primitive.placed_bounds());
                // Wrapping drains and re-inserts this primitive's commands
                // behind a `PushGroup`, so no batch may span the edit.
                clip_dests_into(&primitive.kind, clips, origin, &mut scratch.clip_dests);
                if !scratch.clip_dests.is_empty() {
                    scratch.batching.close_all();
                }
                let command_start = scratch.commands.len();
                let painted_start = scratch.batching.painted.len();
                match &primitive.kind {
                    ScenePrimitiveKind::Quad {
                        background,
                        border_color,
                        border_width,
                        corner_radius,
                        shadow,
                        surface,
                    } if quad_paints_nothing(
                        *background,
                        *border_color,
                        *border_width,
                        shadow.as_ref(),
                        surface,
                    ) =>
                    {
                        // A box with no fill, edge, shadow or image draws no
                        // pixel. Shading it anyway costs its whole area: a
                        // window-sized rounded container on every frame.
                        target_res
                            .natural_sizes
                            .note_quad(&core.url_cache, id, bounds, surface);
                    }
                    ScenePrimitiveKind::Quad {
                        background,
                        border_color,
                        border_width,
                        corner_radius,
                        shadow,
                        surface,
                    } => {
                        let painted = quad_painted_bounds(
                            bounds,
                            shadow.as_ref(),
                            surface,
                            affine,
                            persp,
                            scale,
                            scissor,
                            dest_physical,
                        );
                        if let Some(index) = core.quads.push_with_work(
                            &mut target_res.quads,
                            &core.device,
                            &core.queue,
                            &mut core.url_cache,
                            bounds,
                            clip,
                            vertex_clip,
                            affine,
                            persp,
                            *background,
                            *border_color,
                            *border_width,
                            *corner_radius,
                            *shadow,
                            opacity,
                            surface,
                            Some(&gpu_work),
                        ) {
                            // The blurred backdrop is part of the surface's
                            // paint and takes its opacity, as CSS's does: a
                            // fully transparent surface shows none of it.
                            if let Some(filter) = surface
                                .backdrop_filter
                                .filter(|f| f.is_active() && opacity > 0.0)
                            {
                                let world_bounds =
                                    transformed_aabb_projective(bounds, affine, persp);
                                let phys = physical_bounds(world_bounds, scale, scissor);
                                let radii = corner_radius.map(|r| r * scale);
                                let bidx = BackdropPipeline::push(
                                    &mut target_res.backdrop,
                                    index,
                                    [
                                        phys.x as f32,
                                        phys.y as f32,
                                        phys.width as f32,
                                        phys.height as f32,
                                    ],
                                    radii,
                                    filter,
                                    opacity,
                                    frag_clip.for_physical_pixels(scale),
                                    scale,
                                    dest_physical,
                                    bounds,
                                    affine,
                                );
                                scratch.batching.close_all();
                                scratch.commands.push(DrawCommand::Backdrop { index: bidx });
                            }
                            for quad_index in index..target_res.quads.pending_len() {
                                push_quad(
                                    &mut scratch.commands,
                                    &mut scratch.batching,
                                    quad_index,
                                    scissor,
                                    painted,
                                );
                            }
                        }
                        target_res
                            .natural_sizes
                            .note_quad(&core.url_cache, id, bounds, surface);
                    }
                    ScenePrimitiveKind::QuadBatch {
                        bounds: batch,
                        background,
                        border_color,
                        border_width,
                        corner_radius,
                        shadow,
                        surface,
                    } => {
                        for item in batch {
                            let item_bounds = local_rect(primitive.place(*item));
                            let painted = quad_painted_bounds(
                                item_bounds,
                                shadow.as_ref(),
                                surface,
                                affine,
                                persp,
                                scale,
                                scissor,
                                dest_physical,
                            );
                            if let Some(index) = core.quads.push_with_work(
                                &mut target_res.quads,
                                &core.device,
                                &core.queue,
                                &mut core.url_cache,
                                item_bounds,
                                clip,
                                vertex_clip,
                                affine,
                                persp,
                                *background,
                                *border_color,
                                *border_width,
                                *corner_radius,
                                *shadow,
                                opacity,
                                surface,
                                Some(&gpu_work),
                            ) {
                                if let Some(filter) = surface
                                    .backdrop_filter
                                    .filter(|f| f.is_active() && opacity > 0.0)
                                {
                                    let world_bounds =
                                        transformed_aabb_projective(item_bounds, affine, persp);
                                    let phys = physical_bounds(world_bounds, scale, scissor);
                                    let radii = corner_radius.map(|r| r * scale);
                                    let bidx = BackdropPipeline::push(
                                        &mut target_res.backdrop,
                                        index,
                                        [
                                            phys.x as f32,
                                            phys.y as f32,
                                            phys.width as f32,
                                            phys.height as f32,
                                        ],
                                        radii,
                                        filter,
                                        opacity,
                                        frag_clip.for_physical_pixels(scale),
                                        scale,
                                        dest_physical,
                                        item_bounds,
                                        affine,
                                    );
                                    scratch.batching.close_all();
                                    scratch.commands.push(DrawCommand::Backdrop { index: bidx });
                                }
                                for quad_index in index..target_res.quads.pending_len() {
                                    push_quad(
                                        &mut scratch.commands,
                                        &mut scratch.batching,
                                        quad_index,
                                        scissor,
                                        painted,
                                    );
                                }
                            }
                        }
                    }
                    ScenePrimitiveKind::Text {
                        content,
                        color,
                        size,
                        weight,
                        family,
                        line_height,
                        wrap,
                        ellipsis,
                        max_lines,
                        shaping,
                        horizontal_alignment,
                        vertical_alignment,
                        spans,
                        letter_spacing,
                        text_shadow,
                        underline: _,
                        line_through: _,
                        font_features,
                        italic,
                        wrap_break,
                        opentype,
                        // The Runtime's retained handle: the layout this
                        // node was *measured* with. The painter draws it
                        // rather than laying the same paragraph out again,
                        // so there is one paragraph, not two that agree.
                        layout,
                        rich,
                        presentation,
                    } => {
                        // A text node's glyphs are retained per node and
                        // per pass, so the shadows under a label are their
                        // own entries over the same shaped paragraph and
                        // the same glyph bitmaps.
                        let node = primitive.node.get();
                        let slot = id.slot;
                        let mut pass = 0u32;
                        let opaque_backdrop = group_depth == 0;
                        // Shadows, outlines and decoration lines are
                        // instances of the paragraph's own entry, in paint
                        // order, built from the same glyphs.
                        core.text.set_effects(rich.as_ref(), *text_shadow);
                        core.text.set_glyph_presentation(presentation.as_ref());
                        let mut push_text = |commands: &mut Vec<DrawCommand>,
                                             batching: &mut Batching,
                                             extra_offset: [f32; 2],
                                             color_override: Option<[f32; 4]>,
                                             linear_color_override: Option<[f32; 4]>,
                                             paint_color_override: Option<
                            nana_ui_core::PaintColor,
                        >| {
                            core.text.set_linear_color_override(linear_color_override);
                            core.text.set_paint_color_override(paint_color_override);
                            let prepared = core.text.prepare(
                                &mut target_res.text,
                                &core.device,
                                bounds,
                                clip,
                                scale,
                                content,
                                color_override.or(color.map(nana_ui_core::PaintColor::to_srgb)),
                                *size,
                                *weight,
                                family.as_deref(),
                                *line_height,
                                *wrap,
                                *wrap_break,
                                *italic,
                                *ellipsis,
                                *max_lines,
                                *shaping,
                                *horizontal_alignment,
                                *vertical_alignment,
                                spans,
                                *letter_spacing,
                                font_features,
                                opentype,
                                affine,
                                persp,
                                frag_clip,
                                opacity,
                                extra_offset,
                                text::EntryKey { node, slot, pass },
                                primitive.revision,
                                layout.as_ref(),
                                opaque_backdrop,
                            );
                            pass += 1;
                            if let Some(prepared) = prepared {
                                let painted = painted_bounds(
                                    prepared.ink,
                                    0.0,
                                    affine,
                                    persp,
                                    scale,
                                    scissor,
                                );
                                push_text_run(
                                    commands,
                                    batching,
                                    &mut target_res.text,
                                    prepared,
                                    scissor,
                                    painted,
                                );
                            }
                        };
                        push_text(
                            &mut scratch.commands,
                            &mut scratch.batching,
                            [0.0, 0.0],
                            None,
                            explicit_text_color(*color).map(pack_paint_color),
                            explicit_text_color(*color),
                        );
                    }
                    ScenePrimitiveKind::QuadColorBatch {
                        bounds: batch,
                        colors,
                        border_color,
                        border_width,
                        corner_radius,
                    } => {
                        // Per-item solid colors (editor color swatches); no
                        // shadow and no surface paint by construction, so the
                        // batch collapses to one quads.push per item.
                        let no_shadow: Option<nana_ui_runtime::ComponentElevation> = None;
                        let default_surface = nana_ui_scene::QuadSurfacePaint::default();
                        for (item, color) in batch.iter().zip(colors.iter()) {
                            let item_bounds = local_rect(primitive.place(*item));
                            let painted = quad_painted_bounds(
                                item_bounds,
                                None,
                                &default_surface,
                                affine,
                                persp,
                                scale,
                                scissor,
                                dest_physical,
                            );
                            if let Some(index) = core.quads.push_with_work(
                                &mut target_res.quads,
                                &core.device,
                                &core.queue,
                                &mut core.url_cache,
                                item_bounds,
                                clip,
                                vertex_clip,
                                affine,
                                persp,
                                Some(nana_ui_core::PaintColor::srgb(*color)),
                                border_color.map(nana_ui_core::PaintColor::srgb),
                                *border_width,
                                *corner_radius,
                                no_shadow,
                                opacity,
                                &default_surface,
                                Some(&gpu_work),
                            ) {
                                for quad_index in index..target_res.quads.pending_len() {
                                    push_quad(
                                        &mut scratch.commands,
                                        &mut scratch.batching,
                                        quad_index,
                                        scissor,
                                        painted,
                                    );
                                }
                            }
                        }
                    }
                    ScenePrimitiveKind::Icon { icon, color } => {
                        if let Some(prepared) = core.icons.prepare_with_work(
                            &mut target_res.icons,
                            &core.device,
                            &core.queue,
                            bounds,
                            affine,
                            persp,
                            scale,
                            *icon,
                            color.unwrap_or(BLACK),
                            opacity,
                            vertex_clip,
                            Some(&gpu_work),
                        ) {
                            push_icon(
                                &mut scratch.commands,
                                &mut scratch.batching,
                                &target_res.icons,
                                prepared,
                                scissor,
                                painted_bounds(bounds, 0.0, affine, persp, scale, scissor),
                            );
                        }
                    }
                    ScenePrimitiveKind::IconBatch {
                        bounds: batch,
                        icon,
                        color,
                    } => {
                        for item in batch {
                            let item_bounds = local_rect(primitive.place(*item));
                            if let Some(prepared) = core.icons.prepare_with_work(
                                &mut target_res.icons,
                                &core.device,
                                &core.queue,
                                item_bounds,
                                affine,
                                persp,
                                scale,
                                *icon,
                                nana_ui_core::PaintColor::srgb(
                                    color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                ),
                                opacity,
                                vertex_clip,
                                Some(&gpu_work),
                            ) {
                                push_icon(
                                    &mut scratch.commands,
                                    &mut scratch.batching,
                                    &target_res.icons,
                                    prepared,
                                    scissor,
                                    painted_bounds(item_bounds, 0.0, affine, persp, scale, scissor),
                                );
                            }
                        }
                    }
                    ScenePrimitiveKind::Spinner { phase, color } => {
                        if let Some(range) = target_res.meshes.push_spinner(
                            bounds,
                            mesh_affine(affine, persp),
                            *phase,
                            color.unwrap_or(BLACK),
                            opacity,
                            frag_clip,
                        ) {
                            push_mesh_draw(
                                &mut scratch.commands,
                                &mut scratch.batching,
                                range,
                                scissor,
                                painted_bounds(
                                    bounds,
                                    0.0,
                                    mesh_affine(affine, persp),
                                    [0.0, 0.0],
                                    scale,
                                    scissor,
                                ),
                            );
                        }
                    }
                    ScenePrimitiveKind::Stroke {
                        points,
                        width,
                        color,
                        widths,
                        cap,
                        pattern,
                    } => {
                        let (dash, dash_offset, colors, path_length) = match pattern.as_deref() {
                            Some(pattern) => (
                                pattern.dash.as_slice(),
                                pattern.dash_offset,
                                pattern.colors.as_slice(),
                                pattern.path_length,
                            ),
                            None => ([].as_slice(), 0.0, [].as_slice(), 0.0),
                        };
                        let points =
                            placed_points(&mut core.placed_points, points, primitive.origin);
                        if let Some(range) = target_res.meshes.push_stroke_with_path_length(
                            points,
                            StrokeStyle {
                                width: *width,
                                widths,
                                cap: *cap,
                                dash,
                                dash_offset,
                                colors,
                            },
                            mesh_affine(affine, persp),
                            *color,
                            opacity,
                            frag_clip,
                            path_length,
                        ) {
                            // Caps and joins reach half a stroke width past
                            // the polyline; one more pixel covers AA.
                            let reach =
                                widths.iter().copied().fold(*width, f32::max).max(0.0) + 1.0;
                            push_mesh_draw(
                                &mut scratch.commands,
                                &mut scratch.batching,
                                range,
                                scissor,
                                painted_bounds(
                                    polyline_bounds(points).unwrap_or(bounds),
                                    reach,
                                    mesh_affine(affine, persp),
                                    [0.0, 0.0],
                                    scale,
                                    scissor,
                                ),
                            );
                        }
                    }
                    ScenePrimitiveKind::LayerBegin { .. } | ScenePrimitiveKind::LayerEnd { .. } => {
                        unreachable!("painter layers are handled before culling")
                    }
                    ScenePrimitiveKind::Path {
                        mesh,
                        origin,
                        node_mask,
                    } => {
                        let path_affine = mesh_affine(affine, persp);
                        if let Some(range) = target_res.meshes.push_path(
                            mesh,
                            [
                                primitive.origin[0] + origin[0],
                                primitive.origin[1] + origin[1],
                            ],
                            path_affine,
                            opacity,
                            frag_clip,
                            node_mask.as_ref(),
                        ) {
                            // The AA fringe is physical pixels past the
                            // mesh whatever the transform: one along an
                            // edge, up to the miter limit (4) at a corner.
                            let drawn =
                                transformed_aabb_projective(bounds, path_affine, [0.0, 0.0]);
                            let fringe = 5.0 / scale;
                            push_path_draw(
                                &mut scratch.commands,
                                &mut scratch.batching,
                                range,
                                scissor,
                                physical_bounds(
                                    LogicalRect::from_xywh(
                                        drawn.x - fringe,
                                        drawn.y - fringe,
                                        drawn.width + fringe * 2.0,
                                        drawn.height + fringe * 2.0,
                                    ),
                                    scale,
                                    scissor,
                                ),
                            );
                        }
                    }
                    ScenePrimitiveKind::Custom {
                        node: custom,
                        mask,
                        corner_radius,
                    } => {
                        let handled_builtin = {
                            #[cfg(feature = "charts")]
                            if custom.renderer.as_ref() == nana_ui_runtime::CHART_RENDERER {
                                if let Some(payload) = custom.payload.as_ref().and_then(|p| {
                                    p.downcast_ref::<nana_ui_charts::ChartGpuPayload>()
                                }) {
                                    let chart_origin = [
                                        primitive.origin[0] + payload.origin[0],
                                        primitive.origin[1] + payload.origin[1],
                                    ];
                                    let pipeline = match core.charts.as_mut() {
                                        Some(pipeline) => pipeline,
                                        None => {
                                            let built = chart::ChartPipeline::try_new(
                                                &core.device,
                                                core.format,
                                                core.quads.motion_layout(),
                                            )?;
                                            core.charts.insert(built)
                                        }
                                    };
                                    let chart_affine = mesh_affine(affine, persp);
                                    if let Some(slot) = target_res.charts.push(
                                        pipeline,
                                        &core.device,
                                        &core.queue,
                                        primitive.id,
                                        chart::ChartPlacement {
                                            marks: &payload.marks,
                                            origin: chart_origin,
                                            hover: &payload.hover,
                                            affine: chart_affine,
                                            opacity,
                                            clip: frag_clip,
                                            viewport_scale: scale,
                                        },
                                        corner_exponent,
                                        dest_physical,
                                        Some(&gpu_work),
                                    ) {
                                        let drawn = payload.marks.extent.map_or(
                                            bounds,
                                            |[x0, y0, x1, y1]| {
                                                LogicalRect::from_xywh(
                                                    chart_origin[0] + x0,
                                                    chart_origin[1] + y0,
                                                    x1 - x0,
                                                    y1 - y0,
                                                )
                                            },
                                        );
                                        let painted = painted_bounds(
                                            drawn,
                                            payload.hover.growth + 2.0,
                                            chart_affine,
                                            [0.0, 0.0],
                                            scale,
                                            scissor,
                                        );
                                        for draw in 0..target_res.charts.draw_count(slot) {
                                            scratch.batching.note_geometry();
                                            scratch.batching.open(scratch.commands.len(), painted);
                                            scratch.commands.push(DrawCommand::Chart {
                                                slot,
                                                draw,
                                                scissor,
                                            });
                                        }
                                    }
                                }
                                true
                            } else {
                                false
                            }
                            #[cfg(not(feature = "charts"))]
                            {
                                false
                            }
                        };
                        if handled_builtin {
                            // Built-in inline GPU payload already emitted into batch.
                        } else if custom.renderer.as_ref() == "nana.host-texture" {
                            // The registry is a shared RwLock: an entry validated at
                            // frame start can be removed before prepare. Skip the
                            // node for this frame instead of panicking.
                            let Some(binding) = host_textures
                                .and_then(|registry| registry.get(custom.resource.as_ref()))
                            else {
                                continue;
                            };
                            let dest = nana_ui_core::LogicalRect::new(
                                bounds.x,
                                bounds.y,
                                bounds.width,
                                bounds.height,
                            )
                            .fitted(
                                binding.width as f32,
                                binding.height as f32,
                                custom.fit,
                            );
                            scratch.batching.close_all();
                            let clear_premul = if viewport.clear {
                                [
                                    clear.r as f32,
                                    clear.g as f32,
                                    clear.b as f32,
                                    clear.a as f32,
                                ]
                            } else {
                                [0.0; 4]
                            };
                            let prepared = core.host_textures.prepare(
                                &mut target_res.host_textures,
                                &core.device,
                                &core.queue,
                                &mut core.url_cache,
                                binding,
                                primitive.id.node.get(),
                                primitive.id.slot,
                                LogicalRect::from_xywh(dest.x, dest.y, dest.width, dest.height),
                                affine,
                                persp,
                                scissor,
                                opacity,
                                *corner_radius,
                                bounds,
                                vertex_clip,
                                dest_physical,
                                scale,
                                mask.clone(),
                                Some(&gpu_work),
                                custom.checkerboard,
                                custom.zoom,
                                custom.sampling,
                                clear_premul,
                            );
                            target_res
                                .painted_demand
                                .note(&custom.resource, prepared.painted);
                            scratch.commands.push(DrawCommand::HostTexture(prepared));
                        } else if let Some(renderer) = gpu_renderers
                            .and_then(|registry| registry.get(custom.renderer.as_ref()))
                        {
                            let node = SceneGpuNode {
                                id: primitive.id,
                                custom: custom.clone(),
                                opacity,
                            };
                            let custom_bounds = custom_paint_bounds(bounds, affine, persp, scale);
                            renderer.prepare(
                                &node,
                                SceneGpuPrepareContext {
                                    gpu: &core.gpu,
                                    target_format: __framework::format_from_wgpu(core.format),
                                    presentation: core.presentation,
                                    bounds: custom_bounds.to_core(),
                                    scale_factor: scale,
                                    dest_size: dest_physical,
                                    gpu_work: Some(&gpu_work),
                                },
                            );
                            scratch.batching.close_all();
                            scratch.commands.push(DrawCommand::Custom {
                                node,
                                renderer,
                                bounds: physical_bounds(custom_bounds, scale, scissor),
                                clip: scissor,
                            });
                        } else if custom.payload.is_some() {
                            return Err(ScenePaintError::UnsupportedCustomRenderer(primitive.id));
                        } else {
                            continue;
                        }
                    }
                }
                clip_dests_cutting(
                    &mut scratch.clip_dests,
                    &scratch.commands[command_start..],
                    &scratch.batching.painted[painted_start..],
                    scale,
                );
                if !scratch.clip_dests.is_empty()
                    && wrap_drawn_with_clip_dests(
                        &mut scratch.commands,
                        command_start,
                        &mut group_depth,
                        &mut group_slots,
                        &mut scratch.max_group_depth,
                        &mut scratch.group_slots,
                        &scratch.clip_dests,
                        &mut scratch.drained_commands,
                        scale,
                    )
                {
                    scratch.batching.close_all();
                }
            }
            if !scratch.painter_layers.is_empty() {
                scratch.batching.close_all();
                while scratch.painter_layers.pop().is_some() {
                    group_depth = group_depth.saturating_sub(1);
                    scratch.commands.push(DrawCommand::PopGroup);
                }
            }
            if sync_opacity_groups(
                &mut scratch.commands,
                &mut scratch.group_stack,
                &mut group_depth,
                &mut group_slots,
                &mut scratch.max_group_depth,
                &mut scratch.group_slots,
                &[],
                scene,
                origin,
                scale,
            ) {
                scratch.batching.close_all();
            }
            // Text prepare stays inside the batch window: it is the same
            // work the per-primitive prepare did, only once per run.
            core.text.flush_runs(&mut target_res.text);
            #[cfg(feature = "charts")]
            target_res.charts.finish_frame();
            target_res.painted_demand.commit(host_textures);
            core.url_cache.commit_demand(target_res.painted_demand.id());
            target_res.natural_sizes.end_pass();
            let batch = batch_started.elapsed();

            let upload_started = Instant::now();
            core.quads.upload(
                &mut target_res.quads,
                &core.device,
                &core.queue,
                &core.url_cache,
                dest_physical,
                scale,
                Some(&gpu_work),
            );
            core.meshes.upload(
                &mut target_res.meshes,
                &core.device,
                &core.queue,
                dest_physical,
                scale,
                Some(&gpu_work),
            );
            core.icons.upload(
                &mut target_res.icons,
                &core.device,
                &core.queue,
                corner_exponent,
                Some(&gpu_work),
            );
            core.text.upload_with(
                &mut target_res.text,
                &core.device,
                &core.queue,
                encoder,
                Some(&gpu_work),
            );
            BackdropPipeline::upload(
                &mut target_res.backdrop,
                &core.device,
                &core.queue,
                dest_physical,
                corner_exponent,
                Some(&gpu_work),
            );
            let gpu_upload = upload_started.elapsed();
            group_extents_into(
                &scratch.commands,
                &scratch.batching.painted,
                &scratch.group_slots,
                &mut scratch.group_extents,
                &mut scratch.group_extent_stack,
            );
            scratch.glyph_then_quad = scratch.batching.glyph_then_quad;

            (batch, gpu_upload)
        };

        let active_batch = if reused {
            let cached = target_res
                .prepared_batch
                .as_ref()
                .expect("checked reused above");
            BatchView {
                commands: &cached.commands,
                max_group_depth: cached.max_group_depth,
                group_slots: &cached.group_slots,
                group_extents: &cached.group_extents,
                glyph_then_quad: cached.glyph_then_quad,
            }
        } else {
            BatchView {
                commands: &scratch.commands,
                max_group_depth: scratch.max_group_depth,
                group_slots: &scratch.group_slots,
                group_extents: &scratch.group_extents,
                glyph_then_quad: scratch.glyph_then_quad,
            }
        };

        let encode_started = Instant::now();
        // The MSAA-geometry then single-sample-glyph optimization is valid only
        // when glyphs form a suffix. A later panel, modal scrim or mesh must
        // cover earlier text/icons in document order, so `Batching` reads that
        // half off the emit order — folding a quad into an earlier batch can
        // leave glyphs at the end and must not change the sample count. The
        // commands that rule out MSAA outright are never merged, so they are
        // still read from the list.
        let gpu_interleaved = active_batch.glyph_then_quad
            || active_batch.commands.iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::HostTexture(_)
                        | DrawCommand::Custom { .. }
                        | DrawCommand::PushGroup { .. }
                        | DrawCommand::PopGroup
                        | DrawCommand::Backdrop { .. }
                )
            })
            || BackdropPipeline::needs_backdrop(&target_res.backdrop);
        // Per-glyph presentation moves text alone. While nothing else moves
        // and the batch is the last frame's, the dest as it stood before its
        // first text draw is the last frame's too: put that back instead of
        // drawing it again.
        let split = if glyph_live && !motion_live && !gpu_payload_live {
            prefix_split(active_batch.commands, gpu_interleaved, viewport.clear)
        } else {
            None
        };
        let target_format = __framework::format_to_wgpu(core.presentation.target_format);
        let has_external_pixels = active_batch.commands.iter().any(|command| {
            matches!(
                command,
                DrawCommand::HostTexture(_) | DrawCommand::Custom { .. }
            )
        });
        let has_backdrop = BackdropPipeline::needs_backdrop(&target_res.backdrop)
            || active_batch
                .commands
                .iter()
                .any(|command| matches!(command, DrawCommand::Backdrop { .. }));
        let has_reading_blend = active_batch
            .group_slots
            .iter()
            .any(dest::slot_reads_backdrop);
        let identity_linear_blit = options.alpha == AlphaEncoding::Linear
            && core.format == target_format
            && !core.presentation.requires_float_working_target()
            && !clear_color_is_extended(clear_color)
            && (core.presentation.is_direct_linear()
                || (core.presentation.color_space == ScenePresentationColorSpace::Srgb
                    && core.presentation.target_format.is_srgb()));
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum SurfaceRouting {
            Direct,
            Retained,
        }
        const DIRECT_STREAK_THRESHOLD: u8 = 2;

        match target_res.last_encoded_key {
            Some(prev) if prev == painted => target_res.changing_frame_streak = 0,
            Some(prev) if prev.instance == painted.instance => {
                target_res.changing_frame_streak =
                    target_res.changing_frame_streak.saturating_add(1);
            }
            _ => target_res.changing_frame_streak = 0,
        }
        target_res.last_encoded_key = Some(painted);

        let target_extent = target.texture().size();
        let target_matches_dest =
            target_extent.width == dest_physical[0] && target_extent.height == dest_physical[1];
        let can_render_directly = split.is_none()
            && !has_backdrop
            && !has_reading_blend
            && identity_linear_blit
            && viewport.clear
            && blit_origin == [0.0, 0.0]
            && dest_physical == viewport.physical_size
            && target_matches_dest;
        let uncacheable_frame = has_external_pixels || gpu_motion_live;
        let streaming_scene = target_res.changing_frame_streak >= DIRECT_STREAK_THRESHOLD;
        let routing = if can_render_directly && (uncacheable_frame || streaming_scene) {
            SurfaceRouting::Direct
        } else {
            SurfaceRouting::Retained
        };

        let mut dest_passes = DestPassCounts::default();
        let need_dest = routing == SurfaceRouting::Retained
            || !gpu_interleaved
            || active_batch.max_group_depth > 0;
        let prefix = if need_dest {
            DestTarget::try_ensure(
                &mut target_res.dest,
                core.dest_pipelines.as_ref(),
                &core.device,
                core.format,
                target_format,
                core.presentation,
                dest_physical[0],
                dest_physical[1],
                !gpu_interleaved,
                routing == SurfaceRouting::Retained,
                core.gpu.policy(),
            )?;
            let dest = target_res.dest.as_mut().expect("dest target");
            dest.set_presentation_parameters(&core.queue, presentation_parameters, Some(&gpu_work));
            if active_batch.max_group_depth > 0 {
                dest.try_prepare_groups(
                    &core.device,
                    &core.queue,
                    active_batch.max_group_depth,
                    active_batch.group_slots,
                    corner_exponent,
                    Some(&gpu_work),
                )?;
            } else {
                dest.trim_idle_groups(&core.device, 0, 0, false, Some(&gpu_work));
            }
            dest_passes.msaa_allocated = dest.msaa_allocated;
            if routing == SurfaceRouting::Direct {
                dest.release_prefix();
                PrefixUse::Paint
            } else {
                match split {
                    None => {
                        dest.release_prefix();
                        PrefixUse::Paint
                    }
                    Some(split) if reused && dest.prefix_holds(split) => PrefixUse::Restore(split),
                    // Only a batch painted twice is stored: one rebuilt every
                    // frame would pay the copy for nothing.
                    Some(split) if reused => {
                        dest.reserve_prefix(&core.device);
                        PrefixUse::Store(split)
                    }
                    Some(_) => {
                        dest.forget_prefix();
                        PrefixUse::Paint
                    }
                }
            }
        } else {
            if let Some(dest) = target_res.dest.as_mut() {
                dest.release_prefix();
                dest.release_msaa();
                dest.trim_idle_groups(&core.device, 0, 0, false, Some(&gpu_work));
                dest.trim_idle_color();
                dest_passes.msaa_allocated = dest.msaa_allocated;
                if dest.is_idle_empty() {
                    target_res.dest = None;
                }
            }
            PrefixUse::Paint
        };
        let sample_count = if gpu_interleaved { 1 } else { 4 };
        let initial_load = if viewport.clear {
            wgpu::LoadOp::Clear(clear)
        } else {
            wgpu::LoadOp::Load
        };
        core.motion.sync_with_work(
            &core.device,
            &core.queue,
            scene,
            Some(&gpu_work),
            Some(encoder),
        );
        if routing == SurfaceRouting::Direct {
            if gpu_interleaved {
                let full_rect = PhysicalRect {
                    x: 0,
                    y: 0,
                    width: dest_physical[0],
                    height: dest_physical[1],
                };
                let active_damage = requested_damage.and_then(|rect| {
                    let clamped = crate::gpu_view::intersect_physical(rect, full_rect);
                    let total_px = u64::from(dest_physical[0]) * u64::from(dest_physical[1]);
                    let dirty_px = u64::from(clamped.width) * u64::from(clamped.height);
                    let fits_area = clamped.width > 0
                        && clamped.height > 0
                        && dirty_px.saturating_mul(4) < total_px.saturating_mul(3);
                    let supported_commands = !active_batch.commands.iter().any(|command| {
                        matches!(
                            command,
                            DrawCommand::Custom { .. } | DrawCommand::PathMask { .. }
                        )
                    });
                    let leading_host_replaces = matches!(
                        active_batch.commands.first(),
                        Some(DrawCommand::HostTexture(prepared))
                            if prepared.can_replace_clear_in(clamped, surface_clear)
                    );
                    (fits_area
                        && active_batch.max_group_depth == 0
                        && !has_backdrop
                        && !has_reading_blend
                        && viewport.clear
                        && supported_commands
                        && leading_host_replaces)
                        .then_some(clamped)
                });
                core.last_frame_used_partial_damage = active_damage.is_some();
                let dest = (active_batch.max_group_depth > 0)
                    .then(|| target_res.dest.as_ref().expect("dest target"));
                let mut ordered = EncodeOrdered {
                    quads: &core.quads,
                    quad_target: &target_res.quads,
                    meshes: &core.meshes,
                    mesh_target: &target_res.meshes,
                    #[cfg(feature = "charts")]
                    charts: core.charts.as_ref(),
                    #[cfg(feature = "charts")]
                    chart_target: &target_res.charts,
                    icons: &core.icons,
                    icon_target: &target_res.icons,
                    text: &core.text,
                    text_target: &target_res.text,
                    host_textures: &core.host_textures,
                    host_texture_target: &target_res.host_textures,
                    backdrop: &core.backdrop,
                    backdrop_target: &mut target_res.backdrop,
                    gpu: &core.gpu,
                    format: __framework::format_from_wgpu(core.format),
                    presentation: core.presentation,
                    gpu_work: &gpu_work,
                    motion: core.motion.bind_group(),
                    group_extents: active_batch.group_extents,
                    group_extents_exact: !gpu_motion_live,
                };
                encode_ordered(
                    &mut ordered,
                    &mut scratch.encode_stack,
                    &mut scratch.encode_layer_ready,
                    encoder,
                    dest,
                    target,
                    dest_physical,
                    wgpu::LoadOp::Clear(surface_clear),
                    active_batch.commands,
                    &mut dest_passes,
                    active_damage,
                );
            } else {
                let dest = target_res.dest.as_ref().expect("4x MSAA direct dest");
                let mut pass = dest.begin_msaa_pass_with_resolve(
                    encoder,
                    target,
                    surface_clear,
                    &mut dest_passes,
                );
                restore_dest_viewport(&mut pass, dest_physical);
                encode_msaa_geometry_commands(
                    &core.quads,
                    &target_res.quads,
                    &core.meshes,
                    &target_res.meshes,
                    #[cfg(feature = "charts")]
                    core.charts.as_ref(),
                    #[cfg(feature = "charts")]
                    &target_res.charts,
                    &mut pass,
                    active_batch.commands,
                    sample_count,
                    core.motion.bind_group(),
                    &gpu_work,
                );
                drop(pass);
                encode_glyph_pass_on_view(
                    &core.text,
                    &target_res.text,
                    &core.icons,
                    &target_res.icons,
                    target,
                    encoder,
                    active_batch.commands,
                    dest_physical,
                    &gpu_work,
                    &mut dest_passes,
                );
            }
        } else {
            let dest = target_res.dest.as_ref().expect("dest target");
            if gpu_interleaved {
                let mut ordered = EncodeOrdered {
                    quads: &core.quads,
                    quad_target: &target_res.quads,
                    meshes: &core.meshes,
                    mesh_target: &target_res.meshes,
                    #[cfg(feature = "charts")]
                    charts: core.charts.as_ref(),
                    #[cfg(feature = "charts")]
                    chart_target: &target_res.charts,
                    icons: &core.icons,
                    icon_target: &target_res.icons,
                    text: &core.text,
                    text_target: &target_res.text,
                    host_textures: &core.host_textures,
                    host_texture_target: &target_res.host_textures,
                    backdrop: &core.backdrop,
                    backdrop_target: &mut target_res.backdrop,
                    gpu: &core.gpu,
                    format: __framework::format_from_wgpu(core.format),
                    presentation: core.presentation,
                    gpu_work: &gpu_work,
                    motion: core.motion.bind_group(),
                    group_extents: active_batch.group_extents,
                    group_extents_exact: !gpu_motion_live,
                };
                let (load, rest) = match prefix {
                    PrefixUse::Restore(PrefixSplit::Ordered(split)) => {
                        dest.restore_prefix(encoder, &mut dest_passes);
                        (wgpu::LoadOp::Load, &active_batch.commands[split..])
                    }
                    PrefixUse::Store(PrefixSplit::Ordered(split)) => {
                        encode_ordered(
                            &mut ordered,
                            &mut scratch.encode_stack,
                            &mut scratch.encode_layer_ready,
                            encoder,
                            Some(dest),
                            dest.color_view(),
                            dest_physical,
                            initial_load,
                            &active_batch.commands[..split],
                            &mut dest_passes,
                            None,
                        );
                        dest.store_prefix(encoder, &mut dest_passes);
                        (wgpu::LoadOp::Load, &active_batch.commands[split..])
                    }
                    _ => (initial_load, active_batch.commands),
                };
                encode_ordered(
                    &mut ordered,
                    &mut scratch.encode_stack,
                    &mut scratch.encode_layer_ready,
                    encoder,
                    Some(dest),
                    dest.color_view(),
                    dest_physical,
                    load,
                    rest,
                    &mut dest_passes,
                    None,
                );
            } else if let PrefixUse::Restore(PrefixSplit::Geometry) = prefix {
                dest.restore_prefix(encoder, &mut dest_passes);
                encode_glyph_pass_on_view(
                    &core.text,
                    &target_res.text,
                    &core.icons,
                    &target_res.icons,
                    dest.color_view(),
                    encoder,
                    active_batch.commands,
                    dest_physical,
                    &gpu_work,
                    &mut dest_passes,
                );
            } else {
                let mut pass = dest.begin_msaa_pass(encoder, clear, &mut dest_passes);
                restore_dest_viewport(&mut pass, dest_physical);
                encode_msaa_geometry_commands(
                    &core.quads,
                    &target_res.quads,
                    &core.meshes,
                    &target_res.meshes,
                    #[cfg(feature = "charts")]
                    core.charts.as_ref(),
                    #[cfg(feature = "charts")]
                    &target_res.charts,
                    &mut pass,
                    active_batch.commands,
                    sample_count,
                    core.motion.bind_group(),
                    &gpu_work,
                );
                drop(pass);
                if let PrefixUse::Store(PrefixSplit::Geometry) = prefix {
                    dest.store_prefix(encoder, &mut dest_passes);
                }
                encode_glyph_pass_on_view(
                    &core.text,
                    &target_res.text,
                    &core.icons,
                    &target_res.icons,
                    dest.color_view(),
                    encoder,
                    active_batch.commands,
                    dest_physical,
                    &gpu_work,
                    &mut dest_passes,
                );
            }
            dest.blit(
                encoder,
                target,
                blit_origin[0],
                blit_origin[1],
                viewport.physical_size,
                viewport.clear.then_some(surface_clear),
                options.alpha,
                Some(&gpu_work),
                &mut dest_passes,
            );
            if let PrefixUse::Store(split) = prefix {
                target_res
                    .dest
                    .as_mut()
                    .expect("dest target")
                    .mark_prefix(split);
            }
        }
        let encode = encode_started.elapsed();
        core.backdrop
            .finish_frame(&mut target_res.backdrop, &core.device, Some(&gpu_work));
        if !reused {
            core.url_cache.trim();
            core.quads
                .finish_frame(&mut target_res.quads, &core.url_cache);
            HostTexturePipeline::trim(&mut target_res.host_textures);
        }
        for _ in 0..target_res.text.take_frame_gpu_allocations() {
            gpu_work.record_realloc();
        }
        core.last_gpu_work = Some(gpu_work.snapshot());
        core.last_gpu_timings = Some(GpuStageTimings {
            batch,
            gpu_upload,
            encode,
            submit: std::time::Duration::ZERO,
        });
        core.last_dest_pass_counts = Some(dest_passes);
        // Host-driven or GPU-animated pixels can change while the scene
        // stands still, and `Direct` frames do not paint `dest.color`, so
        // only stationary `Retained` frames may reuse the cached dest.
        target_res.painted =
            (routing == SurfaceRouting::Retained && !has_external_pixels && !gpu_motion_live)
                .then_some(painted);
        if !reused && resolved.cacheable {
            let slot = target_res
                .prepared_batch
                .get_or_insert_with(|| PreparedBatch {
                    key: painted,
                    resources: Vec::new(),
                    renderers: Vec::new(),
                    image_revision: core.image_revision,
                    commands: Vec::new(),
                    max_group_depth: 0,
                    group_slots: Vec::new(),
                    group_extents: Vec::new(),
                    glyph_then_quad: false,
                    text_placement_epoch: 0,
                });
            slot.key = painted;
            slot.resources = resolved.resources;
            slot.renderers = resolved.renderers;
            slot.image_revision = core.image_revision;
            std::mem::swap(&mut slot.commands, &mut scratch.commands);
            slot.max_group_depth = scratch.max_group_depth;
            std::mem::swap(&mut slot.group_slots, &mut scratch.group_slots);
            std::mem::swap(&mut slot.group_extents, &mut scratch.group_extents);
            slot.glyph_then_quad = scratch.glyph_then_quad;
            slot.text_placement_epoch = core.text.placement_epoch();
        }
        Ok(())
    }
}

/// Stroke/spinner cannot take a homography without a new mesh pipeline.
/// Planar 3D paints identity here so we do not squash like `scaleX(cos)`.
fn mesh_affine(affine: [f32; 6], persp: [f32; 2]) -> [f32; 6] {
    if persp[0].abs() > 1e-8 || persp[1].abs() > 1e-8 {
        clip::IDENTITY_AFFINE
    } else {
        affine
    }
}

/// Non-HostTexture custom renderers have no projective VS. Identity dest
/// when `(g,h)` is live; 2D affine still maps the AABB.
fn custom_paint_bounds(
    bounds: LogicalRect,
    affine: [f32; 6],
    persp: [f32; 2],
    scale: f32,
) -> LogicalRect {
    if persp[0].abs() > 1e-8 || persp[1].abs() > 1e-8 {
        bounds
    } else if clip::is_translation_projective(affine, persp) {
        clip::translated_on_grid(bounds, affine, scale)
    } else {
        transformed_aabb(bounds, affine)
    }
}

/// `points`, a stroke's in its node's coordinates, put at `origin`: one add
/// per coordinate, into `buffer` unless there is nothing to add.
fn placed_points<'a>(
    buffer: &'a mut Vec<[f32; 2]>,
    points: &'a [[f32; 2]],
    [x, y]: [f32; 2],
) -> &'a [[f32; 2]] {
    if [x, y] == [0.0, 0.0] {
        return points;
    }
    buffer.clear();
    buffer.extend(points.iter().map(|&[px, py]| [x + px, y + py]));
    buffer
}

fn clip_dests_into(
    kind: &ScenePrimitiveKind,
    clips: &[nana_ui_scene::ClipRegion],
    origin: PaintOrigin,
    out: &mut Vec<FragmentClip>,
) {
    out.clear();
    // Custom has no vertex clip so wrap every clip the scissor cannot express;
    // built-ins wrap extras only.
    let keep_innermost = matches!(
        kind,
        ScenePrimitiveKind::Custom { node: custom, .. }
            if custom.renderer.as_ref() != "nana.host-texture" && custom.payload.is_none()
    );
    if keep_innermost {
        *out = clip::all_fragment_clips(clips, origin);
    } else if matches!(
        kind,
        ScenePrimitiveKind::Text { .. }
            | ScenePrimitiveKind::Stroke { .. }
            | ScenePrimitiveKind::Spinner { .. }
            | ScenePrimitiveKind::Path { .. }
    ) || matches!(
        kind,
        ScenePrimitiveKind::Custom { node: custom, .. } if custom.payload.is_some()
    ) {
        // Their shaders test the innermost polygon or ellipse themselves.
        *out = mesh_extra_fragment_clips(clips, origin);
    } else {
        *out = extra_fragment_clips(clips, origin);
    }
}

#[cfg(test)]
fn clip_dests_for(
    kind: &ScenePrimitiveKind,
    clips: &[nana_ui_scene::ClipRegion],
    origin: PaintOrigin,
) -> Vec<FragmentClip> {
    let mut out = Vec::new();
    clip_dests_into(kind, clips, origin, &mut out);
    out
}

/// The clip dests that still cut into what a primitive drew. `drawn` are the
/// commands it emitted and `painted` their dest-pixel bounds: every batched
/// command records one, and those bounds never under-cover what the GPU
/// paints. A rounded clip the drawing stays clear of the corners of keeps all
/// of it, so it needs no group; a nested rounded clip around content well
/// inside it (an input's text inside a rounded card) is the common case.
/// Anything else drawn — GPU nodes, masks, groups — keeps every clip.
fn clip_dests_cutting(
    clip_dests: &mut Vec<FragmentClip>,
    drawn: &[DrawCommand],
    painted: &[PhysicalRect],
    scale: f32,
) {
    if clip_dests.is_empty() || drawn.len() != painted.len() {
        return;
    }
    let all_batched = drawn.iter().all(|command| {
        let batched = matches!(
            command,
            DrawCommand::Quads { .. }
                | DrawCommand::Mesh { .. }
                | DrawCommand::Path { .. }
                | DrawCommand::Text { .. }
                | DrawCommand::Icon { .. }
        );
        #[cfg(feature = "charts")]
        let batched = batched || matches!(command, DrawCommand::Chart { .. });
        batched
    });
    let Some(area) = painted.iter().copied().reduce(union_physical) else {
        return;
    };
    if all_batched {
        clip_dests.retain(|clip| !clip::keeps_all_of(clip.for_physical_pixels(scale), area));
    }
}

#[allow(clippy::too_many_arguments)]
fn wrap_drawn_with_clip_dests(
    commands: &mut Vec<DrawCommand>,
    start: usize,
    depth: &mut usize,
    slots: &mut u32,
    max_depth: &mut usize,
    uniforms: &mut Vec<GroupSlot>,
    clips: &[FragmentClip],
    drained: &mut Vec<DrawCommand>,
    scale: f32,
) -> bool {
    if clips.is_empty() || commands.len() == start {
        return false;
    }
    drained.clear();
    drained.extend(commands.drain(start..));
    for clip in clips {
        let layer = *depth;
        let slot = *slots;
        *slots = slots.saturating_add(1);
        *depth = depth.saturating_add(1);
        *max_depth = (*max_depth).max(*depth);
        uniforms.push(GroupSlot::clip(clip.for_physical_pixels(scale)));
        commands.push(DrawCommand::PushGroup { layer, slot });
    }
    commands.extend(drained.drain(..));
    for _ in clips {
        *depth = depth.saturating_sub(1);
        commands.push(DrawCommand::PopGroup);
    }
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "Explicit fields of the host or GPU projection contract"
)]
///
/// Reports whether a `PushGroup` or `PopGroup` is now in the list; no batch may
/// span one. A group that opened and closed with nothing inside takes its own
/// `PushGroup` back out, and since nothing was emitted while it was open, no
/// batch can name a command at or past it.
fn sync_opacity_groups(
    commands: &mut Vec<DrawCommand>,
    stack: &mut Vec<nana_ui_scene::OpacityGroup>,
    depth: &mut usize,
    slots: &mut u32,
    max_depth: &mut usize,
    uniforms: &mut Vec<GroupSlot>,
    needed: &[nana_ui_scene::OpacityGroup],
    scene: &nana_ui_scene::UiScene,
    origin: PaintOrigin,
    scale: f32,
) -> bool {
    // Retain the scene's stacking/culling boundary for terminal holds without
    // keeping an identity composition pass alive once the card is opaque.
    // A scroll view keeps its group at either end of its content, where
    // neither edge fade band shows.
    let needed = needed.iter().filter(|group| !group.is_identity());
    // A triggered menu's trigger composites its menu's logical opacity while
    // the hanging surface fades: one node, two groups. A scroll view's bars
    // composite without the edge fade its content has.
    let common = stack
        .iter()
        .zip(needed.clone())
        .take_while(|(open, want)| {
            open.node == want.node
                && open.opacity == want.opacity
                && open.filter == want.filter
                && open.edge_fade == want.edge_fade
                && open.motion_streak == want.motion_streak
        })
        .count();
    let mut emitted = false;
    while stack.len() > common {
        emitted |= pop_opacity_group(commands, stack, depth, slots, uniforms);
    }
    for group in needed.skip(common) {
        let layer = *depth;
        let slot = *slots;
        *slots = slots.saturating_add(1);
        *depth = depth.saturating_add(1);
        *max_depth = (*max_depth).max(*depth);
        uniforms.push(dest_group_slot(group, scene, origin, scale));
        commands.push(DrawCommand::PushGroup { layer, slot });
        stack.push(*group);
        emitted = true;
    }
    emitted
}

fn dest_group_slot(
    group: &nana_ui_scene::OpacityGroup,
    scene: &nana_ui_scene::UiScene,
    origin: PaintOrigin,
    scale: f32,
) -> GroupSlot {
    let filter = group.filter;
    let [streak_x, streak_y] = group.motion_streak;
    // A streak reaches back along its own length; the clip grows to keep it.
    let pad = filter.dest_extent_pad().max(streak_x.hypot(streak_y));
    let clip = if pad > 0.0 {
        scene
            .draw_node_bounds(group.node)
            .map(|bounds| {
                clip::FragmentClip {
                    rect: [
                        bounds.x + origin.offset[0] - pad,
                        bounds.y + origin.offset[1] - pad,
                        (bounds.width + pad * 2.0).max(0.0),
                        (bounds.height + pad * 2.0).max(0.0),
                    ],
                    inv_abcd: [1.0, 0.0, 0.0, 1.0],
                    inv_ef: [0.0, 0.0],
                    corner_radius: 0.0,
                    polygon_count: 0,
                    polygon: [[0.0; 2]; 8],
                }
                .for_physical_pixels(scale)
            })
            .unwrap_or(clip::FragmentClip::PASS)
    } else {
        clip::FragmentClip::PASS
    };
    let physical_blur = if scale.is_finite() && scale > 0.0 {
        filter.blur_radius * scale
    } else {
        filter.blur_radius
    };
    let mut slot = GroupSlot::dest(
        group.opacity * filter.opacity,
        [filter.brightness, filter.saturate, filter.contrast],
        filter.hue_rotate_deg,
        physical_blur,
        group.mix_blend.gpu_index(),
        clip,
    );
    slot.filter_invert = filter.invert;
    slot.filter_blur_taps = dest::element_blur_taps(
        filter
            .blur_radius
            .max(filter.drop_shadow.map_or(0.0, |shadow| shadow.blur_radius)),
    );
    let physical = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    slot.motion_streak = [streak_x * physical, streak_y * physical];
    if let Some(shadow) = filter.drop_shadow {
        let (ox, oy, blur) = if scale.is_finite() && scale > 0.0 {
            (
                shadow.offset_x * scale,
                shadow.offset_y * scale,
                shadow.blur_radius * scale,
            )
        } else {
            (shadow.offset_x, shadow.offset_y, shadow.blur_radius)
        };
        slot.drop_shadow_offset = [ox, oy];
        slot.drop_shadow_blur = blur;
        slot.drop_shadow_color = shadow
            .paint_color
            .map(pack_paint_color)
            .unwrap_or_else(|| pack_linear(shadow.color));
    }
    if let Some(fade) = group
        .edge_fade
        .filter(nana_ui_scene::EdgeFadeMask::is_visible)
        && let Some(port) = scene.draw_node_rect(group.node, fade.bounds)
    {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        // The bands stretch with the scrollport under a scaling transform.
        let along = if fade.bounds.height > 0.0 {
            port.height / fade.bounds.height
        } else {
            1.0
        };
        let top = port.y + origin.offset[1];
        slot.edge_fade = [
            top * scale,
            (top + port.height) * scale,
            fade.top * along * scale,
            fade.bottom * along * scale,
        ];
    }
    slot
}

fn pop_opacity_group(
    commands: &mut Vec<DrawCommand>,
    stack: &mut Vec<nana_ui_scene::OpacityGroup>,
    depth: &mut usize,
    slots: &mut u32,
    uniforms: &mut Vec<GroupSlot>,
) -> bool {
    stack.pop();
    *depth = depth.saturating_sub(1);
    if matches!(commands.last(), Some(DrawCommand::PushGroup { .. })) {
        commands.pop();
        uniforms.pop();
        *slots = slots.saturating_sub(1);
        return false;
    }
    commands.push(DrawCommand::PopGroup);
    true
}

/// A draw command later primitives may still be folded into.
struct OpenBatch {
    /// Position of the command in the display list.
    command: usize,
    /// Its place in [`Batching::painted`].
    ordinal: usize,
    /// Union of the dest-pixel bounds of everything already in it.
    painted: PhysicalRect,
}

/// Display-list batching state.
///
/// A primitive may join a batch that is not the newest one, which draws it
/// earlier than its document position. That is sound exactly when it covers no
/// pixel of anything emitted after that batch — and everything emitted after it
/// is still open, because every command whose place in the list is part of the
/// GPU contract (`HostTexture`, `Custom`, `Backdrop`, `PushGroup`, `PopGroup`)
/// closes all of them. Overlapping primitives therefore keep painter's-algorithm
/// order to the pixel, and GPU content never moves.
#[derive(Default)]
struct Batching {
    open: Vec<OpenBatch>,
    saw_glyph: bool,
    /// A quad or mesh followed a glyph in **document order**. Read from the
    /// emit order rather than from the merged list, because folding a quad into
    /// an earlier batch can leave glyphs at the end and that must not turn a
    /// single-sampled frame into a 4x MSAA one.
    glyph_then_quad: bool,
    /// Dest-pixel bounds of every batched command (quads, meshes, paths,
    /// text, icons), in list order. Indexed by order among those commands
    /// rather than list position: clip dests wrap drawn commands in groups
    /// after the fact, which moves them but never reorders them.
    painted: Vec<PhysicalRect>,
}

impl Batching {
    fn clear(&mut self) {
        self.open.clear();
        self.saw_glyph = false;
        self.glyph_then_quad = false;
        self.painted.clear();
    }

    /// Close every batch. Required around anything that fixes a command's
    /// position or moves commands within the list.
    fn close_all(&mut self) {
        self.open.clear();
    }

    fn note_glyph(&mut self) {
        self.saw_glyph = true;
    }

    fn note_geometry(&mut self) {
        self.glyph_then_quad |= self.saw_glyph;
    }

    /// Position in `commands` of the batch this primitive may join. The
    /// candidate is identified by `extendable`, which reads the command itself
    /// — including its scissor — so a batch stores no copy of either.
    fn target(
        &mut self,
        commands: &[DrawCommand],
        painted: PhysicalRect,
        extendable: impl Fn(&DrawCommand) -> bool,
    ) -> Option<usize> {
        let position = self
            .open
            .iter()
            .rposition(|batch| extendable(&commands[batch.command]))?;
        if self.open[position + 1..]
            .iter()
            .any(|later| overlaps_physical(painted, later.painted))
        {
            return None;
        }
        let batch = &mut self.open[position];
        batch.painted = union_physical(batch.painted, painted);
        self.painted[batch.ordinal] = batch.painted;
        Some(batch.command)
    }

    fn open(&mut self, command: usize, painted: PhysicalRect) {
        self.open.push(OpenBatch {
            command,
            ordinal: self.painted.len(),
            painted,
        });
        self.painted.push(painted);
    }
}

/// Dest-pixel bounds a primitive can paint, used only to decide whether folding
/// it into an earlier batch would cross something. It must never under-cover
/// what the GPU paints, so callers pass the outset their paint adds (shadow,
/// outline, stroke half-width) before the transform.
fn painted_bounds(
    local: LogicalRect,
    outset: f32,
    affine: [f32; 6],
    persp: [f32; 2],
    scale: f32,
    scissor: PhysicalRect,
) -> PhysicalRect {
    let padded = LogicalRect::from_xywh(
        local.x - outset,
        local.y - outset,
        local.width + outset * 2.0,
        local.height + outset * 2.0,
    );
    physical_bounds(
        transformed_aabb_projective(padded, affine, persp),
        scale,
        scissor,
    )
}

/// AABB of a polyline in its own local space. `None` when it has no finite
/// point, which is also when the mesh pipeline refuses it.
fn polyline_bounds(points: &[[f32; 2]]) -> Option<LogicalRect> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in points {
        if !point[0].is_finite() || !point[1].is_finite() {
            return None;
        }
        min = [min[0].min(point[0]), min[1].min(point[1])];
        max = [max[0].max(point[0]), max[1].max(point[1])];
    }
    (min[0] <= max[0])
        .then(|| LogicalRect::from_xywh(min[0], min[1], max[0] - min[0], max[1] - min[1]))
}

/// [`painted_bounds`] for a quad: its box grown by the largest outer shadow and
/// the outline. The antialiased edge needs no margin of its own — the box is
/// rounded outwards to whole pixels when it is put in dest space. A filtered
/// surface has no rectangular reach, so it takes the whole dest and can then
/// neither join a batch nor be jumped over.
#[allow(clippy::too_many_arguments)]
fn quad_painted_bounds(
    local: LogicalRect,
    shadow: Option<&nana_ui_runtime::ComponentElevation>,
    surface: &nana_ui_scene::QuadSurfacePaint,
    affine: [f32; 6],
    persp: [f32; 2],
    scale: f32,
    scissor: PhysicalRect,
    dest_physical: [u32; 2],
) -> PhysicalRect {
    if surface.filter.is_some() || surface.backdrop_filter.is_some() {
        return PhysicalRect {
            x: 0,
            y: 0,
            width: dest_physical[0],
            height: dest_physical[1],
        };
    }
    let outset = shadow
        .into_iter()
        .chain(surface.extra_shadows.iter())
        .filter(|shadow| !shadow.inset)
        .map(|shadow| {
            shadow.offset_x.abs().max(shadow.offset_y.abs())
                + shadow.blur_radius * 3.0
                + shadow.spread_radius.max(0.0)
        })
        .fold(surface.outline_width.max(0.0) + 1.0, f32::max);
    painted_bounds(local, outset, affine, persp, scale, scissor)
}

/// Extend an open icon batch when the new slot shares its atlas, scissor and
/// vertex adjacency.
fn push_icon(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    icons: &icon::IconPipelineTarget,
    prepared: PreparedIcon,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_glyph();
    let slot = prepared.index;
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Icon { first, count, scissor: open }
            if *open == scissor && icon::IconPipeline::can_extend_run(icons, *first, *count, slot))
    }) {
        let DrawCommand::Icon { count, .. } = &mut commands[target] else {
            unreachable!("an icon batch names an icon command")
        };
        *count += 1;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Icon {
        first: slot,
        count: 1,
        scissor,
    });
}

/// Fold a prepared text into an open text run.
fn push_text_run(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    text: &mut text::TextPipelineTarget,
    prepared: PreparedText,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_glyph();
    text.set_painted(
        &prepared,
        u64::from(painted.width) * u64::from(painted.height),
    );
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Text { prepared: previous, scissor: open }
            if *open == scissor && text.can_merge_runs(previous, &prepared))
    }) {
        let DrawCommand::Text {
            prepared: previous, ..
        } = &commands[target]
        else {
            unreachable!("a text batch names a text command")
        };
        text.merge_runs(previous, &prepared);
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Text { prepared, scissor });
}

fn push_quad(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    index: u32,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_geometry();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Quads { range, scissor: open }
            if *open == scissor && range.end == index)
    }) {
        let DrawCommand::Quads { range, .. } = &mut commands[target] else {
            unreachable!("a quad batch names a quad command")
        };
        range.end = index + 1;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Quads {
        range: index..index + 1,
        scissor,
    });
}

fn push_mesh_draw(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    range: MeshRange,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_geometry();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Mesh { range: previous, scissor: open }
            if *open == scissor
                && previous.first_instance + previous.instance_count == range.first_instance)
    }) {
        let DrawCommand::Mesh {
            range: previous, ..
        } = &mut commands[target]
        else {
            unreachable!("a mesh batch names a mesh command")
        };
        previous.instance_count += range.instance_count;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Mesh { range, scissor });
}

fn push_path_draw(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    range: PathRange,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_geometry();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Path { range: previous, scissor: open }
            if *open == scissor
                && previous.instance == range.instance
                && previous.first_index + previous.index_count == range.first_index)
    }) {
        let DrawCommand::Path {
            range: previous, ..
        } = &mut commands[target]
        else {
            unreachable!("a path batch names a path command")
        };
        previous.index_count += range.index_count;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Path { range, scissor });
}

struct EncodeOrdered<'a> {
    quads: &'a QuadPipeline,
    quad_target: &'a quad::QuadPipelineTarget,
    meshes: &'a MeshPipeline,
    mesh_target: &'a mesh::MeshPipelineTarget,
    #[cfg(feature = "charts")]
    charts: Option<&'a chart::ChartPipeline>,
    #[cfg(feature = "charts")]
    chart_target: &'a chart::ChartPipelineTarget,
    icons: &'a IconPipeline,
    icon_target: &'a icon::IconPipelineTarget,
    text: &'a TextPipeline,
    text_target: &'a text::TextPipelineTarget,
    host_textures: &'a HostTexturePipeline,
    host_texture_target: &'a crate::gpu_texture::GpuTextureTarget,
    backdrop: &'a BackdropPipeline,
    backdrop_target: &'a mut backdrop::BackdropPipelineTarget,
    gpu: &'a GpuContext,
    format: GpuTextureFormat,
    presentation: ScenePresentationProfile,
    gpu_work: &'a GpuWorkSink,
    motion: &'a wgpu::BindGroup,
    /// What each group can change, by slot ([`group_extents_into`]).
    group_extents: &'a [GroupExtent],
    /// The extents hold for this frame: nothing moves on the GPU past the
    /// bounds the CPU encoded (a live transform, glyph or chart motion).
    group_extents_exact: bool,
}

/// Which dest pixels a group can change when it composites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupExtent {
    /// It drew nothing.
    Empty,
    Rect(PhysicalRect),
    /// Not known to be less than the whole target.
    All,
}

impl GroupExtent {
    fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::All, _) | (_, Self::All) => Self::All,
            (Self::Empty, other) | (other, Self::Empty) => other,
            (Self::Rect(a), Self::Rect(b)) => Self::Rect(union_physical(a, b)),
        }
    }
}

/// Each group's extent, by slot: the union of what its commands paint, and
/// the whole target once anything in it has no known bounds or its filter
/// can move pixels past what was drawn.
fn group_extents_into(
    commands: &[DrawCommand],
    painted: &[PhysicalRect],
    slots: &[GroupSlot],
    extents: &mut Vec<GroupExtent>,
    stack: &mut Vec<(u32, GroupExtent)>,
) {
    extents.clear();
    extents.resize(slots.len(), GroupExtent::All);
    if slots.is_empty() {
        return;
    }
    stack.clear();
    let mut batched = painted.iter();
    for command in commands {
        let drawn = match command {
            DrawCommand::Quads { .. }
            | DrawCommand::Mesh { .. }
            | DrawCommand::Path { .. }
            | DrawCommand::Text { .. }
            | DrawCommand::Icon { .. } => batched
                .next()
                .map_or(GroupExtent::All, |rect| GroupExtent::Rect(*rect)),
            #[cfg(feature = "charts")]
            DrawCommand::Chart { .. } => batched
                .next()
                .map_or(GroupExtent::All, |rect| GroupExtent::Rect(*rect)),
            // A mask only takes away from the layer it is applied to.
            DrawCommand::PathMask { .. } => GroupExtent::Empty,
            // A custom renderer may open its own pass, which nothing
            // scissors to the bounds it was given.
            DrawCommand::Custom { .. }
            | DrawCommand::HostTexture(_)
            | DrawCommand::Backdrop { .. } => GroupExtent::All,
            DrawCommand::PushGroup { slot, .. } => {
                stack.push((*slot, GroupExtent::Empty));
                continue;
            }
            DrawCommand::PopGroup => {
                let Some((slot, extent)) = stack.pop() else {
                    continue;
                };
                let extent = match slots.get(slot as usize) {
                    Some(group) if !group.spreads() => extent,
                    _ => GroupExtent::All,
                };
                if let Some(held) = extents.get_mut(slot as usize) {
                    *held = extent;
                }
                extent
            }
        };
        if let Some((_, extent)) = stack.last_mut() {
            *extent = extent.union(drawn);
        }
    }
}

#[cfg(test)]
fn group_extents(
    commands: &[DrawCommand],
    painted: &[PhysicalRect],
    slots: &[GroupSlot],
) -> Vec<GroupExtent> {
    let mut extents = Vec::new();
    let mut stack = Vec::new();
    group_extents_into(commands, painted, slots, &mut extents, &mut stack);
    extents
}

struct GroupFrame {
    layer: usize,
    slot: u32,
}

/// What a frame does with the dest's text-free prefix ([`PrefixSplit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixUse {
    /// Paint every command, keep nothing.
    Paint,
    /// Paint every command and keep the prefix for the frames after.
    Store(PrefixSplit),
    /// Put the kept prefix back and paint only what follows it.
    Restore(PrefixSplit),
}

/// Where per-glyph presentation, which moves text alone, starts changing a
/// frame's pixels. A UI-only frame draws all its geometry before any glyph.
/// An ordered frame changes from its first text draw on, or from the group
/// that holds it; a GPU node before it draws pixels the scene does not
/// describe, so a frame with one there keeps no prefix. So does an ordered
/// frame that loads the dest rather than clearing it, as its prefix then
/// depends on what was there.
fn prefix_split(commands: &[DrawCommand], ordered: bool, clears: bool) -> Option<PrefixSplit> {
    if !ordered {
        return Some(PrefixSplit::Geometry);
    }
    if !clears {
        return None;
    }
    let mut depth = 0usize;
    let mut top = 0usize;
    for (index, command) in commands.iter().enumerate() {
        if depth == 0 {
            top = index;
        }
        match command {
            DrawCommand::Text { .. } => return (top > 0).then_some(PrefixSplit::Ordered(top)),
            DrawCommand::HostTexture(_) | DrawCommand::Custom { .. } => return None,
            DrawCommand::PushGroup { .. } => depth += 1,
            DrawCommand::PopGroup => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn encode_msaa_geometry_commands<'a>(
    quads: &'a QuadPipeline,
    quad_target: &'a quad::QuadPipelineTarget,
    meshes: &'a MeshPipeline,
    mesh_target: &'a mesh::MeshPipelineTarget,
    #[cfg(feature = "charts")] charts: Option<&'a chart::ChartPipeline>,
    #[cfg(feature = "charts")] chart_target: &'a chart::ChartPipelineTarget,
    pass: &mut wgpu::RenderPass<'a>,
    commands: &'a [DrawCommand],
    sample_count: u32,
    motion: &'a wgpu::BindGroup,
    gpu_work: &GpuWorkSink,
) {
    for command in commands {
        match command {
            DrawCommand::Quads { range, scissor } => {
                quads.draw(
                    quad_target,
                    pass,
                    range.clone(),
                    *scissor,
                    sample_count,
                    motion,
                    Some(gpu_work),
                );
            }
            DrawCommand::Mesh { range, scissor } => {
                meshes.draw(
                    mesh_target,
                    pass,
                    range,
                    *scissor,
                    sample_count,
                    Some(gpu_work),
                );
            }
            DrawCommand::Path { range, scissor } => {
                meshes.draw_path(
                    mesh_target,
                    pass,
                    range,
                    *scissor,
                    sample_count,
                    Some(gpu_work),
                );
            }
            #[cfg(feature = "charts")]
            DrawCommand::Chart {
                slot,
                draw,
                scissor,
            } => {
                if let Some(pipeline) = charts {
                    chart_target.draw(
                        pipeline,
                        pass,
                        *slot,
                        *draw,
                        *scissor,
                        sample_count,
                        motion,
                        Some(gpu_work),
                    );
                }
            }
            // A mask only ever sits inside a group, and a frame with
            // groups is encoded in order, not here.
            DrawCommand::PathMask { .. } => {}
            DrawCommand::Text { .. }
            | DrawCommand::Icon { .. }
            | DrawCommand::HostTexture(_)
            | DrawCommand::Backdrop { .. }
            | DrawCommand::Custom { .. }
            | DrawCommand::PushGroup { .. }
            | DrawCommand::PopGroup => {}
        }
    }
}

/// A UI-only frame's glyphs and icons, over its geometry.
#[allow(clippy::too_many_arguments)]
fn encode_glyph_pass_on_view(
    text: &TextPipeline,
    text_target: &text::TextPipelineTarget,
    icons: &IconPipeline,
    icon_target: &icon::IconPipelineTarget,
    root_view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    commands: &[DrawCommand],
    dest_physical: [u32; 2],
    gpu_work: &GpuWorkSink,
    dest_passes: &mut DestPassCounts,
) {
    if !commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Text { .. } | DrawCommand::Icon { .. }))
    {
        return;
    }
    let mut pass =
        DestTarget::begin_color_pass_on_view(encoder, root_view, wgpu::LoadOp::Load, dest_passes);
    restore_dest_viewport(&mut pass, dest_physical);
    for command in commands {
        match command {
            DrawCommand::Text { prepared, scissor } => {
                text.draw(text_target, &mut pass, prepared, *scissor, Some(gpu_work));
            }
            DrawCommand::Icon {
                first,
                count,
                scissor,
            } => {
                icons.draw(
                    icon_target,
                    &mut pass,
                    *first,
                    *count,
                    *scissor,
                    Some(gpu_work),
                );
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_ordered(
    pipelines: &mut EncodeOrdered<'_>,
    stack: &mut Vec<GroupFrame>,
    layer_ready: &mut Vec<bool>,
    encoder: &mut wgpu::CommandEncoder,
    dest: Option<&DestTarget>,
    root_view: &wgpu::TextureView,
    dest_physical: [u32; 2],
    initial_load: wgpu::LoadOp<wgpu::Color>,
    commands: &[DrawCommand],
    dest_passes: &mut DestPassCounts,
    partial_damage: Option<PhysicalRect>,
) {
    const SAMPLE_COUNT: u32 = 1;
    let mut index = 0;
    let mut dest_load = initial_load;
    stack.clear();
    layer_ready.clear();
    while index < commands.len() {
        match &commands[index] {
            DrawCommand::PushGroup { layer, slot } => {
                if *layer >= layer_ready.len() {
                    layer_ready.resize(*layer + 1, false);
                }
                layer_ready[*layer] = false;
                stack.push(GroupFrame {
                    layer: *layer,
                    slot: *slot,
                });
                index += 1;
            }
            DrawCommand::PopGroup => {
                let Some(frame) = stack.pop() else {
                    index += 1;
                    continue;
                };
                if !layer_ready.get(frame.layer).copied().unwrap_or(false) {
                    // Nothing drew into the layer, so it was never cleared:
                    // it still holds whatever last used that depth. An empty
                    // group composites to nothing, filters and blends alike.
                    index += 1;
                    continue;
                }
                // Where the group can change its parent. Outside it the layer
                // is still transparent, which no blend composites into a
                // change: a small group composites only there instead of over
                // the whole dest. A blend that reads the backdrop also copies
                // only there. While something moves on the GPU the extents are
                // the encoded bounds, not where it is now, so only the
                // backdrop copy — which needs them — is limited.
                let reads_backdrop = dest
                    .expect("dest target present for groups")
                    .reads_backdrop(frame.slot);
                let whole = PhysicalRect {
                    x: 0,
                    y: 0,
                    width: dest_physical[0],
                    height: dest_physical[1],
                };
                let mut area = None;
                if reads_backdrop || pipelines.group_extents_exact {
                    let region = match pipelines.group_extents.get(frame.slot as usize) {
                        Some(GroupExtent::Empty) => {
                            // Nothing drawn: every blend of a transparent
                            // source leaves the backdrop as it was.
                            index += 1;
                            continue;
                        }
                        Some(GroupExtent::Rect(rect)) => intersect_physical(*rect, whole),
                        Some(GroupExtent::All) | None => whole,
                    };
                    if region.width == 0 || region.height == 0 {
                        index += 1;
                        continue;
                    }
                    area = Some(region);
                }
                if let Some(region) = area.filter(|_| reads_backdrop) {
                    let group_dest = dest.expect("dest target present for reading blend");
                    // The blend reads what it lands on: bring the parent up
                    // to date — a first use still has to clear it — and copy
                    // it before compositing over it.
                    let parent = stack.last().map(|parent| parent.layer);
                    match parent {
                        Some(layer) => {
                            if let wgpu::LoadOp::Clear(_) = group_layer_load(layer_ready, layer) {
                                let mut pass = group_dest.begin_group_pass(
                                    encoder,
                                    layer,
                                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    dest_passes,
                                );
                                restore_dest_viewport(&mut pass, dest_physical);
                            }
                        }
                        None => ensure_backdrop_target_ready(
                            root_view,
                            encoder,
                            dest_physical,
                            &mut dest_load,
                            dest_passes,
                        ),
                    }
                    group_dest.copy_backdrop(encoder, parent, region);
                }
                let group_dest = dest.expect("dest target present for groups");
                let mut pass = match stack.last() {
                    Some(parent) => {
                        let load = group_layer_load(layer_ready, parent.layer);
                        group_dest.begin_group_pass(encoder, parent.layer, load, dest_passes)
                    }
                    None => {
                        let pass = DestTarget::begin_color_pass_on_view(
                            encoder,
                            root_view,
                            dest_load,
                            dest_passes,
                        );
                        dest_load = wgpu::LoadOp::Load;
                        pass
                    }
                };
                restore_dest_viewport(&mut pass, dest_physical);
                let area = area.unwrap_or(whole);
                pass.set_scissor_rect(area.x, area.y, area.width, area.height);
                dest_passes.composite_pixels += u64::from(area.width) * u64::from(area.height);
                group_dest.composite_group(
                    &mut pass,
                    frame.layer,
                    frame.slot,
                    Some(pipelines.gpu_work),
                );
                drop(pass);
                index += 1;
            }
            _ => {
                if let DrawCommand::Backdrop {
                    index: backdrop_index,
                } = &commands[index]
                {
                    let Some(request) =
                        BackdropPipeline::request(pipelines.backdrop_target, *backdrop_index)
                            .copied()
                    else {
                        index += 1;
                        continue;
                    };
                    let backdrop_dest = dest.expect("dest target present for backdrop");
                    ensure_backdrop_target_ready(
                        root_view,
                        encoder,
                        dest_physical,
                        &mut dest_load,
                        dest_passes,
                    );
                    pipelines.backdrop.encode(
                        pipelines.backdrop_target,
                        __framework::device(pipelines.gpu),
                        encoder,
                        backdrop_dest.color_view(),
                        dest_physical,
                        backdrop_dest.color_view(),
                        &request,
                        pipelines.quad_target.paint_buffer(),
                        Some(pipelines.gpu_work),
                        &mut dest_passes.backdrop,
                    );
                    dest_load = wgpu::LoadOp::Load;
                    index += 1;
                    continue;
                }
                let mut replaced_first_clear = false;
                let partial_first_replace = partial_damage.filter(|damage| {
                    stack.is_empty()
                        && index == 0
                        && matches!(
                            (&dest_load, &commands[index]),
                            (wgpu::LoadOp::Clear(load_color), DrawCommand::HostTexture(prepared))
                                if prepared.can_replace_clear_in(*damage, *load_color)
                        )
                });
                let pass_load = if partial_first_replace.is_some() {
                    replaced_first_clear = true;
                    wgpu::LoadOp::Load
                } else if stack.is_empty()
                    && let wgpu::LoadOp::Clear(load_color) = dest_load
                    && let DrawCommand::HostTexture(prepared) = &commands[index]
                    && prepared.can_replace_clear(dest_physical, load_color)
                {
                    replaced_first_clear = true;
                    // SAFETY: `commands[index]` is a full-viewport `BlendState::REPLACE`
                    // `HostTexture` whose fragment shader overwrites every pixel of `root_view`.
                    unsafe { wgpu::LoadOp::DontCare(wgpu::LoadOpDontCare::enabled()) }
                } else {
                    match stack.last() {
                        Some(frame) => group_layer_load(layer_ready, frame.layer),
                        None => dest_load,
                    }
                };
                let mut pass = match stack.last() {
                    Some(frame) => dest
                        .expect("dest target present for groups")
                        .begin_group_pass(encoder, frame.layer, pass_load, dest_passes),
                    None => {
                        let pass = DestTarget::begin_color_pass_on_view(
                            encoder,
                            root_view,
                            pass_load,
                            dest_passes,
                        );
                        dest_load = wgpu::LoadOp::Load;
                        pass
                    }
                };
                restore_dest_viewport(&mut pass, dest_physical);
                if replaced_first_clear && let DrawCommand::HostTexture(prepared) = &commands[index]
                {
                    if let Some(dmg) = partial_first_replace {
                        pipelines.host_textures.draw_replace_clipped(
                            pipelines.host_texture_target,
                            prepared,
                            &mut pass,
                            dest_physical,
                            dmg,
                            Some(pipelines.gpu_work),
                        );
                    } else {
                        pipelines.host_textures.draw_replace(
                            pipelines.host_texture_target,
                            prepared,
                            &mut pass,
                            dest_physical,
                            Some(pipelines.gpu_work),
                        );
                    }
                    restore_dest_viewport(&mut pass, dest_physical);
                    index += 1;
                }
                let clip_scissor = |scissor: PhysicalRect| match partial_damage {
                    Some(dmg) => crate::gpu_view::intersect_physical(scissor, dmg),
                    None => scissor,
                };
                while index < commands.len() {
                    match &commands[index] {
                        DrawCommand::PushGroup { .. } | DrawCommand::PopGroup => break,
                        DrawCommand::Backdrop { .. } => break,
                        DrawCommand::Quads { range, scissor } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.quads.draw(
                                    pipelines.quad_target,
                                    &mut pass,
                                    range.clone(),
                                    clipped,
                                    SAMPLE_COUNT,
                                    pipelines.motion,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::Mesh { range, scissor } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.meshes.draw(
                                    pipelines.mesh_target,
                                    &mut pass,
                                    range,
                                    clipped,
                                    SAMPLE_COUNT,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::Path { range, scissor } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.meshes.draw_path(
                                    pipelines.mesh_target,
                                    &mut pass,
                                    range,
                                    clipped,
                                    SAMPLE_COUNT,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        #[cfg(feature = "charts")]
                        DrawCommand::Chart {
                            slot,
                            draw,
                            scissor,
                        } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0
                                && clipped.height > 0
                                && let Some(pipeline) = pipelines.charts
                            {
                                pipelines.chart_target.draw(
                                    pipeline,
                                    &mut pass,
                                    *slot,
                                    *draw,
                                    clipped,
                                    SAMPLE_COUNT,
                                    pipelines.motion,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::PathMask {
                            range,
                            scissor,
                            mode,
                        } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.meshes.draw_mask(
                                    pipelines.mesh_target,
                                    &mut pass,
                                    range,
                                    clipped,
                                    *mode,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::Text { prepared, scissor } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.text.draw(
                                    pipelines.text_target,
                                    &mut pass,
                                    prepared,
                                    clipped,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::Icon {
                            first,
                            count,
                            scissor,
                        } => {
                            let clipped = clip_scissor(*scissor);
                            if clipped.width > 0 && clipped.height > 0 {
                                pipelines.icons.draw(
                                    pipelines.icon_target,
                                    &mut pass,
                                    *first,
                                    *count,
                                    clipped,
                                    Some(pipelines.gpu_work),
                                );
                            }
                        }
                        DrawCommand::HostTexture(prepared) => {
                            if let Some(dmg) = partial_damage {
                                pipelines.host_textures.draw_clipped(
                                    pipelines.host_texture_target,
                                    prepared,
                                    &mut pass,
                                    dest_physical,
                                    dmg,
                                    Some(pipelines.gpu_work),
                                );
                            } else {
                                pipelines.host_textures.draw(
                                    pipelines.host_texture_target,
                                    prepared,
                                    &mut pass,
                                    dest_physical,
                                    Some(pipelines.gpu_work),
                                );
                            }
                            restore_dest_viewport(&mut pass, dest_physical);
                        }
                        DrawCommand::Custom {
                            node,
                            renderer,
                            bounds,
                            clip,
                        } if bounds.width > 0 && bounds.height > 0 => {
                            if !node.custom.dedicated_pass {
                                let encoded = draw_custom_run(
                                    commands,
                                    index,
                                    renderer,
                                    &mut ScenePass::new(&mut pass, dest_physical),
                                    pipelines.gpu,
                                    pipelines.format,
                                    pipelines.presentation,
                                    pipelines.gpu_work,
                                );
                                if encoded > 0 {
                                    restore_dest_viewport(&mut pass, dest_physical);
                                    index += encoded;
                                    continue;
                                }
                                if renderer.draw_in_pass(
                                    node,
                                    &mut ScenePass::new(&mut pass, dest_physical),
                                    SceneGpuPassContext {
                                        gpu: pipelines.gpu,
                                        target_format: pipelines.format,
                                        presentation: pipelines.presentation,
                                        bounds: *bounds,
                                        clip: *clip,
                                        dest_size: dest_physical,
                                        gpu_work: Some(pipelines.gpu_work),
                                    },
                                ) {
                                    restore_dest_viewport(&mut pass, dest_physical);
                                    index += 1;
                                    continue;
                                }
                            }
                            break;
                        }
                        DrawCommand::Custom { .. } => {}
                    }
                    index += 1;
                }
                drop(pass);
                if let Some(DrawCommand::Custom {
                    node,
                    renderer,
                    bounds,
                    clip,
                }) = commands.get(index)
                {
                    if bounds.width > 0 && bounds.height > 0 {
                        let target = match stack.last() {
                            Some(frame) => dest
                                .expect("dest target present for groups")
                                .group_view(frame.layer),
                            None => root_view,
                        };
                        renderer.render(
                            node,
                            SceneGpuRenderContext::new(
                                pipelines.gpu,
                                pipelines.format,
                                pipelines.presentation,
                                *bounds,
                                *clip,
                                dest_physical,
                                Some(pipelines.gpu_work),
                                encoder,
                                target,
                            ),
                        );
                    }
                    index += 1;
                }
            }
        }
    }
    // Every command may have been an empty group, skipped: the dest still
    // owes its clear.
    ensure_backdrop_target_ready(
        root_view,
        encoder,
        dest_physical,
        &mut dest_load,
        dest_passes,
    );
}

/// Offer the renderer the run of `Custom` commands starting at `start` that
/// share its renderer instance and want no dedicated pass.
///
/// The run is a contiguous slice of the display list, so any other command
/// between two custom nodes ends it and document order is untouched. Returns
/// how many commands the renderer encoded; `0` means it declined and the caller
/// falls back to the single-node path.
#[allow(clippy::too_many_arguments)]
fn draw_custom_run(
    commands: &[DrawCommand],
    start: usize,
    renderer: &Arc<dyn SceneGpuRenderer>,
    pass: &mut ScenePass<'_, '_>,
    gpu: &GpuContext,
    format: GpuTextureFormat,
    presentation: ScenePresentationProfile,
    gpu_work: &GpuWorkSink,
) -> usize {
    let capacity = renderer.batch_capacity();
    if capacity <= 1 {
        return 0;
    }
    let mut run: Vec<SceneGpuBatchNode<'_>> = Vec::new();
    for command in &commands[start..] {
        let DrawCommand::Custom {
            node,
            renderer: candidate,
            bounds,
            clip,
        } = command
        else {
            break;
        };
        if !Arc::ptr_eq(renderer, candidate)
            || node.custom.dedicated_pass
            || bounds.width == 0
            || bounds.height == 0
            || run.len() == capacity
        {
            break;
        }
        run.push(SceneGpuBatchNode {
            node,
            bounds: *bounds,
            clip: *clip,
        });
    }
    if run.len() < 2 {
        return 0;
    }
    let encoded = renderer.draw_batch_in_pass(
        &run,
        pass,
        SceneGpuBatchPassContext {
            gpu,
            target_format: format,
            presentation,
            dest_size: pass.dest_size(),
            gpu_work: Some(gpu_work),
        },
    );
    encoded.min(run.len())
}

fn group_layer_load(ready: &mut Vec<bool>, layer: usize) -> wgpu::LoadOp<wgpu::Color> {
    if layer >= ready.len() {
        ready.resize(layer + 1, false);
    }
    if ready[layer] {
        wgpu::LoadOp::Load
    } else {
        ready[layer] = true;
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
    }
}

fn restore_dest_viewport(pass: &mut wgpu::RenderPass<'_>, dest_physical: [u32; 2]) {
    pass.set_viewport(
        0.0,
        0.0,
        dest_physical[0].max(1) as f32,
        dest_physical[1].max(1) as f32,
        0.0,
        1.0,
    );
}

fn ensure_backdrop_target_ready(
    root_view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    dest_physical: [u32; 2],
    dest_load: &mut wgpu::LoadOp<wgpu::Color>,
    dest_passes: &mut DestPassCounts,
) {
    if matches!(*dest_load, wgpu::LoadOp::Clear(_)) {
        let mut pass =
            DestTarget::begin_color_pass_on_view(encoder, root_view, *dest_load, dest_passes);
        restore_dest_viewport(&mut pass, dest_physical);
        drop(pass);
        *dest_load = wgpu::LoadOp::Load;
    }
}

/// Whether a quad paints no pixel at all: no fill, no visible edge, outline
/// or shadow, no image, material or backdrop. Such boxes still lay out, hit
/// and clip; they only have nothing to draw.
fn quad_paints_nothing(
    background: Option<nana_ui_core::PaintColor>,
    border_color: Option<nana_ui_core::PaintColor>,
    border_width: f32,
    shadow: Option<&nana_ui_runtime::ComponentElevation>,
    surface: &nana_ui_scene::QuadSurfacePaint,
) -> bool {
    let edge = (border_width > 0.0 && border_color.is_some())
        || surface
            .border_widths
            .iter()
            .zip(&surface.border_colors)
            .any(|(width, color)| *width > 0.0 && (color.is_some() || border_color.is_some()));
    background.is_none()
        && !edge
        && shadow.is_none()
        && surface.extra_shadows.is_empty()
        && surface.border_image.is_none()
        && !(surface.outline_width > 0.0 && surface.outline_color.is_some())
        && surface.background_material.is_none()
        && surface.border_material.is_none()
        && surface.background_image.is_none()
        && surface.background_layers.is_empty()
        && surface.content_image.is_none()
        && surface
            .backdrop_filter
            .is_none_or(|filter| !filter.is_active())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pipeline_cache_tests;

#[cfg(test)]
mod presentation_tests;

#[cfg(test)]
mod scroll_fade_tests;

#[cfg(all(test, feature = "rich-text"))]
mod glyph_prefix_tests;

/// Whether `rect` (layout space), under `affine` into logical target space,
/// reaches the `size` physical-pixel target.
fn rect_meets_target(
    rect: nana_ui_scene::SceneRect,
    affine: [f32; 6],
    scale: f32,
    size: [u32; 2],
) -> bool {
    let corners = [
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y],
        [rect.x, rect.y + rect.height],
        [rect.x + rect.width, rect.y + rect.height],
    ]
    .map(|[x, y]| clip::transform_point(affine, x, y));
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for corner in corners {
        for axis in 0..2 {
            min[axis] = min[axis].min(corner[axis] * scale);
            max[axis] = max[axis].max(corner[axis] * scale);
        }
    }
    // One pixel of slack for the clip's anti-aliased edge; NaN fails closed
    // to drawing.
    !(max[0] < -1.0
        || max[1] < -1.0
        || min[0] > size[0] as f32 + 1.0
        || min[1] > size[1] as f32 + 1.0)
}

/// Painter tests record into encoders and targets of the shared test device
/// directly; the frame contract has tests of its own.
#[cfg(test)]
impl SceneWgpuPainter {
    pub(crate) fn for_test(format: wgpu::TextureFormat) -> Self {
        let guard = crate::test_gpu::lock();
        let mut painter = Self::new(
            &crate::test_gpu::context(),
            __framework::format_from_wgpu(format),
        );
        painter._test_gpu_guard = Some(guard);
        painter
    }

    pub(crate) fn paint_encoder(
        &mut self,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.paint_encoder_with_options(
            scene,
            encoder,
            target,
            viewport,
            ScenePaintOptions::default(),
            host_textures,
            gpu_renderers,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_encoder_with_options(
        &mut self,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        options: ScenePaintOptions,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.outside_frames();
        self.try_sync_effective_presentation(scene, &viewport, host_textures)?;
        self.paint_recorded(
            DEFAULT_TARGET_ID,
            scene,
            encoder,
            target,
            viewport,
            options,
            host_textures,
            gpu_renderers,
            None,
        )
    }

    /// An encoder of the caller's own is submitted on its own: its queue
    /// writes land ahead of its passes alone, as the first paint's do.
    fn outside_frames(&mut self) {
        self.core.last_frame = None;
        self.core.text.set_follows_in_frame(false);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_target_encoder(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.paint_target_encoder_with_options(
            id,
            scene,
            encoder,
            target,
            viewport,
            ScenePaintOptions::default(),
            host_textures,
            gpu_renderers,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_target_encoder_with_options(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        options: ScenePaintOptions,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        self.outside_frames();
        self.try_sync_effective_presentation(scene, &viewport, host_textures)?;
        self.paint_recorded(
            id,
            scene,
            encoder,
            target,
            viewport,
            options,
            host_textures,
            gpu_renderers,
            None,
        )
    }

    fn default_target(&self) -> Option<&TargetResources> {
        self.targets.get(&DEFAULT_TARGET_ID)
    }

    fn default_target_mut(&mut self) -> &mut TargetResources {
        let core = &self.core;
        self.targets
            .entry(DEFAULT_TARGET_ID)
            .or_insert_with(|| core.new_target_resources())
    }
}
