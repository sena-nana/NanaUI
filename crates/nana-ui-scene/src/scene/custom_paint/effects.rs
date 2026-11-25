//! Mesh caching, gradient/fill/shadow ops, and anti-aliased shadow/ring mesh building.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::float::simplify::SimplifyShape;

use super::*;

// ---------------------------------------------------------------------------
// Reuse across recordings
// ---------------------------------------------------------------------------

/// Hashes a key's words. Keys are what a node's own painter recorded, not
/// input chosen to collide, so a multiplicative hash (FxHash's) is enough;
/// the map still compares the whole key.
#[derive(Default, Clone, Copy)]
pub(super) struct KeyHasher(u64);

impl KeyHasher {
    fn mix(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, tail) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.mix(u64::from_le_bytes(*chunk));
        }
        if !tail.is_empty() {
            let mut last = [0u8; 8];
            last[..tail.len()].copy_from_slice(tail);
            self.mix(u64::from_le_bytes(last));
        }
    }

    fn write_usize(&mut self, n: usize) {
        self.mix(n as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub(super) type KeyMap<V> = HashMap<Box<[u32]>, V, BuildHasherDefault<KeyHasher>>;

/// The meshes and clips one recording built, by key. It holds nothing the
/// recording's ops do not: it is replaced, not added to, by the next
/// recording, so it costs only the keys.
#[derive(Debug, Default)]
pub(super) struct MeshCache {
    meshes: KeyMap<CachedMesh>,
    clips: KeyMap<Arc<BuiltClip>>,
}

#[derive(Debug, Clone)]
pub(super) struct CachedMesh {
    /// `None`: the op draws nothing (an empty path, or one clipped away).
    mesh: Option<Arc<PathMesh>>,
    /// The vertex colour it was built with.
    color: [f32; 4],
}

/// How a mesh is painted, apart from its geometry: what a theme or colour
/// change alters without moving a vertex.
pub(super) struct MeshPaint {
    /// Straight sRGB of the vertices (shadow bands scale its alpha).
    color: [f32; 4],
    paint_color: Option<nana_ui_core::PaintColor>,
    gradient: Option<Arc<ResolvedGradient>>,
    pattern: Option<Arc<ResolvedPattern>>,
}

/// Builds a recording's meshes, reusing the previous recording's where the
/// op is the same.
///
/// Every mesh-producing op (a fill, stroke or shadow, or a rounded
/// rectangle drawn as paths) is keyed by everything its geometry depends on,
/// as raw words: its kind and parameters (path verbs and points, fill rule,
/// stroke style, shadow offset, blur and spread), the painter's transform,
/// the key of the clip it is cut by, and whether a gradient is evaluated
/// over it (which places [`PathVertex::paint_pos`]). Curves are flattened at
/// a fixed node-local tolerance and meshes stay in node-local space (the AA
/// fringe is extruded on the GPU), so no scale is part of the key. Its
/// colour is not: a hit with another colour is recoloured, not rebuilt.
pub(super) struct MeshReuse<'a> {
    previous: Vec<&'a MeshCache>,
    next: MeshCache,
    /// The key being assembled for the current op.
    key: Vec<u32>,
    /// Meshes built rather than reused.
    pub(super) built: usize,
}

impl<'a> MeshReuse<'a> {
    pub(super) fn new(previous: Vec<&'a MeshCache>) -> Self {
        Self {
            previous,
            next: MeshCache::default(),
            key: Vec::new(),
            built: 0,
        }
    }

    pub(super) fn finish(self) -> MeshCache {
        self.next
    }

    /// Start the key of an op of kind `tag` under `t`, cut to `clip`.
    fn begin(&mut self, tag: u32, t: Affine, clip: Option<&ActiveClip>) {
        self.key.clear();
        self.key.push(tag);
        self.affine(t);
        match clip {
            None => self.key.push(u32::MAX),
            Some(clip) => {
                self.key.push(clip.key.len() as u32);
                self.key.extend_from_slice(&clip.key);
            }
        }
    }

    fn word(&mut self, value: f32) {
        self.key.push(value.to_bits());
    }

    fn words(&mut self, values: &[f32]) {
        self.key.extend(values.iter().map(|value| value.to_bits()));
    }

    fn affine(&mut self, t: Affine) {
        self.words(&t);
    }

    fn path(&mut self, path: &PaintPath) {
        self.key.push(path.fill_rule() as u32);
        self.key.push(path.verbs().len() as u32);
        for verb in path.verbs() {
            match *verb {
                PathVerb::MoveTo(p) => {
                    self.key.push(0);
                    self.words(&p);
                }
                PathVerb::LineTo(p) => {
                    self.key.push(1);
                    self.words(&p);
                }
                PathVerb::QuadTo(c, p) => {
                    self.key.push(2);
                    self.words(&c);
                    self.words(&p);
                }
                PathVerb::CubicTo(a, b, p) => {
                    self.key.push(3);
                    self.words(&a);
                    self.words(&b);
                    self.words(&p);
                }
                PathVerb::Close => self.key.push(4),
            }
        }
    }

    /// The mesh for the op keyed so far, painted `paint`: the one built for
    /// the same key, recoloured if need be, or `build(colour)` if there is
    /// none.
    fn mesh(
        &mut self,
        paint: &MeshPaint,
        build: impl FnOnce([f32; 4]) -> Option<PathMesh>,
    ) -> Option<Arc<PathMesh>> {
        let dress = |mut mesh: PathMesh| {
            mesh.paint_color = paint.paint_color;
            mesh.gradient = paint.gradient.clone();
            mesh.pattern = paint.pattern.clone();
            Arc::new(mesh)
        };
        if !paint.color.iter().all(|channel| channel.is_finite()) {
            // Nothing to recolour from or to: build it as asked.
            self.built += 1;
            return build(paint.color).map(dress);
        }
        if paint.color[3] <= 0.0 {
            return None;
        }
        if let Some(cached) = self.next.meshes.get(self.key.as_slice()) {
            // The same op twice in one recording.
            return cached
                .mesh
                .as_ref()
                .map(|mesh| recolored(mesh, cached.color, paint));
        }
        let cached = self
            .previous
            .iter()
            .find_map(|previous| previous.meshes.get(self.key.as_slice()));
        let mesh = match cached {
            Some(cached) => cached
                .mesh
                .as_ref()
                .map(|mesh| recolored(mesh, cached.color, paint)),
            None => {
                self.built += 1;
                build(paint.color).map(dress)
            }
        };
        self.next.meshes.insert(
            self.key.as_slice().into(),
            CachedMesh {
                mesh: mesh.clone(),
                color: paint.color,
            },
        );
        mesh
    }

    /// The clip keyed `key`: the one built for it, or `build()`.
    fn clip(&mut self, key: &[u32], build: impl FnOnce() -> BuiltClip) -> Arc<BuiltClip> {
        if let Some(built) = self.next.clips.get(key) {
            return Arc::clone(built);
        }
        let built = self
            .previous
            .iter()
            .find_map(|previous| previous.clips.get(key))
            .cloned()
            .unwrap_or_else(|| Arc::new(build()));
        self.next.clips.insert(key.into(), Arc::clone(&built));
        built
    }
}

/// `mesh`, built painted `from`, painted as `paint` asks. The same mesh when
/// nothing differs; otherwise a copy with every vertex recoloured. A vertex
/// of exactly `from` takes the new colour; one whose alpha a shadow band
/// scaled keeps that scale.
pub(super) fn recolored(mesh: &Arc<PathMesh>, from: [f32; 4], paint: &MeshPaint) -> Arc<PathMesh> {
    let bits = |c: [f32; 4]| c.map(f32::to_bits);
    let same_color = bits(from) == bits(paint.color);
    let same_gradient = match (&mesh.gradient, &paint.gradient) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    };
    let same_pattern = match (&mesh.pattern, &paint.pattern) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    };
    if same_color && same_gradient && same_pattern && mesh.paint_color == paint.paint_color {
        return Arc::clone(mesh);
    }
    let mut out = PathMesh::clone(mesh);
    if !same_color {
        let to = paint.color;
        let scale = if from[3] > 0.0 { to[3] / from[3] } else { 0.0 };
        for vertex in &mut out.vertices {
            vertex.color = if bits(vertex.color) == bits(from) {
                to
            } else {
                [to[0], to[1], to[2], vertex.color[3] * scale]
            };
        }
    }
    out.paint_color = paint.paint_color;
    out.gradient = paint.gradient.clone();
    out.pattern = paint.pattern.clone();
    Arc::new(out)
}

