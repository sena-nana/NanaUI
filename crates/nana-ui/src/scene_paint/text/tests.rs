//! Unit tests for `scene_paint::text`.

use super::*;
use nana_ui_scene::SceneTextEffects;

const SHAPE_CACHE_CAP: usize = 512;

struct TextPipeline {
    inner: super::TextPipeline,
    target: TextPipelineTarget,
}

impl std::ops::Deref for TextPipeline {
    type Target = super::TextPipeline;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for TextPipeline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl TextPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let inner = super::TextPipeline::new(device, queue, format);
        let target = inner.new_target(device);
        Self { inner, target }
    }

    fn with_atlas_limits(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        limits: GlyphAtlasLimits,
    ) -> Self {
        let inner = super::TextPipeline::with_atlas_limits(device, format, limits);
        let target = inner.new_target(device);
        Self { inner, target }
    }

    fn begin_frame(&mut self, physical_size: [u32; 2]) {
        self.inner.begin_frame(&mut self.target, physical_size);
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        bounds: LogicalRect,
        clip: LogicalRect,
        scale_factor: f32,
        content: &str,
        color: Option<[f32; 4]>,
        size: f32,
        weight: Option<u16>,
        family: Option<&str>,
        line_height: Option<LineHeightSpec>,
        wrap: bool,
        wrap_break: nana_ui_core::TextWrapBreak,
        italic: bool,
        ellipsis: bool,
        max_lines: Option<u16>,
        shaping: TextShaping,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
        spans: &[SceneTextSpan],
        letter_spacing: f32,
        font_features: &[nana_ui_core::FontFeatureSetting],
        opentype: &SceneTextOpenType,
        affine: [f32; 6],
        persp: [f32; 2],
        fragment_clip: clip::FragmentClip,
        opacity: f32,
        paint_offset: [f32; 2],
        entry_key: EntryKey,
        revision: u64,
        text_layout: Option<&nana_ui_runtime::RetainedTextLayout>,
        opaque_backdrop: bool,
    ) -> Option<PreparedText> {
        self.inner.prepare(
            &mut self.target,
            device,
            bounds,
            clip,
            scale_factor,
            content,
            color,
            size,
            weight,
            family,
            line_height,
            wrap,
            wrap_break,
            italic,
            ellipsis,
            max_lines,
            shaping,
            horizontal,
            vertical,
            spans,
            letter_spacing,
            font_features,
            opentype,
            affine,
            persp,
            fragment_clip,
            opacity,
            paint_offset,
            entry_key,
            revision,
            text_layout,
            opaque_backdrop,
        )
    }

    fn flush_runs(&mut self) {
        self.inner.flush_runs(&mut self.target);
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.inner.upload(&mut self.target, device, queue, work);
    }

    fn upload_with(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.inner
            .upload_with(&mut self.target, device, queue, encoder, work);
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.inner
            .draw(&self.target, pass, prepared, scissor, None, gpu_work);
    }

    fn can_merge_runs(&self, previous: &PreparedText, next: &PreparedText) -> bool {
        self.target.can_merge_runs(previous, next)
    }

    fn merge_runs(&mut self, previous: &PreparedText, next: &PreparedText) {
        self.target.merge_runs(previous, next);
    }

    fn glyph_counters(&self) -> TextGlyphCounters {
        self.inner
            .glyph_counters_across(std::iter::once(&self.target))
    }

    fn drop_every_entry(&mut self) {
        super::TextPipeline::drop_every_entry(&mut self.inner.atlas, &mut self.target);
    }

    fn order_settle_due(&self) -> bool {
        self.target.order_settle_due()
    }

    fn write_motion(
        &mut self,
        queue: &wgpu::Queue,
        now: std::time::Duration,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.inner.write_motion(&mut self.target, queue, now, work);
    }

    fn swap_test_target(&mut self, target: &mut Option<TextPipelineTarget>, device: &wgpu::Device) {
        let target = target.get_or_insert_with(|| self.inner.new_target(device));
        std::mem::swap(&mut self.target, target);
    }
}

#[test]
fn ellipsis_paint_keeps_exact_fit_and_truncates_only_narrow_boxes() {
    use nana_ui_runtime::{
        ComputedStyle, StableNodeId, TextContent, TextShapeConstraints, TextShaper,
    };

    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    pipeline.begin_frame([512, 64]);
    let mut shaper = crate::NanaTextShaper::default();
    let style = ComputedStyle::with(|s| {
        s.font_size = 12.0;
        s.font_weight = Some(600);
    });
    for (content, shaping) in [
        ("未命名 1", TextShaping::Advanced),
        ("shade", TextShaping::Advanced),
        ("Cost $0.30", TextShaping::Auto),
        ("Save memory", TextShaping::Auto),
    ] {
        let natural = shaper
            .shape(
                StableNodeId::new(1).unwrap(),
                &TextContent {
                    value: content.into(),
                },
                &style,
                TextShapeConstraints {
                    wrap: false,
                    shaping,
                    ..TextShapeConstraints::default()
                },
            )
            .width;
        // Revisit the exact-fit entry after inserting a truncated one.
        let mut full_glyphs = Vec::new();
        for (width, ellipsis) in [
            (natural, false),
            (natural, true),
            (natural * 0.5, true),
            (natural, true),
        ] {
            pipeline
                .prepare(
                    &device,
                    LogicalRect::from_xywh(0.0, 0.0, width, 24.0),
                    LogicalRect::from_xywh(0.0, 0.0, 512.0, 64.0),
                    1.0,
                    content,
                    Some([1.0; 4]),
                    12.0,
                    Some(600),
                    None,
                    None,
                    false,
                    nana_ui_core::TextWrapBreak::Word,
                    false,
                    ellipsis,
                    None,
                    shaping,
                    TextHorizontalAlignment::Start,
                    TextVerticalAlignment::Top,
                    &[],
                    0.0,
                    &[],
                    &SceneTextOpenType::default(),
                    clip::IDENTITY_AFFINE,
                    [0.0; 2],
                    clip::FragmentClip::PASS,
                    1.0,
                    [0.0; 2],
                    EntryKey {
                        node: 1,
                        slot: 0,
                        pass: 0,
                    },
                    UNTRACKED_REVISION,
                    None,
                    true,
                )
                .expect("label must prepare");
            let layout = pipeline.lay_out(
                content,
                None,
                Some(600),
                12.0,
                12.0 * 1.2,
                0.0,
                false,
                false,
                nana_ui_core::TextWrapBreak::Word,
                ellipsis,
                None,
                width,
                24.0,
                TextAlignSpec::Start,
                &SceneTextOpenType::default(),
            );
            let painted_width = measure(&layout).0;
            let painted_glyphs: Vec<_> = layout
                .runs
                .iter()
                .flat_map(|run| {
                    run.glyphs
                        .iter()
                        .map(|glyph| (run.font, glyph.glyph_id, glyph.cluster))
                })
                .collect();
            if !ellipsis {
                assert!(!painted_glyphs.is_empty());
                full_glyphs = painted_glyphs;
                continue;
            }
            if width == natural {
                assert_eq!(
                    painted_glyphs, full_glyphs,
                    "exact-fit paint must retain every original glyph"
                );
                assert!(
                    (painted_width - natural).abs() < 0.01,
                    "exact-fit paint changed {content:?}: {painted_width} vs {natural}"
                );
            } else {
                assert_ne!(
                    painted_glyphs, full_glyphs,
                    "narrow paint must replace the text tail"
                );
                assert!(
                    painted_width <= width + 0.5 && painted_width < natural,
                    "narrow paint must truncate {content:?}"
                );
            }
        }
    }
}

/// An animated font axis interns a new coordinate set every frame. Past
/// the ceiling the coldest sets go, one by one: the glyph caches of every
/// other text stay, a set in use keeps its id, and no id is reissued.
#[test]
fn variation_ids_are_bounded_by_evicting_the_coldest_sets() {
    use raster::VARIATION_CAP;

    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    pipeline.begin_frame([64, 64]);
    let Some(face) = ({
        let engine = crate::text_engine::lock_engine(&pipeline.engine);
        engine.fonts().faces().into_iter().next()
    }) else {
        return;
    };
    let intern = |pipeline: &mut TextPipeline, value: usize| {
        pipeline
            .rasterizer
            .intern_instance(&nana_text::font::FontInstanceKey {
                font: face,
                coords: vec![nana_text::font::AxisCoord {
                    tag: *b"wdth",
                    value: value as f32,
                }]
                .into(),
                synthesis: nana_text::font::Synthesis::default(),
            })
            .1
    };
    let first = intern(&mut pipeline, 0);
    for value in 1..VARIATION_CAP {
        intern(&mut pipeline, value);
    }
    let epoch = pipeline.raster.generation();
    for _ in 0..4 {
        pipeline.begin_frame([64, 64]);
    }
    // The first set stays warm; everything else from that frame is cold.
    assert_eq!(intern(&mut pipeline, 0), first);
    let newest = intern(&mut pipeline, VARIATION_CAP);
    pipeline.begin_frame([64, 64]);
    assert!(pipeline.rasterizer.variation_count() <= VARIATION_CAP);
    assert_eq!(
        pipeline.raster.generation(),
        epoch,
        "no glyph cache starts over"
    );
    assert_eq!(intern(&mut pipeline, 0), first, "a set in use keeps its id");
    assert_eq!(intern(&mut pipeline, VARIATION_CAP), newest);
    let again = intern(&mut pipeline, 1);
    assert!(
        again.0 > newest.0,
        "an evicted set comes back under a new id"
    );
}

#[test]
fn rtl_latin_in_wide_box_places_first_glyph_on_the_right() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    pipeline.begin_frame([256, 64]);
    let bounds = LogicalRect::from_xywh(0.0, 0.0, 200.0, 24.0);
    let clip = LogicalRect::from_xywh(0.0, 0.0, 200.0, 24.0);
    let opentype = SceneTextOpenType {
        direction: nana_ui_core::DirSpec::Rtl,
        ..SceneTextOpenType::default()
    };
    pipeline
        .prepare(
            &device,
            bounds,
            clip,
            1.0,
            "Hello",
            Some([1.0, 1.0, 1.0, 1.0]),
            16.0,
            None,
            None,
            None,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Advanced,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &opentype,
            clip::IDENTITY_AFFINE,
            [0.0, 0.0],
            clip::FragmentClip::PASS,
            1.0,
            [0.0, 0.0],
            EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            },
            UNTRACKED_REVISION,
            None,
            true,
        )
        .expect("rtl latin must prepare");
    let layout = pipeline.lay_out(
        "Hello",
        None,
        None,
        16.0,
        16.0 * 1.2,
        0.0,
        false,
        false,
        nana_ui_core::TextWrapBreak::Word,
        false,
        None,
        200.0,
        24.0,
        TextAlignSpec::Start,
        &opentype,
    );
    let glyph_x = layout
        .runs
        .iter()
        .flat_map(|run| run.glyph_cells())
        .map(|(left, _)| left)
        .fold(f32::INFINITY, f32::min);
    assert!(
        glyph_x.is_finite(),
        "rtl latin must lay out a content glyph"
    );
    assert!(
        glyph_x > 100.0,
        "first Latin glyph must sit on the right of a 200px RTL box, got {glyph_x}"
    );
}

#[test]
fn text_box_origin_keeps_left_edge_and_centers_vertically() {
    let bounds = LogicalRect::from_xywh(10.0, 20.0, 100.0, 40.0);
    assert_eq!(
        text_box_origin(bounds, TextVerticalAlignment::Top, 12.0),
        [10.0, 20.0]
    );
    assert_eq!(
        text_box_origin(bounds, TextVerticalAlignment::Center, 12.0),
        [10.0, 34.0]
    );
    assert_eq!(
        text_box_origin(bounds, TextVerticalAlignment::Bottom, 12.0),
        [10.0, 48.0]
    );
}

#[test]
fn perspective_text_foreshortens_the_far_edge() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut pipeline = TextPipeline::new(&device, &queue, format);
    let mat = nana_ui_core::PaintMat4::perspective(120.0)
        .expect("finite depth")
        .then(nana_ui_core::PaintMat4::rotate_y(45_f32.to_radians()))
        .around_origin(0.0, 0.0, 64.0, 64.0);
    let (affine, persp) = mat.planar_homography().expect("homography");
    let pixels = paint_block(
        &device,
        &queue,
        &mut pipeline,
        LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
        LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
        affine,
        persp,
        clip::FragmentClip::PASS,
    );
    let ink = ink_aabb(&pixels, 64, 64).expect("perspective text must paint");
    let column_height = |x: u32| {
        let mut top = None;
        let mut bottom = 0;
        for y in 0..64u32 {
            if inked(pixel(&pixels, 64, x, y)) {
                top.get_or_insert(y);
                bottom = y;
            }
        }
        top.map(|top| bottom - top + 1)
    };
    // The tallest column in each outer quarter of the ink: the outermost
    // columns alone are the edge's one-pixel ramp.
    let quarter = (ink.2 - ink.0) / 4;
    let near = (ink.0..=ink.0 + quarter)
        .filter_map(column_height)
        .max()
        .expect("a near column");
    let far = (ink.2 - quarter..=ink.2)
        .filter_map(column_height)
        .max()
        .expect("a far column");
    assert_ne!(
        near, far,
        "a perspective homography must foreshorten one edge of the text, \
             near={near} far={far}"
    );
}

#[test]
fn rotated_text_prepare_vertices_are_not_the_unrotated_aabb() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut pipeline = TextPipeline::new(&device, &queue, format);
    let bounds = LogicalRect::from_xywh(8.0, 16.0, 48.0, 20.0);
    let clip = LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0);
    let identity = clip::IDENTITY_AFFINE;
    // x' = -y + 40, y' = x keeps a 90° rotation on-screen.
    let rot90 = [0.0, 1.0, -1.0, 0.0, 40.0, 0.0];
    let identity_pixels = paint_text(
        &device,
        &queue,
        &mut pipeline,
        bounds,
        clip,
        identity,
        [0.0, 0.0],
        clip::FragmentClip::PASS,
    );
    let rotated_pixels = paint_text(
        &device,
        &queue,
        &mut pipeline,
        bounds,
        clip,
        rot90,
        [0.0, 0.0],
        clip::FragmentClip::PASS,
    );
    let identity_ink = ink_aabb(&identity_pixels, 64, 64).expect("unrotated text must paint");
    let rotated_ink = ink_aabb(&rotated_pixels, 64, 64).expect("rotated text must paint");
    assert_ne!(
        identity_ink, rotated_ink,
        "rotated glyphs must not occupy the unrotated AABB"
    );
    let identity_w = identity_ink.2 - identity_ink.0 + 1;
    let identity_h = identity_ink.3 - identity_ink.1 + 1;
    let rotated_w = rotated_ink.2 - rotated_ink.0 + 1;
    let rotated_h = rotated_ink.3 - rotated_ink.1 + 1;
    assert!(
        identity_w > identity_h,
        "unrotated 'Hi' ink must be wide, got {identity_ink:?}"
    );
    assert!(
        rotated_h > rotated_w,
        "90° glyph quads must paint a tall AABB, not a translated wide run, got {rotated_ink:?}"
    );
}

#[test]
fn rotated_clip_discards_affine_glyph_in_aabb_outside_rect() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut pipeline = TextPipeline::new(&device, &queue, format);
    let bounds = LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let clips = [nana_ui_scene::ClipRegion {
        bounds: nana_ui_scene::SceneRect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        },
        transform: nana_ui_scene::AffineTransform::from_matrix(
            nana_ui_core::PaintTransform {
                a: k,
                b: k,
                c: -k,
                d: k,
                ..nana_ui_core::PaintTransform::default()
            }
            .around_center(16.0, 16.0, 32.0, 32.0),
        ),
        corner_radius: 0.0,
        polygon_clip: None,
    }];
    let origin = clip::PaintOrigin::from(clip::paint_origin([0.0, 0.0], [0.0, 0.0]));
    let aabb = clip::intersect_clips(
        LogicalRect::viewport([0.0, 0.0], [64.0, 64.0]),
        &clips,
        origin,
    )
    .unwrap();
    let frag = clip::fragment_clip(&clips, origin);
    let unclipped = paint_block(
        &device,
        &queue,
        &mut pipeline,
        bounds,
        LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
        clip::IDENTITY_AFFINE,
        [0.0, 0.0],
        clip::FragmentClip::PASS,
    );
    let clipped = paint_block(
        &device,
        &queue,
        &mut pipeline,
        bounds,
        aabb,
        clip::IDENTITY_AFFINE,
        [0.0, 0.0],
        frag,
    );
    let mut probe = None;
    for y in 0..64u32 {
        for x in 0..64u32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if px >= aabb.x
                && py >= aabb.y
                && px < aabb.x + aabb.width
                && py < aabb.y + aabb.height
                && !clip::point_in_fragment_clip(px, py, frag)
                && inked(pixel(&unclipped, 64, x, y))
            {
                probe = Some((x, y));
                break;
            }
        }
        if probe.is_some() {
            break;
        }
    }
    let (probe_x, probe_y) =
        probe.expect("unclipped glyphs must ink a pixel in AABB-outside-rotated-rect");
    let probe_clipped = pixel(&clipped, 64, probe_x, probe_y);
    let mut inside = false;
    for y in 0..64u32 {
        for x in 0..64u32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if clip::point_in_fragment_clip(px, py, frag) && inked(pixel(&clipped, 64, x, y)) {
                inside = true;
                break;
            }
        }
        if inside {
            break;
        }
    }
    assert!(
        !inked(probe_clipped),
        "affine glyphs must discard AABB-outside-rotated-rect, pixel ({probe_x},{probe_y})={probe_clipped:?}"
    );
    assert!(inside, "rotated clip interior must still paint the glyph");
}

