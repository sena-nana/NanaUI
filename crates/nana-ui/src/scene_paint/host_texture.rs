use super::clip::{self, LogicalRect, physical_scissor};
use crate::gpu_texture::{
    GpuTexturePipeline, GpuTexturePrimitive, HostTextureDrawMode, HostTextureLayer,
};
use crate::gpu_view::intersect_physical;
use crate::{HostTextureAlphaMode, HostTextureBinding, PhysicalRect};

pub(super) struct HostTexturePipeline {
    pipeline: GpuTexturePipeline,
    /// The scene's corner curve exponent, set each frame before prepare.
    corner_exponent: f32,
}

pub(super) struct PreparedHostTexture {
    primitive: GpuTexturePrimitive,
    pub(super) slot_name: nana_ui_runtime::TextureSlotName,
    pub(super) src_size: [u32; 2],
    /// Dest AABB in physical target pixels when axis-aligned (`affine[1] == 0`,
    /// `affine[2] == 0`, `persp == [0, 0]`), used to project slot damage rects.
    pub(super) aligned_dest_px: Option<[f32; 4]>,
    pub(super) clip: PhysicalRect,
    /// 这一帧这张宿主纹理实际覆盖的设备像素范围(变换后 AABB × 缩放)。
    /// 消费方据此知道该按多少像素准备内容,不必从布局盒自己反推。
    pub(super) painted: [u32; 2],
    simple: bool,
    opaque_full_alpha: bool,
    clear_premul: [f32; 4],
}

impl PreparedHostTexture {
    /// True when this axis-aligned layer covers `region` and can replace an
    /// initial `LoadOp::Clear(load_color)` with `BlendState::REPLACE` inside
    /// `region` without reading back prior pixels.
    pub(super) fn can_replace_clear_in(
        &self,
        region: PhysicalRect,
        load_color: wgpu::Color,
    ) -> bool {
        if !self.simple
            || region.width == 0
            || region.height == 0
            || region.x < self.clip.x
            || region.y < self.clip.y
            || region.x.saturating_add(region.width) > self.clip.x.saturating_add(self.clip.width)
            || region.y.saturating_add(region.height) > self.clip.y.saturating_add(self.clip.height)
        {
            return false;
        }
        if self.opaque_full_alpha {
            return true;
        }
        const EPS: f32 = 1.0e-5;
        (self.clear_premul[0] - load_color.r as f32).abs() <= EPS
            && (self.clear_premul[1] - load_color.g as f32).abs() <= EPS
            && (self.clear_premul[2] - load_color.b as f32).abs() <= EPS
            && (self.clear_premul[3] - load_color.a as f32).abs() <= EPS
    }

    /// True when this full-viewport axis-aligned layer can replace the pass's
    /// initial `LoadOp::Clear(load_color)` with `LoadOp::DontCare` + `BlendState::REPLACE`.
    pub(super) fn can_replace_clear(
        &self,
        dest_physical: [u32; 2],
        load_color: wgpu::Color,
    ) -> bool {
        self.clip
            == (PhysicalRect {
                x: 0,
                y: 0,
                width: dest_physical[0],
                height: dest_physical[1],
            })
            && self.can_replace_clear_in(self.clip, load_color)
    }

    /// Maps a source texel damage rectangle to clamped physical target pixels,
    /// or returns `None` if the mapping is non-axis-aligned.
    pub(super) fn map_source_damage(&self, damage: PhysicalRect) -> Option<PhysicalRect> {
        if damage.width == 0 || damage.height == 0 {
            return Some(PhysicalRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        }
        let [dx, dy, dw, dh] = self.aligned_dest_px?;
        let src_w = self.src_size[0].max(1) as f32;
        let src_h = self.src_size[1].max(1) as f32;
        let sx0 = (damage.x as f32 / src_w) * dw + dx;
        let sy0 = (damage.y as f32 / src_h) * dh + dy;
        let sx1 = ((damage.x.saturating_add(damage.width)) as f32 / src_w) * dw + dx;
        let sy1 = ((damage.y.saturating_add(damage.height)) as f32 / src_h) * dh + dy;
        // 1px safety pad covers bilinear sampling fringes when scaled.
        let pad = if self.painted == self.src_size && self.simple {
            0.0
        } else {
            1.0
        };
        let x0 = (sx0 - pad).floor().max(0.0) as u32;
        let y0 = (sy0 - pad).floor().max(0.0) as u32;
        let x1 = (sx1 + pad).ceil().max(0.0) as u32;
        let y1 = (sy1 + pad).ceil().max(0.0) as u32;
        Some(intersect_physical(
            PhysicalRect {
                x: x0,
                y: y0,
                width: x1.saturating_sub(x0),
                height: y1.saturating_sub(y0),
            },
            self.clip,
        ))
    }
}

impl HostTexturePipeline {
    pub(super) fn keep_alive_until_complete(
        target: &crate::gpu_texture::GpuTextureTarget,
        frame: &mut nana_gpu::FrameContext,
    ) {
        GpuTexturePipeline::keep_alive_until_complete(target, frame);
    }