/// Key tags of the mesh-producing ops.
pub(super) const KEY_FILL: u32 = 1;
pub(super) const KEY_STROKE: u32 = 2;
pub(super) const KEY_SHADOW: u32 = 3;
pub(super) const KEY_BOX_SHADOW: u32 = 4;
pub(super) const KEY_BOX_FILL: u32 = 5;
pub(super) const KEY_BOX_BORDER: u32 = 6;
pub(super) const KEY_CLIP: u32 = 7;

/// What the painter's state says about the op being built.
#[derive(Clone, Copy)]
pub(super) struct OpState {
    transform: Affine,
    opacity: f32,
}

/// `masked`: the node has a `mask-image` its painter output takes. Then
/// every fill, stroke and shadow is a mesh, which the path shader masks —
/// none takes the quad shortcut, whose shader evaluates a mask over the
/// quad's own box — and glyphs, icons and images, which no shader of theirs
/// masks, each go into a layer the mask is kept on.
pub(super) fn build_ops(ops: &[PaintOp], masked: bool, reuse: &mut MeshReuse) -> Vec<BuiltOp> {
    let mut built = Vec::with_capacity(ops.len());
    let mut stack: Vec<ActiveClip> = Vec::new();
    let no_clips: Arc<[LocalClip]> = Arc::from(Vec::new());
    let mut state = OpState {
        transform: AFFINE_IDENTITY,
        opacity: 1.0,
    };
    let mut implicit_root_layer = false;
    let mut user_depth: usize = 0;
    let isolate_phase_root = |built: &mut Vec<BuiltOp>, implicit_root_layer: &mut bool| {
        if !*implicit_root_layer {
            // Wrap top-level phase content in an isolated layer so a
            // standalone `push_mask` or top-level Porter-Duff op only
            // masks/clears this painter's own layer rather than the outer backdrop.
            *implicit_root_layer = true;
            built.insert(
                0,
                BuiltOp::LayerBegin {
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                    clip: None,
                    mask: None,
                },
            );
        }
    };
    for op in ops {
        let clip = stack.last();
        let t = state.transform;
        let alpha = state.opacity;
        match op {
            PaintOp::SetTransform(transform) => state.transform = *transform,
            PaintOp::SetOpacity(opacity) => state.opacity = *opacity,
            PaintOp::PushLayer { opacity, blend } => {
                if user_depth == 0 && blend.is_porter_duff() {
                    isolate_phase_root(&mut built, &mut implicit_root_layer);
                }
                user_depth += 1;
                built.push(BuiltOp::LayerBegin {
                    opacity: *opacity,
                    blend: *blend,
                    clip: None,
                    mask: None,
                });
            }
            PaintOp::PopLayer => {
                user_depth = user_depth.saturating_sub(1);
                built.push(BuiltOp::LayerEnd { mask: None });
            }
            PaintOp::PushMask { kind } => {
                if user_depth == 0 {
                    isolate_phase_root(&mut built, &mut implicit_root_layer);
                }
                user_depth += 1;
                built.push(BuiltOp::LayerBegin {
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                    clip: None,
                    mask: Some(*kind),
                });
            }
            PaintOp::PopMask => {
                user_depth = user_depth.saturating_sub(1);
                built.push(BuiltOp::LayerEnd { mask: None });
            }
            PaintOp::PushClip { path } => {
                reuse.key.clear();
                if let Some(outer) = clip {
                    reuse.key.extend_from_slice(&outer.key);
                }
                reuse.key.push(KEY_CLIP);
                reuse.affine(t);
                // Whether the path was drawn as a (rounded) rectangle decides
                // whether the GPU can clip to it exactly.
                match path.as_rounded_rect_corners() {
                    None => reuse.key.push(0),
                    Some((rect, radii)) => {
                        reuse.key.push(1);
                        reuse.words(&[rect.x, rect.y, rect.width, rect.height]);
                        reuse.words(&radii);
                    }
                }
                reuse.path(path);
                let key: Arc<[u32]> = reuse.key.as_slice().into();
                let built = reuse.clip(&key, || {
                    let own = fill_shapes(path, t);
                    let exact = local_clip(path, &own, t);
                    let shapes = match clip {
                        Some(outer) => intersect(&own, &outer.built.shapes),
                        None => own,
                    };
                    let local = match (clip.map(|outer| outer.built.local.clone()), exact) {
                        (None, Some(exact)) => Some(Arc::from(vec![exact])),
                        (Some(Some(outer)), Some(exact)) => {
                            let mut chain = outer.to_vec();
                            chain.push(exact);
                            Some(chain.into())
                        }
                        _ => None,
                    };
                    BuiltClip {
                        triangles: triangulate(&shapes),
                        shapes,
                        local,
                        erase: OnceLock::new(),
                    }
                });
                stack.push(ActiveClip { key, built });
            }
            PaintOp::PopClip => {
                stack.pop();
            }
            PaintOp::FillPath { path, paint } => {
                // A solid (rounded) rectangle is a quad: nothing to
                // triangulate, and a handful of instance bytes a frame instead
                // of a mesh's worth of vertices.
                if !masked
                    && clip.is_none()
                    && let Some(color) = paint_srgb(paint)
                    && let Some((rect, radii)) = quad_shape(path)
                {
                    built.push(BuiltOp::Quad {
                        rect,
                        radii,
                        fill: Some(fade(color, alpha)),
                        fill_color_space: paint_authoring_color(paint, alpha),
                        border: None,
                        border_color_space: None,
                        shadow: None,
                        shadow_solid: false,
                        transform: t,
                    });
                    continue;
                }
                reuse.begin(KEY_FILL, t, clip);
                reuse.path(path);
                push_fill(&mut built, reuse, paint, state, clip, || {
                    fill_shapes(path, t)
                });
            }
            PaintOp::StrokePath {
                path,
                paint,
                stroke,
            } => {
                // Likewise a solid, undashed stroke of one: a border drawn on
                // the rectangle grown by half the width.
                if !masked
                    && clip.is_none()
                    && let Some(color) = paint_srgb(paint)
                    && let Some((rect, radii)) = quad_shape(path)
                    && let Some(outer) = stroke_quad_radii(radii, stroke)
                {
                    let half = stroke.width * 0.5;
                    built.push(BuiltOp::Quad {
                        rect: outset(rect, half),
                        radii: outer,
                        fill: None,
                        fill_color_space: None,
                        border: Some((fade(color, alpha), stroke.width)),
                        border_color_space: paint_authoring_color(paint, alpha),
                        shadow: None,
                        shadow_solid: false,
                        transform: t,
                    });
                    continue;
                }
                // Thin solid undashed strokes route to GPU capsule segments
                // without i_overlay / lyon polygon tessellation (#222).
                let scale = affine_scale(t).max(1e-3);
                let screen_width = stroke.width * scale;
                if !masked
                    && stroke.width.is_finite()
                    && stroke.width > 0.0
                    && screen_width <= 1.5
                    && stroke.cap != LineCap::Square
                    && dash_pattern(&stroke.dash).is_none()
                    && matches!(paint, ResolvedPaint::Solid(_) | ResolvedPaint::Authoring(_))
                    && let Some(color) = paint_srgb(paint)
                    && let Some(clips_for_op) =
                        clip.map_or(Some(Arc::clone(&no_clips)), |c| c.built.local.clone())
                    && path_is_drawable(path, t)
                {
                    let tol = TOLERANCE.max(0.25) / scale;
                    let cap = match stroke.cap {
                        LineCap::Round => StrokeCap::Round,
                        _ => StrokeCap::Butt,
                    };
                    let subpaths = flatten(path, AFFINE_IDENTITY, tol);
                    if subpaths.len() == 1 {
                        let (mut pts, closed) = subpaths.into_iter().next().expect("len == 1");
                        if closed && pts.len() >= 2 && pts.first() != pts.last() {
                            pts.push(pts[0]);
                        }
                        if pts.len() >= 2 {
                            built.push(BuiltOp::Stroke {
                                points: pts
                                    .into_iter()
                                    .map(|[x, y]| [x as f32, y as f32])
                                    .collect(),
                                width: stroke.width.min(MAX_LENGTH),
                                color: fade(color, alpha),
                                cap,
                                clips: clips_for_op,
                                transform: t,
                            });
                        }
                        continue;
                    }
                }
                reuse.begin(KEY_STROKE, t, clip);
                reuse.words(&[stroke.width, stroke.miter_limit, stroke.dash_offset]);
                reuse.key.push(stroke.join as u32);
                reuse.key.push(stroke.cap as u32);
                reuse.key.push(stroke.dash.len() as u32);
                reuse.words(&stroke.dash);
                reuse.path(path);
                push_fill(&mut built, reuse, paint, state, clip, || {
                    stroke_shapes(path, stroke, t)
                });
            }
            PaintOp::Shadow { path, shadow } => {
                if !masked
                    && clip.is_none()
                    && path_is_drawable(path, t)
                    && let Some((rect, radii)) = quad_shape(path)
                    && let Some(quad_shadow) = similarity_shadow(*shadow, t, alpha)
                {
                    if quad_shadow.color.to_srgb()[3] > 0.0 {
                        built.push(BuiltOp::Quad {
                            rect,
                            radii,
                            fill: None,
                            fill_color_space: None,
                            border: None,
                            border_color_space: None,
                            shadow: Some(quad_shadow),
                            shadow_solid: true,
                            transform: t,
                        });
                    }
                    continue;
                }
                reuse.begin(KEY_SHADOW, t, clip);
                reuse.path(path);
                push_shadow(&mut built, reuse, shadow, alpha, clip, || {
                    fill_shapes(path, t)
                });
            }
            PaintOp::RoundedRect {
                rect,
                radii,
                fill,
                border,
                border_paint,
                shadow,
            } => {
                let rect = scene_rect(*rect);
                let fill_color_space = fill
                    .as_ref()
                    .and_then(|paint| paint_authoring_color(paint, alpha));
                let border_color_space = border_paint.map(|(color, _)| match color {
                    nana_ui_runtime::PaintColor::Authoring(color) => {
                        scale_paint_alpha(color, alpha)
                    }
                    _ => unreachable!(),
                });
                let solid = match fill {
                    None => Some(None),
                    Some(paint) => paint_srgb(paint).map(|color| Some(fade(color, alpha))),
                };
                if !masked
                    && clip.is_none()
                    && let Some(fill_solid) = solid
                {
                    if shadow.is_none() {
                        built.push(BuiltOp::Quad {
                            rect,
                            radii: *radii,
                            fill: fill_solid,
                            fill_color_space,
                            border: border.map(|(color, width)| (fade(color, alpha), width)),
                            border_color_space,
                            shadow: None,
                            shadow_solid: false,
                            transform: t,
                        });
                        continue;
                    }
                    if let Some(s) = shadow
                        && let Some(quad_shadow) = similarity_shadow(*s, t, alpha)
                    {
                        built.push(BuiltOp::Quad {
                            rect,
                            radii: *radii,
                            fill: fill_solid,
                            fill_color_space,
                            border: border.map(|(color, width)| (fade(color, alpha), width)),
                            border_color_space,
                            shadow: (quad_shadow.color.to_srgb()[3] > 0.0).then_some(quad_shadow),
                            shadow_solid: false,
                            transform: t,
                        });
                        continue;
                    }
                }
                let outline = OnceCell::new();
                let outline = || outline.get_or_init(|| rounded_shapes(rect, *radii, t));
                let key_box = |reuse: &mut MeshReuse, tag: u32| {
                    reuse.begin(tag, t, clip);
                    reuse.words(&[rect.x, rect.y, rect.width, rect.height]);
                    reuse.words(radii);
                };
                if let Some(shadow) = shadow {
                    key_box(reuse, KEY_BOX_SHADOW);
                    push_shadow(&mut built, reuse, shadow, alpha, clip, || outline().clone());
                }
                if let Some(fill) = fill {
                    key_box(reuse, KEY_BOX_FILL);
                    push_fill(&mut built, reuse, fill, state, clip, || outline().clone());
                }
                if let Some((color, width)) = border {
                    let ring = || {
                        let inner = rounded_shapes(
                            SceneRect {
                                x: rect.x + width,
                                y: rect.y + width,
                                width: (rect.width - width * 2.0).max(0.0),
                                height: (rect.height - width * 2.0).max(0.0),
                            },
                            radii.map(|radius| (radius - width).max(0.0)),
                            t,
                        );
                        difference_shapes(outline(), &inner)
                    };
                    let paint =
                        border_paint.map_or(
                            ResolvedPaint::Solid(*color),
                            |(color, _)| match color {
                                nana_ui_runtime::PaintColor::Authoring(color) => {
                                    ResolvedPaint::Authoring(color)
                                }
                                _ => unreachable!("border_paint only stores authoring colors"),
                            },
                        );
                    key_box(reuse, KEY_BOX_BORDER);
                    reuse.word(*width);
                    push_fill(&mut built, reuse, &paint, state, clip, ring);
                }
            }
            PaintOp::Image {
                rect,
                source,
                fit,
                sampling,
                radii,
                src_rect,
                nine_slice,
                nine_slice_fill,
            } => {
                let rect = scene_rect(*rect);
                let start = built.len();
                clipped(&mut built, clip, &no_clips, |clips| BuiltOp::Image {
                    rect,
                    source: Arc::clone(source),
                    fit: *fit,
                    sampling: *sampling,
                    radii: *radii,
                    src_rect: src_rect.map(scene_rect),
                    nine_slice: *nine_slice,
                    nine_slice_fill: *nine_slice_fill,
                    opacity: alpha,
                    clips,
                    transform: t,
                });
                if masked {
                    keep_mask(&mut built, start, local_area(rect, 1.0, t));
                }
            }
            PaintOp::TextureRegionBatch {
                source,
                sampling,
                items,
            } => {
                let start = built.len();
                let mut bounds: Option<SceneRect> = None;
                for (src_rect, dest_rect) in items {
                    let rect = scene_rect(*dest_rect);
                    bounds = Some(match bounds {
                        Some(b) => super::union_rect(&[b, rect]),
                        None => rect,
                    });
                    clipped(&mut built, clip, &no_clips, |clips| BuiltOp::Image {
                        rect,
                        source: Arc::clone(source),
                        fit: ImageFit::Fill,
                        sampling: *sampling,
                        radii: [0.0; 4],
                        src_rect: Some(scene_rect(*src_rect)),
                        nine_slice: None,
                        nine_slice_fill: true,
                        opacity: alpha,
                        clips,
                        transform: t,
                    });
                }
                if masked && let Some(area_rect) = bounds {
                    keep_mask(&mut built, start, local_area(area_rect, 1.0, t));
                }
            }
            PaintOp::FillImage {
                path,
                rect,
                source,
                fit,
                sampling,
            } => {
                let shapes = fill_shapes(path, t);
                let shapes = match clip {
                    Some(outer) => intersect(&shapes, &outer.built.shapes),
                    None => shapes,
                };
                let Some(area) = shapes_bounds(&shapes) else {
                    continue;
                };
                let area = outset(area, 1.0);
                let start = built.len();
                built.push(BuiltOp::LayerBegin {
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                    clip: Some(area),
                    mask: None,
                });
                built.push(BuiltOp::Image {
                    rect: scene_rect(*rect),
                    source: Arc::clone(source),
                    fit: *fit,
                    sampling: *sampling,
                    radii: [0.0; 4],
                    src_rect: None,
                    nine_slice: None,
                    nine_slice_fill: true,
                    opacity: alpha,
                    clips: Arc::clone(&no_clips),
                    transform: t,
                });
                built.push(BuiltOp::LayerEnd {
                    mask: erase_outside(area, &shapes).map(|mesh| (mesh, LayerMaskMode::Erase)),
                });
                if masked {
                    keep_mask(&mut built, start, Some(area));
                }
            }
            PaintOp::Text {
                rect,
                content,
                paint,
                size,
                weight,
                family,
                italic,
                line_height,
                letter_spacing,
                wrap,
                max_lines,
                horizontal,
                vertical,
                effects,
            } => {
                let rect = scene_rect(*rect);
                let make_text = |rich: Option<Arc<SceneRichPaint>>| {
                    Arc::new(TextStyle {
                        content: Arc::clone(content),
                        size: *size,
                        weight: *weight,
                        family: family.clone(),
                        italic: *italic,
                        line_height: *line_height,
                        letter_spacing: *letter_spacing,
                        wrap: *wrap,
                        max_lines: *max_lines,
                        horizontal: *horizontal,
                        vertical: *vertical,
                        effects: rich,
                    })
                };
                let paint_is_gradient = matches!(
                    paint,
                    ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_)
                );
                let stroke_is_gradient = effects
                    .as_ref()
                    .and_then(|e| e.stroke.as_ref())
                    .is_some_and(|s| {
                        matches!(
                            s.paint,
                            ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_)
                        )
                    });

                let resolve_solid_stroke = |stroke: &nana_ui_runtime::ResolvedPaintTextStroke| {
                    let color = match &stroke.paint {
                        ResolvedPaint::Solid(rgba) => {
                            Some(nana_ui_core::PaintColor::srgb(fade(*rgba, alpha)))
                        }
                        ResolvedPaint::Authoring(c) => Some(scale_paint_alpha(*c, alpha)),
                        ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_) => None,
                    }?;
                    Some(nana_ui_core::RichTextStroke {
                        width_px: stroke.width,
                        color,
                        join: stroke.join,
                        placement: stroke.placement,
                    })
                };
                let resolve_shadows = |shadows: &[nana_ui_core::RichTextShadow]| {
                    let list: Vec<_> = shadows
                        .iter()
                        .map(|s| nana_ui_core::RichTextShadow {
                            color: scale_paint_alpha(s.color, alpha),
                            ..*s
                        })
                        .collect();
                    Arc::<[nana_ui_core::RichTextShadow]>::from(list)
                };
                let resolve_deco = |deco: &nana_ui_runtime::ResolvedPaintTextDecoration| {
                    (
                        deco.line,
                        deco.color.map(|c| scale_paint_alpha(c, alpha)),
                        deco.style,
                        deco.thickness,
                    )
                };

                let base_effects = effects.as_ref().map(|e| {
                    let (decoration, decoration_color, decoration_style, decoration_thickness_px) =
                        e.decoration.as_ref().map_or(
                            (
                                nana_ui_core::TextDecorationLine::default(),
                                None,
                                nana_ui_core::TextDecorationStyle::default(),
                                None,
                            ),
                            resolve_deco,
                        );
                    SceneTextEffects {
                        material: None,
                        decoration,
                        decoration_color,
                        decoration_style,
                        decoration_thickness_px,
                        stroke: e.stroke.as_ref().and_then(resolve_solid_stroke),
                        shadows: resolve_shadows(&e.shadows),
                        emphasis_color: None,
                    }
                });
                let reach = (size * 0.5).max(
                    base_effects.as_ref().map_or(0.0, |fx| fx.reach()).max(
                        effects
                            .as_ref()
                            .and_then(|e| e.stroke.as_ref())
                            .map_or(0.0, |s| s.width * 0.5),
                    ),
                );
                let start = built.len();
                if !paint_is_gradient && !stroke_is_gradient {
                    let rich = base_effects.and_then(|fx| SceneRichPaint::new(fx, Vec::new()));
                    let text = make_text(rich);
                    tinted(
                        &mut built,
                        paint,
                        rect,
                        reach,
                        state,
                        |color, paint_color| {
                            clipped_op(clip, &no_clips, |clips| BuiltOp::Text {
                                rect,
                                text: Arc::clone(&text),
                                color,
                                paint_color,
                                clips,
                                transform: t,
                            })
                        },
                    );
                } else if let Some(eff) = effects.as_ref() {
                    let emit_plain =
                        |built: &mut Vec<BuiltOp>, rich: SceneTextEffects, p: &ResolvedPaint| {
                            let Some(rich) = SceneRichPaint::new(rich, Vec::new()) else {
                                return;
                            };
                            let text = make_text(Some(rich));
                            tinted(built, p, rect, reach, state, |color, paint_color| {
                                clipped_op(clip, &no_clips, |clips| BuiltOp::Text {
                                    rect,
                                    text: Arc::clone(&text),
                                    color,
                                    paint_color,
                                    clips,
                                    transform: t,
                                })
                            });
                        };
                    // 1. Shadows (never tinted by text fill gradient).
                    if !eff.shadows.is_empty() {
                        emit_plain(
                            &mut built,
                            SceneTextEffects {
                                shadows: resolve_shadows(&eff.shadows),
                                ..SceneTextEffects::default()
                            },
                            &ResolvedPaint::Solid([0.0; 4]),
                        );
                    }
                    // 2. Under-stroke.
                    if let Some(stroke) = eff
                        .stroke
                        .as_ref()
                        .filter(|s| s.placement == nana_ui_core::TextStrokePlacement::Under)
                    {
                        let white_stroke = nana_ui_core::RichTextStroke {
                            width_px: stroke.width,
                            color: nana_ui_core::PaintColor::srgb([1.0, 1.0, 1.0, 1.0]),
                            join: stroke.join,
                            placement: stroke.placement,
                        };
                        let fx = SceneTextEffects {
                            stroke: Some(resolve_solid_stroke(stroke).unwrap_or(white_stroke)),
                            ..SceneTextEffects::default()
                        };
                        let stroke_paint = if matches!(
                            stroke.paint,
                            ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_)
                        ) {
                            &stroke.paint
                        } else {
                            &ResolvedPaint::Solid([0.0; 4])
                        };
                        emit_plain(&mut built, fx, stroke_paint);
                    }
                    // 3. Fill (+ decoration if decoration inherits fill).
                    let deco_uses_fill = eff.decoration.as_ref().is_some_and(|d| d.color.is_none());
                    let fill_rich = if deco_uses_fill {
                        eff.decoration.as_ref().and_then(|d| {
                            let (
                                decoration,
                                decoration_color,
                                decoration_style,
                                decoration_thickness_px,
                            ) = resolve_deco(d);
                            SceneRichPaint::new(
                                SceneTextEffects {
                                    decoration,
                                    decoration_color,
                                    decoration_style,
                                    decoration_thickness_px,
                                    ..SceneTextEffects::default()
                                },
                                Vec::new(),
                            )
                        })
                    } else {
                        None
                    };
                    let fill_text = make_text(fill_rich);
                    tinted(
                        &mut built,
                        paint,
                        rect,
                        reach,
                        state,
                        |color, paint_color| {
                            clipped_op(clip, &no_clips, |clips| BuiltOp::Text {
                                rect,
                                text: Arc::clone(&fill_text),
                                color,
                                paint_color,
                                clips,
                                transform: t,
                            })
                        },
                    );
                    // 4. Over-stroke.
                    if let Some(stroke) = eff
                        .stroke
                        .as_ref()
                        .filter(|s| s.placement == nana_ui_core::TextStrokePlacement::Over)
                    {
                        let white_stroke = nana_ui_core::RichTextStroke {
                            width_px: stroke.width,
                            color: nana_ui_core::PaintColor::srgb([1.0, 1.0, 1.0, 1.0]),
                            join: stroke.join,
                            placement: stroke.placement,
                        };
                        let fx = SceneTextEffects {
                            stroke: Some(resolve_solid_stroke(stroke).unwrap_or(white_stroke)),
                            ..SceneTextEffects::default()
                        };
                        let stroke_paint = if matches!(
                            stroke.paint,
                            ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_)
                        ) {
                            &stroke.paint
                        } else {
                            &ResolvedPaint::Solid([0.0; 4])
                        };
                        emit_plain(&mut built, fx, stroke_paint);
                    }
                    // 5. Explicit-color decoration.
                    if let Some(deco) = eff.decoration.as_ref().filter(|d| d.color.is_some()) {
                        let (
                            decoration,
                            decoration_color,
                            decoration_style,
                            decoration_thickness_px,
                        ) = resolve_deco(deco);
                        emit_plain(
                            &mut built,
                            SceneTextEffects {
                                decoration,
                                decoration_color,
                                decoration_style,
                                decoration_thickness_px,
                                ..SceneTextEffects::default()
                            },
                            &ResolvedPaint::Solid([0.0; 4]),
                        );
                    }
                } else {
                    let text = make_text(None);
                    tinted(
                        &mut built,
                        paint,
                        rect,
                        reach,
                        state,
                        |color, paint_color| {
                            clipped_op(clip, &no_clips, |clips| BuiltOp::Text {
                                rect,
                                text: Arc::clone(&text),
                                color,
                                paint_color,
                                clips,
                                transform: t,
                            })
                        },
                    );
                }
                if masked {
                    keep_mask(&mut built, start, local_area(rect, reach, t));
                }
            }
            PaintOp::Icon { rect, icon, paint } => {
                let rect = scene_rect(*rect);
                let start = built.len();
                tinted(&mut built, paint, rect, 1.0, state, |color, paint_color| {
                    clipped_op(clip, &no_clips, |clips| BuiltOp::Icon {
                        rect,
                        icon: *icon,
                        color,
                        paint_color,
                        clips,
                        transform: t,
                    })
                });
                if masked {
                    keep_mask(&mut built, start, local_area(rect, 1.0, t));
                }
            }
            PaintOp::DrawDefault => {
                let ops = clipped_op(clip, &no_clips, |clips| BuiltOp::Default {
                    clips,
                    transform: t,
                    opacity: alpha,
                });
                if ops.is_empty() {
                    // An empty clip hides the default visual, but it still has
                    // to be placed: the node emits it either way, and what
                    // follows draws over it.
                    built.push(BuiltOp::Default {
                        clips: Arc::clone(&no_clips),
                        transform: t,
                        opacity: 0.0,
                    });
                } else {
                    built.extend(ops);
                }
            }
        }
    }
    if implicit_root_layer {
        built.push(BuiltOp::LayerEnd { mask: None });
    }
    run_quads(built)
}