/// `white-space: normal` folds an authored newline into a space. Runtime
/// measures that way, so the painter has to lay it out that way: a
/// paragraph measured as one line and painted as two overflows its box.
#[test]
fn an_authored_newline_only_breaks_the_line_when_the_scene_says_it_does() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let laid_out = |pipeline: &mut TextPipeline, preserve_lines: bool| {
        let layout = pipeline.lay_out(
            "first\nsecond",
            None,
            None,
            16.0,
            20.0,
            0.0,
            false,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            None,
            400.0,
            64.0,
            TextAlignSpec::Start,
            &SceneTextOpenType {
                preserve_lines,
                ..SceneTextOpenType::default()
            },
        );
        layout.lines.len()
    };
    assert_eq!(
        laid_out(&mut pipeline, false),
        1,
        "`white-space: normal` folds the newline into a space"
    );
    assert_eq!(
        laid_out(&mut pipeline, true),
        2,
        "a preserved newline is a line break"
    );
}

/// Lay a paragraph out through the engine the way Runtime would, and hand
/// it back as the scene's retained handle.
fn runtime_layout(
    content: &str,
    box_width: f32,
    align: TextAlignSpec,
) -> nana_ui_runtime::RetainedTextLayout {
    let source = nana_text::TextSource::new(content);
    let style = nana_text::TextStyle {
        font_size_px: 16.0,
        line_height: Some(LineHeightSpec::Absolute(20.0)),
        ..nana_text::TextStyle::default()
    };
    let constraints = nana_text::TextConstraints {
        max_width_px: Some(box_width),
        align,
        preserve_lines: true,
        ..nana_text::TextConstraints::default()
    };
    let engine = crate::text_engine::nana_text_engine();
    let mut engine = crate::text_engine::lock_engine(&engine);
    let mut counters = nana_text::TextWorkCounters::default();
    let layout = nana_text::TextEngine::layout(
        &mut *engine,
        nana_text::TextKind::Label,
        &source,
        &style,
        &constraints,
        &mut counters,
    );
    nana_ui_runtime::RetainedTextLayout {
        id: layout.id,
        layout,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_with_layout(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    content: &str,
    box_width: f32,
    handle: Option<&nana_ui_runtime::RetainedTextLayout>,
) {
    pipeline.begin_frame([512, 64]);
    pipeline.prepare(
        device,
        LogicalRect::from_xywh(0.0, 0.0, box_width, 32.0),
        LogicalRect::from_xywh(0.0, 0.0, 512.0, 64.0),
        1.0,
        content,
        Some([1.0, 1.0, 1.0, 1.0]),
        16.0,
        None,
        None,
        Some(LineHeightSpec::Absolute(20.0)),
        false,
        nana_ui_core::TextWrapBreak::Word,
        false,
        false,
        None,
        TextShaping::Auto,
        TextHorizontalAlignment::Start,
        TextVerticalAlignment::Top,
        &[],
        0.0,
        &[],
        &SceneTextOpenType {
            preserve_lines: true,
            ..SceneTextOpenType::default()
        },
        clip::IDENTITY_AFFINE,
        [0.0; 2],
        clip::FragmentClip::PASS,
        1.0,
        [0.0; 2],
        EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        },
        UNTRACKED_REVISION,
        handle,
        true,
    );
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
}

/// The point of the handle: the paragraph Runtime measured is the one that
/// is drawn. Nothing is laid out here, and no string is hashed to find
/// that out.
#[test]
fn a_paragraph_the_runtime_retains_is_drawn_without_laying_it_out_again() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let handle = runtime_layout("Retained paragraph", 240.0, TextAlignSpec::Start);
    prepare_with_layout(
        &device,
        &queue,
        &mut pipeline,
        "Retained paragraph",
        240.0,
        Some(&handle),
    );
    let drawn = pipeline.glyph_counters();
    assert!(
        drawn.text_retained_layouts_drawn >= 1,
        "the handle is what identified the paragraph: {drawn:?}"
    );
    assert!(
        drawn.glyph_resolve_requests > 0,
        "and its glyphs were resolved from it"
    );
    let (hits, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        (hits, misses),
        (0, 0),
        "the painter's own paragraph cache is never consulted"
    );
}

/// The handle is only usable when the box it was measured into is the box
/// being painted: a centred line inside a wider box would otherwise be
/// drawn off-centre. A mismatch falls back rather than misplacing text.
#[test]
fn a_layout_measured_into_a_different_box_is_not_drawn_from_its_handle() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let handle = runtime_layout("Centred", 240.0, TextAlignSpec::Center);
    prepare_with_layout(
        &device,
        &queue,
        &mut pipeline,
        "Centred",
        180.0,
        Some(&handle),
    );
    let drawn = pipeline.glyph_counters();
    assert_eq!(
        drawn.text_retained_layouts_drawn, 0,
        "the alignment box is not the paint box, so the handle is refused"
    );
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(misses, 1, "and the painter laid the paragraph out itself");
}

/// Paint one rich-text label with the given spans, under a fixed revision.
fn paint_rich(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    spans: &[SceneTextSpan],
) {
    pipeline.begin_frame([256, 64]);
    pipeline.prepare(
        device,
        LogicalRect::from_xywh(0.0, 0.0, 240.0, 32.0),
        LogicalRect::from_xywh(0.0, 0.0, 256.0, 64.0),
        1.0,
        "warm and cold",
        Some([1.0, 1.0, 1.0, 1.0]),
        16.0,
        None,
        None,
        None,
        false,
        nana_ui_core::TextWrapBreak::Word,
        false,
        false,
        None,
        TextShaping::Auto,
        TextHorizontalAlignment::Start,
        TextVerticalAlignment::Top,
        spans,
        0.0,
        &[],
        &SceneTextOpenType::default(),
        clip::IDENTITY_AFFINE,
        [0.0; 2],
        clip::FragmentClip::PASS,
        1.0,
        [0.0; 2],
        EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        },
        UNTRACKED_REVISION,
        None,
        true,
    );
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
}

/// A rich span's colour is not part of the layout any more, so the layout
/// hash cannot tell two paints of one string apart. The entry has to.
#[test]
fn repainting_a_span_in_a_new_color_rebuilds_that_entry_without_relayout() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let span = |color: [f32; 4]| {
        vec![SceneTextSpan {
            start: 0,
            end: "warm".len(),
            color: nana_ui_core::PaintColor::srgb(color),
        }]
    };
    paint_rich(&device, &queue, &mut pipeline, &span([1.0, 0.0, 0.0, 1.0]));
    let warm = pipeline.glyph_counters();
    let (_, misses_before, _) = pipeline.shape_cache_stats();
    paint_rich(&device, &queue, &mut pipeline, &span([0.0, 0.0, 1.0, 1.0]));
    let recolored = pipeline.glyph_counters();
    let (_, misses_after, _) = pipeline.shape_cache_stats();
    assert_eq!(
        recolored.text_instance_rebuilds - warm.text_instance_rebuilds,
        1,
        "the span paints a different colour, so its instances are resolved again"
    );
    assert_eq!(
        misses_after, misses_before,
        "but colour is not part of the layout, so nothing is laid out again"
    );
    assert_eq!(
        recolored.glyph_rasterized, warm.glyph_rasterized,
        "nor rasterized: the bitmaps never depended on the colour"
    );

    // And repainting the same spans is still a steady frame.
    paint_rich(&device, &queue, &mut pipeline, &span([0.0, 0.0, 1.0, 1.0]));
    let steady = pipeline.glyph_counters();
    assert_eq!(
        steady.text_instance_rebuilds, recolored.text_instance_rebuilds,
        "an unchanged rich repaint resolves nothing"
    );
}

/// Paint "warm and cold" under `effects`, as the scene painter does.
fn paint_effects(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    effects: Option<&Arc<SceneRichPaint>>,
    shadow: Option<nana_ui_core::TextShadowSpec>,
) {
    pipeline.set_effects(effects, shadow);
    paint_rich(device, queue, pipeline, &[]);
}

#[test]
fn material_parameter_changes_reuse_layout_and_atlas() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let material = nana_ui_core::MaterialRef::new(nana_ui_core::MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return i.color * material_param(0u);}",
    ));
    let first = effects(SceneTextEffects {
        material: Some(material.clone().with_parameters(Arc::from([[1.; 4]]))),
        ..Default::default()
    });
    paint_effects(&device, &queue, &mut pipeline, first.as_ref(), None);
    let warm = pipeline.glyph_counters();
    let layouts = pipeline.shape_cache_stats();
    let next = effects(SceneTextEffects {
        material: Some(material.with_parameters(Arc::from([[0.5; 4]]))),
        ..Default::default()
    });
    paint_effects(&device, &queue, &mut pipeline, next.as_ref(), None);
    let changed = pipeline.glyph_counters();
    assert_eq!(
        pipeline.shape_cache_stats().1,
        layouts.1,
        "parameters changed shaping"
    );
    assert_eq!(changed.glyph_rasterized, warm.glyph_rasterized);
    assert_eq!(changed.glyph_upload_bytes, warm.glyph_upload_bytes);
    paint_effects(&device, &queue, &mut pipeline, next.as_ref(), None);
    let steady = pipeline.glyph_counters();
    assert_eq!(
        steady.text_instance_upload_bytes, changed.text_instance_upload_bytes,
        "static material uploads glyphs"
    );
    assert_eq!(steady.glyph_upload_bytes, changed.glyph_upload_bytes);
}

#[test]
fn transition_updates_reuse_instances_layout_and_atlas() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    paint_rich(&device, &queue, &mut pipeline, &[]);
    let warm = pipeline.glyph_counters();
    let layouts = pipeline.shape_cache_stats();
    let mask = nana_ui_core::MaterialRef::new(nana_ui_core::MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return vec4<f32>(i.color.rgb,1.-i.progress);}",
    ));
    let mut p = presentation(vec![], vec![], None);
    Arc::make_mut(&mut p).transitions.exit = Some(nana_ui_core::PresentationMask {
        material: mask,
        space: nana_ui_core::PresentationMaskSpace::Target,
        start: std::time::Duration::ZERO,
        duration: std::time::Duration::from_secs(1),
    });
    for now in [0, 100, 300, 700, 1000] {
        pipeline.set_glyph_presentation(Some(&p));
        pipeline.write_motion(&queue, std::time::Duration::from_millis(now), None);
        paint_rich(&device, &queue, &mut pipeline, &[]);
    }
    let after = pipeline.glyph_counters();
    assert_eq!(pipeline.shape_cache_stats().1, layouts.1);
    assert_eq!(after.glyph_rasterized, warm.glyph_rasterized);
    assert_eq!(after.glyph_upload_bytes, warm.glyph_upload_bytes);
    assert_eq!(after.text_instance_rebuilds, warm.text_instance_rebuilds);
}

fn effects(base: SceneTextEffects) -> Option<Arc<SceneRichPaint>> {
    SceneRichPaint::new(base, Vec::new())
}

fn outline(width: f32, color: [f32; 4]) -> SceneTextEffects {
    SceneTextEffects {
        stroke: Some(nana_ui_core::RichTextStroke::new(
            width,
            nana_ui_core::PaintColor::srgb(color),
        )),
        decoration: nana_ui_core::TextDecorationLine {
            underline: true,
            line_through: false,
            overline: false,
        },
        ..SceneTextEffects::default()
    }
}

/// The paint tier of rich text (outline, decoration, shadow) is instances
/// of the same paragraph: changing it rebuilds them and lays nothing out;
/// changing only a colour rasterizes nothing new either.
#[test]
fn repainting_an_outline_rebuilds_instances_without_relayout() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let black = effects(outline(2.0, [0.0, 0.0, 0.0, 1.0]));
    paint_effects(&device, &queue, &mut pipeline, black.as_ref(), None);
    let first = pipeline.glyph_counters();
    let (_, misses, _) = pipeline.shape_cache_stats();

    let white = effects(outline(2.0, [1.0, 1.0, 1.0, 1.0]));
    paint_effects(&device, &queue, &mut pipeline, white.as_ref(), None);
    let recolored = pipeline.glyph_counters();
    assert_eq!(
        recolored.text_instance_rebuilds - first.text_instance_rebuilds,
        1,
        "a new outline colour resolves the entry again"
    );
    assert_eq!(
        recolored.glyph_rasterized, first.glyph_rasterized,
        "the outline bitmaps never depended on its colour"
    );
    assert_eq!(
        pipeline.shape_cache_stats().1,
        misses,
        "nothing is laid out again"
    );

    let wide = effects(outline(4.0, [1.0, 1.0, 1.0, 1.0]));
    paint_effects(&device, &queue, &mut pipeline, wide.as_ref(), None);
    let widened = pipeline.glyph_counters();
    assert!(
        widened.glyph_rasterized > recolored.glyph_rasterized,
        "a wider outline is new bitmaps"
    );
    assert_eq!(
        pipeline.shape_cache_stats().1,
        misses,
        "and still no relayout"
    );

    paint_effects(&device, &queue, &mut pipeline, wide.as_ref(), None);
    let steady = pipeline.glyph_counters();
    assert_eq!(
        steady.text_instance_rebuilds,
        widened.text_instance_rebuilds
    );
    assert_eq!(steady.glyph_rasterized, widened.glyph_rasterized);
}

/// A blurred shadow is rasterized and uploaded once. Every frame after
/// that is the same entry: no raster, no atlas upload, no rebuild.
#[test]
fn a_steady_frame_with_a_blurred_shadow_rasterizes_and_uploads_nothing() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let shadow = nana_ui_core::TextShadowSpec {
        offset_x: 2.0,
        offset_y: 2.0,
        blur_radius: 6.0,
        color: [0.0, 0.0, 0.0, 0.6].into(),
    };
    paint_effects(&device, &queue, &mut pipeline, None, Some(shadow));
    let first = pipeline.glyph_counters();
    assert!(first.glyph_rasterized > 0);
    for _ in 0..3 {
        paint_effects(&device, &queue, &mut pipeline, None, Some(shadow));
    }
    let steady = pipeline.glyph_counters();
    assert_eq!(steady.glyph_rasterized, first.glyph_rasterized, "no raster");
    assert_eq!(
        steady.glyph_upload_bytes, first.glyph_upload_bytes,
        "no atlas upload"
    );
    assert_eq!(
        steady.text_instance_rebuilds, first.text_instance_rebuilds,
        "no rebuild"
    );
    assert_eq!(
        steady.text_instance_upload_bytes,
        first.text_instance_upload_bytes
    );
}

/// One entry, in CSS paint order: shadow layers under outlines under the
/// underline under the fills, the strikeout on top.
#[test]
fn effects_are_instances_of_one_entry_in_paint_order() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let paint = nana_ui_core::PaintColor::srgb;
    let rich = effects(SceneTextEffects {
        material: None,
        decoration: nana_ui_core::TextDecorationLine {
            underline: true,
            line_through: true,
            overline: false,
        },
        decoration_style: Default::default(),
        decoration_color: None,
        decoration_thickness_px: None,
        emphasis_color: None,
        stroke: Some(nana_ui_core::RichTextStroke::new(
            2.0,
            paint([0.0, 0.0, 0.0, 1.0]),
        )),
        shadows: Arc::from([
            nana_ui_core::RichTextShadow::new([1.0, 1.0], 0.0, paint([1.0, 0.0, 0.0, 1.0])),
            nana_ui_core::RichTextShadow::new([3.0, 3.0], 4.0, paint([0.0, 0.0, 1.0, 0.5])),
        ]),
    });
    paint_effects(&device, &queue, &mut pipeline, rich.as_ref(), None);
    let id = pipeline
        .target
        .entries
        .lookup(EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        })
        .expect("one entry");
    let entry = pipeline.target.entries.get(id).unwrap();
    let roles: Vec<u32> = pipeline
        .target
        .entries
        .instances(entry)
        .iter()
        .take(entry.glyphs as usize)
        .map(|instance| instance.role())
        .collect();
    let first = |role: u32| roles.iter().position(|r| *r == role).unwrap();
    let last = |role: u32| roles.iter().rposition(|r| *r == role).unwrap();
    assert!(last(pipeline::ROLE_SHADOW) < first(pipeline::ROLE_STROKE));
    assert!(last(pipeline::ROLE_STROKE) < first(pipeline::ROLE_SOLID));
    let solids: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, role)| **role == pipeline::ROLE_SOLID)
        .map(|(index, _)| index)
        .collect();
    assert!(solids.len() >= 2, "an underline and a strikeout: {roles:?}");
    assert!(
        solids[0] < first(pipeline::ROLE_FILL),
        "the underline under the fill"
    );
    assert!(
        *solids.last().unwrap() > last(pipeline::ROLE_FILL),
        "the strikeout over it: {roles:?}"
    );
    let shadows = roles
        .iter()
        .filter(|role| **role == pipeline::ROLE_SHADOW)
        .count();
    let fills = roles
        .iter()
        .filter(|role| **role == pipeline::ROLE_FILL)
        .count();
    assert_eq!(shadows, fills * 2, "a copy of every glyph per shadow layer");
}

