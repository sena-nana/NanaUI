//! A [`wgpu`] renderer for [Iced].
//!
//! ![The native path of the Iced ecosystem](https://github.com/iced-rs/iced/blob/0525d76ff94e828b7b21634fa94a747022001c83/docs/graphs/native.png?raw=true)
//!
//! [`wgpu`] supports most modern graphics backends: Vulkan, Metal, DX11, and
//! DX12 (OpenGL and WebGL are still WIP). Additionally, it will support the
//! incoming [WebGPU API].
//!
//! Currently, `iced_wgpu` supports the following primitives:
//! - Text, which is rendered using [`glyphon`].
//! - Quads or rectangles, with rounded borders and a solid background color.
//! - Clip areas, useful to implement scrollables or hide overflowing content.
//! - Images and SVG, loaded from memory or the file system.
//! - Meshes of triangles, useful to draw geometry freely.
//!
//! [Iced]: https://github.com/iced-rs/iced
//! [`wgpu`]: https://github.com/gfx-rs/wgpu-rs
//! [WebGPU API]: https://gpuweb.github.io/gpuweb/
//! [`glyphon`]: https://github.com/grovesNL/glyphon
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/iced-rs/iced/9ab6923e943f784985e9ef9ca28b10278297225d/docs/logo.svg"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(missing_docs)]
pub mod layer;
pub mod primitive;
pub mod window;

#[cfg(feature = "geometry")]
pub mod geometry;

mod buffer;
mod color;
mod engine;
mod quad;
mod text;
mod triangle;

#[cfg(any(feature = "image", feature = "svg"))]
#[path = "image/mod.rs"]
mod image;
mod opacity;

#[cfg(not(any(feature = "image", feature = "svg")))]
#[path = "image/null.rs"]
mod image;

use buffer::Buffer;

use iced_debug as debug;
pub use iced_graphics as graphics;
pub use iced_graphics::core;

pub use wgpu;

pub use engine::Engine;
pub use layer::Layer;
pub use primitive::Primitive;

#[cfg(feature = "geometry")]
pub use geometry::Geometry;

use crate::core::renderer;
use crate::core::{Background, Color, Font, Pixels, Point, Rectangle, Size, Transformation};
use crate::graphics::mesh;
use crate::graphics::text::{Editor, Paragraph};
use crate::graphics::{Shell, Viewport};

/// A [`wgpu`] graphics renderer for [`iced`].
///
/// [`wgpu`]: https://github.com/gfx-rs/wgpu-rs
/// [`iced`]: https://github.com/iced-rs/iced
pub struct Renderer {
    engine: Engine,
    settings: renderer::Settings,

    layers: layer::Stack,
    scale: Option<renderer::Scale>,

    quad: quad::State,
    triangle: triangle::State,
    text: text::State,
    text_viewport: text::Viewport,

    #[cfg(any(feature = "svg", feature = "image"))]
    image: image::State,

    // TODO: Centralize all the image feature handling
    #[cfg(any(feature = "svg", feature = "image"))]
    image_cache: std::cell::RefCell<image::Cache>,

    staging_belt: wgpu::util::StagingBelt,
    opacity: opacity::Pipeline,

    /// Renderer-level isolation groups recorded by widgets during draw.
    /// The ranges refer to the stable layer order of the current frame.
    opacity_groups: Vec<OpacityGroup>,
    open_opacity_groups: Vec<OpenOpacityGroup>,
    next_layer_index: usize,
    group_textures: Vec<GroupTexture>,
}

struct GroupTexture {
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Debug, Clone, Copy)]
struct OpacityGroup {
    start: usize,
    end: usize,
    bounds: Rectangle,
    opacity: f32,
    affine: [f32; 6],
}

#[derive(Debug, Clone, Copy)]
struct OpenOpacityGroup {
    start: usize,
    bounds: Rectangle,
    opacity: f32,
    affine: [f32; 6],
}