/// Fold each stretch of two or more solid quads that can share a primitive
/// into a [`BuiltOp::QuadRun`]: a canvas of thousands of boxes then costs
/// the scene and the painter one primitive, not one each. Order is kept —
/// a run only ever holds quads that were adjacent.

/// Maps a node-local shadow into local quad space when `t` is a similarity
/// transform (uniform scale + rotation/reflection). Returns `None` under
/// non-uniform scale or shear, or when any shadow length is non-finite.
fn similarity_shadow(
    shadow: ComponentElevation,
    t: Affine,
    alpha: f32,
) -> Option<ComponentElevation> {
    let lengths = [
        shadow.offset_x,
        shadow.offset_y,
        shadow.blur_radius,
        shadow.spread_radius,
    ];
    if lengths.iter().any(|l| !l.is_finite()) || !alpha.is_finite() {
        return None;
    }
    let [a, b, c, d, _, _] = t;
    let len_x2 = a * a + b * b;
    let len_y2 = c * c + d * d;
    let dot = a * c + b * d;
    let det = a * d - b * c;
    if !det.is_finite()
        || det.abs() <= 1e-6
        || (len_x2 - len_y2).abs() > 1e-4 * (len_x2 + len_y2 + 1.0)
        || dot.abs() > 1e-4 * (len_x2 + 1.0)
    {
        return None;
    }
    let s = len_x2.sqrt();
    if s <= 1e-6 {
        return None;
    }
    let inv_ox = (d * shadow.offset_x - c * shadow.offset_y) / det;
    let inv_oy = (a * shadow.offset_y - b * shadow.offset_x) / det;
    Some(ComponentElevation {
        color: scale_paint_alpha(shadow.color, alpha),
        offset_x: inv_ox,
        offset_y: inv_oy,
        blur_radius: (shadow.blur_radius.min(MAX_LENGTH) / s).max(0.0),
        spread_radius: shadow.spread_radius.clamp(-MAX_LENGTH, MAX_LENGTH) / s,
        inset: shadow.inset,
    })
}