/// Measurement is paint: a rich paragraph the Runtime laid out, spans at
/// their own sizes, is drawn from its handle, glyph for glyph.
#[test]
fn a_rich_paragraph_is_drawn_from_the_layout_it_was_measured_with() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let content = "small BIG";
    let mut source = nana_text::TextSource::new(content);
    let base = nana_text::TextStyle {
        font_size_px: 16.0,
        line_height: Some(LineHeightSpec::Absolute(20.0)),
        ..nana_text::TextStyle::default()
    };
    source.set_spans(vec![nana_text::TextSpan {
        range: 6..9,
        style: nana_text::TextStyle {
            font_size_px: 28.0,
            ..base.clone()
        },
        composition: None,
    }]);
    let constraints = nana_text::TextConstraints {
        max_width_px: Some(240.0),
        preserve_lines: true,
        ..nana_text::TextConstraints::default()
    };
    let layout = {
        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let mut counters = nana_text::TextWorkCounters::default();
        nana_text::TextEngine::layout(
            &mut *engine,
            nana_text::TextKind::Label,
            &source,
            &base,
            &constraints,
            &mut counters,
        )
    };
    assert!(layout.runs.iter().any(|run| run.font_size_px == 28.0));
    let handle = nana_ui_runtime::RetainedTextLayout {
        id: layout.id,
        layout,
    };
    prepare_with_layout(
        &device,
        &queue,
        &mut pipeline,
        content,
        240.0,
        Some(&handle),
    );
    let drawn = pipeline.glyph_counters();
    assert!(
        drawn.text_retained_layouts_drawn >= 1,
        "drawn from the handle"
    );
    assert_eq!(pipeline.shape_cache_stats().1, 0, "never laid out again");
    let id = pipeline
        .target
        .entries
        .lookup(EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        })
        .expect("one entry");
    let entry = pipeline.target.entries.get(id).unwrap();
    let heights: Vec<u32> = pipeline
        .target
        .entries
        .instances(entry)
        .iter()
        .filter(|instance| !instance.is_solid())
        .filter_map(|instance| instance.placement().map(|(_, size)| size[1]))
        .collect();
    let tallest = heights.iter().copied().max().unwrap_or(0);
    let shortest = heights
        .iter()
        .copied()
        .filter(|h| *h > 0)
        .min()
        .unwrap_or(0);
    assert!(
        tallest as f32 > shortest as f32 * 1.4,
        "the span's glyphs are rasterized at its own size: {heights:?}"
    );
}

/// A ruby annotation is drawn from the layout, glyph for glyph, above its
/// base and smaller than it.
#[test]
fn a_ruby_annotation_is_drawn_above_its_base() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let content = "漢字";
    let mut source = nana_text::TextSource::new(content);
    source.set_rubies(vec![nana_text::RubySpan {
        range: 0..6,
        text: Arc::from("kanji"),
    }]);
    let base = nana_text::TextStyle {
        font_size_px: 16.0,
        line_height: Some(LineHeightSpec::Absolute(20.0)),
        ..nana_text::TextStyle::default()
    };
    let constraints = nana_text::TextConstraints {
        max_width_px: Some(240.0),
        preserve_lines: true,
        ..nana_text::TextConstraints::default()
    };
    let layout = {
        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let mut counters = nana_text::TextWorkCounters::default();
        nana_text::TextEngine::layout(
            &mut *engine,
            nana_text::TextKind::Label,
            &source,
            &base,
            &constraints,
            &mut counters,
        )
    };
    assert_eq!(layout.rubies.len(), 1);
    let annotation_glyphs: usize = layout.rubies[0]
        .runs
        .iter()
        .map(|run| run.glyphs.len())
        .sum();
    let base_glyphs: usize = layout.runs.iter().map(|run| run.glyphs.len()).sum();
    let handle = nana_ui_runtime::RetainedTextLayout {
        id: layout.id,
        layout,
    };
    prepare_with_layout(
        &device,
        &queue,
        &mut pipeline,
        content,
        240.0,
        Some(&handle),
    );
    let id = pipeline
        .target
        .entries
        .lookup(EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        })
        .expect("one entry");
    let entry = pipeline.target.entries.get(id).unwrap();
    let mut placed: Vec<([i32; 2], [u32; 2])> = pipeline
        .target
        .entries
        .instances(entry)
        .iter()
        .take(entry.glyphs as usize)
        .filter(|instance| !instance.is_solid())
        .filter_map(|instance| {
            instance
                .placement()
                .map(|(_, size)| (instance.screen_origin(), size))
        })
        .filter(|(_, size)| size[1] > 0)
        .collect();
    assert_eq!(
        placed.len(),
        base_glyphs + annotation_glyphs,
        "the base and every annotation glyph"
    );
    placed.sort_by_key(|(origin, size)| origin[1] + size[1] as i32);
    let (top, small) = placed[0];
    let (bottom, large) = placed[placed.len() - 1];
    assert!(
        top[1] + (small[1] as i32) <= bottom[1] + 2,
        "the annotation stands above the base"
    );
    assert!(small[1] < large[1], "and is smaller");
}

/// A placed glyph's origin and size.
type GlyphBox = ([i32; 2], [u32; 2]);

/// Placed glyph boxes, base and marks, of "漢字" with emphasis marks at
/// `position`, painted from the layout Runtime retained.
fn emphasis_glyphs(position: nana_ui_core::TextEmphasisPosition) -> (usize, usize, Vec<GlyphBox>) {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let content = "漢字";
    let mut source = nana_text::TextSource::new(content);
    source.set_emphasis(vec![nana_text::EmphasisSpan {
        range: 0..6,
        mark: Arc::from("\u{2022}"),
        position,
    }]);
    let base = nana_text::TextStyle {
        font_size_px: 16.0,
        line_height: Some(LineHeightSpec::Absolute(20.0)),
        ..nana_text::TextStyle::default()
    };
    let constraints = nana_text::TextConstraints {
        max_width_px: Some(240.0),
        preserve_lines: true,
        ..nana_text::TextConstraints::default()
    };
    let layout = {
        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let mut counters = nana_text::TextWorkCounters::default();
        nana_text::TextEngine::layout(
            &mut *engine,
            nana_text::TextKind::Label,
            &source,
            &base,
            &constraints,
            &mut counters,
        )
    };
    assert_eq!(layout.emphasis.len(), 2, "one mark per character");
    let base_glyphs: usize = layout.runs.iter().map(|run| run.glyphs.len()).sum();
    let mark_glyphs: usize = layout
        .emphasis
        .iter()
        .map(|mark| {
            layout.emphasis_marks[mark.mark as usize]
                .iter()
                .map(|run| run.glyphs.len())
                .sum::<usize>()
        })
        .sum();
    let handle = nana_ui_runtime::RetainedTextLayout {
        id: layout.id,
        layout,
    };
    prepare_with_layout(
        &device,
        &queue,
        &mut pipeline,
        content,
        240.0,
        Some(&handle),
    );
    let id = pipeline
        .target
        .entries
        .lookup(EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        })
        .expect("one entry");
    let entry = pipeline.target.entries.get(id).unwrap();
    let mut placed: Vec<([i32; 2], [u32; 2])> = pipeline
        .target
        .entries
        .instances(entry)
        .iter()
        .take(entry.glyphs as usize)
        .filter(|instance| !instance.is_solid())
        .filter_map(|instance| {
            instance
                .placement()
                .map(|(_, size)| (instance.screen_origin(), size))
        })
        .filter(|(_, size)| size[1] > 0)
        .collect();
    placed.sort_by_key(|(_, size)| size[1]);
    (base_glyphs, mark_glyphs, placed)
}

#[test]
fn emphasis_marks_are_drawn_over_or_under_their_base() {
    for (position, over) in [
        (nana_ui_core::TextEmphasisPosition::OVER, true),
        (nana_ui_core::TextEmphasisPosition::UNDER, false),
    ] {
        let (base_glyphs, mark_glyphs, placed) = emphasis_glyphs(position);
        assert_eq!(
            placed.len(),
            base_glyphs + mark_glyphs,
            "the base and every mark glyph"
        );
        // The marks are the smaller glyphs.
        let (marks, bases) = placed.split_at(mark_glyphs);
        assert!(marks.iter().all(|(_, size)| size[1] < bases[0].1[1]));
        for (origin, size) in marks {
            for (base, base_size) in bases {
                if over {
                    assert!(
                        origin[1] + size[1] as i32 <= base[1] + 1,
                        "a mark over the text stands above it"
                    );
                } else {
                    assert!(
                        origin[1] >= base[1] + base_size[1] as i32 - 1,
                        "a mark under the text hangs below it"
                    );
                }
            }
        }
    }
}

fn presentation(
    effects: Vec<nana_ui_core::GlyphEffect>,
    cluster_effects: Vec<u16>,
    reveal: Option<nana_ui_core::RevealSchedule>,
) -> Arc<SceneGlyphPresentation> {
    Arc::new(SceneGlyphPresentation {
        transitions: Default::default(),
        effects: effects.into(),
        effect_chains: Arc::from([]),
        cluster_effects: cluster_effects.into(),
        reveal,
        writing_mode: nana_ui_core::WritingModeSpec::HorizontalTb,
    })
}

/// "H" painted at `now` under `presented`.
fn paint_presented(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    presented: Option<&Arc<SceneGlyphPresentation>>,
    now: std::time::Duration,
) -> Option<(u32, u32, u32, u32)> {
    pipeline.write_motion(queue, now, None);
    pipeline.set_glyph_presentation(presented);
    let pixels = paint_text_with(
        device,
        queue,
        pipeline,
        ("H", 32.0),
        LogicalRect::from_xywh(16.0, 32.0, 96.0, 64.0),
        LogicalRect::from_xywh(0.0, 0.0, 128.0, 128.0),
        clip::IDENTITY_AFFINE,
        [0.0; 2],
        clip::FragmentClip::PASS,
        [128, 128],
    );
    ink_aabb(&pixels, 128, 128)
}

/// The text shader moves a glyph where the CPU evaluator says it is.
#[test]
fn the_shader_presents_a_glyph_where_the_cpu_evaluator_puts_it() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let still = paint_presented(
        &device,
        &queue,
        &mut pipeline,
        None,
        std::time::Duration::ZERO,
    )
    .expect("the glyph draws");
    let wave = nana_ui_core::GlyphEffect::wave(6.0);
    let crest = std::time::Duration::from_secs_f32(0.25 / wave.frequency_hz);
    let expected = nana_ui_core::evaluate_glyph(Some(&wave), None, 0, crest);
    let waved = presentation(vec![wave], vec![0], None);
    let moved =
        paint_presented(&device, &queue, &mut pipeline, Some(&waved), crest).expect("still drawn");
    let shift = moved.1 as f32 - still.1 as f32;
    assert!(
        (shift - expected.offset[1]).abs() <= 1.0,
        "the shader moved it {shift}px, the evaluator says {}",
        expected.offset[1]
    );
    assert_eq!(moved.0, still.0, "a wave only moves it up and down");

    // Ink bounds use an RGB threshold, and coverage correction also
    // depends on the foreground. Compare equal colours so a change in
    // brightness cannot masquerade as a change in geometry.
    let rainbow = nana_ui_core::GlyphEffect::rainbow();
    let [r, g, b] = nana_ui_core::evaluate_glyph(Some(&rainbow), None, 0, crest)
        .color
        .expect("rainbow supplies a fill");
    let ([r, g, b], alpha) = nana_ui_core::PaintColor::srgb([r, g, b, 1.0]).to_linear_sc_rgb();
    pipeline.set_linear_color_override(Some([r, g, b, alpha]));
    let coloured_wave = paint_presented(&device, &queue, &mut pipeline, Some(&waved), crest)
        .expect("the reference wave draws in the evaluated rainbow colour");
    let chain = Arc::new(SceneGlyphPresentation {
        transitions: Default::default(),
        effects: Arc::from([]),
        effect_chains: vec![nana_ui_core::GlyphEffectChain::new(vec![wave, rainbow])].into(),
        cluster_effects: vec![0].into(),
        reveal: None,
        writing_mode: nana_ui_core::WritingModeSpec::HorizontalTb,
    });
    let composed = paint_presented(&device, &queue, &mut pipeline, Some(&chain), crest)
        .expect("combined effects draw");
    assert_eq!(
        composed, coloured_wave,
        "rainbow preserves the wave geometry on the GPU"
    );

    let reveal = nana_ui_core::RevealSchedule::new(std::time::Duration::from_secs(10), vec![0.5]);
    let hidden = presentation(Vec::new(), Vec::new(), Some(reveal));
    assert!(
        paint_presented(
            &device,
            &queue,
            &mut pipeline,
            Some(&hidden),
            std::time::Duration::from_millis(10_400)
        )
        .is_none(),
        "not revealed yet: nothing drawn"
    );
    let shown = paint_presented(
        &device,
        &queue,
        &mut pipeline,
        Some(&hidden),
        std::time::Duration::from_millis(10_600),
    )
    .expect("revealed");
    assert_eq!(shown, still);

    let popped = nana_ui_core::RevealSchedule::new(std::time::Duration::ZERO, vec![0.0])
        .intro(nana_ui_core::GlyphIntro::pop(1.0));
    let half = std::time::Duration::from_millis(100);
    let expected = nana_ui_core::evaluate_glyph(None, Some(&popped), 0, half);
    let small = paint_presented(
        &device,
        &queue,
        &mut pipeline,
        Some(&presentation(Vec::new(), Vec::new(), Some(popped))),
        half,
    )
    .expect("entering");
    let (still_w, small_w) = ((still.2 - still.0) as f32, (small.2 - small.0) as f32);
    assert!(
        (small_w - still_w * expected.scale).abs() <= 2.0,
        "scaled to {small_w} of {still_w}, the evaluator says x{}",
        expected.scale
    );
}

/// Header word 11 tells `glyph_look` which way "up" is.
#[test]
fn the_glyph_fx_header_carries_the_line_over_side() {
    for (mode, word) in [
        (nana_ui_core::WritingModeSpec::HorizontalTb, 0),
        (nana_ui_core::WritingModeSpec::VerticalRl, 1),
        (nana_ui_core::WritingModeSpec::VerticalLr, 1),
    ] {
        let mut presented = presentation(vec![nana_ui_core::GlyphEffect::wave(2.0)], vec![0], None);
        Arc::make_mut(&mut presented).writing_mode = mode;
        let mut fx = vec![0; 3];
        let at = pack_glyph_fx(&mut fx, &presented) as usize;
        assert_eq!(fx[at + 11], word, "{mode:?}");
    }
}

/// Text in a vertical writing mode waves, jumps and rises across its
/// line, on the GPU where the CPU evaluator says: `(0, dy)` becomes
/// `(-dy, 0)`, toward the line-over side on the right.
#[test]
fn the_shader_turns_a_vertical_line_s_motion_where_the_cpu_evaluator_does() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let still = paint_presented(
        &device,
        &queue,
        &mut pipeline,
        None,
        std::time::Duration::ZERO,
    )
    .expect("the glyph draws");
    let wave = nana_ui_core::GlyphEffect::wave(6.0);
    let jump = nana_ui_core::GlyphEffect::jump(5.0);
    let crest = std::time::Duration::from_secs_f32(0.25 / wave.frequency_hz);
    let rising = nana_ui_core::RevealSchedule::new(std::time::Duration::ZERO, vec![0.0])
        .intro(nana_ui_core::GlyphIntro::rise(12.0, 1.0));
    let early = std::time::Duration::from_millis(100);
    for mode in [
        nana_ui_core::WritingModeSpec::VerticalRl,
        nana_ui_core::WritingModeSpec::VerticalLr,
    ] {
        for (effects, reveal, now) in [
            (vec![wave], None, crest),
            (vec![jump], None, crest),
            (Vec::new(), Some(rising.clone()), early),
        ] {
            let mut presented = presentation(
                effects.clone(),
                if effects.is_empty() {
                    Vec::new()
                } else {
                    vec![0]
                },
                reveal.clone(),
            );
            Arc::make_mut(&mut presented).writing_mode = mode;
            let expected =
                nana_ui_core::evaluate_glyph_chain_in(&effects, reveal.as_ref(), 0, now, mode);
            assert_eq!(expected.offset[1], 0.0);
            assert!(expected.offset[0].abs() >= 3.0, "{expected:?}");
            let moved = paint_presented(&device, &queue, &mut pipeline, Some(&presented), now)
                .expect("still drawn");
            let shift = moved.0 as f32 - still.0 as f32;
            assert!(
                (shift - expected.offset[0]).abs() <= 1.0,
                "{mode:?}: the shader moved it {shift}px, the evaluator says {}",
                expected.offset[0]
            );
            assert_eq!(
                (moved.1, moved.3),
                (still.1, still.3),
                "{mode:?}: across a vertical line is left and right only"
            );
        }
    }
}

/// Presentation is not part of what an entry was built from: changing it,
/// and sixty frames of it moving, rebuild and upload no glyph.
#[test]
fn a_live_presentation_rebuilds_and_uploads_no_glyph() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    paint_effects(&device, &queue, &mut pipeline, None, None);
    let first = pipeline.glyph_counters();
    let waved = presentation(
        vec![nana_ui_core::GlyphEffect::wave(4.0)],
        vec![0; 13],
        None,
    );
    for frame in 0..60u64 {
        pipeline.write_motion(&queue, std::time::Duration::from_millis(frame * 16), None);
        pipeline.set_glyph_presentation(Some(&waved));
        paint_rich(&device, &queue, &mut pipeline, &[]);
    }
    let steady = pipeline.glyph_counters();
    assert_eq!(steady.text_instance_rebuilds, first.text_instance_rebuilds);
    assert_eq!(steady.glyph_rasterized, first.glyph_rasterized);
    assert_eq!(steady.glyph_upload_bytes, first.glyph_upload_bytes);
    assert_eq!(
        steady.text_instance_upload_bytes, first.text_instance_upload_bytes,
        "no instance moves: the shader reads the clock"
    );
}

/// Prepare one label, flush it and upload, reporting the GPU work.
#[allow(clippy::too_many_arguments)]
fn prepare_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    content: &str,
    scale: f32,
) -> crate::GpuWorkObservation {
    pipeline.begin_frame([256, 64]);
    pipeline
        .prepare(
            device,
            LogicalRect::from_xywh(0.0, 0.0, 240.0, 32.0),
            LogicalRect::from_xywh(0.0, 0.0, 256.0, 64.0),
            scale,
            content,
            Some([1.0, 1.0, 1.0, 1.0]),
            16.0,
            None,
            None,
            None,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Auto,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &SceneTextOpenType::default(),
            clip::IDENTITY_AFFINE,
            [0.0; 2],
            clip::FragmentClip::PASS,
            1.0,
            [0.0; 2],
            EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            },
            UNTRACKED_REVISION,
            None,
            true,
        )
        .expect("label must prepare");
    pipeline.flush_runs();
    let work = crate::gpu_work::GpuWorkSink::new();
    pipeline.upload(device, queue, Some(&work));
    work.snapshot()
}