impl Renderer {
    pub fn new(engine: Engine, settings: renderer::Settings) -> Self {
        Self {
            settings,
            layers: layer::Stack::new(),
            scale: None,

            quad: quad::State::new(),
            triangle: triangle::State::new(&engine.device, &engine.triangle_pipeline),
            text: text::State::new(),
            text_viewport: engine.text_pipeline.create_viewport(&engine.device),

            #[cfg(any(feature = "svg", feature = "image"))]
            image: image::State::new(),

            #[cfg(any(feature = "svg", feature = "image"))]
            image_cache: std::cell::RefCell::new(engine.create_image_cache()),

            // TODO: Resize belt smartly (?)
            // It would be great if the `StagingBelt` API exposed methods
            // for introspection to detect when a resize may be worth it.
            staging_belt: wgpu::util::StagingBelt::new(
                engine.device.clone(),
                buffer::MAX_WRITE_SIZE as u64,
            ),
            opacity: opacity::Pipeline::new(&engine.device, engine.format),

            opacity_groups: Vec::new(),
            open_opacity_groups: Vec::new(),
            next_layer_index: 1,
            group_textures: Vec::new(),

            engine,
        }
    }

    /// Starts an isolated opacity group. All primitives recorded until the
    /// matching [`end_opacity_group`](Self::end_opacity_group) are composited
    /// together before `opacity` is applied.
    pub fn start_opacity_group(&mut self, bounds: Rectangle, opacity: f32) {
        self.layers.push_clip(bounds);
        let bounds = bounds * self.layers.transformation();
        let start = self.next_layer_index;
        self.next_layer_index += 1;
        self.open_opacity_groups.push(OpenOpacityGroup {
            start,
            bounds,
            opacity: opacity.clamp(0.0, 1.0),
            affine: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        });
    }

    /// Starts an isolated 2D affine group. The matrix uses the CSS/Canvas
    /// `matrix(a, b, c, d, e, f)` convention in logical coordinates.
    pub fn start_affine_group(&mut self, bounds: Rectangle, affine: [f32; 6]) {
        self.layers.push_clip(bounds);
        let bounds = bounds * self.layers.transformation();
        let start = self.next_layer_index;
        self.next_layer_index += 1;
        self.open_opacity_groups.push(OpenOpacityGroup {
            start,
            bounds,
            opacity: 1.0,
            affine,
        });
    }

    /// Ends the latest isolated affine group.
    pub fn end_affine_group(&mut self) {
        self.end_opacity_group();
    }

    /// Ends the latest isolated opacity group.
    pub fn end_opacity_group(&mut self) {
        let Some(group) = self.open_opacity_groups.pop() else {
            debug_assert!(false, "end_opacity_group without a matching start");
            return;
        };
        self.layers.pop_clip();
        self.opacity_groups.push(OpacityGroup {
            start: group.start,
            end: self.next_layer_index,
            bounds: group.bounds,
            opacity: group.opacity,
            affine: group.affine,
        });
    }