/// Straight RGBA with its alpha scaled.
pub(super) fn fade([r, g, b, a]: [f32; 4], alpha: f32) -> [f32; 4] {
    [r, g, b, a * alpha]
}

pub(super) fn outset(rect: SceneRect, by: f32) -> SceneRect {
    SceneRect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + by * 2.0,
        height: rect.height + by * 2.0,
    }
}

/// An op the CPU cannot cut, under the active clip: with the exact GPU clips
/// when there are some, or wrapped in a layer masked to the clip otherwise.
pub(super) fn clipped(
    built: &mut Vec<BuiltOp>,
    clip: Option<&ActiveClip>,
    no_clips: &Arc<[LocalClip]>,
    op: impl FnOnce(Arc<[LocalClip]>) -> BuiltOp,
) {
    built.extend(clipped_op(clip, no_clips, op));
}

pub(super) fn clipped_op(
    clip: Option<&ActiveClip>,
    no_clips: &Arc<[LocalClip]>,
    op: impl FnOnce(Arc<[LocalClip]>) -> BuiltOp,
) -> Vec<BuiltOp> {
    let Some(clip) = clip else {
        return vec![op(Arc::clone(no_clips))];
    };
    if let Some(local) = &clip.built.local {
        return vec![op(Arc::clone(local))];
    }
    let Some(area) = shapes_bounds(&clip.built.shapes) else {
        // An empty clip shows nothing.
        return Vec::new();
    };
    let area = outset(area, 1.0);
    vec![
        BuiltOp::LayerBegin {
            opacity: 1.0,
            blend: BlendMode::Normal,
            clip: Some(area),
            mask: None,
        },
        op(Arc::clone(no_clips)),
        BuiltOp::LayerEnd {
            mask: clip
                .built
                .erase
                .get_or_init(|| erase_outside(area, &clip.built.shapes))
                .clone()
                .map(|mesh| (mesh, LayerMaskMode::Erase)),
        },
    ]
}