/// Place one hand-made glyph and return its handle, so a batching test can
/// mix mask and color glyphs without depending on which faces this machine
/// happens to have.
fn place(
    pipeline: &mut TextPipeline,
    device: &wgpu::Device,
    glyph: u32,
    format: raster::GlyphImageFormat,
) -> atlas::GlyphAtlasEntryId {
    let key = glyph::GlyphRasterKey {
        font: glyph::GlyphFontId(0),
        font_generation: 0,
        variation: glyph::GlyphVariationId(0),
        glyph,
        size_bits: 16f32.to_bits(),
        subpixel_x: glyph::SubpixelBin::default(),
        subpixel_y: glyph::SubpixelBin::default(),
        synthesis: glyph::GlyphSynthesis::NONE,
        mode: GlyphRenderMode::Mask,
    };
    let image = std::sync::Arc::new(raster::GlyphImage {
        format,
        width: 4,
        height: 6,
        left: 0,
        top: 6,
        data: vec![255; 4 * 6 * format.bytes_per_pixel()],
    });
    let super::TextPipeline {
        atlas,
        raster,
        uploads,
        ..
    } = &mut pipeline.inner;
    atlas
        .insert(device, key, &image, raster, uploads)
        .expect("an empty atlas must place one glyph")
        .0
}

/// Open a run over already-placed glyphs, the way `build_entry` would.
fn hand_built_run(pipeline: &mut TextPipeline, handles: &[atlas::GlyphAtlasEntryId]) -> usize {
    let index = pipeline.target.live_runs;
    let node = index as u64 + 1;
    let placeholders = [
        pipeline.atlas.placeholder_page(AtlasPageKind::Mask),
        pipeline.atlas.placeholder_page(AtlasPageKind::Color),
    ];
    let key = EntryKey {
        node,
        slot: 0,
        pass: 0,
    };
    let super::TextPipeline { atlas, .. } = &mut pipeline.inner;
    let id = pipeline
        .target
        .entries
        .begin_build(key, handles.len() as u32, |handle| atlas.release(handle));
    let mut segments: Vec<EntrySegment> = Vec::new();
    for (offset, handle) in handles.iter().enumerate() {
        let placement = *pipeline.atlas.entry(*handle).expect("just placed");
        let content = match placement.kind {
            AtlasPageKind::Mask => CONTENT_MASK,
            AtlasPageKind::Color => CONTENT_COLOR,
        };
        let (mask_page, color_page) = match placement.kind {
            AtlasPageKind::Mask => (placement.page, placeholders[1]),
            AtlasPageKind::Color => (placeholders[0], placement.page),
        };
        push_entry_segment(
            &mut segments,
            placeholders,
            mask_page,
            color_page,
            offset as u32,
            0,
        );
        pipeline.atlas.retain(*handle);
        pipeline.target.entries.push_glyph(
            id,
            offset as u32,
            *handle,
            GlyphInstance::new(
                [offset as i32 * 6, 0],
                placement.size,
                placement.origin,
                pipeline::pack_srgb([1.0; 4]),
                content,
                [0.0; 4],
            ),
        );
    }
    let epoch = pipeline.atlas.placement_epoch();
    let entry = pipeline.target.entries.get_mut(id).expect("just built");
    entry.atlas_epoch = epoch;
    entry.segments = segments;
    if index == pipeline.target.runs.len() {
        pipeline.target.runs.push(TextRun {
            entry: id,
            presentation: RunPresentation {
                origin: [0.0; 2],
                color: [1.0; 4],
                opacity: 1.0,
                flags: 0,
                presentation: 0,
                raster: 1.0,
                fx: 0,
                transitions: [0; 2],
                motion_opacity_id: 0,
            },
            next: NO_RUN,
            last: index as u32,
            folded: false,
            segments: 0..0,
            pixels: 0,
        });
    }
    pipeline.target.runs[index].entry = id;
    pipeline.target.runs[index].next = NO_RUN;
    pipeline.target.runs[index].last = index as u32;
    pipeline.target.runs[index].folded = false;
    pipeline.target.live_runs += 1;
    index
}

#[test]
fn a_color_glyph_between_mask_glyphs_still_draws_as_one_batch() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    pipeline.begin_frame([64, 64]);
    let handles = [
        place(&mut pipeline, &device, 1, raster::GlyphImageFormat::Mask),
        place(
            &mut pipeline,
            &device,
            2,
            raster::GlyphImageFormat::ColorRgba,
        ),
        place(&mut pipeline, &device, 3, raster::GlyphImageFormat::Mask),
    ];
    let run = hand_built_run(&mut pipeline, &handles);
    pipeline.flush_runs();
    assert_eq!(
        pipeline.target.runs[run].segments.len(),
        1,
        "a mask page and a color page fit one bind group, so an emoji in a \
             line of text must not split the batch"
    );
    let segment = pipeline.target.segments[0];
    assert_eq!(segment.count, 3);
    assert_ne!(
        segment.color_page,
        pipeline.atlas.placeholder_page(AtlasPageKind::Color),
        "the one segment must name the real color page, not the placeholder"
    );
    assert_ne!(
        segment.mask_page,
        pipeline.atlas.placeholder_page(AtlasPageKind::Mask),
        "and the real mask page"
    );
}

#[test]
fn two_paragraphs_folded_into_one_command_draw_as_one_segment() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    pipeline.begin_frame([64, 64]);
    let first = [place(
        &mut pipeline,
        &device,
        1,
        raster::GlyphImageFormat::Mask,
    )];
    let second = [place(
        &mut pipeline,
        &device,
        2,
        raster::GlyphImageFormat::Mask,
    )];
    let a = hand_built_run(&mut pipeline, &first);
    let b = hand_built_run(&mut pipeline, &second);
    // Different origin, different color, different opacity: none of that
    // is a batch key any more, because each instance names its own run.
    pipeline.target.runs[b].presentation.origin = [40.0, 12.0];
    pipeline.target.runs[b].presentation.color = [1.0, 0.0, 0.0, 1.0];
    pipeline.target.runs[b].presentation.opacity = 0.5;
    let previous = PreparedText {
        index: a,
        ink: LogicalRect::from_xywh(0.0, 0.0, 1.0, 1.0),
    };
    let next = PreparedText {
        index: b,
        ink: LogicalRect::from_xywh(0.0, 0.0, 1.0, 1.0),
    };
    assert!(pipeline.can_merge_runs(&previous, &next));
    pipeline.merge_runs(&previous, &next);
    pipeline.flush_runs();
    assert_eq!(
        pipeline.target.runs[a].segments.len(),
        1,
        "two paragraphs that only present differently must stay one draw"
    );
    let a_capacity = pipeline
        .target
        .entries
        .get(pipeline.target.runs[a].entry)
        .expect("entry")
        .capacity;
    assert_eq!(
        pipeline.target.segments[0],
        pipeline::DrawSegment {
            material: 0,
            mask_page: pipeline.target.segments[0].mask_page,
            color_page: pipeline.target.segments[0].color_page,
            first: 0,
            count: a_capacity + 1,
        },
        "the one draw spans both blocks, slack included"
    );
    assert_eq!(
        pipeline.target.run_table.len(),
        2,
        "but each keeps its own presentation row"
    );
}

/// One label's presentation, so a gate can change exactly one thing.
#[derive(Clone, Copy)]
struct Label<'a> {
    content: &'a str,
    key: EntryKey,
    color: [f32; 4],
    opacity: f32,
    affine: [f32; 6],
    top: f32,
    left: f32,
}

impl<'a> Label<'a> {
    fn new(content: &'a str, node: u64) -> Self {
        Self {
            content,
            key: EntryKey {
                node,
                slot: 0,
                pass: 0,
            },
            color: [1.0, 1.0, 1.0, 1.0],
            opacity: 1.0,
            affine: clip::IDENTITY_AFFINE,
            top: 0.0,
            left: 0.0,
        }
    }
}

/// Prepare, flush and upload one frame of `labels`.
fn text_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    labels: &[Label<'_>],
) {
    text_frame_at(device, queue, pipeline, labels, 1.0);
}

/// [`text_frame`] on a display of `scale` physical px per logical px.
fn text_frame_at(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    labels: &[Label<'_>],
    scale: f32,
) {
    pipeline.begin_frame([512, 256]);
    for label in labels {
        pipeline.prepare(
            device,
            LogicalRect::from_xywh(label.left, label.top, 480.0, 32.0),
            LogicalRect::from_xywh(0.0, 0.0, 512.0, 256.0),
            scale,
            label.content,
            Some(label.color),
            16.0,
            None,
            None,
            None,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Auto,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &SceneTextOpenType::default(),
            label.affine,
            [0.0; 2],
            clip::FragmentClip::PASS,
            label.opacity,
            [0.0; 2],
            label.key,
            UNTRACKED_REVISION,
            None,
            true,
        );
    }
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
}

#[test]
fn more_distinct_paragraphs_than_the_cache_holds_still_shape_once_each() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // One more label than the cache's nominal capacity, all on screen at
    // once, behind a label that changes every frame. That first insert is
    // what a fixed capacity answers by evicting the rows it is about to
    // draw — and then it reshapes every one of them, every frame.
    let contents = (0..SHAPE_CACHE_CAP + 1)
        .map(|index| format!("Distinct row {index}"))
        .collect::<Vec<_>>();
    let frame = |pipeline: &mut TextPipeline, tick: usize| {
        let ticking = format!("tick {tick}");
        let mut labels = vec![Label::new(ticking.as_str(), 1)];
        labels.extend(contents.iter().enumerate().map(|(index, content)| Label {
            top: index as f32 * 2.0,
            ..Label::new(content.as_str(), index as u64 + 2)
        }));
        text_frame(&device, &queue, pipeline, &labels);
    };
    frame(&mut pipeline, 0);
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        warm_misses,
        contents.len() + 1,
        "the first frame shapes each paragraph once"
    );
    for tick in 1..4 {
        frame(&mut pipeline, tick);
    }
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses - warm_misses,
        3,
        "only the label that really changed is reshaped; the cache has to \
             hold one frame's worth of text, which is a property of the view"
    );
}

#[test]
fn a_face_set_change_gives_back_the_blocks_and_rows_its_entries_held() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let labels = (0..12)
        .map(|index| Label {
            top: index as f32 * 18.0,
            ..Label::new("Face set row", index + 1)
        })
        .collect::<Vec<_>>();
    text_frame(&device, &queue, &mut pipeline, &labels);
    let rows = pipeline.target.run_table.len();
    let slots = pipeline.target.arena.len();
    assert_eq!(rows, labels.len());
    assert!(slots > 0);
    // What `@font-face` does: every entry stops meaning what it meant.
    pipeline.drop_every_entry();
    text_frame(&device, &queue, &mut pipeline, &labels);
    assert_eq!(
        pipeline.target.run_table.len(),
        rows,
        "the rows the dropped entries held are handed out again, not added to"
    );
    assert_eq!(
        pipeline.target.arena.len(),
        slots,
        "and so are their arena blocks"
    );
    assert_eq!(pipeline.glyph_counters().text_gpu_entries_active, 12);
}

#[test]
fn a_paragraph_that_stopped_being_drawn_is_eventually_given_back() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let keep = Label::new("Still here", 1);
    let gone = Label {
        top: 40.0,
        ..Label::new("Closed panel", 2)
    };
    text_frame(&device, &queue, &mut pipeline, &[keep, gone]);
    let warm = pipeline.glyph_counters();
    assert_eq!(warm.text_gpu_entries_active, 2);
    assert_eq!(warm.text_gpu_entries_destroyed, 0);
    // The panel closes. Long enough that a tab flipped back and forth
    // would have paid for neither direction, and then long enough that
    // this one is really gone.
    for _ in 0..RETIRE_AFTER_FRAMES + RETIRE_INTERVAL {
        text_frame(&device, &queue, &mut pipeline, &[keep]);
    }
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_gpu_entries_active, 1,
        "the entry of a paragraph nothing draws any more is given back"
    );
    assert_eq!(after.text_gpu_entries_destroyed, 1);
    assert!(
        after.text_gpu_entry_glyphs < warm.text_gpu_entry_glyphs,
        "and so are its instances"
    );
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "the paragraph that stayed was never resolved again"
    );
}

/// Issue #211: the fallback lays text out with the typography the
/// Runtime measured it with, and a typography change misses the shape
/// cache rather than paint the old lines.
#[test]
fn fallback_layout_follows_the_scene_typography() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let content = "中文排版测试中文排版测试中文排版测试中文";
    let justified = SceneTextOpenType {
        typography: nana_ui_core::UsedTextTypography {
            justify_lines: true,
            justify: nana_ui_core::TextJustifySpec::InterCharacter,
            ..Default::default()
        },
        ..SceneTextOpenType::default()
    };
    let mut lay_out = |opentype: &SceneTextOpenType| {
        pipeline.lay_out(
            content,
            None,
            None,
            16.0,
            20.0,
            0.0,
            false,
            true,
            nana_ui_core::TextWrapBreak::Word,
            false,
            None,
            101.0,
            1000.0,
            TextAlignSpec::Start,
            opentype,
        )
    };
    let ragged = lay_out(&SceneTextOpenType::default());
    let filled = lay_out(&justified);
    assert!(filled.lines.len() > 1);
    let (_, lines) = filled.lines.split_last().unwrap();
    for line in lines {
        assert!(
            (line.metrics.width_px - 101.0).abs() < 0.05,
            "{}",
            line.metrics.width_px
        );
    }
    assert!(ragged.lines[0].metrics.width_px < 101.0 - 0.05);
}

#[test]
fn two_windows_taking_turns_keep_each_others_paragraphs_shaped() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut second: Option<TextPipelineTarget> = None;
    // Each window alone fits the cache; together they do not. What window
    // A drew last frame is still on A's screen while B is drawing.
    let half = SHAPE_CACHE_CAP * 2 / 3;
    let window = |offset: usize| {
        (0..half)
            .map(|index| format!("Window {offset} row {index}"))
            .collect::<Vec<_>>()
    };
    let (left, right) = (window(0), window(1));
    let paint = |pipeline: &mut TextPipeline, contents: &[String]| {
        let labels = contents
            .iter()
            .enumerate()
            .map(|(index, content)| Label {
                top: index as f32 * 2.0,
                ..Label::new(content.as_str(), index as u64 + 1)
            })
            .collect::<Vec<_>>();
        text_frame(&device, &queue, pipeline, &labels);
    };
    paint(&mut pipeline, &left);
    pipeline.swap_test_target(&mut second, &device);
    paint(&mut pipeline, &right);
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    for _ in 0..3 {
        pipeline.swap_test_target(&mut second, &device);
        paint(&mut pipeline, &left);
        pipeline.swap_test_target(&mut second, &device);
        paint(&mut pipeline, &right);
    }
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses, warm_misses,
        "neither window may evict the other's text: both are on screen"
    );
}

#[test]
fn a_scrolled_viewport_paints_the_text_its_offset_brought_into_view() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // The node sits below the viewport in its own space; the scroll offset
    // lives in the transform, exactly as `paint_transform` folds a scene
    // origin into it. Comparing the two spaces directly would drop it.
    let scrolled = [1.0, 0.0, 0.0, 1.0, 0.0, -120.0];
    let pixels = paint_text(
        &device,
        &queue,
        &mut pipeline,
        LogicalRect::from_xywh(4.0, 130.0, 48.0, 20.0),
        LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
        scrolled,
        [0.0, 0.0],
        clip::FragmentClip::PASS,
    );
    assert!(
        ink_aabb(&pixels, 64, 64).is_some(),
        "text the scroll offset brought into view must still paint"
    );
}

#[test]
fn a_static_steady_frame_resolves_no_glyph_and_moves_no_instance() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let labels = (0..8)
        .map(|index| Label {
            top: index as f32 * 24.0,
            ..Label::new("Steady label", index + 1)
        })
        .collect::<Vec<_>>();
    text_frame(&device, &queue, &mut pipeline, &labels);
    let warm = pipeline.glyph_counters();
    for _ in 0..3 {
        text_frame(&device, &queue, &mut pipeline, &labels);
    }
    let steady = pipeline.glyph_counters();
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses, 1,
        "one paragraph is shaped once, however many nodes draw it"
    );
    assert_eq!(
        steady.glyph_resolve_requests, warm.glyph_resolve_requests,
        "a static frame must not resolve a glyph"
    );
    assert_eq!(
        steady.glyph_rasterized, warm.glyph_rasterized,
        "nor rasterize one"
    );
    assert_eq!(
        steady.glyph_upload_bytes, warm.glyph_upload_bytes,
        "nor upload an atlas region"
    );
    assert_eq!(
        steady.text_instance_rebuilds, warm.text_instance_rebuilds,
        "nor rebuild an entry"
    );
    assert_eq!(
        steady.text_instance_patches, warm.text_instance_patches,
        "nor patch one"
    );
    assert_eq!(
        steady.text_instance_upload_bytes, warm.text_instance_upload_bytes,
        "nor move an instance byte"
    );
    assert_eq!(
        steady.text_presentation_upload_bytes, warm.text_presentation_upload_bytes,
        "nor a presentation byte"
    );
    assert_eq!(steady.text_gpu_entries_active, 8);
    assert_eq!(
        steady.text_prepare_nodes_skipped - warm.text_prepare_nodes_skipped,
        24,
        "every node of every steady frame is answered by its entry"
    );
}