    /// Record commands that draw the current primitives to the target texture view.
    ///
    /// You must call [`finish`](Self::finish) and [`recall`](Self::recall) when submitting
    /// the resulting [`wgpu::CommandEncoder`].
    pub fn draw(
        &mut self,
        clear_color: Option<Color>,
        target: &wgpu::TextureView,
        viewport: &Viewport,
    ) -> wgpu::CommandEncoder {
        let mut encoder =
            self.engine
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("iced_wgpu encoder"),
                });

        self.prepare(&mut encoder, viewport);
        self.render(&mut encoder, target, clear_color, viewport);

        self.quad.trim();
        self.triangle.trim();
        self.text.trim();

        // TODO: Provide window id (?)
        self.engine.trim();

        #[cfg(any(feature = "svg", feature = "image"))]
        {
            self.image.trim();
            self.image_cache.borrow_mut().trim();
        }

        encoder
    }

    pub fn present(
        &mut self,
        clear_color: Option<Color>,
        _format: wgpu::TextureFormat,
        frame: &wgpu::TextureView,
        viewport: &Viewport,
    ) -> wgpu::SubmissionIndex {
        let encoder = self.draw(clear_color, frame, viewport);

        self.staging_belt.finish();
        let submission = self.engine.queue.submit([encoder.finish()]);
        self.staging_belt.recall();
        submission
    }

    /// Renders the current surface to an offscreen buffer.
    ///
    /// Returns RGBA bytes of the texture data.
    pub fn screenshot(&mut self, viewport: &Viewport, background_color: Color) -> Vec<u8> {
        #[derive(Clone, Copy, Debug)]
        struct BufferDimensions {
            width: u32,
            height: u32,
            unpadded_bytes_per_row: usize,
            padded_bytes_per_row: usize,
        }

        impl BufferDimensions {
            fn new(size: Size<u32>) -> Self {
                let unpadded_bytes_per_row = size.width as usize * 4; //slice of buffer per row; always RGBA
                let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize; //256
                let padded_bytes_per_row_padding =
                    (alignment - unpadded_bytes_per_row % alignment) % alignment;
                let padded_bytes_per_row = unpadded_bytes_per_row + padded_bytes_per_row_padding;

                Self {
                    width: size.width,
                    height: size.height,
                    unpadded_bytes_per_row,
                    padded_bytes_per_row,
                }
            }
        }

        let dimensions = BufferDimensions::new(viewport.physical_size());

        let texture_extent = wgpu::Extent3d {
            width: dimensions.width,
            height: dimensions.height,
            depth_or_array_layers: 1,
        };

        let texture = self.engine.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("iced_wgpu.offscreen.source_texture"),
            size: texture_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.engine.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.draw(Some(background_color), &view, viewport);

        let texture = crate::color::convert(
            &self.engine.device,
            &mut encoder,
            texture,
            if graphics::color::GAMMA_CORRECTION {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
        );

        let output_buffer = self.engine.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("iced_wgpu.offscreen.output_texture_buffer"),
            size: (dimensions.padded_bytes_per_row * dimensions.height as usize) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dimensions.padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            texture_extent,
        );

        self.staging_belt.finish();
        let index = self.engine.queue.submit([encoder.finish()]);
        self.staging_belt.recall();

        let slice = output_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});

        let _ = self.engine.device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        });

        let mapped_buffer = slice
            .get_mapped_range()
            .expect("the screenshot output buffer must be mapped after polling");

        mapped_buffer
            .chunks(dimensions.padded_bytes_per_row)
            .fold(vec![], |mut acc, row| {
                acc.extend(&row[..dimensions.unpadded_bytes_per_row]);
                acc
            })
    }

    fn prepare(&mut self, encoder: &mut wgpu::CommandEncoder, viewport: &Viewport) {
        let scale_factor = viewport.scale_factor();

        self.text_viewport
            .update(&self.engine.queue, viewport.physical_size());

        let viewport_physical_bounds =
            Rectangle::<f32>::from(Rectangle::with_size(viewport.physical_size()));

        // Isolation ranges depend on stable layer boundaries. Merging across
        // an opacity boundary would destroy both ordering and group semantics.
        if self.opacity_groups.is_empty() {
            self.layers.merge();
        } else {
            self.layers.flush();
        }

        for layer in self.layers.iter() {
            let clip_bounds = layer.bounds * scale_factor;

            if viewport_physical_bounds
                .intersection(&clip_bounds)
                .and_then(Rectangle::snap)
                .is_none()
            {
                continue;
            }

            if !layer.quads.is_empty() {
                let prepare_span = debug::prepare(debug::Primitive::Quad);

                self.quad.prepare(
                    &self.engine.quad_pipeline,
                    &self.engine.device,
                    &mut self.staging_belt,
                    encoder,
                    &layer.quads,
                    viewport.projection(),
                    scale_factor,
                );

                prepare_span.finish();
            }

            if !layer.triangles.is_empty() {
                let prepare_span = debug::prepare(debug::Primitive::Triangle);

                self.triangle.prepare(
                    &self.engine.triangle_pipeline,
                    &self.engine.device,
                    &mut self.staging_belt,
                    encoder,
                    &layer.triangles,
                    Transformation::scale(scale_factor),
                    viewport.physical_size(),
                );

                prepare_span.finish();
            }

            if !layer.primitives.is_empty() {
                let prepare_span = debug::prepare(debug::Primitive::Shader);

                let mut primitive_storage = self
                    .engine
                    .primitive_storage
                    .write()
                    .expect("Write primitive storage");

                for instance in &layer.primitives {
                    instance.primitive.prepare(
                        &mut primitive_storage,
                        &self.engine.device,
                        &self.engine.queue,
                        self.engine.format,
                        &instance.bounds,
                        viewport,
                    );
                }

                prepare_span.finish();
            }

            #[cfg(any(feature = "svg", feature = "image"))]
            if !layer.images.is_empty() {
                let prepare_span = debug::prepare(debug::Primitive::Image);

                self.image.prepare(
                    &self.engine.image_pipeline,
                    &self.engine.device,
                    &mut self.staging_belt,
                    encoder,
                    &mut self.image_cache.borrow_mut(),
                    &layer.images,
                    viewport.projection(),
                    scale_factor,
                );

                prepare_span.finish();
            }

            if !layer.text.is_empty() {
                let prepare_span = debug::prepare(debug::Primitive::Text);

                self.text.prepare(
                    &self.engine.text_pipeline,
                    &self.engine.device,
                    &self.engine.queue,
                    &self.text_viewport,
                    encoder,
                    &layer.text,
                    layer.bounds,
                    Transformation::scale(scale_factor),
                );

                prepare_span.finish();
            }
        }
    }

    fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        clear_color: Option<Color>,
        viewport: &Viewport,
    ) {
        use std::mem::ManuallyDrop;

        struct GroupSurface {
            group: OpacityGroup,
            view: wgpu::TextureView,
        }

        let extent = wgpu::Extent3d {
            width: viewport.physical_width(),
            height: viewport.physical_height(),
            depth_or_array_layers: 1,
        };
        let group_count = self.opacity_groups.len();
        while self.group_textures.len() < group_count {
            self.group_textures.push(create_group_texture(
                &self.engine.device,
                self.engine.format,
                extent.width,
                extent.height,
            ));
        }
        self.group_textures.truncate(group_count);
        for texture in &mut self.group_textures {
            if texture.width != extent.width || texture.height != extent.height {
                *texture = create_group_texture(
                    &self.engine.device,
                    self.engine.format,
                    extent.width,
                    extent.height,
                );
            }
        }
        let group_surfaces = self
            .opacity_groups
            .iter()
            .copied()
            .zip(&self.group_textures)
            .map(|(group, texture)| GroupSurface {
                group,
                view: texture.view.clone(),
            })
            .collect::<Vec<_>>();

        let mut render_pass =
            ManuallyDrop::new(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("iced_wgpu render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match clear_color {
                            Some(background_color) => wgpu::LoadOp::Clear({
                                let [r, g, b, a] =
                                    graphics::color::pack(background_color).components();

                                wgpu::Color {
                                    r: f64::from(r * a),
                                    g: f64::from(g * a),
                                    b: f64::from(b * a),
                                    a: f64::from(a),
                                }
                            }),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));

        let mut quad_layer = 0;
        let mut mesh_layer = 0;
        let mut text_layer = 0;

        #[cfg(any(feature = "svg", feature = "image"))]
        let mut image_layer = 0;

        let scale_factor = viewport.scale_factor();
        let viewport_physical_bounds =
            Rectangle::<f32>::from(Rectangle::with_size(viewport.physical_size()));

        let scale = Transformation::scale(scale_factor);

        for (layer_index, layer) in self.layers.iter().enumerate() {
            let active_group = group_surfaces
                .iter()
                .enumerate()
                .filter(|(_, surface)| {
                    layer_index >= surface.group.start && layer_index < surface.group.end
                })
                .min_by_key(|(_, surface)| surface.group.end - surface.group.start)
                .map(|(index, _)| index);
            let active_target = active_group
                .map(|index| &group_surfaces[index].view)
                .unwrap_or(frame);

            let _ = ManuallyDrop::into_inner(render_pass);
            render_pass =
                ManuallyDrop::new(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("iced_wgpu layer render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: active_target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if active_group.is_some_and(|index| {
                                group_surfaces[index].group.start == layer_index
                            }) {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }));
            if let Some((physical_bounds, scissor_rect)) = viewport_physical_bounds
                .intersection(&(layer.bounds * scale_factor))
                .and_then(|bounds| bounds.snap().map(|scissor| (bounds, scissor)))
            {
                if !layer.quads.is_empty() {
                    let render_span = debug::render(debug::Primitive::Quad);
                    self.quad.render(
                        &self.engine.quad_pipeline,
                        quad_layer,
                        scissor_rect,
                        &layer.quads,
                        &mut render_pass,
                    );
                    render_span.finish();

                    quad_layer += 1;
                }

                if !layer.triangles.is_empty() {
                    let _ = ManuallyDrop::into_inner(render_pass);

                    let render_span = debug::render(debug::Primitive::Triangle);
                    mesh_layer += self.triangle.render(
                        &self.engine.triangle_pipeline,
                        encoder,
                        active_target,
                        mesh_layer,
                        &layer.triangles,
                        physical_bounds,
                        scale,
                    );
                    render_span.finish();

                    render_pass =
                        ManuallyDrop::new(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("iced_wgpu render pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: active_target,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        }));
                }

                if !layer.primitives.is_empty() {
                    let render_span = debug::render(debug::Primitive::Shader);

                    let primitive_storage = self
                        .engine
                        .primitive_storage
                        .read()
                        .expect("Read primitive storage");

                    let mut need_render = Vec::new();

                    for instance in &layer.primitives {
                        let bounds = instance.bounds * scale;

                        if let Some(clip_bounds) = (instance.bounds * scale)
                            .intersection(&physical_bounds)
                            .and_then(Rectangle::snap)
                        {
                            render_pass.set_viewport(
                                bounds.x,
                                bounds.y,
                                bounds.width,
                                bounds.height,
                                0.0,
                                1.0,
                            );

                            render_pass.set_scissor_rect(
                                clip_bounds.x,
                                clip_bounds.y,
                                clip_bounds.width,
                                clip_bounds.height,
                            );

                            let drawn = instance
                                .primitive
                                .draw(&primitive_storage, &mut render_pass);

                            if !drawn {
                                need_render.push((instance, clip_bounds));
                            }
                        }
                    }

                    render_pass.set_viewport(
                        0.0,
                        0.0,
                        viewport.physical_width() as f32,
                        viewport.physical_height() as f32,
                        0.0,
                        1.0,
                    );

                    render_pass.set_scissor_rect(
                        0,
                        0,
                        viewport.physical_width(),
                        viewport.physical_height(),
                    );

                    if !need_render.is_empty() {
                        let _ = ManuallyDrop::into_inner(render_pass);

                        for (instance, clip_bounds) in need_render {
                            instance.primitive.render(
                                &primitive_storage,
                                encoder,
                                active_target,
                                &clip_bounds,
                            );
                        }

                        render_pass = ManuallyDrop::new(encoder.begin_render_pass(
                            &wgpu::RenderPassDescriptor {
                                label: Some("iced_wgpu render pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: active_target,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            },
                        ));
                    }

                    render_span.finish();
                }

                #[cfg(any(feature = "svg", feature = "image"))]
                if !layer.images.is_empty() {
                    let render_span = debug::render(debug::Primitive::Image);
                    self.image.render(
                        &self.engine.image_pipeline,
                        image_layer,
                        scissor_rect,
                        &mut render_pass,
                    );
                    render_span.finish();

                    image_layer += 1;
                }

                if !layer.text.is_empty() {
                    let render_span = debug::render(debug::Primitive::Text);
                    text_layer += self.text.render(
                        &self.engine.text_pipeline,
                        &self.text_viewport,
                        text_layer,
                        &layer.text,
                        scissor_rect,
                        &mut render_pass,
                    );
                    render_span.finish();
                }
            }

            let _ = ManuallyDrop::into_inner(render_pass);

            let mut ending = group_surfaces
                .iter()
                .enumerate()
                .filter(|(_, surface)| surface.group.end == layer_index + 1)
                .collect::<Vec<_>>();
            ending.sort_by_key(|(_, surface)| surface.group.end - surface.group.start);
            for (group_index, surface) in ending {
                let parent = group_surfaces
                    .iter()
                    .enumerate()
                    .filter(|(index, parent)| {
                        *index != group_index
                            && parent.group.start <= surface.group.start
                            && parent.group.end >= surface.group.end
                    })
                    .min_by_key(|(_, parent)| parent.group.end - parent.group.start)
                    .map(|(_, parent)| &parent.view)
                    .unwrap_or(frame);
                let transformed_bounds = affine_bounds(surface.group.bounds, surface.group.affine);
                let Some(scissor) = (transformed_bounds * scale_factor)
                    .intersection(&viewport_physical_bounds)
                    .and_then(Rectangle::snap)
                else {
                    continue;
                };
                self.opacity.composite(
                    &self.engine.device,
                    encoder,
                    &surface.view,
                    parent,
                    surface.group.opacity,
                    surface.group.affine,
                    scale_factor,
                    viewport.physical_size(),
                    scissor,
                );
            }

            render_pass =
                ManuallyDrop::new(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("iced_wgpu continuation render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }));
        }

        let _ = ManuallyDrop::into_inner(render_pass);

        debug::layers_rendered(|| {
            self.layers
                .iter()
                .filter(|layer| {
                    !layer.is_empty()
                        && viewport_physical_bounds
                            .intersection(&(layer.bounds * scale_factor))
                            .is_some_and(|viewport| viewport.snap().is_some())
                })
                .count()
        });
    }

    /// Prepares currently mapped buffers for use in a submission.
    ///
    /// Usually, this method is only needed if you are calling [`Renderer::draw`] directly,
    /// instead of relying on [`Renderer::present`].
    ///
    /// You must call this method _before_ submitting the resulting [`wgpu::CommandEncoder`]
    /// of [`Renderer::draw`] to a [`wgpu::Queue`].
    pub fn finish(&mut self) {
        self.staging_belt.finish();
    }

    /// Recalls all of the closed buffers back to be reused.
    ///
    /// Usually, this method is only needed if you are calling [`Renderer::draw`] directly,
    /// instead of relying on [`Renderer::present`] to a [`wgpu::Queue`].
    ///
    /// You must call this method _after_ submitting the resulting [`wgpu::CommandEncoder`]
    /// of [`Renderer::draw`] to a [`wgpu::Queue`].
    pub fn recall(&mut self) {
        self.staging_belt.recall();
    }
}

fn create_group_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> GroupTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("iced_wgpu isolated group texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GroupTexture {
        width,
        height,
        _texture: texture,
        view,
    }
}

fn affine_bounds(bounds: Rectangle, [a, b, c, d, e, f]: [f32; 6]) -> Rectangle {
    let points = [
        (bounds.x, bounds.y),
        (bounds.x + bounds.width, bounds.y),
        (bounds.x, bounds.y + bounds.height),
        (bounds.x + bounds.width, bounds.y + bounds.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in points {
        let tx = a * x + c * y + e;
        let ty = b * x + d * y + f;
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    Rectangle::new(
        Point::new(min_x, min_y),
        Size::new((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
    )
}

impl core::Renderer for Renderer {
    fn start_layer(&mut self, bounds: Rectangle) {
        self.layers.push_clip(bounds);
        self.next_layer_index += 1;
    }

    fn end_layer(&mut self) {
        self.layers.pop_clip();
    }

    fn start_transformation(&mut self, transformation: Transformation) {
        self.layers.push_transformation(transformation);
    }

    fn end_transformation(&mut self) {
        self.layers.pop_transformation();
    }

    fn fill_quad(&mut self, quad: core::renderer::Quad, background: impl Into<Background>) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_quad(quad, background.into(), transformation);
    }

    fn allocate_image(
        &mut self,
        _handle: &core::image::Handle,
        _callback: impl FnOnce(Result<core::image::Allocation, core::image::Error>) + Send + 'static,
    ) {
        #[cfg(feature = "image")]
        self.image_cache
            .get_mut()
            .allocate_image(_handle, _callback);
    }

    fn hint(&mut self, scale_factor: renderer::Scale) {
        self.scale = Some(scale_factor);
    }

    fn scale(&self) -> Option<renderer::Scale> {
        let scale_factor = self.scale?;

        Some(renderer::Scale {
            application: scale_factor.application * self.layers.transformation().scale_factor(),
            ..scale_factor
        })
    }

    fn tick(&mut self) {
        #[cfg(feature = "image")]
        self.image_cache.get_mut().receive();
    }

    fn reset(&mut self, new_bounds: Rectangle) {
        self.layers.reset(new_bounds);
        self.opacity_groups.clear();
        self.open_opacity_groups.clear();
        self.next_layer_index = 1;
    }

    fn settings(&self) -> renderer::Settings {
        self.settings
    }
}

impl core::text::Renderer for Renderer {
    type Font = Font;
    type Paragraph = Paragraph;
    type Editor = Editor;

    const ICON_FONT: Font = Font::new("Iced-Icons");
    const CHECKMARK_ICON: char = '\u{f00c}';
    const ARROW_DOWN_ICON: char = '\u{e800}';
    const ICED_LOGO: char = '\u{e801}';
    const SCROLL_UP_ICON: char = '\u{e802}';
    const SCROLL_DOWN_ICON: char = '\u{e803}';
    const SCROLL_LEFT_ICON: char = '\u{e804}';
    const SCROLL_RIGHT_ICON: char = '\u{e805}';

    fn default_font(&self) -> Self::Font {
        self.settings.default_font
    }

    fn default_size(&self) -> Pixels {
        self.settings.default_text_size
    }

    fn fill_paragraph(
        &mut self,
        text: &Self::Paragraph,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();

        layer.draw_paragraph(text, position, color, clip_bounds, transformation);
    }

    fn fill_editor(
        &mut self,
        editor: &Self::Editor,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_editor(editor, position, color, clip_bounds, transformation);
    }

    fn fill_text(
        &mut self,
        text: core::Text,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text(text, position, color, clip_bounds, transformation);
    }
}

impl graphics::text::Renderer for Renderer {
    fn fill_raw(&mut self, raw: graphics::text::Raw) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text_raw(raw, transformation);
    }
}

#[cfg(feature = "image")]
impl core::image::Renderer for Renderer {
    type Handle = core::image::Handle;

    fn load_image(
        &self,
        handle: &Self::Handle,
    ) -> Result<core::image::Allocation, core::image::Error> {
        self.image_cache
            .borrow_mut()
            .load_image(&self.engine.device, &self.engine.queue, handle)
    }

    fn measure_image(&self, handle: &Self::Handle) -> Option<core::Size<u32>> {
        self.image_cache.borrow_mut().measure_image(handle)
    }

    fn draw_image(&mut self, image: core::Image, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_raster(image, bounds, clip_bounds, transformation);
    }
}

#[cfg(feature = "svg")]
impl core::svg::Renderer for Renderer {
    fn measure_svg(&self, handle: &core::svg::Handle) -> core::Size<u32> {
        self.image_cache.borrow_mut().measure_svg(handle)
    }

    fn draw_svg(&mut self, svg: core::Svg, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_svg(svg, bounds, clip_bounds, transformation);
    }
}

impl graphics::mesh::Renderer for Renderer {
    fn draw_mesh(&mut self, mesh: graphics::Mesh) {
        debug_assert!(
            !mesh.indices().is_empty(),
            "Mesh must not have empty indices"
        );

        debug_assert!(
            mesh.indices().len().is_multiple_of(3),
            "Mesh indices length must be a multiple of 3"
        );

        let (layer, transformation) = self.layers.current_mut();
        layer.draw_mesh(mesh, transformation);
    }

    fn draw_mesh_cache(&mut self, cache: mesh::Cache) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_mesh_cache(cache, transformation);
    }
}

#[cfg(feature = "geometry")]
impl graphics::geometry::Renderer for Renderer {
    type Geometry = Geometry;
    type Frame = geometry::Frame;

    fn new_frame(&self, bounds: Rectangle) -> Self::Frame {
        geometry::Frame::new(bounds)
    }

    fn draw_geometry(&mut self, geometry: Self::Geometry) {
        let (layer, transformation) = self.layers.current_mut();

        match geometry {
            Geometry::Live {
                meshes,
                images,
                text,
            } => {
                layer.draw_mesh_group(meshes, transformation);

                for image in images {
                    layer.draw_image(image, transformation);
                }

                layer.draw_text_group(text, transformation);
            }
            Geometry::Cached(cache) => {
                if let Some(meshes) = cache.meshes {
                    layer.draw_mesh_cache(meshes, transformation);
                }

                if let Some(images) = cache.images {
                    for image in images.iter().cloned() {
                        layer.draw_image(image, transformation);
                    }
                }

                if let Some(text) = cache.text {
                    layer.draw_text_cache(text, transformation);
                }
            }
        }
    }
}

impl primitive::Renderer for Renderer {
    fn draw_primitive(&mut self, bounds: Rectangle, primitive: impl Primitive) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_primitive(bounds, primitive, transformation);
    }
}

impl graphics::compositor::Default for crate::Renderer {
    type Compositor = window::Compositor;
}

impl renderer::Headless for Renderer {
    async fn new(settings: renderer::Settings, backend: Option<&str>) -> Option<Self> {
        if backend.is_some_and(|backend| backend != "wgpu") {
            return None;
        }

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
            flags: wgpu::InstanceFlags::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .ok()?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("iced_wgpu [headless]"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    max_bind_groups: 2,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .ok()?;

        let engine = Engine::new(
            &adapter,
            device,
            queue,
            if graphics::color::GAMMA_CORRECTION {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
            Some(graphics::Antialiasing::MSAAx4),
            Shell::headless(),
        );

        Some(Self::new(engine, settings))
    }

    fn name(&self) -> String {
        "wgpu".to_owned()
    }

    fn screenshot(
        &mut self,
        size: Size<u32>,
        scale_factor: f32,
        background_color: Color,
    ) -> Vec<u8> {
        self.screenshot(
            &Viewport::with_physical_size(
                size,
                renderer::Scale {
                    window: 1.0,
                    application: scale_factor,
                },
            ),
            background_color,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Renderer as _;
    use crate::core::renderer;

    #[test]
    fn opacity_group_isolates_overlapping_primitives() {
        let Some(mut renderer) = futures::executor::block_on(
            <Renderer as renderer::Headless>::new(renderer::Settings::default(), Some("wgpu")),
        ) else {
            // A machine without a headless adapter cannot exercise pixel output.
            return;
        };

        let size = Size::new(8, 4);
        renderer.reset(Rectangle::with_size(Size::new(8.0, 4.0)));
        renderer.start_opacity_group(Rectangle::with_size(Size::new(8.0, 4.0)), 0.5);
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(Point::new(0.0, 0.0), Size::new(6.0, 4.0)),
                ..renderer::Quad::default()
            },
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(Point::new(2.0, 0.0), Size::new(6.0, 4.0)),
                ..renderer::Quad::default()
            },
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.end_opacity_group();

        let pixels =
            <Renderer as renderer::Headless>::screenshot(&mut renderer, size, 1.0, Color::WHITE);
        let pixel = |x: usize, y: usize| &pixels[(y * 8 + x) * 4..(y * 8 + x + 1) * 4];
        let single = pixel(1, 1);
        let overlap = pixel(3, 1);

        assert_eq!(
            single, overlap,
            "group opacity must be applied after overlap composition"
        );
        assert!(
            single[0] > 245 && (175..=200).contains(&single[1]),
            "{single:?}"
        );
        assert_eq!(single[1], single[2]);
    }

    #[test]
    fn affine_group_rotates_primitives_as_one_composited_subtree() {
        let Some(mut renderer) = futures::executor::block_on(
            <Renderer as renderer::Headless>::new(renderer::Settings::default(), Some("wgpu")),
        ) else {
            return;
        };

        let size = Size::new(8, 6);
        renderer.reset(Rectangle::with_size(Size::new(8.0, 6.0)));
        // 90 degree clockwise screen-space rotation: (x, y) -> (8 - y, x).
        renderer.start_affine_group(
            Rectangle::with_size(Size::new(8.0, 6.0)),
            [0.0, 1.0, -1.0, 0.0, 8.0, 0.0],
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(Point::new(1.0, 1.0), Size::new(3.0, 1.0)),
                ..renderer::Quad::default()
            },
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.end_affine_group();

        let pixels =
            <Renderer as renderer::Headless>::screenshot(&mut renderer, size, 1.0, Color::WHITE);
        let pixel = |x: usize, y: usize| &pixels[(y * 8 + x) * 4..(y * 8 + x + 1) * 4];
        assert!(
            pixel(6, 2)[0] > 245 && pixel(6, 2)[1] < 10,
            "{:?}",
            pixel(6, 2)
        );
        assert!(pixel(2, 1)[1] > 245, "source position should be cleared");
        assert!(pixel(5, 2)[1] > 245, "rotation width must remain one pixel");
    }
}