/// Glyph-like content in `paint`: a solid colour goes straight onto it; a
/// gradient paints the content white in a layer, then tints it.
pub(super) fn tinted(
    built: &mut Vec<BuiltOp>,
    paint: &ResolvedPaint,
    rect: SceneRect,
    reach: f32,
    state: OpState,
    content: impl FnOnce([f32; 4], Option<nana_ui_core::PaintColor>) -> Vec<BuiltOp>,
) {
    match paint {
        ResolvedPaint::Solid(color) => built.extend(content(fade(*color, state.opacity), None)),
        ResolvedPaint::Authoring(color) => built.extend(content(
            fade(color.to_srgb(), state.opacity),
            Some(scale_paint_alpha(*color, state.opacity)),
        )),
        ResolvedPaint::Gradient(gradient) => {
            let over = outset(rect, reach);
            let mut area = PaintPath::new();
            area.rect(LayoutBox {
                x: over.x,
                y: over.y,
                width: over.width,
                height: over.height,
            });
            let mask = gradient_mesh(fill_shapes(&area, state.transform), gradient, state);
            built.push(BuiltOp::LayerBegin {
                opacity: 1.0,
                blend: BlendMode::Normal,
                clip: None,
                mask: None,
            });
            built.extend(content([1.0; 4], None));
            built.push(BuiltOp::LayerEnd {
                mask: mask.map(|mesh| (mesh, LayerMaskMode::Tint)),
            });
        }
        ResolvedPaint::Pattern(pattern) => {
            let over = outset(rect, reach);
            let mut area = PaintPath::new();
            area.rect(LayoutBox {
                x: over.x,
                y: over.y,
                width: over.width,
                height: over.height,
            });
            let mask = pattern_mesh(fill_shapes(&area, state.transform), pattern, state);
            built.push(BuiltOp::LayerBegin {
                opacity: 1.0,
                blend: BlendMode::Normal,
                clip: None,
                mask: None,
            });
            built.extend(content([1.0; 4], None));
            built.push(BuiltOp::LayerEnd {
                mask: mask.map(|mesh| (mesh, LayerMaskMode::Tint)),
            });
        }
    }
}