#[test]
fn recoloring_and_fading_are_presentation_only() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let label = Label::new("Recolor me", 1);
    text_frame(&device, &queue, &mut pipeline, &[label]);
    let warm = pipeline.glyph_counters();
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    for (index, color) in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]
        .into_iter()
        .enumerate()
    {
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                color,
                opacity: 1.0 - index as f32 * 0.25,
                ..label
            }],
        );
    }
    let after = pipeline.glyph_counters();
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(misses, warm_misses, "a color or a fade must not reshape");
    assert_eq!(
        after.glyph_resolve_requests, warm.glyph_resolve_requests,
        "nor resolve a glyph"
    );
    assert_eq!(
        after.glyph_rasterized, warm.glyph_rasterized,
        "nor rasterize one"
    );
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "nor rebuild the entry"
    );
    assert_eq!(
        after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
        "nor move an instance byte"
    );
    assert!(
        after.text_presentation_upload_bytes > warm.text_presentation_upload_bytes,
        "the new color and opacity reach the GPU as run rows"
    );
}

#[test]
fn a_transform_animation_does_not_rebuild_text_instances() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let label = Label::new("Spinning", 1);
    let rotated = |angle: f32| {
        let (sin, cos) = angle.sin_cos();
        [cos, sin, -sin, cos, 24.0, 24.0]
    };
    text_frame(
        &device,
        &queue,
        &mut pipeline,
        &[Label {
            affine: rotated(0.1),
            ..label
        }],
    );
    let warm = pipeline.glyph_counters();
    for step in 1..8 {
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                affine: rotated(0.1 + step as f32 * 0.02),
                ..label
            }],
        );
    }
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "a rotation is presentation: the glyphs it turns are the same ones"
    );
    assert_eq!(
        after.glyph_rasterized, warm.glyph_rasterized,
        "and no angle is a new bitmap"
    );
    assert_eq!(
        after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
        "nor an instance byte"
    );
    assert!(
        after.text_presentation_upload_bytes > warm.text_presentation_upload_bytes,
        "each angle is a presentation row"
    );
}

#[test]
fn labels_coming_and_going_do_not_renumber_the_ones_that_stayed() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let all = (0..12)
        .map(|index| Label {
            top: index as f32 * 18.0,
            ..Label::new("Row content", index + 1)
        })
        .collect::<Vec<_>>();
    text_frame(&device, &queue, &mut pipeline, &all);
    text_frame(&device, &queue, &mut pipeline, &all);
    let warm = pipeline.glyph_counters();
    // The first label leaves. If a run row were the draw-order index, every
    // remaining label would be renumbered and every instance of every one
    // of them would have to be rewritten.
    text_frame(&device, &queue, &mut pipeline, &all[1..]);
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "nothing was reshaped or re-resolved"
    );
    assert_eq!(
        after.text_instance_patches, warm.text_instance_patches,
        "and no surviving label had its run rebound"
    );
}

#[test]
fn a_scene_scale_animation_rasterizes_each_step_it_passes_once_not_each_frame() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let label = Label::new("Zooming", 1);
    let scaled = |factor: f32| [factor, 0.0, 0.0, factor, 8.0, 8.0];
    text_frame(
        &device,
        &queue,
        &mut pipeline,
        &[Label {
            affine: scaled(1.0),
            ..label
        }],
    );
    let warm = pipeline.glyph_counters();
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    let glyphs = warm.glyph_rasterized;
    // Twenty-three frames from 1.0 to 2.1: a zoom animation that passes
    // two raster steps (√2 and 2) on its way.
    for step in 1..24 {
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                affine: scaled(1.0 + step as f32 * 0.05),
                ..label
            }],
        );
    }
    let after = pipeline.glyph_counters();
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses, warm_misses,
        "a scene scale must not lay the paragraph out again"
    );
    // The DPI scale decides the raster size frame by frame; a scene
    // transform only moves it in half-octave steps. Letting the two share
    // a policy is what turns a zoom into a bitmap per frame and a cache
    // that never stops growing.
    // Per step, at most what the first frame rasterized: sub-pixel bins
    // can merge two of a paragraph's glyphs at a larger size.
    let rasterized = after.glyph_rasterized - warm.glyph_rasterized;
    assert!(
        rasterized > glyphs && rasterized <= 2 * glyphs,
        "one raster size per step passed, not one per frame: {rasterized} \
             glyphs for two steps of a {glyphs}-glyph label"
    );
    assert_eq!(
        after.text_instance_rebuilds - warm.text_instance_rebuilds,
        2,
        "and one rebuild per step"
    );
}

#[test]
fn a_whole_pixel_scroll_rewrites_one_shared_row_and_no_run() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let labels = |offset: f32| {
        (0..12)
            .map(|index| Label {
                top: 3.3 + index as f32 * 17.6,
                affine: [1.0, 0.0, 0.0, 1.0, 0.0, -offset],
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>()
    };
    text_frame(&device, &queue, &mut pipeline, &labels(0.0));
    for offset in 1..6 {
        let warm = pipeline.glyph_counters();
        text_frame(&device, &queue, &mut pipeline, &labels(offset as f32 * 3.0));
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "a whole-pixel scroll keeps every glyph"
        );
        let moved = after.text_presentation_upload_bytes - warm.text_presentation_upload_bytes;
        assert!(
            moved <= std::mem::size_of::<pipeline::TextPresentationGpu>() as u64,
            "and moves one presentation row, not twelve run rows: {moved} bytes"
        );
    }
}

#[test]
fn a_whole_pixel_horizontal_scroll_keeps_every_glyph() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // Labels at fractional x, as flex layout leaves them. Scrolled by
    // whole pixels their glyphs are the same glyphs at the same phase, so
    // not one of them may be resolved again — however the float that
    // carries the scroll happens to round.
    let labels = |offset: f32| {
        (0..12)
            .map(|index| Label {
                left: 0.709 + index as f32 * 38.766,
                affine: [1.0, 0.0, 0.0, 1.0, -offset, 0.0],
                ..Label::new("Row", index + 1)
            })
            .collect::<Vec<_>>()
    };
    text_frame(&device, &queue, &mut pipeline, &labels(0.0));
    let warm = pipeline.glyph_counters();
    for step in 1..40 {
        text_frame(&device, &queue, &mut pipeline, &labels(step as f32 * 7.0));
    }
    assert_eq!(
        pipeline.glyph_counters().text_instance_rebuilds,
        warm.text_instance_rebuilds,
        "a whole-pixel horizontal scroll must not re-resolve a label"
    );
}

#[test]
fn a_snapped_translation_reads_as_its_whole_pixels() {
    // The CPU and the vertex stage place a translated run by these whole
    // pixels. At 110%, 120% and 175% a snapped translation comes back
    // through the scale an ulp low for some offsets; read as `floor`, the
    // label would sit a pixel left of its background on those frames.
    for scale in [1.1f32, 1.2, 1.75] {
        for step in 0..20_000 {
            let offset = step as f32 * -2.37;
            let affine =
                clip::snap_translation([1.0, 0.0, 0.0, 1.0, offset, offset], [0.0; 2], scale);
            let pixels = (offset * scale).round();
            assert_eq!(
                pipeline::whole_translation(affine, scale),
                [pixels; 2],
                "{offset} at {scale}x"
            );
        }
    }
}

#[test]
fn a_fractional_vertical_scroll_keeps_every_glyph() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // A trackpad scrolls by fractions of a pixel. Vertically that is free:
    // a translated line box is snapped to whole pixels, so its glyphs are
    // at the same vertical phase wherever the scroll left it.
    let labels = |offset: f32| {
        (0..12)
            .map(|index| Label {
                top: 3.3 + index as f32 * 17.6,
                affine: [1.0, 0.0, 0.0, 1.0, 0.0, -offset],
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>()
    };
    text_frame(&device, &queue, &mut pipeline, &labels(0.0));
    let warm = pipeline.glyph_counters();
    for step in 1..24 {
        text_frame(&device, &queue, &mut pipeline, &labels(step as f32 * 0.37));
    }
    assert_eq!(
        pipeline.glyph_counters().text_instance_rebuilds,
        warm.text_instance_rebuilds,
        "a fractional vertical scroll must not re-resolve a label"
    );
}

#[test]
fn a_fractional_horizontal_scroll_keeps_every_glyph() {
    // What the painter hands text under a trackpad's horizontal scroll:
    // the translation snapped to whole device pixels (#223). At 110%, 120%
    // and 175% — not 125% or 150%, where it is exact — some of those come
    // back through the scale an ulp off the pixel, near the start of the
    // list and more often thousands of pixels into it.
    for scale in [1.0, 1.1, 1.2, 1.25, 1.5, 1.75] {
        for start in [0.0, 4096.0] {
            let (device, queue) = test_device();
            let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
            let labels = |offset: f32| {
                let affine = clip::snap_translation(
                    [1.0, 0.0, 0.0, 1.0, -(start + offset), 0.0],
                    [0.0; 2],
                    scale,
                );
                (0..12)
                    .map(|index| Label {
                        left: start + 0.709 + index as f32 * 38.766,
                        top: 3.3 + index as f32 * 17.6,
                        affine,
                        ..Label::new("Row", index + 1)
                    })
                    .collect::<Vec<_>>()
            };
            text_frame_at(&device, &queue, &mut pipeline, &labels(0.0), scale);
            let warm = pipeline.glyph_counters();
            for step in 1..40 {
                text_frame_at(
                    &device,
                    &queue,
                    &mut pipeline,
                    &labels(step as f32 * 0.37),
                    scale,
                );
            }
            let after = pipeline.glyph_counters();
            assert_eq!(
                after.text_instance_rebuilds, warm.text_instance_rebuilds,
                "a fractional horizontal scroll at {scale}x, {start} px in, \
                     must not re-resolve a label"
            );
            assert!(
                after.text_presentation_upload_bytes > warm.text_presentation_upload_bytes,
                "while the labels do move: the shared row is rewritten"
            );
        }
    }
}

#[test]
fn a_container_that_starts_or_stops_turning_keeps_its_labels_glyphs() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let labels = |affine: [f32; 6]| {
        (0..12)
            .map(|index| Label {
                top: 3.3 + index as f32 * 17.6,
                affine,
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>()
    };
    let turned = |degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        [cos, sin, -sin, cos, 0.0, 0.0]
    };
    let pressed = |factor: f32| [factor, 0.0, 0.0, factor, 0.0, 0.0];
    text_frame(
        &device,
        &queue,
        &mut pipeline,
        &labels(clip::IDENTITY_AFFINE),
    );
    let warm = pipeline.glyph_counters();
    // A wobble through upright, and a press that springs back: the
    // container leaves the identity and returns to it on every pass.
    for affine in [
        turned(2.0),
        turned(-2.0),
        clip::IDENTITY_AFFINE,
        turned(3.0),
        pressed(0.97),
        pressed(0.985),
        clip::IDENTITY_AFFINE,
    ] {
        text_frame(&device, &queue, &mut pipeline, &labels(affine));
    }
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "entering or leaving a transform is presentation: the glyphs are \
             resolved at the same phase either side of it"
    );
    assert_eq!(after.glyph_rasterized, warm.glyph_rasterized);
}

#[test]
fn a_container_with_a_fractional_offset_keeps_its_glyphs_as_it_turns() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // A container scrolled to a fraction of a pixel, starting and stopping
    // a turn. Upright, the painter draws its translation on whole pixels
    // (#223) and the labels' phase is their own position's; turned, the
    // projected path's phase is the same. The turn is presentation.
    let labels = |affine: [f32; 6]| {
        (0..12)
            .map(|index| Label {
                top: 3.3 + index as f32 * 17.6,
                left: 0.709,
                affine,
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>()
    };
    let upright = clip::snap_translation([1.0, 0.0, 0.0, 1.0, 10.3, 0.0], [0.0; 2], 1.0);
    let turned = |degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        [cos, sin, -sin, cos, 10.3, 0.0]
    };
    text_frame(&device, &queue, &mut pipeline, &labels(upright));
    let warm = pipeline.glyph_counters();
    for affine in [turned(2.0), upright, turned(-2.0), upright] {
        text_frame(&device, &queue, &mut pipeline, &labels(affine));
    }
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "a fractionally scrolled container entering or leaving a turn keeps its labels"
    );
}

#[test]
fn a_magnification_hovering_at_a_step_boundary_does_not_flip_bitmaps() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let label = Label::new("Hover", 1);
    // √(√2) is exactly half-way between the first two steps.
    let boundary = 2f32.powf(0.25);
    let scaled = |factor: f32| [factor, 0.0, 0.0, factor, 8.0, 8.0];
    text_frame(
        &device,
        &queue,
        &mut pipeline,
        &[Label {
            affine: scaled(boundary * 1.02),
            ..label
        }],
    );
    let warm = pipeline.glyph_counters();
    for frame in 0..20 {
        let jitter = if frame % 2 == 0 { 0.97 } else { 1.03 };
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                affine: scaled(boundary * jitter),
                ..label
            }],
        );
    }
    assert_eq!(
        pipeline.glyph_counters().text_instance_rebuilds,
        warm.text_instance_rebuilds,
        "a scale jittering across a step boundary keeps the step it holds"
    );
}

#[test]
fn text_a_transform_scales_is_rasterized_at_the_size_it_is_seen() {
    let (device, queue) = test_device();
    // The same text scaled and upright at the size it is seen: 16 px under
    // `scale(2)` against 32 px, and 32 px under `scale(0.5)` against 16 px.
    // Drawn from bitmaps of its own size, a magnified glyph would be
    // blurred and a shrunk one's edge half a pixel with aliased stems.
    let canvas = LogicalRect::from_xywh(0.0, 0.0, 256.0, 96.0);
    let paint = |size: f32, factor: f32| {
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        paint_text_with(
            &device,
            &queue,
            &mut pipeline,
            ("Sharp", size),
            LogicalRect::from_xywh(0.0, 0.0, size * 7.5, size * 2.5),
            canvas,
            [factor, 0.0, 0.0, factor, 0.0, 0.0],
            [0.0; 2],
            clip::FragmentClip::PASS,
            [256, 96],
        )
    };
    for (size, factor) in [(16.0, 2.0), (32.0, 0.5)] {
        let (scaled, upright) = (paint(size, factor), paint(size * factor, 1.0));
        let (scaled, upright) = (scaled.as_chunks::<4>().0, upright.as_chunks::<4>().0);
        let differing = scaled
            .iter()
            .zip(upright)
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 24))
            .count();
        let inked = upright.iter().filter(|pixel| inked(**pixel)).count();
        assert!(inked > 100, "the reference must paint something");
        assert!(
            differing * 20 < inked,
            "{size} px text under scale({factor}) must be the {} px glyphs: \
                 {differing} of {inked} inked pixels differ",
            size * factor
        );
    }
}

#[test]
fn text_squeezed_along_one_axis_keeps_its_ink_at_any_sub_pixel_phase() {
    // `scale(1, 0.3)` rasterizes for the larger scale and shrinks the
    // other axis 3.3 times: one bilinear tap per pixel skips rows, and
    // which ones depends on the sub-pixel phase. Filtered over the
    // pixel's footprint, the ink a paragraph lays down does not.
    let (device, queue) = test_device();
    let canvas = LogicalRect::from_xywh(0.0, 0.0, 256.0, 96.0);
    let ink = |phase: f32| {
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let pixels = paint_text_with(
            &device,
            &queue,
            &mut pipeline,
            ("Sharp lines", 24.0),
            LogicalRect::from_xywh(0.0, 0.0, 200.0, 40.0),
            canvas,
            [1.0, 0.0, 0.0, 0.3, 8.0, 16.0 + phase],
            [0.0; 2],
            clip::FragmentClip::PASS,
            [256, 96],
        );
        pixels
            .chunks(4)
            .map(|pixel| u64::from(pixel[1]))
            .sum::<u64>() as f32
    };
    let inks = [0.0, 0.25, 0.5, 0.75].map(ink);
    let mean = inks.iter().sum::<f32>() / 4.0;
    let spread = inks
        .iter()
        .fold(0.0f32, |spread, ink| spread.max((ink - mean).abs()))
        / mean;
    // Not zero: the gamma correction a glyph's coverage takes is not linear.
    assert!(
        spread < 0.05,
        "ink varies {:.1}% with the sub-pixel phase: {inks:?}",
        spread * 100.0
    );
}

#[test]
fn raster_steps_are_half_octaves_held_with_hysteresis_and_capped_by_size() {
    let scaled = |factor: f32| [factor, 0.0, 0.0, factor, 0.0, 0.0];
    assert_eq!(raster_step(scaled(1.0), None, 16.0), 0);
    assert_eq!(raster_step(scaled(0.9), None, 16.0), 0, "a press stays");
    assert_eq!(raster_step(scaled(0.5), None, 16.0), -2, "shrinking steps");
    assert_eq!(raster_step(scaled(0.01), None, 16.0), -4, "down to 1/4");
    assert_eq!(raster_step(scaled(2.0), None, 16.0), 2);
    assert_eq!(raster_step(scaled(1.4), None, 16.0), 1);
    let turned = [0.0, 2.0, -2.0, 0.0, 0.0, 0.0];
    assert_eq!(
        raster_step(turned, None, 16.0),
        2,
        "a rotation keeps its scale"
    );
    assert_eq!(
        raster_step(scaled(1.25), Some(0), 16.0),
        0,
        "held below the step"
    );
    assert_eq!(raster_step(scaled(1.25), Some(1), 16.0), 1, "and above it");
    assert_eq!(
        raster_step(scaled(1.3), Some(0), 16.0),
        1,
        "until it is clearly past"
    );
    assert_eq!(
        raster_step(scaled(64.0), None, 16.0),
        4,
        "four times at most"
    );
    assert_eq!(
        raster_step(scaled(4.0), None, 128.0),
        2,
        "and never an em past what a tile is worth"
    );
    assert_eq!(raster_step(scaled(f32::NAN), Some(2), 16.0), 2);
}