    pub(super) fn invalidate_target_image_bindings(
        target: &mut crate::gpu_texture::GpuTextureTarget,
    ) {
        GpuTexturePipeline::invalidate_target_image_bindings(target);
    }
    pub(super) fn try_new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        policy: &nana_gpu::GpuDeviceState,
        gpu: &nana_gpu::GpuContext,
    ) -> Result<Self, super::validate::ScenePaintError> {
        Ok(Self {
            pipeline: GpuTexturePipeline::try_new_with_policy(
                device,
                queue,
                format,
                Some(policy),
                Some(gpu),
            )?,
            corner_exponent: 2.0,
        })
    }

    /// The scene's corner curve exponent, for the host textures prepared
    /// this frame.
    pub(super) fn set_corner_exponent(&mut self, exponent: f32) {
        self.corner_exponent = exponent;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &self,
        target: &mut crate::gpu_texture::GpuTextureTarget,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        url_cache: &mut super::url_texture_cache::UrlTextureCache,
        binding: HostTextureBinding,
        node: u64,
        slot: u64,
        bounds: LogicalRect,
        affine: [f32; 6],
        persp: [f32; 2],
        clip: PhysicalRect,
        opacity: f32,
        corner_radii: [f32; 4],
        rounded_clip: LogicalRect,
        fragment_clip: super::clip::FragmentClip,
        physical_size: [u32; 2],
        scale_factor: f32,
        mask: Option<nana_ui_core::MaskImage>,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
        checkerboard: bool,
        zoom: f32,
        sampling: nana_ui_core::ImageSampling,
        clear_premul: [f32; 4],
    ) -> PreparedHostTexture {
        let slot_name = binding.slot.clone();
        let binding_size = [binding.width, binding.height];
        let opaque_full_alpha =
            binding.alpha_mode == HostTextureAlphaMode::Opaque && opacity >= 1.0 - 1.0e-6;
        let primitive = GpuTexturePrimitive::from_scene(
            node,
            slot,
            HostTextureLayer::from_binding(binding)
                .with_opacity(opacity)
                .with_corner_radii(corner_radii)
                .with_clip(crate::geometry::LogicalRect::new(
                    rounded_clip.x,
                    rounded_clip.y,
                    rounded_clip.width,
                    rounded_clip.height,
                ))
                .with_fragment_clip(
                    fragment_clip.rect,
                    fragment_clip.inv_abcd,
                    fragment_clip.inv_ef,
                    fragment_clip.corner_radius,
                )
                .with_mask(mask.clone())
                .with_checkerboard(checkerboard)
                .with_zoom(zoom)
                .with_sampling(sampling)
                .with_corner_exponent(self.corner_exponent),
        );
        primitive.prepare(
            &self.pipeline,
            target,
            url_cache,
            device,
            queue,
            crate::geometry::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            affine,
            persp,
            scale_factor,
            physical_size,
            clear_premul,
            gpu_work,
        );
        let world = if clip::is_translation_projective(affine, persp) {
            clip::translated_on_grid(bounds, affine, scale_factor)
        } else {
            clip::transformed_aabb_projective(bounds, affine, persp)
        };
        let painted = [
            physical_extent(world.width, scale_factor),
            physical_extent(world.height, scale_factor),
        ];
        let clip = physical_scissor(world, scale_factor, physical_size)
            .map(|world| intersect_physical(world, clip))
            .unwrap_or(PhysicalRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        let simple = is_simple_host_texture(
            bounds,
            affine,
            persp,
            clip,
            corner_radii,
            rounded_clip,
            &fragment_clip,
            scale_factor,
            mask.as_ref(),
            checkerboard,
            zoom,
            sampling,
            binding_size,
            painted,
        );
        let aligned_dest_px = (affine[1].abs() <= 1.0e-6
            && affine[2].abs() <= 1.0e-6
            && affine[0] > 0.0
            && affine[3] > 0.0
            && persp[0].abs() <= 1.0e-6
            && persp[1].abs() <= 1.0e-6
            && scale_factor.is_finite()
            && scale_factor > 0.0)
            .then(|| {
                [
                    (affine[0] * bounds.x + affine[4]) * scale_factor,
                    (affine[3] * bounds.y + affine[5]) * scale_factor,
                    affine[0] * bounds.width * scale_factor,
                    affine[3] * bounds.height * scale_factor,
                ]
            });
        PreparedHostTexture {
            primitive,
            slot_name,
            src_size: binding_size,
            aligned_dest_px,
            clip,
            painted,
            simple,
            opaque_full_alpha,
            clear_premul,
        }
    }

    pub(super) fn draw(
        &self,
        target: &crate::gpu_texture::GpuTextureTarget,
        prepared: &PreparedHostTexture,
        pass: &mut wgpu::RenderPass<'_>,
        dest_size: [u32; 2],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.draw_clipped(target, prepared, pass, dest_size, prepared.clip, gpu_work);
    }

    pub(super) fn draw_clipped(
        &self,
        target: &crate::gpu_texture::GpuTextureTarget,
        prepared: &PreparedHostTexture,
        pass: &mut wgpu::RenderPass<'_>,
        dest_size: [u32; 2],
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let mode = if prepared.simple {
            HostTextureDrawMode::SimpleBlend
        } else {
            HostTextureDrawMode::Full
        };
        prepared.primitive.draw_in_pass_with_mode(
            &self.pipeline,
            target,
            pass,
            intersect_physical(prepared.clip, scissor),
            dest_size,
            mode,
            gpu_work,
        );
    }

    pub(super) fn draw_replace(
        &self,
        target: &crate::gpu_texture::GpuTextureTarget,
        prepared: &PreparedHostTexture,
        pass: &mut wgpu::RenderPass<'_>,
        dest_size: [u32; 2],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.draw_replace_clipped(target, prepared, pass, dest_size, prepared.clip, gpu_work);
    }

    pub(super) fn draw_replace_clipped(
        &self,
        target: &crate::gpu_texture::GpuTextureTarget,
        prepared: &PreparedHostTexture,
        pass: &mut wgpu::RenderPass<'_>,
        dest_size: [u32; 2],
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        prepared.primitive.draw_in_pass_with_mode(
            &self.pipeline,
            target,
            pass,
            intersect_physical(prepared.clip, scissor),
            dest_size,
            HostTextureDrawMode::SimpleReplace,
            gpu_work,
        );
    }

    pub(super) fn trim(target: &mut crate::gpu_texture::GpuTextureTarget) {
        GpuTexturePipeline::trim(target);
    }
}

#[allow(clippy::too_many_arguments)]
fn is_simple_host_texture(
    bounds: LogicalRect,
    affine: [f32; 6],
    persp: [f32; 2],
    clip: PhysicalRect,
    corner_radii: [f32; 4],
    rounded_clip: LogicalRect,
    fragment_clip: &super::clip::FragmentClip,
    scale_factor: f32,
    mask: Option<&nana_ui_core::MaskImage>,
    checkerboard: bool,
    zoom: f32,
    sampling: nana_ui_core::ImageSampling,
    binding_size: [u32; 2],
    painted: [u32; 2],
) -> bool {
    const EPS: f32 = 1.0e-4;
    const PX_EPS: f32 = 1.0e-3;
    if affine[1].abs() > 1.0e-6
        || affine[2].abs() > 1.0e-6
        || affine[0] <= 0.0
        || affine[3] <= 0.0
        || persp[0].abs() > 1.0e-6
        || persp[1].abs() > 1.0e-6
        || !scale_factor.is_finite()
        || scale_factor <= 0.0
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || clip.width == 0
        || clip.height == 0
        || !corner_radii.iter().all(|&r| r <= 0.0)
        || rounded_clip.x > bounds.x + EPS
        || rounded_clip.y > bounds.y + EPS
        || rounded_clip.x + rounded_clip.width < bounds.x + bounds.width - EPS
        || rounded_clip.y + rounded_clip.height < bounds.y + bounds.height - EPS
        || fragment_clip.rect[2] < 1.0e7
        || fragment_clip.rect[3] < 1.0e7
        || fragment_clip.polygon_count != 0
        || mask.is_some()
        || checkerboard
        || zoom > 1.0
        || sampling != nana_ui_core::ImageSampling::Resample
        || binding_size[0] == 0
        || binding_size[1] == 0
        || painted[0] < binding_size[0]
        || painted[1] < binding_size[1]
    {
        return false;
    }
    let x0 = (affine[0] * bounds.x + affine[4]) * scale_factor;
    let y0 = (affine[3] * bounds.y + affine[5]) * scale_factor;
    let x1 = (affine[0] * (bounds.x + bounds.width) + affine[4]) * scale_factor;
    let y1 = (affine[3] * (bounds.y + bounds.height) + affine[5]) * scale_factor;
    if (x0 - x0.round()).abs() > PX_EPS
        || (y0 - y0.round()).abs() > PX_EPS
        || (x1 - x1.round()).abs() > PX_EPS
        || (y1 - y1.round()).abs() > PX_EPS
    {
        return false;
    }
    let qx0 = x0.round() as i64;
    let qy0 = y0.round() as i64;
    let qx1 = x1.round() as i64;
    let qy1 = y1.round() as i64;
    let cx0 = i64::from(clip.x);
    let cy0 = i64::from(clip.y);
    let cx1 = cx0 + i64::from(clip.width);
    let cy1 = cy0 + i64::from(clip.height);
    qx1 > qx0 && qy1 > qy0 && cx0 >= qx0 && cy0 >= qy0 && cx1 <= qx1 && cy1 <= qy1
}

/// 逻辑长度换成设备像素。非有限值与负值当作 0——没画到就是没画到,
/// 让消费方拿到一个「无」而不是一个巨大的假尺寸。
fn physical_extent(logical: f32, scale_factor: f32) -> u32 {
    let pixels = logical * scale_factor;
    if !pixels.is_finite() || pixels <= 0.0 {
        return 0;
    }
    pixels.round() as u32
}