/// Under a node mask, put what was built from `start` on into a layer cut to
/// `area` (node-local px), and keep the mask on that layer as it closes.
/// `area` must hold everything those ops draw: what falls outside is cut.
pub(super) fn keep_mask(built: &mut Vec<BuiltOp>, start: usize, area: Option<SceneRect>) {
    let Some(area) = area else {
        return;
    };
    if built.len() == start {
        return;
    }
    let mut mesh = MeshBuilder::default();
    mesh.fill(&rect_shapes(area), [1.0; 4]);
    let Some(mesh) = mesh.finish() else {
        return;
    };
    built.insert(
        start,
        BuiltOp::LayerBegin {
            opacity: 1.0,
            blend: BlendMode::Normal,
            clip: Some(area),
            mask: None,
        },
    );
    built.push(BuiltOp::LayerEnd {
        mask: Some((Arc::new(mesh), LayerMaskMode::Keep)),
    });
}

/// Node-local bounds of `rect` grown by `reach`, under the painter's local
/// transform `t`.
pub(super) fn local_area(rect: SceneRect, reach: f32, t: Affine) -> Option<SceneRect> {
    shapes_bounds(&rounded_shapes(outset(rect, reach), [0.0; 4], t))
}

/// Coverage of everything in `area` outside `shapes`, for erasing a layer
/// down to the clip.
pub(super) fn erase_outside(area: SceneRect, shapes: &Shapes) -> Option<Arc<PathMesh>> {
    let bounds = rect_shapes(area);
    let outside = if shapes.is_empty() {
        bounds
    } else {
        difference_shapes(&bounds, shapes)
    };
    let mut mesh = MeshBuilder::default();
    mesh.fill(&outside, [0.0, 0.0, 0.0, 1.0]);
    mesh.finish().map(Arc::new)
}

pub(super) fn rect_shapes(rect: SceneRect) -> Shapes {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return Vec::new();
    }
    let (x0, y0) = (f64::from(rect.x), f64::from(rect.y));
    let (x1, y1) = (x0 + f64::from(rect.width), y0 + f64::from(rect.height));
    simplify_contours(
        vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]],
        FillRule::NonZero,
    )
}

pub(super) fn gradient_mesh(
    shapes: Shapes,
    gradient: &Arc<ResolvedGradient>,
    state: OpState,
) -> Option<Arc<PathMesh>> {
    let mut mesh = fill_mesh(
        shapes,
        [1.0, 1.0, 1.0, state.opacity],
        Some(affine_multiply(state.transform, gradient.transform)),
        None,
    )?;
    mesh.gradient = Some(Arc::clone(gradient));
    Some(Arc::new(mesh))
}

pub(super) fn pattern_mesh(
    shapes: Shapes,
    pattern: &Arc<ResolvedPattern>,
    state: OpState,
) -> Option<Arc<PathMesh>> {
    let mut mesh = fill_mesh(
        shapes,
        [1.0, 1.0, 1.0, state.opacity],
        Some(affine_multiply(state.transform, pattern.transform)),
        None,
    )?;
    mesh.pattern = Some(Arc::clone(pattern));
    Some(Arc::new(mesh))
}

/// `shapes` cut to `clip`, filled `color`. With `gradient_space`, the
/// painter transform a gradient over the mesh was defined before.
pub(super) fn fill_mesh(
    shapes: Shapes,
    color: [f32; 4],
    gradient_space: Option<Affine>,
    clip: Option<&ActiveClip>,
) -> Option<PathMesh> {
    let shapes = match clip {
        Some(clip) => intersect(&shapes, &clip.built.shapes),
        None => shapes,
    };
    let mut mesh = MeshBuilder::default();
    mesh.fill(&shapes, color);
    if let Some(inverse) = gradient_space.and_then(affine_inverse) {
        for vertex in &mut mesh.vertices {
            vertex.paint_pos = affine_apply(inverse, vertex.position);
        }
    }
    mesh.finish()
}

/// Fill what `shapes()` returns with `paint`, cut to `clip`. The caller has
/// keyed the geometry in `reuse`; `shapes` only runs if it was not built.
pub(super) fn push_fill(
    built: &mut Vec<BuiltOp>,
    reuse: &mut MeshReuse,
    paint: &ResolvedPaint,
    state: OpState,
    clip: Option<&ActiveClip>,
    shapes: impl FnOnce() -> Shapes,
) {
    if paint.is_invisible() || state.opacity <= 0.0 {
        return;
    }
    let (paint, paint_transform) = match paint {
        ResolvedPaint::Solid(color) => (
            MeshPaint {
                color: fade(*color, state.opacity),
                paint_color: None,
                gradient: None,
                pattern: None,
            },
            None,
        ),
        ResolvedPaint::Authoring(color) => (
            MeshPaint {
                color: fade(color.to_srgb(), state.opacity),
                paint_color: Some(*color),
                gradient: None,
                pattern: None,
            },
            None,
        ),
        ResolvedPaint::Gradient(gradient) => (
            MeshPaint {
                color: [1.0, 1.0, 1.0, state.opacity],
                paint_color: None,
                gradient: Some(Arc::clone(gradient)),
                pattern: None,
            },
            Some(affine_multiply(state.transform, gradient.transform)),
        ),
        ResolvedPaint::Pattern(pattern) => (
            MeshPaint {
                color: [1.0, 1.0, 1.0, state.opacity],
                paint_color: None,
                gradient: None,
                pattern: Some(Arc::clone(pattern)),
            },
            Some(affine_multiply(state.transform, pattern.transform)),
        ),
    };
    reuse.key.push(u32::from(paint_transform.is_some()));
    if let Some(pt) = paint_transform {
        reuse.affine(pt);
    }
    if let Some(mesh) = reuse.mesh(&paint, |color| {
        fill_mesh(shapes(), color, paint_transform, clip)
    }) {
        built.push(BuiltOp::Mesh(mesh));
    }
}

pub(super) fn paint_srgb(paint: &ResolvedPaint) -> Option<[f32; 4]> {
    match paint {
        ResolvedPaint::Solid(color) => Some(*color),
        ResolvedPaint::Authoring(color) => Some(color.to_srgb()),
        ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_) => None,
    }
}

pub(super) fn paint_authoring_color(
    paint: &ResolvedPaint,
    opacity: f32,
) -> Option<nana_ui_core::PaintColor> {
    match paint {
        ResolvedPaint::Authoring(color) => Some(scale_paint_alpha(*color, opacity)),
        ResolvedPaint::Solid(_) | ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_) => None,
    }
}

pub(super) fn scale_paint_alpha(
    color: nana_ui_core::PaintColor,
    opacity: f32,
) -> nana_ui_core::PaintColor {
    match color {
        nana_ui_core::PaintColor::Srgb { mut rgba } => {
            rgba[3] *= opacity;
            nana_ui_core::PaintColor::Srgb { rgba }
        }
        nana_ui_core::PaintColor::Oklch { l, c, h, alpha } => nana_ui_core::PaintColor::Oklch {
            l,
            c,
            h,
            alpha: alpha * opacity,
        },
        nana_ui_core::PaintColor::Hsv { h, s, v, alpha } => nana_ui_core::PaintColor::Hsv {
            h,
            s,
            v,
            alpha: alpha * opacity,
        },
        nana_ui_core::PaintColor::LinearScRgb { channels, alpha } => {
            nana_ui_core::PaintColor::LinearScRgb {
                channels,
                alpha: alpha * opacity,
            }
        }
        other => scale_paint_alpha(
            other.resolve(nana_ui_core::StyleModelRef::default()),
            opacity,
        ),
    }
}