#[test]
fn a_dpi_change_reresolves_glyphs_without_laying_the_paragraph_out_again() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let paint = |pipeline: &mut TextPipeline, scale: f32| {
        pipeline.begin_frame([1024, 512]);
        pipeline.prepare(
            &device,
            LogicalRect::from_xywh(0.0, 0.0, 480.0, 32.0),
            LogicalRect::from_xywh(0.0, 0.0, 512.0, 256.0),
            scale,
            "DPI policy",
            Some([1.0; 4]),
            16.0,
            None,
            None,
            None,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Auto,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &SceneTextOpenType::default(),
            clip::IDENTITY_AFFINE,
            [0.0; 2],
            clip::FragmentClip::PASS,
            1.0,
            [0.0; 2],
            EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            },
            UNTRACKED_REVISION,
            None,
            true,
        );
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
    };
    paint(&mut pipeline, 1.0);
    paint(&mut pipeline, 2.0);
    let warm = pipeline.glyph_counters();
    paint(&mut pipeline, 1.0);
    paint(&mut pipeline, 2.0);
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.glyph_rasterized, warm.glyph_rasterized,
        "a DPI round trip must answer from the caches both scales filled"
    );
    assert_eq!(
        after.text_instance_rebuilds - warm.text_instance_rebuilds,
        2,
        "but each scale places its glyphs at its own pixels, so its own \
             entry content: the entry is keyed by the node and the two take turns"
    );
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses, 1,
        "the layout itself is scale-free: a DPI change re-resolves glyphs \
             and lays nothing out"
    );
}

#[test]
fn moving_a_label_by_whole_pixels_keeps_its_glyphs() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let label = Label::new("Scrolling row", 1);
    text_frame(&device, &queue, &mut pipeline, &[label]);
    let warm = pipeline.glyph_counters();
    for step in 1..6 {
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                top: step as f32 * 3.0,
                ..label
            }],
        );
    }
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.text_instance_rebuilds, warm.text_instance_rebuilds,
        "a whole-pixel move leaves the sub-pixel phase alone, so the \
             bitmaps and the instances that name them still hold"
    );
    assert_eq!(
        after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
        "and the origin the instances are relative to is a run row"
    );
}

#[test]
fn editing_one_label_does_not_retransmit_the_others() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut labels = (0..16)
        .map(|index| Label {
            top: index as f32 * 14.0,
            ..Label::new("Row content", index + 1)
        })
        .collect::<Vec<_>>();
    text_frame(&device, &queue, &mut pipeline, &labels);
    text_frame(&device, &queue, &mut pipeline, &labels);
    let block =
        u64::from(entry::capacity_for(11)) * std::mem::size_of::<pipeline::GlyphInstance>() as u64;
    // A row in the middle, so every later block would move if the arena
    // could not absorb the edit where it happened.
    for (content, blocks, note) in [
        ("New content", 1, "the same glyph count"),
        (
            "New contents",
            1,
            "one more glyph, which the block's slack absorbs",
        ),
    ] {
        let warm = pipeline.glyph_counters();
        labels[7].content = content;
        text_frame(&device, &queue, &mut pipeline, &labels);
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_instance_rebuilds - warm.text_instance_rebuilds,
            1,
            "one label changed, so one entry is resolved again ({note})"
        );
        let moved = after.text_instance_upload_bytes - warm.text_instance_upload_bytes;
        assert!(
            moved > 0 && moved <= block * blocks,
            "only the changed paragraph's block may move ({note}): \
                 {moved} bytes against a block of {block} and a list of \
                 {} blocks",
            labels.len()
        );
    }
}

#[test]
fn a_block_rebuilt_shorter_does_not_keep_drawing_the_glyphs_it_dropped() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let head = EntryKey {
        node: 7,
        slot: 0,
        pass: 0,
    };
    let tail = EntryKey {
        node: 8,
        slot: 0,
        pass: 0,
    };
    // Two paragraphs folded into one draw, so the command spans the slack
    // the first one's size class left. Whatever sits there is painted.
    let long = paint_labels(
        &device,
        &queue,
        &mut pipeline,
        &[("Force majeure", head, 0.0), ("tail", tail, 24.0)],
    );
    let short = paint_labels(
        &device,
        &queue,
        &mut pipeline,
        &[("F", head, 0.0), ("tail", tail, 24.0)],
    );
    let long_top = ink_aabb(&long[..], 256, 64).expect("the long label must paint");
    let short_top = ink_aabb(&short[..], 256, 64).expect("the short label must paint");
    assert!(
        short_top.2 < long_top.2,
        "a block rebuilt shorter must not keep painting the letters it \
             dropped: long={long_top:?} short={short_top:?}"
    );
}

fn row_key(node: u64) -> EntryKey {
    EntryKey {
        node,
        slot: 0,
        pass: 0,
    }
}

/// One frame of `rows` folded into one command, flushed and uploaded.
/// Returns what it wrote: instance bytes, index bytes and draws.
fn merged_rows(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    rows: &[&str],
) -> (u64, u64, usize) {
    let labels = rows
        .iter()
        .enumerate()
        .map(|(index, content)| (*content, row_key(index as u64 + 1), index as f32 * 14.0))
        .collect::<Vec<_>>();
    let warm = pipeline.glyph_counters();
    let height = (rows.len() as u32 * 14 + 32).max(512);
    prepare_merged(device, pipeline, &labels, [512, height]).expect("the rows draw");
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
    let after = pipeline.glyph_counters();
    (
        after.text_instance_upload_bytes - warm.text_instance_upload_bytes,
        after.text_index_upload_bytes - warm.text_index_upload_bytes,
        pipeline.target.segments.len(),
    )
}

fn row_entry(pipeline: &TextPipeline, node: u64) -> &entry::TextGpuEntry {
    let id = pipeline
        .target
        .entries
        .lookup(row_key(node))
        .expect("drawn");
    pipeline.target.entries.get(id).expect("live")
}

const INSTANCE: u64 = std::mem::size_of::<pipeline::GlyphInstance>() as u64;

#[test]
fn a_paragraph_that_outgrows_its_block_moves_alone() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut rows = vec!["Row content"; 16];
    merged_rows(&device, &queue, &mut pipeline, &rows);
    assert_eq!(
        merged_rows(&device, &queue, &mut pipeline, &rows),
        (0, 0, 1),
        "a steady frame writes nothing and is one draw"
    );
    let before = row_entry(&pipeline, 8).capacity;
    rows[7] = "Row content that grew well past its block";
    let (instances, indices, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let entry = row_entry(&pipeline, 8);
    assert!(entry.capacity > before, "the row must have changed class");
    assert_eq!(
        instances,
        u64::from(entry.capacity) * INSTANCE,
        "the row that grew writes its own block and nobody else's"
    );
    assert_eq!(
        indices,
        u64::from(entry.capacity) * 4,
        "and its own index range"
    );
    assert_eq!(
        draws, 3,
        "its range sits away from its neighbours: the rows before it, it, \
             the rows after it"
    );
    assert_eq!(
        merged_rows(&device, &queue, &mut pipeline, &rows),
        (0, 0, 3),
        "which is kept, not rewritten every frame, while it is within budget"
    );
}

#[test]
fn an_arena_repack_rewrites_the_indices_of_the_blocks_it_moved_and_no_others() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut rows = vec!["Row content"; 16];
    merged_rows(&device, &queue, &mut pipeline, &rows);
    // Row 8 outgrows its block and is placed after every other one.
    rows[7] = "Row content, longer";
    let (_, _, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let before = (1..=16u64)
        .map(|node| {
            let entry = row_entry(&pipeline, node);
            (entry.arena_offset, entry.order_offset)
        })
        .collect::<Vec<_>>();
    // Whatever makes the arena repack — room, holes — it does so apart
    // from the draw order. Forcing it here is the one way to see that.
    pipeline.target.arena.note_breaks(u32::MAX);
    let (instances, indices, again) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let mut moved = 0;
    let mut total = 0;
    for (node, (arena, order)) in (1..=16u64).zip(before) {
        let entry = row_entry(&pipeline, node);
        total += entry.capacity;
        assert_eq!(
            entry.order_offset, order,
            "row {node}: an arena repack leaves the draw order alone"
        );
        if entry.arena_offset != arena {
            moved += entry.capacity;
        }
    }
    assert!(
        moved > 0,
        "the repack laid the blocks out in draw order again"
    );
    assert_eq!(
        instances,
        u64::from(total) * INSTANCE,
        "every block is written"
    );
    assert_eq!(
        indices,
        u64::from(moved) * 4,
        "and only the indices of the blocks that landed somewhere new"
    );
    assert_eq!(
        again, draws,
        "the draws follow the order, which did not change"
    );
}

#[test]
fn repacking_the_draw_order_moves_no_instance() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut rows = vec!["Row content"; 16];
    merged_rows(&device, &queue, &mut pipeline, &rows);
    // Three rows apart from each other: two breaks each, past the budget
    // a list this short gets. Grown by a little, so the order still has
    // room for them and it is the breaks that repack it.
    for row in [3, 7, 11] {
        rows[row] = "Row content, longer";
    }
    let (instances, _, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    assert_eq!(draws, 7);
    let grown: u32 = [4, 8, 12]
        .into_iter()
        .map(|node| row_entry(&pipeline, node).capacity)
        .sum();
    assert_eq!(instances, u64::from(grown) * INSTANCE);
    let (instances, indices, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let total: u32 = (1..=16u64)
        .map(|node| row_entry(&pipeline, node).order_capacity)
        .sum();
    assert_eq!(draws, 1, "the frame after is one draw again");
    assert_eq!(
        indices,
        u64::from(total) * 4,
        "by rewriting the index table in draw order"
    );
    assert_eq!(
        instances, 0,
        "and not one instance: where a block sits no longer decides what it \
             batches with"
    );
}

#[test]
fn text_that_keeps_growing_grows_in_place_once_the_order_has_left_it_room() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut rows = vec!["Row content"; 64];
    merged_rows(&device, &queue, &mut pipeline, &rows);
    assert!(
        !pipeline.target.order_gapped,
        "a first layout leaves no gaps"
    );
    // Four rows apart grow: eight breaks, past what a thousand slots may
    // cost, and every move a growth.
    for row in [5, 20, 35, 50] {
        rows[row] = "Row content, longer";
    }
    merged_rows(&device, &queue, &mut pipeline, &rows);
    let (_, _, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    assert!(
        pipeline.target.order_gapped,
        "what split the draws was text growing, so the repack left room"
    );
    assert_eq!(draws, 1, "and a draw runs straight across the gaps");
    // A row that has not grown before.
    let before = row_entry(&pipeline, 11).order_offset;
    rows[10] = "Row content, longer";
    let (instances, indices, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let entry = row_entry(&pipeline, 11);
    assert_eq!(entry.order_offset, before, "it grew where it was");
    assert_eq!(draws, 1, "so the draw did not split");
    assert_eq!(instances, u64::from(entry.capacity) * INSTANCE);
    assert!(
        indices <= u64::from(GAP_EVERY + entry.capacity) * 4,
        "and only it and the rows up to the next gap moved: {indices} bytes"
    );
}

#[test]
fn text_that_comes_and_goes_is_not_left_gaps_it_would_not_use() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let frame = |pipeline: &mut TextPipeline, nodes: &[u64]| {
        let labels = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| ("Row content", row_key(*node), index as f32 * 14.0))
            .collect::<Vec<_>>();
        prepare_merged(&device, pipeline, &labels, [512, 1024]).expect("the rows draw");
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
        pipeline.target.segments.len()
    };
    let mut nodes = (1..=64u64).collect::<Vec<_>>();
    frame(&mut pipeline, &nodes);
    for (at, node) in [(48, 204), (32, 203), (16, 202), (8, 201)] {
        nodes.insert(at, node);
    }
    frame(&mut pipeline, &nodes);
    assert_eq!(frame(&mut pipeline, &nodes), 1, "repacked");
    assert!(
        !pipeline.target.order_gapped,
        "rows arriving split the draws; room to grow would not have helped"
    );
}

/// Sixty-four rows folded into one command, four of them grown far enough
/// apart that the order repacked with gaps. Returns the rows as drawn.
fn rows_in_a_gapped_order(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
) -> Vec<&'static str> {
    let mut rows = vec!["Row content"; 64];
    merged_rows(device, queue, pipeline, &rows);
    for row in [5, 20, 35, 50] {
        rows[row] = "Row content, longer";
    }
    merged_rows(device, queue, pipeline, &rows);
    merged_rows(device, queue, pipeline, &rows);
    assert!(pipeline.target.order_gapped, "the order repacked with gaps");
    rows
}

/// Index slots this frame's draws span: the quads the vertex stage runs.
fn spanned_slots(pipeline: &TextPipeline) -> u32 {
    pipeline
        .target
        .segments
        .iter()
        .map(|segment| segment.count)
        .sum()
}

#[test]
fn text_that_stopped_growing_gives_its_gaps_back() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let rows = rows_in_a_gapped_order(&device, &queue, &mut pipeline);
    let gapped = spanned_slots(&pipeline);
    let generation = pipeline.target.order.generation();
    while pipeline.target.quiet_frames + 1 < SETTLE_FRAMES {
        assert_eq!(
            merged_rows(&device, &queue, &mut pipeline, &rows),
            (0, 0, 1),
            "a still frame before the text has stood still long enough \
                 writes nothing"
        );
    }
    assert_eq!(pipeline.target.order.generation(), generation);
    assert!(pipeline.target.order_gapped, "and keeps the gaps");
    let (instances, indices, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    let held: u32 = (1..=64u64)
        .map(|node| row_entry(&pipeline, node).order_capacity)
        .sum();
    let blocks: u32 = (1..=64u64)
        .map(|node| row_entry(&pipeline, node).capacity)
        .sum();
    assert_eq!(
        pipeline.target.order.generation(),
        generation + 1,
        "the frame the text has stood still for long enough repacks once"
    );
    assert!(!pipeline.target.order_gapped, "without gaps");
    assert_eq!(instances, 0, "moving no instance");
    assert_eq!(
        indices,
        u64::from(held) * 4,
        "and rewriting the index table, ranges only"
    );
    assert_eq!(held, blocks, "every range is its block again");
    assert_eq!(draws, 1, "still one draw");
    let packed = spanned_slots(&pipeline);
    assert!(packed <= held, "which spans no slot outside a range");
    assert!(
        packed < gapped,
        "and fewer quads than across the gaps: {packed} of {gapped}"
    );
    assert_eq!(
        merged_rows(&device, &queue, &mut pipeline, &rows),
        (0, 0, 1),
        "after which a still frame writes nothing again"
    );
}

#[test]
fn text_that_grows_again_after_giving_its_gaps_back_gets_them_again() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut rows = rows_in_a_gapped_order(&device, &queue, &mut pipeline);
    while pipeline.target.order_gapped {
        merged_rows(&device, &queue, &mut pipeline, &rows);
    }
    // Grows again, somewhere else: the rows split the draws and the next
    // repack is, as before, down to text growing.
    for row in [10, 25, 40, 55] {
        rows[row] = "Row content, longer";
    }
    merged_rows(&device, &queue, &mut pipeline, &rows);
    let (_, _, draws) = merged_rows(&device, &queue, &mut pipeline, &rows);
    assert!(
        pipeline.target.order_gapped,
        "growing text gets its room back"
    );
    assert_eq!(draws, 1);
    // And keeps it for as long as it keeps growing: one row a frame,
    // each a little longer than the last time it grew.
    let texts = (0..24)
        .map(|len| format!("Row content {}", "x".repeat(4 + len * 3)))
        .collect::<Vec<_>>();
    let mut grown = vec![0usize; 64];
    let generation = pipeline.target.order.generation();
    let mut repacks = 0;
    for frame in 0..SETTLE_FRAMES as usize * 2 {
        let row = frame * 13 % 64;
        grown[row] += 1;
        rows[row] = texts[grown[row].min(texts.len() - 1)].as_str();
        let before = pipeline.target.order.generation();
        merged_rows(&device, &queue, &mut pipeline, &rows);
        if pipeline.target.order.generation() != before {
            repacks += 1;
        }
        assert!(
            pipeline.target.order_gapped,
            "frame {frame}: text that keeps growing keeps its gaps"
        );
    }
    assert!(
        repacks <= 4,
        "and repacks when its breaks are over budget, not every frame: \
             {repacks} repacks, generation {generation} -> {}",
        pipeline.target.order.generation()
    );
}

#[test]
fn gaps_given_back_in_a_frame_already_repacking_repack_once() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let rows = rows_in_a_gapped_order(&device, &queue, &mut pipeline);
    pipeline.target.quiet_frames = SETTLE_FRAMES - 1;
    pipeline.target.order.note_breaks(u32::MAX);
    let generation = pipeline.target.order.generation();
    merged_rows(&device, &queue, &mut pipeline, &rows);
    assert_eq!(pipeline.target.order.generation(), generation + 1);
    assert!(
        !pipeline.target.order_gapped,
        "a repack over budget in text that has stopped growing leaves no gaps"
    );
}

#[test]
fn dropping_every_entry_forgets_the_gaps() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    rows_in_a_gapped_order(&device, &queue, &mut pipeline);
    pipeline.drop_every_entry();
    assert!(
        !pipeline.target.order_gapped,
        "an order reset holds no gaps, so there are none to give back"
    );
    assert!(!pipeline.order_settle_due());
}

#[test]
fn a_draw_does_not_run_across_the_space_a_paragraph_left() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let frame = |pipeline: &mut TextPipeline, rows: &[(u64, &'static str)]| {
        let labels = rows
            .iter()
            .enumerate()
            .map(|(index, (node, content))| (*content, row_key(*node), index as f32 * 14.0))
            .collect::<Vec<_>>();
        prepare_merged(&device, pipeline, &labels, [512, 512]).expect("the rows draw");
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
        pipeline.target.segments.len()
    };
    let rows = (1..=16u64)
        .map(|node| (node, "Row content"))
        .collect::<Vec<_>>();
    frame(&mut pipeline, &rows);
    // Row 8 outgrows its range, which is given back, and is now drawn
    // last: rows 7 and 9 are next to each other in the draw with the
    // space row 8 left between them in the table. That space still holds
    // row 8's old indices, so a draw spanning it would paint them.
    let mut reordered = rows
        .iter()
        .copied()
        .filter(|(node, _)| *node != 8)
        .collect::<Vec<_>>();
    reordered.push((8, "Row content, longer"));
    frame(&mut pipeline, &reordered);
    // The frame after: the space is free now, and rows 7 and 9 are still
    // drawn one after the other.
    assert_eq!(
        frame(&mut pipeline, &reordered),
        2,
        "rows 7 and 9 are two draws, not one across what row 8 left"
    );
}

#[test]
fn two_paints_of_one_target_before_one_submit_each_draw_their_own_text() {
    let (device, queue) = test_device();
    let target = |label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 256,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let paint = |pipeline: &mut TextPipeline,
                 encoder: &mut wgpu::CommandEncoder,
                 texture: &wgpu::Texture,
                 text: &str| {
        let command = prepare_merged(&device, pipeline, &[(text, row_key(1), 0.0)], [256, 64]);
        pipeline.flush_runs();
        pipeline.upload_with(&device, &queue, encoder, None);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui text two paints"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let scissor = PhysicalRect {
            x: 0,
            y: 0,
            width: 256,
            height: 64,
        };
        pipeline.draw(&mut pass, &command.expect("drawn"), scissor, None);
    };
    let reference = |text: &str| {
        let mut fresh = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        paint_labels(&device, &queue, &mut fresh, &[(text, row_key(1), 0.0)])
    };
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // The same paragraph, rebuilt in between: the second paint rewrites
    // the block the first one's pass reads.
    let (first, second) = (target("first"), target("second"));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui text two paints"),
    });
    paint(&mut pipeline, &mut encoder, &first, "First");
    paint(&mut pipeline, &mut encoder, &second, "Second");
    let first_pixels = readback_rgba(&device, &queue, encoder, &first, 256, 64);
    let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui text two paints readback"),
    });
    let second_pixels = readback_rgba(&device, &queue, encoder, &second, 256, 64);
    assert!(
        first_pixels == reference("First"),
        "the first paint draws its own glyphs, not the ones the second \
             paint wrote before the encoder was submitted"
    );
    assert!(second_pixels == reference("Second"));
}

#[test]
fn a_paragraph_that_moved_in_the_arena_paints_where_it_did() {
    let (device, queue) = test_device();
    let head = row_key(1);
    let middle = row_key(2);
    let tail = row_key(3);
    let frame = |middle_text| {
        [
            ("Head", head, 0.0),
            (middle_text, middle, 20.0),
            ("Tail", tail, 40.0),
        ]
    };
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    paint_labels(&device, &queue, &mut pipeline, &frame("ab"));
    // Out of its block, then out of the next one: moved in the arena and
    // in the draw order both times.
    for text in ["abcdefghijk", "abcdefghijklmnopqrst"] {
        let moved = paint_labels(&device, &queue, &mut pipeline, &frame(text));
        let mut fresh = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let expected = paint_labels(&device, &queue, &mut fresh, &frame(text));
        assert!(
            moved == expected,
            "a block that moved in the arena must paint exactly what a \
                 fresh one does ({text})"
        );
    }
}

#[test]
fn an_eviction_recovers_the_entry_it_hit_without_reshaping_anything() {
    let (device, queue) = test_device();
    // One page that either paragraph fits in and the two together do not,
    // so each one's entry keeps losing its placements to the other. Fits,
    // not with room to spare: a paragraph fills most of the page, so the
    // holes the other one's evicted glyphs leave are the wrong shape and
    // only a repack places the last few.
    let mut pipeline = TextPipeline::with_atlas_limits(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        GlyphAtlasLimits {
            page_edge: 80,
            byte_budget: 80 * 80 + 8,
        },
    );
    const LOWER: &str = "abcdefghijklmnopqrstuvwxy";
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXY";
    let first = EntryKey {
        node: 1,
        slot: 0,
        pass: 0,
    };
    let second = EntryKey {
        node: 2,
        slot: 0,
        pass: 0,
    };
    let fresh = paint_labels(&device, &queue, &mut pipeline, &[(LOWER, first, 0.0)]);
    let fresh_ink = ink_aabb(&fresh, 256, 64).expect("the first paragraph paints");
    paint_labels(&device, &queue, &mut pipeline, &[(UPPER, second, 0.0)]);
    let warm = pipeline.glyph_counters();
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    let mut recovered = Vec::new();
    for _ in 0..4 {
        recovered = paint_labels(&device, &queue, &mut pipeline, &[(LOWER, first, 0.0)]);
        paint_labels(&device, &queue, &mut pipeline, &[(UPPER, second, 0.0)]);
    }
    let after = pipeline.glyph_counters();
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert!(
        after.glyph_atlas_evict > warm.glyph_atlas_evict,
        "the two paragraphs must really be evicting each other"
    );
    assert!(
        after.atlas_stale_handle_rejects > warm.atlas_stale_handle_rejects,
        "and the entry must really be meeting handles the eviction reissued"
    );
    assert_eq!(
        ink_aabb(&recovered, 256, 64),
        Some(fresh_ink),
        "the paragraph comes back whole: a rejected handle costs its glyphs, \
             never the wrong ones"
    );
    // The box only proves the ends are inked. A glyph written before a
    // repack that moved it would still ink the same box while sampling
    // whatever the repack put at its old rectangle.
    assert!(
        recovered == fresh,
        "every glyph samples its own rectangle, including the ones a repack \
             moved while the paragraph was still faulting glyphs in"
    );
    assert_eq!(
        misses, warm_misses,
        "an eviction costs an entry its glyphs, never its shaped paragraph"
    );
    assert!(
        after.text_instance_rebuilds > warm.text_instance_rebuilds,
        "the entry the eviction hit is resolved again, and only that one"
    );
}

#[test]
fn a_glyph_the_atlas_had_no_room_for_is_placed_once_there_is() {
    let (device, queue) = test_device();
    // Either paragraph alone fills a bit over half the page, so after the
    // other one is evicted there is room to spare even for a packer that
    // strands some of it.
    let limits = GlyphAtlasLimits {
        page_edge: 96,
        byte_budget: 96 * 96 + 8,
    };
    const LOWER: &str = "abcdefghijklmnopqrstuvwxy";
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXY";
    let lower = EntryKey {
        node: 1,
        slot: 0,
        pass: 0,
    };
    let upper = EntryKey {
        node: 2,
        slot: 0,
        pass: 0,
    };
    let mut alone =
        TextPipeline::with_atlas_limits(&device, wgpu::TextureFormat::Rgba8Unorm, limits);
    let whole = ink_aabb(
        &paint_labels(&device, &queue, &mut alone, &[(UPPER, upper, 0.0)]),
        256,
        64,
    )
    .expect("the paragraph paints on its own");
    let mut pipeline =
        TextPipeline::with_atlas_limits(&device, wgpu::TextureFormat::Rgba8Unorm, limits);
    // Both at once: the page holds either paragraph and not the two, and
    // every glyph in it is this frame's, so nothing can be evicted to make
    // room. The second paragraph comes up short.
    let crowded = paint_labels(
        &device,
        &queue,
        &mut pipeline,
        &[(LOWER, lower, 0.0), (UPPER, upper, 32.0)],
    );
    let short = ink_aabb(&crowded[256 * 4 * 32..], 256, 32);
    assert_ne!(
        short.map(|(x, _, right, _)| (x, right)),
        Some((whole.0, whole.2)),
        "the page must really be too small for both for this to test anything"
    );
    let (_, warm_misses, _) = pipeline.shape_cache_stats();
    // The crowd is gone. Nothing about the paragraph changed, so only the
    // entry remembering that it is incomplete can bring the rest back.
    let recovered = paint_labels(&device, &queue, &mut pipeline, &[(UPPER, upper, 0.0)]);
    assert_eq!(
        ink_aabb(&recovered, 256, 64),
        Some(whole),
        "a paragraph the atlas could not finish must be finished once it can"
    );
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(misses, warm_misses, "without laying anything out again");
    let steady = pipeline.glyph_counters();
    paint_labels(&device, &queue, &mut pipeline, &[(UPPER, upper, 0.0)]);
    assert_eq!(
        pipeline.glyph_counters().text_instance_rebuilds,
        steady.text_instance_rebuilds,
        "and once whole it is retained like any other"
    );
}

#[test]
fn a_glyph_too_big_for_any_page_does_not_rebuild_its_paragraph_every_frame() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::with_atlas_limits(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        GlyphAtlasLimits {
            page_edge: 64,
            byte_budget: 64 * 64 * 4 + 8,
        },
    );
    // 16 px at 6x is a glyph no 64 px page can hold, next frame or ever.
    // Waiting for room would rebuild the paragraph on every frame for a
    // glyph that is never coming.
    prepare_label(&device, &queue, &mut pipeline, "Wide", 6.0);
    let warm = pipeline.glyph_counters();
    for _ in 0..4 {
        prepare_label(&device, &queue, &mut pipeline, "Wide", 6.0);
    }
    assert_eq!(
        pipeline.glyph_counters().text_instance_rebuilds,
        warm.text_instance_rebuilds,
        "a glyph that can never be placed is not a reason to try again"
    );
}

#[test]
fn evicting_a_glyph_moves_the_placement_epoch_that_gates_retained_commands() {
    let (device, queue) = test_device();
    // One page barely wider than a line of text, so a second paragraph of
    // different glyphs cannot coexist with the first.
    let mut pipeline = TextPipeline::with_atlas_limits(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        GlyphAtlasLimits {
            page_edge: 64,
            byte_budget: 64 * 64 + 8,
        },
    );
    prepare_label(&device, &queue, &mut pipeline, "abcdefghij", 1.0);
    let steady = pipeline.placement_epoch();
    prepare_label(&device, &queue, &mut pipeline, "abcdefghij", 1.0);
    assert_eq!(
        pipeline.placement_epoch(),
        steady,
        "a repaint that places nothing new must leave retained commands valid"
    );

    for label in [
        "KLMNOPQRST",
        "UVWXYZ0123",
        "456789!?@#",
        "\u{4e2d}\u{6587}\u{6e2c}\u{8a66}\u{6587}\u{5b57}",
    ] {
        prepare_label(&device, &queue, &mut pipeline, label, 1.0);
    }
    assert!(
        pipeline.glyph_counters().glyph_atlas_evict > 0,
        "the corpus must outgrow this page for the test to mean anything"
    );
    assert_ne!(
        pipeline.placement_epoch(),
        steady,
        "an eviction must invalidate commands built against the old placements"
    );
}

#[test]
fn a_corpus_larger_than_the_atlas_keeps_placing_glyphs_and_never_samples_a_stale_one() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::with_atlas_limits(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        GlyphAtlasLimits {
            page_edge: 128,
            byte_budget: 128 * 128 + 8,
        },
    );
    // Far more distinct CJK glyphs than one small page holds, three times
    // over, so the atlas fills, evicts, repacks and refills.
    let corpus: Vec<String> = (0..48)
        .map(|line: u32| {
            (0..8)
                .filter_map(|index| char::from_u32(0x4e00 + line * 8 + index))
                .collect()
        })
        .collect();
    for _ in 0..3 {
        for label in &corpus {
            prepare_label(&device, &queue, &mut pipeline, label, 1.0);
        }
    }
    let counters = pipeline.glyph_counters();
    assert!(
        counters.glyph_atlas_evict > 0,
        "the corpus has to outgrow the page for this to test anything"
    );
    assert!(
        counters.glyph_atlas_bytes <= (128 * 128 + 8) as u64,
        "the atlas must stay inside its byte budget, got {} bytes",
        counters.glyph_atlas_bytes
    );
    assert!(
        counters.glyph_raster_cache_hit > 0,
        "a second pass over the corpus must be answered from the raster cache"
    );
    // Every glyph a frame placed is protected from that frame's own
    // eviction, so the flush that turns placements into instances must
    // never meet a handle that has gone stale.
    assert_eq!(
        counters.atlas_stale_handle_rejects, 0,
        "a frame must never build an instance from a placement it lost"
    );
    // One region per glyph faulted in, and not one more: a full page must
    // not repack — and therefore re-upload its whole live set — for every
    // glyph it cannot place.
    assert_eq!(
        counters.glyph_upload_regions, counters.glyph_atlas_miss,
        "a churning atlas must upload each placement once"
    );
}

#[test]
fn a_second_window_on_one_device_reuses_the_first_windows_glyphs() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    prepare_label(&device, &queue, &mut pipeline, "Shared chrome", 1.0);
    let first = pipeline.glyph_counters();
    assert!(first.glyph_rasterized > 0);
    assert!(first.glyph_upload_regions > 0);

    // A second render target on the same device. Its own instance buffers
    // are separate; the atlas and the rasterized glyphs are not.
    let mut second = None;
    pipeline.swap_test_target(&mut second, &device);
    prepare_label(&device, &queue, &mut pipeline, "Shared chrome", 1.0);
    let shared = pipeline.glyph_counters();
    assert_eq!(
        shared.glyph_rasterized, first.glyph_rasterized,
        "a second window must not rasterize the first window's glyphs again"
    );
    assert_eq!(
        shared.glyph_upload_regions, first.glyph_upload_regions,
        "nor upload them to a second atlas"
    );
    assert_eq!(
        shared.glyph_atlas_hit,
        first.glyph_atlas_hit + first.glyph_atlas_hit + first.glyph_atlas_miss,
        "every atlas lookup of the second window must hit the shared atlas"
    );
    assert_eq!(
        shared.glyph_atlas_miss, first.glyph_atlas_miss,
        "a second window must not fault a glyph into the shared atlas"
    );
    assert_eq!(shared.glyph_atlas_pages, first.glyph_atlas_pages);
}

#[test]
fn closing_one_window_leaves_the_others_glyphs_placed() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut first = None;
    let mut second = None;

    pipeline.swap_test_target(&mut first, &device);
    prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
    pipeline.swap_test_target(&mut first, &device);

    pipeline.swap_test_target(&mut second, &device);
    prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
    let shared = pipeline.glyph_counters();
    pipeline.swap_test_target(&mut second, &device);

    // The first window closes. Its instance buffers go with it; the glyphs
    // it faulted into the shared atlas must not.
    drop(first);

    pipeline.swap_test_target(&mut second, &device);
    prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
    let after = pipeline.glyph_counters();
    assert_eq!(
        after.glyph_rasterized, shared.glyph_rasterized,
        "closing a window must not cost the surviving one a re-rasterization"
    );
    assert_eq!(
        after.glyph_upload_regions, shared.glyph_upload_regions,
        "nor a re-upload"
    );
    assert_eq!(
        after.glyph_atlas_evict, 0,
        "nor drop anything from the shared atlas"
    );
}

#[test]
fn a_closed_window_gives_back_its_claims_on_shared_glyphs() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut first = None;
    let mut second = None;
    for window in [&mut first, &mut second] {
        pipeline.swap_test_target(window, &device);
        prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
        pipeline.swap_test_target(window, &device);
    }
    let handles = |pipeline: &TextPipeline, target: &TextPipelineTarget| {
        let id = target
            .entries
            .lookup(EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            })
            .expect("the label's entry");
        let entry = target.entries.get(id).expect("live");
        target
            .entries
            .live_glyphs(entry)
            .map(|(_, handle)| pipeline.atlas.claims(handle))
            .collect::<Vec<_>>()
    };
    let second_target = second.take().expect("second window");
    let shared = handles(&pipeline, &second_target);
    assert!(
        shared.iter().all(|claims| claims.is_some_and(|n| n >= 2)),
        "both windows claim the chrome's glyphs: {shared:?}"
    );
    // The first window closes. Its claims go with it — otherwise the
    // atlas would treat those glyphs as in use for the rest of the
    // session and evict everything else first.
    pipeline.close_target(first.take().expect("first window"));
    let after = handles(&pipeline, &second_target);
    // Both windows drew the same paragraph, so each held the same claims —
    // one per occurrence, and a letter that appears twice at the same
    // sub-pixel bin is one handle claimed twice. Closing one gives back
    // exactly half.
    for (before, now) in shared.iter().zip(&after) {
        assert_eq!(
            now.map(|n| n * 2),
            *before,
            "the closed window's claims, and only those, are released"
        );
    }
}

#[test]
fn repainting_unchanged_text_uploads_neither_a_region_nor_an_instance() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    prepare_label(&device, &queue, &mut pipeline, "Steady label", 1.0);
    let warm = pipeline.glyph_counters();
    let work = prepare_label(&device, &queue, &mut pipeline, "Steady label", 1.0);
    let steady = pipeline.glyph_counters();
    assert_eq!(
        steady.glyph_upload_regions, warm.glyph_upload_regions,
        "an unchanged repaint must upload no atlas region"
    );
    assert_eq!(
        steady.glyph_rasterized, warm.glyph_rasterized,
        "and rasterize nothing"
    );
    assert_eq!(
        work.gpu_upload_bytes, 0,
        "the instances are byte-identical, so nothing reaches the queue"
    );
    assert_eq!(work.gpu_buffer_reallocations, 0);
}

#[test]
fn a_scale_round_trip_rerasterizes_once_and_then_answers_from_cache() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    prepare_label(&device, &queue, &mut pipeline, "DPI probe", 1.0);
    let one = pipeline.glyph_counters();
    prepare_label(&device, &queue, &mut pipeline, "DPI probe", 2.0);
    let two = pipeline.glyph_counters();
    assert!(
        two.glyph_rasterized > one.glyph_rasterized,
        "a raster scale change must rasterize at the new size"
    );
    // Back again. The raster cache is keyed by size, not by "current
    // scale", so the first scale's bitmaps are still there and the atlas
    // still holds them.
    prepare_label(&device, &queue, &mut pipeline, "DPI probe", 1.0);
    let back = pipeline.glyph_counters();
    assert_eq!(
        back.glyph_rasterized, two.glyph_rasterized,
        "returning to a scale already drawn must not rasterize again"
    );
    assert_eq!(
        back.glyph_upload_regions, two.glyph_upload_regions,
        "nor re-upload"
    );
    let (_, misses, _) = pipeline.shape_cache_stats();
    assert_eq!(
        misses, 1,
        "the paragraph is laid out in logical px, so neither scale nor the \
             return to the first one is a second layout"
    );
}