/// The shadow `shadow` casts from what `shapes()` returns, cut to `clip`.
/// The caller has keyed the outline in `reuse`; `shapes` only runs if it
/// was not built.
pub(super) fn push_shadow(
    built: &mut Vec<BuiltOp>,
    reuse: &mut MeshReuse,
    shadow: &ComponentElevation,
    alpha: f32,
    clip: Option<&ActiveClip>,
    shapes: impl FnOnce() -> Shapes,
) {
    // Mesh vertices carry straight sRGB and are linearized once by the GPU
    // path uploader; the shadow's own colour rides on the finished mesh for
    // a direct linear scRGB upload.
    let color = fade(shadow.color.to_srgb(), alpha);
    let lengths = [
        shadow.offset_x,
        shadow.offset_y,
        shadow.blur_radius,
        shadow.spread_radius,
    ];
    if color[3].is_nan() || color[3] <= 0.0 || lengths.iter().any(|length| !length.is_finite()) {
        return;
    }
    reuse.words(&lengths);
    reuse.key.push(u32::from(shadow.inset));
    let paint = MeshPaint {
        color,
        paint_color: Some(shadow.color),
        gradient: None,
        pattern: None,
    };
    let mesh = reuse.mesh(&paint, |color| {
        // Past a screen's reach a blur or spread only costs geometry.
        let shadow = ComponentElevation {
            color: nana_ui_core::PaintColor::srgb(color),
            blur_radius: shadow.blur_radius.min(MAX_LENGTH),
            spread_radius: shadow.spread_radius.clamp(-MAX_LENGTH, MAX_LENGTH),
            ..*shadow
        };
        let mut mesh = MeshBuilder::default();
        if shadow.inset {
            mesh.inset_shadow(shapes(), &shadow);
        } else {
            mesh.shadow(shapes(), &shadow, true);
        }
        if let Some(clip) = clip {
            mesh.clip_to(&clip.built.triangles);
        }
        mesh.finish()
    });
    if let Some(mesh) = mesh {
        built.push(BuiltOp::Mesh(mesh));
    }
}

#[derive(Default)]
pub(super) struct MeshBuilder {
    pub(super) vertices: Vec<PathVertex>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn vertex(
        &mut self,
        position: [f32; 2],
        extrude: [f32; 2],
        coverage: f32,
        color: [f32; 4],
    ) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(PathVertex {
            position,
            extrude,
            coverage,
            color,
            paint_pos: position,
        });
        index
    }

    /// Solid interior plus a one-physical-pixel fringe centred on every edge,
    /// as every other edge ramps: its inner ring, where the interior ends, half
    /// a pixel inside the outline and its outer ring half a pixel outside. An
    /// `extrude` of a miter puts a vertex half a physical pixel along it.
    pub(super) fn fill(&mut self, shapes: &Shapes, color: [f32; 4]) {
        if color[3] <= 0.0 {
            return;
        }
        for shape in shapes {
            let Some(Tessellation {
                vertices,
                indices,
                outline,
            }) = tessellate(shape.iter())
            else {
                continue;
            };
            let sign = shape_outward_sign(shape);
            let rings: Vec<_> = shape
                .iter()
                .map(|contour| {
                    let points: Vec<[f32; 2]> =
                        contour.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                    let miters = if points.len() < 3 {
                        Vec::new()
                    } else {
                        miters(&points, sign)
                    };
                    (points, miters)
                })
                .collect();
            // The interior's outline vertices move in with the inner ring, so
            // the two meet without overlapping; a vertex the tessellator added
            // where contours cross stays put.
            let base = self.vertices.len() as u32;
            for (position, outline) in vertices.into_iter().zip(outline) {
                let extrude = outline
                    .and_then(|[contour, point]| rings[contour].1.get(point))
                    .map_or([0.0, 0.0], |miter| [-miter[0], -miter[1]]);
                self.vertex(position, extrude, 1.0, color);
            }
            self.indices
                .extend(indices.iter().map(|index| base + index));
            for (points, miters) in &rings {
                if miters.is_empty() {
                    continue;
                }
                self.band(
                    points,
                    |i| (points[i], [-miters[i][0], -miters[i][1]]),
                    |i| (points[i], miters[i]),
                    color,
                );
            }
        }
    }

    /// A strip between an inner and an outer ring, coverage 1 → 0.
    fn band(
        &mut self,
        points: &[[f32; 2]],
        inner: impl Fn(usize) -> ([f32; 2], [f32; 2]),
        outer: impl Fn(usize) -> ([f32; 2], [f32; 2]),
        color: [f32; 4],
    ) {
        let n = points.len();
        let base = self.vertices.len() as u32;
        for i in 0..n {
            let (position, extrude) = inner(i);
            self.vertex(position, extrude, 1.0, color);
            let (position, extrude) = outer(i);
            self.vertex(position, extrude, 0.0, color);
        }
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (in_i, out_i, in_j, out_j) = (
                base + 2 * i,
                base + 2 * i + 1,
                base + 2 * j,
                base + 2 * j + 1,
            );
            self.indices.extend([in_i, out_i, out_j, in_i, out_j, in_j]);
        }
    }

    /// CSS `box-shadow` along the outline: spread, offset, then a coverage
    /// band `±blur` wide around the edge.
    ///
    /// The band is a strip between an inner and an outer ring that correspond
    /// vertex for vertex, so along a straight edge every quad is an exact
    /// linear ramp in distance to the outline, which the painter's smoothstep
    /// turns into a Gaussian-like edge. Where the outline curves tighter than
    /// the blur, the ring on the inside of the curve would fold back over
    /// itself; there it stops at the local radius of curvature instead, which
    /// only sharpens that corner's falloff a little.
    /// An outer shadow of `shapes`, and the outer edge of its band: the
    /// region it reaches. `fade_thin` lowers the peak of a shape thinner
    /// than the blur, as a real Gaussian would.
    pub(super) fn shadow(
        &mut self,
        mut shapes: Shapes,
        shadow: &ComponentElevation,
        fade_thin: bool,
    ) -> Shapes {
        if shapes.is_empty() {
            return shapes;
        }
        if shadow.spread_radius.abs() > 0.01 {
            shapes = outline_shapes(shapes, shadow.spread_radius);
        }
        let offset = [f64::from(shadow.offset_x), f64::from(shadow.offset_y)];
        for point in shapes.iter_mut().flatten().flatten() {
            point[0] += offset[0];
            point[1] += offset[1];
        }
        let blur = shadow.blur_radius.max(0.0);
        if blur < 0.5 {
            self.fill(&shapes, shadow.color.to_srgb());
            return shapes;
        }
        let mut reach: Shapes = Vec::new();
        for shape in &shapes {
            let Some(bounds) = shapes_bounds(&vec![shape.clone()]) else {
                continue;
            };
            // Past half its narrow side the inner ring would turn inside out.
            let mut inset = blur.min(bounds.width.min(bounds.height) * 0.5);
            let (area, perimeter) = area_and_perimeter(shape);
            if shape.len() > 1 && perimeter > 0.0 {
                // A hole's inner ring runs toward the outline's: past half
                // the shape's thickness (2·area/perimeter for a thin one)
                // they would cross and fill between them solid.
                inset = inset.min(area / perimeter);
            }
            // A shape thinner than its blur never reaches full strength: a
            // Gaussian over a strip `t` wide peaks at erf(t / (2√2·σ)), with
            // σ = blur / 2. 4·area/perimeter is the width of a strip, disc or
            // square, and at most twice it for anything else.
            let thickness = if perimeter > 0.0 {
                bounds.width.min(bounds.height).min(4.0 * area / perimeter)
            } else {
                0.0
            };
            let peak = if fade_thin {
                erf(thickness / (std::f32::consts::SQRT_2 * blur))
            } else {
                1.0
            };
            let [r, g, b, a] = shadow.color.to_srgb();
            let color = [r, g, b, a * peak];
            let sign = shape_outward_sign(shape);
            let mut inner_rings: Vec<Contour> = Vec::new();
            let mut outer_rings: Vec<Contour> = Vec::new();
            for contour in shape {
                let points: Vec<[f32; 2]> =
                    contour.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                if points.len() < 3 {
                    continue;
                }
                let miters = miters(&points, sign);
                let reach = ring_reach(&points, &miters, inset, blur);
                let inner: Vec<[f32; 2]> = (0..points.len())
                    .map(|i| {
                        let d = reach[i].0;
                        [
                            points[i][0] - miters[i][0] * d,
                            points[i][1] - miters[i][1] * d,
                        ]
                    })
                    .collect();
                let outer: Vec<[f32; 2]> = (0..points.len())
                    .map(|i| {
                        let d = reach[i].1;
                        [
                            points[i][0] + miters[i][0] * d,
                            points[i][1] + miters[i][1] * d,
                        ]
                    })
                    .collect();
                inner_rings.push(
                    inner
                        .iter()
                        .map(|p| [f64::from(p[0]), f64::from(p[1])])
                        .collect(),
                );
                outer_rings.push(
                    outer
                        .iter()
                        .map(|p| [f64::from(p[0]), f64::from(p[1])])
                        .collect(),
                );
                self.band(
                    &points,
                    |i| (inner[i], [0.0, 0.0]),
                    |i| (outer[i], [0.0, 0.0]),
                    color,
                );
            }
            if let Some(Tessellation {
                vertices, indices, ..
            }) = tessellate(inner_rings.iter())
            {
                let base = self.vertices.len() as u32;
                for position in vertices {
                    self.vertex(position, [0.0, 0.0], 1.0, color);
                }
                self.indices
                    .extend(indices.iter().map(|index| base + index));
            }
            reach.push(outer_rings);
        }
        reach
    }

    /// CSS `box-shadow: inset` inside the outline: dark where the outline,
    /// offset and shrunk by the spread, does not reach, fading over `±blur`
    /// at its edge, and cut to the outline.
    pub(super) fn inset_shadow(&mut self, shapes: Shapes, shadow: &ComponentElevation) {
        if shapes.is_empty() {
            return;
        }
        let clip = triangulate(&shapes);
        let mut hole = shapes.clone();
        for point in hole.iter_mut().flatten().flatten() {
            point[0] += f64::from(shadow.offset_x);
            point[1] += f64::from(shadow.offset_y);
        }
        if shadow.spread_radius.abs() > 0.01 {
            hole = outline_shapes(hole, -shadow.spread_radius);
        }
        let blur = shadow.blur_radius.max(0.0);
        if blur < 0.5 || hole.is_empty() {
            let dark = if hole.is_empty() {
                shapes
            } else {
                difference_shapes(&shapes, &hole)
            };
            self.fill(&dark, shadow.color.to_srgb());
        } else {
            // The band around the hole, dark side out.
            let start = self.vertices.len();
            let plain = ComponentElevation {
                offset_x: 0.0,
                offset_y: 0.0,
                spread_radius: 0.0,
                inset: false,
                ..*shadow
            };
            // A thin hole still lets its full darkness reach the outline.
            let reach = self.shadow(hole, &plain, false);
            for vertex in &mut self.vertices[start..] {
                vertex.coverage = 1.0 - vertex.coverage;
            }
            // And solid past the band: past its actual outer edge, whose
            // corners are mitred, so the two never cover a point twice.
            let grown = reach.simplify_shape(OverlayFill::NonZero);
            let beyond = difference_shapes(&shapes, &grown);
            for shape in &beyond {
                if let Some(Tessellation {
                    vertices, indices, ..
                }) = tessellate(shape.iter())
                {
                    let base = self.vertices.len() as u32;
                    for position in vertices {
                        self.vertex(position, [0.0, 0.0], 1.0, shadow.color.to_srgb());
                    }
                    self.indices
                        .extend(indices.iter().map(|index| base + index));
                }
            }
        }
        self.clip_to(&clip);
    }

    /// Cut every triangle to the clip triangles, interpolating attributes.
    fn clip_to(&mut self, clip: &[[[f32; 2]; 3]]) {
        let vertices = std::mem::take(&mut self.vertices);
        let indices = std::mem::take(&mut self.indices);
        for tri in indices.as_chunks::<3>().0.iter() {
            let source = [
                vertices[tri[0] as usize],
                vertices[tri[1] as usize],
                vertices[tri[2] as usize],
            ];
            let (min, max) = tri_bounds(source.map(|v| v.position));
            for clip_tri in clip {
                let (clip_min, clip_max) = tri_bounds(*clip_tri);
                if clip_min[0] > max[0]
                    || clip_min[1] > max[1]
                    || clip_max[0] < min[0]
                    || clip_max[1] < min[1]
                {
                    continue;
                }
                let mut polygon = source.to_vec();
                for edge in 0..3 {
                    let a = clip_tri[edge];
                    let b = clip_tri[(edge + 1) % 3];
                    polygon = clip_polygon(&polygon, a, b);
                    if polygon.len() < 3 {
                        break;
                    }
                }
                if polygon.len() < 3 {
                    continue;
                }
                let base = self.vertices.len() as u32;
                self.vertices.extend(polygon.iter().copied());
                for k in 1..polygon.len() as u32 - 1 {
                    self.indices.extend([base, base + k, base + k + 1]);
                }
            }
        }
    }

    pub(super) fn finish(self) -> Option<PathMesh> {
        if self.indices.is_empty() {
            return None;
        }
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for vertex in &self.vertices {
            min = [
                min[0].min(vertex.position[0]),
                min[1].min(vertex.position[1]),
            ];
            max = [
                max[0].max(vertex.position[0]),
                max[1].max(vertex.position[1]),
            ];
        }
        Some(PathMesh {
            vertices: self.vertices,
            indices: self.indices,
            bounds: SceneRect {
                x: min[0],
                y: min[1],
                width: max[0] - min[0],
                height: max[1] - min[1],
            },
            gradient: None,
            pattern: None,
            paint_color: None,
        })
    }
}

/// How far the shadow band may reach in and out at each vertex: `inset` and
/// `blur`, except where offsetting an edge's two ends along their miters
/// would reverse it — the inner ring of a bend tighter than the blur, or the
/// outer ring of a hollow. There the reach stops short of the fold.
pub(super) fn ring_reach(
    points: &[[f32; 2]],
    miters: &[[f32; 2]],
    inset: f32,
    blur: f32,
) -> Vec<(f32, f32)> {
    let n = points.len();
    // Per edge i (points[i] -> points[i + 1]): the largest offset that keeps
    // it pointing the same way, inwards and outwards.
    let limits: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let e = [points[j][0] - points[i][0], points[j][1] - points[i][1]];
            let dm = [miters[j][0] - miters[i][0], miters[j][1] - miters[i][1]];
            let length = e[0] * e[0] + e[1] * e[1];
            let k = dm[0] * e[0] + dm[1] * e[1];
            let keep = 0.9 * length;
            (
                if k > 1e-9 { keep / k } else { f32::INFINITY },
                if k < -1e-9 { keep / -k } else { f32::INFINITY },
            )
        })
        .collect();
    let mut reach: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let (prev, next) = (limits[(i + n - 1) % n], limits[i]);
            (inset.min(prev.0).min(next.0), blur.min(prev.1).min(next.1))
        })
        .collect();
    // Neighbours may differ by at most half the edge between them. Without
    // this a long edge next to a clamped bend keeps its full reach at the
    // shared vertex and its quad shears across the bend's; a full edge length
    // (what a distance field allows) still lets the shared vertex step past
    // the bend's centre.
    let lengths: Vec<f32> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let e = [points[j][0] - points[i][0], points[j][1] - points[i][1]];
            (e[0] * e[0] + e[1] * e[1]).sqrt()
        })
        .collect();
    for _ in 0..2 {
        for i in 0..n {
            let j = (i + 1) % n;
            reach[j].0 = reach[j].0.min(reach[i].0 + lengths[i] * 0.5);
            reach[j].1 = reach[j].1.min(reach[i].1 + lengths[i] * 0.5);
        }
        for i in (0..n).rev() {
            let j = (i + 1) % n;
            reach[i].0 = reach[i].0.min(reach[j].0 + lengths[i] * 0.5);
            reach[i].1 = reach[i].1.min(reach[j].1 + lengths[i] * 0.5);
        }
    }
    reach
}