/// Paint labels folded into one draw command and read the frame back.
/// Prepare `labels` as one draw command, the way the painter folds
/// consecutive text, on a `size` canvas. Neither flushed nor uploaded.
fn prepare_merged(
    device: &wgpu::Device,
    pipeline: &mut TextPipeline,
    labels: &[(&str, EntryKey, f32)],
    size: [u32; 2],
) -> Option<PreparedText> {
    pipeline.begin_frame(size);
    let mut command: Option<PreparedText> = None;
    for (content, key, top) in labels {
        let Some(prepared) = prepare_row(device, pipeline, content, *key, *top, size) else {
            continue;
        };
        match command.as_ref() {
            Some(previous) if pipeline.can_merge_runs(previous, &prepared) => {
                pipeline.merge_runs(previous, &prepared);
            }
            _ => command = Some(prepared),
        }
    }
    command
}

/// One label 240 px wide at `top`, prepared into the frame `size` opened.
fn prepare_row(
    device: &wgpu::Device,
    pipeline: &mut TextPipeline,
    content: &str,
    key: EntryKey,
    top: f32,
    size: [u32; 2],
) -> Option<PreparedText> {
    pipeline.prepare(
        device,
        LogicalRect::from_xywh(0.0, top, 240.0, 32.0),
        LogicalRect::from_xywh(0.0, 0.0, size[0] as f32, size[1] as f32),
        1.0,
        content,
        Some([1.0, 1.0, 1.0, 1.0]),
        16.0,
        None,
        None,
        None,
        false,
        nana_ui_core::TextWrapBreak::Word,
        false,
        false,
        None,
        TextShaping::Auto,
        TextHorizontalAlignment::Start,
        TextVerticalAlignment::Top,
        &[],
        0.0,
        &[],
        &SceneTextOpenType::default(),
        clip::IDENTITY_AFFINE,
        [0.0; 2],
        clip::FragmentClip::PASS,
        1.0,
        [0.0; 2],
        key,
        UNTRACKED_REVISION,
        None,
        true,
    )
}

fn paint_labels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    labels: &[(&str, EntryKey, f32)],
) -> Vec<u8> {
    let command = prepare_merged(device, pipeline, labels, [256, 64]);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui text label prepare"),
    });
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui text label target"),
        size: wgpu::Extent3d {
            width: 256,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui text label pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(prepared) = command.as_ref() {
            pipeline.draw(
                &mut pass,
                prepared,
                PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 256,
                    height: 64,
                },
                None,
            );
        }
    }
    readback_rgba(device, queue, encoder, &texture, 256, 64)
}

#[allow(clippy::too_many_arguments)]
fn paint_text(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    bounds: LogicalRect,
    clip: LogicalRect,
    affine: [f32; 6],
    persp: [f32; 2],
    fragment_clip: clip::FragmentClip,
) -> Vec<u8> {
    paint_text_with(
        device,
        queue,
        pipeline,
        ("Hi", 16.0),
        bounds,
        clip,
        affine,
        persp,
        fragment_clip,
        [64, 64],
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_text_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    (content, size): (&str, f32),
    bounds: LogicalRect,
    clip: LogicalRect,
    affine: [f32; 6],
    persp: [f32; 2],
    fragment_clip: clip::FragmentClip,
    canvas: [u32; 2],
) -> Vec<u8> {
    pipeline.begin_frame(canvas);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui text affine prepare"),
    });
    let opentype = SceneTextOpenType::default();
    let prepared = pipeline
        .prepare(
            device,
            bounds,
            clip,
            1.0,
            content,
            Some([1.0, 1.0, 1.0, 1.0]),
            size,
            None,
            None,
            None,
            false,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Auto,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &opentype,
            affine,
            persp,
            fragment_clip,
            1.0,
            [0.0, 0.0],
            EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            },
            UNTRACKED_REVISION,
            None,
            true,
        )
        .expect("text must prepare");
    // Placements are handles until the run is flushed; nothing is on the
    // GPU until the instances and the atlas regions are uploaded.
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui text affine target"),
        size: wgpu::Extent3d {
            width: canvas[0],
            height: canvas[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui text affine pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pipeline.draw(
            &mut pass,
            &prepared,
            PhysicalRect {
                x: 0,
                y: 0,
                width: canvas[0],
                height: canvas[1],
            },
            None,
        );
    }
    readback_rgba(device, queue, encoder, &texture, canvas[0], canvas[1])
}

#[allow(clippy::too_many_arguments)]
fn paint_block(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut TextPipeline,
    bounds: LogicalRect,
    clip: LogicalRect,
    affine: [f32; 6],
    persp: [f32; 2],
    fragment_clip: clip::FragmentClip,
) -> Vec<u8> {
    pipeline.begin_frame([64, 64]);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui text clip prepare"),
    });
    let opentype = SceneTextOpenType::default();
    let prepared = pipeline
        .prepare(
            device,
            bounds,
            clip,
            1.0,
            "HHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHH",
            Some([1.0, 1.0, 1.0, 1.0]),
            16.0,
            None,
            None,
            None,
            true,
            nana_ui_core::TextWrapBreak::Word,
            false,
            false,
            None,
            TextShaping::Auto,
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Top,
            &[],
            0.0,
            &[],
            &opentype,
            affine,
            persp,
            fragment_clip,
            1.0,
            [0.0, 0.0],
            EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            },
            UNTRACKED_REVISION,
            None,
            true,
        )
        .expect("block text must prepare");
    pipeline.flush_runs();
    pipeline.upload(device, queue, None);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui text clip target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui text clip pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pipeline.draw(
            &mut pass,
            &prepared,
            PhysicalRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            None,
        );
    }
    readback_rgba(device, queue, encoder, &texture, 64, 64)
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

fn inked(color: [u8; 4]) -> bool {
    u16::from(color[0]) + u16::from(color[1]) + u16::from(color[2]) > 24
}

fn ink_aabb(pixels: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            if u16::from(pixels[index])
                + u16::from(pixels[index + 1])
                + u16::from(pixels[index + 2])
                > 24
            {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

fn readback_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let unpadded = width as usize * 4;
    let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nana-ui text affine readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("text affine readback poll");
    let mapped = slice
        .get_mapped_range()
        .expect("text affine readback must be mapped");
    let mut pixels = Vec::with_capacity(unpadded * height as usize);
    for row in mapped.chunks_exact(padded) {
        pixels.extend_from_slice(&row[..unpadded]);
    }
    pixels
}

#[test]
fn glyphs_past_the_devices_storage_binding_leave_out_the_largest_paragraph() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    // A device whose storage binding holds 400 glyphs.
    pipeline.gpu.set_instance_slots(400);
    let log = "overflowing labels ".repeat(60);
    let mut labels = (0..12)
        .map(|row| ("Row content", row_key(row + 1), row as f32 * 14.0))
        .collect::<Vec<_>>();
    labels.insert(5, (log.as_str(), row_key(100), 70.0));
    let frame = |pipeline: &mut TextPipeline, labels: &[(&str, EntryKey, f32)]| {
        prepare_merged(&device, pipeline, labels, [512, 512]).expect("the rows draw");
        pipeline.flush_runs();
        // Binds the instance buffer whole: past the limit, validation
        // would fail here.
        pipeline.upload(&device, &queue, None);
    };
    frame(&mut pipeline, &labels);
    let log_entry = pipeline
        .target
        .entries
        .lookup(row_key(100))
        .expect("resolved");
    assert!(
        pipeline
            .target
            .entries
            .get(log_entry)
            .expect("live")
            .capacity
            > 400,
        "the case that matters: one paragraph larger than the device binds"
    );
    assert_eq!(pipeline.target.skipped, vec![log_entry]);
    assert!(
        pipeline.target.arena.capacity() <= 400,
        "the instance buffer stays inside the binding: {}",
        pipeline.target.arena.capacity()
    );
    // The audit has already checked that every other row is drawn from
    // its own glyphs. The one left out never took a range, so the rows
    // around it are still adjacent and still one draw.
    assert_eq!(pipeline.target.segments.len(), 1);
    labels.remove(5);
    frame(&mut pipeline, &labels);
    assert!(pipeline.target.skipped.is_empty());
    assert_eq!(pipeline.target.segments.len(), 1);
}

/// The instance and index buffers on the GPU hold, slot for slot, what
/// the writes this target sent say they do — the shadow
/// `audit_draw_order` checks every draw against. What that audit cannot
/// see is whether the copies out of the staging ring, or out of a frame's
/// one-off buffers, landed where they were meant to; this reads them back.
fn assert_gpu_holds_the_shadow(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &TextPipeline,
    when: &str,
) {
    let (instances, indices) = pipeline.target.gpu.read_back(device, queue);
    let shadow = &pipeline.target.shadow;
    assert_eq!(
        instances.len(),
        shadow.instances.len(),
        "{when}: instance slots"
    );
    assert_eq!(indices.len(), shadow.indices.len(), "{when}: index slots");
    if let Some(slot) = (0..instances.len()).find(|&slot| instances[slot] != shadow.instances[slot])
    {
        panic!(
            "{when}: instance slot {slot} holds {:?}, the writes put {:?} there",
            instances[slot], shadow.instances[slot]
        );
    }
    if let Some(slot) = (0..indices.len()).find(|&slot| indices[slot] != shadow.indices[slot]) {
        panic!(
            "{when}: index slot {slot} holds {}, the writes put {} there",
            indices[slot], shadow.indices[slot]
        );
    }
}

#[test]
fn a_frame_too_large_for_the_staging_ring_lands_whole() {
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let long = "a paragraph of several hundred glyphs ".repeat(16);
    let mut rows = (0..24).map(|_| long.clone()).collect::<Vec<_>>();
    let frame = |pipeline: &mut TextPipeline, rows: &[String]| {
        let labels = rows
            .iter()
            .enumerate()
            .map(|(row, content)| (content.as_str(), row_key(row as u64 + 1), row as f32 * 14.0))
            .collect::<Vec<_>>();
        let before = pipeline.glyph_counters().text_instance_upload_bytes;
        prepare_merged(&device, pipeline, &labels, [512, 512]).expect("the rows draw");
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
        pipeline.glyph_counters().text_instance_upload_bytes - before
    };
    let written = frame(&mut pipeline, &rows);
    assert!(
        written > 256 * 1024,
        "the case that matters: more than a quarter of the largest ring ({written} B), \
             so the frame is staged in buffers of its own"
    );
    assert_gpu_holds_the_shadow(&device, &queue, &pipeline, "one-off staging");
    // And the frames after it, through the ring, on top of those bytes.
    for row in [3, 11, 17] {
        rows[row] = format!("{row} changed");
        frame(&mut pipeline, &rows);
        assert_gpu_holds_the_shadow(&device, &queue, &pipeline, "the ring");
    }
}

/// xorshift32, so a churn test replays the same frames on every run.
struct Churn(u32);

impl Churn {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as usize % bound.max(1)
    }

    /// A label of anywhere from nothing to a few lines.
    fn text(&mut self) -> String {
        const WORDS: [&str; 10] = [
            "a",
            "Row",
            "cell",
            "12",
            "content",
            "ticked",
            "0.25",
            "overflowing",
            "x y",
            "labels",
        ];
        (0..self.below(12))
            .map(|_| WORDS[self.below(WORDS.len())])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Hundreds of frames of labels changing length, coming and going,
/// swapping places, splitting across commands, drawn twice and dropped
/// all at once, with both order layouts
/// (packed, and gapped for text that keeps growing) in play.
///
/// Every frame goes through `audit_placements` and `audit_draw_order`, so
/// a range left naming a block that moved, a gap a draw ran across that
/// was not vacant, or a buffer replaced under blocks the frame had already
/// placed fails here. And #224's promise is checked on every frame the
/// arena did not repack: the instance bytes written are the blocks of the
/// paragraphs that were rebuilt, never their neighbours'.
#[test]
fn seeded_churn_keeps_every_draw_naming_its_own_glyphs() {
    const INITIAL: usize = 240;
    let (device, queue) = test_device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut churn = Churn(0x2545_f491);
    let mut next_node = 1u64;
    let mut new_row = |churn: &mut Churn| {
        next_node += 1;
        (next_node, churn.text())
    };
    let mut rows = (0..INITIAL)
        .map(|_| new_row(&mut churn))
        .collect::<Vec<_>>();
    let mut drawn_as: HashMap<u64, String> = HashMap::new();
    let mut layouts = [0u32; 2];
    let mut checked = 0;
    let mut settles = 0;
    for frame in 0..600u32 {
        // Text changing length, then standing still for long enough to
        // give its gaps back (#230), then scrolling.
        let phase = frame % 300;
        let still = (75..75 + SETTLE_FRAMES + 30).contains(&phase);
        let scrolling = phase >= 75 + SETTLE_FRAMES + 30;
        // A font registration: every entry dropped at once.
        if !still && churn.below(97) == 0 {
            pipeline.drop_every_entry();
            drawn_as.clear();
        }
        if scrolling {
            for _ in 0..churn.below(6) {
                if rows.len() > INITIAL / 2 {
                    rows.remove(churn.below(rows.len()));
                }
            }
            for _ in 0..churn.below(6) {
                let at = churn.below(rows.len() + 1);
                rows.insert(at, new_row(&mut churn));
            }
            if churn.below(4) == 0 {
                let (a, b) = (churn.below(rows.len()), churn.below(rows.len()));
                rows.swap(a, b);
            }
        } else if !still {
            for _ in 0..1 + churn.below(8) {
                let at = churn.below(rows.len());
                rows[at].1 = churn.text();
            }
        }
        let (start, len) = if scrolling {
            let len = rows.len() * 3 / 4;
            (churn.below(rows.len() - len + 1), len)
        } else {
            (0, rows.len())
        };
        let size = [512, len as u32 * 14 + 32];
        pipeline.begin_frame(size);
        let epoch = pipeline.placement_epoch();
        let arena_generation = pipeline.target.arena.generation();
        let mut rebuilt = Vec::new();
        let mut command: Option<PreparedText> = None;
        for (index, (node, content)) in rows[start..start + len].iter().enumerate() {
            let key = row_key(*node);
            // What may be written: a paragraph that changed, and one an
            // arena repack placed nowhere because it was off screen then.
            let placed = pipeline
                .target
                .entries
                .lookup(key)
                .and_then(|id| pipeline.target.entries.get(id))
                .is_some_and(|entry| entry.arena_generation == Some(arena_generation));
            if !placed || drawn_as.get(node) != Some(content) {
                rebuilt.push(key);
            }
            let top = index as f32 * 14.0;
            let Some(prepared) = prepare_row(&device, &mut pipeline, content, key, top, size)
            else {
                continue;
            };
            drawn_as.insert(*node, content.clone());
            match command.as_ref() {
                Some(previous)
                    if churn.below(24) != 0 && pipeline.can_merge_runs(previous, &prepared) =>
                {
                    pipeline.merge_runs(previous, &prepared);
                }
                _ => command = Some(prepared),
            }
        }
        if churn.below(6) == 0 && len > 0 {
            // One row drawn a second time, by a command of its own.
            let at = start + churn.below(len);
            let (node, content) = &rows[at];
            let _ = prepare_row(&device, &mut pipeline, content, row_key(*node), 0.0, size);
        }
        let before = pipeline.glyph_counters().text_instance_upload_bytes;
        let gapped = pipeline.target.order_gapped;
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
        let written = pipeline.glyph_counters().text_instance_upload_bytes - before;
        if frame % 20 == 0 {
            assert_gpu_holds_the_shadow(&device, &queue, &pipeline, &format!("frame {frame}"));
        }
        layouts[usize::from(pipeline.target.order_gapped)] += 1;
        if still && gapped && !pipeline.target.order_gapped {
            settles += 1;
        }
        if pipeline.target.arena.generation() == arena_generation
            && pipeline.placement_epoch() == epoch
        {
            let own: u64 = rebuilt
                .iter()
                .filter_map(|key| pipeline.target.entries.lookup(*key))
                .map(|id| u64::from(pipeline.target.entries.get(id).expect("live").capacity))
                .sum::<u64>()
                * INSTANCE;
            assert!(
                written <= own,
                "frame {frame}: {written} B of instances written, but the {} paragraphs \
                     rebuilt or placed hold {own}",
                rebuilt.len()
            );
            checked += 1;
        }
    }
    assert!(
        layouts.iter().all(|frames| *frames > 0),
        "both order layouts must have been exercised: packed / gapped {layouts:?}"
    );
    assert!(checked > 300, "only {checked} frames kept their arena");
    assert!(
        settles > 0,
        "text that stood still never gave its gaps back"
    );
}

fn test_device() -> (wgpu::Device, wgpu::Queue) {
    crate::test_gpu::device()
}

#[test]
fn a_retained_glyph_carries_linear_authoring_color() {
    // One per glyph, held between frames and never rewritten while the
    // paragraph, its sub-pixel phase and its atlas placements stand still.
    assert_eq!(
        std::mem::size_of::<super::pipeline::GlyphInstance>(),
        48,
        "the retained glyph carries an optional linear authoring color"
    );
    assert_eq!(std::mem::size_of::<super::pipeline::TextRunGpu>(), 48);
    assert_eq!(
        std::mem::size_of::<super::pipeline::TextPresentationGpu>(),
        176
    );
}
