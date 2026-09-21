// Shared by the text program: the projection, the two atlas pages, the two
// samplers, the presentation tables and the sRGB unpack. Keeping it in one
// file is what makes "an upright label and a rotated one sample the same page"
// true by construction rather than by two declarations agreeing.
struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

// One glyph's presentation. Everything here is *how* a resolved paragraph
// reaches the screen, never which glyphs it is made of, so an animation that
// only moves, fades or recolors text patches this table and leaves every
// instance alone.
struct TextRun {
    // Whole-pixel physical origin the instances are relative to.
    origin: vec2<f32>,
    // Index into `text_presentations`.
    presentation: u32,
    flags: u32,
    // Linear RGB with its own alpha, opacity not yet applied.
    color: vec4<f32>,
    opacity: f32,
    // Physical px per logical px the instances were resolved at: the device
    // scale, times the raster step a magnifying transform earned the entry.
    raster: f32,
    pad0: f32,
    pad1: f32,
}

// The transform and clip a run paints under. Deduplicated: a shell's labels
// nearly all share one identity transform and one clip, so this table stays a
// handful of entries however many paragraphs the frame holds.
struct TextPresentation {
    // `matrix(a, b, c, d, e, f)` as (a, b, c, d) and (e, f, g, h) with (g, h)
    // the projective row.
    affine: vec4<f32>,
    project: vec4<f32>,
    clip_rect: vec4<f32>,
    clip_inv_abcd: vec4<f32>,
    // (x, y) inverse translation, z corner radius, w the scene scale.
    clip_inv_ef: vec4<f32>,
    // `clip-path: polygon(...)` vertices, two per vec4.
    polygon: array<vec4<f32>, 4>,
    polygon_count: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

// Sampling is bilinear rather than nearest: the quad no longer lands on the
// texel grid.
const RUN_LINEAR: u32 = 1u;
// The run carries a clip the scissor cannot express.
const RUN_CLIP: u32 = 2u;
// The run's presentation is more than a translation, so each corner goes
// through the homography.
const RUN_PROJECT: u32 = 4u;

@group(0) @binding(1)
var<storage, read> text_runs: array<TextRun>;

@group(0) @binding(2)
var<storage, read> text_presentations: array<TextPresentation>;

@group(1) @binding(0)
var mask_atlas: texture_2d<f32>;

@group(1) @binding(1)
var color_atlas: texture_2d<f32>;

@group(1) @binding(2)
var atlas_nearest: sampler;

@group(1) @binding(3)
var atlas_linear: sampler;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// Scene colors are sRGB-encoded and the target is linear, so the conversion is
// the same one every other pipeline applies. Doing it here rather than on the
// CPU is what lets an instance carry four bytes of color.
fn unpack_srgb(color: u32) -> vec4<f32> {
    return vec4<f32>(
        srgb_to_linear(f32((color & 0x00ff0000u) >> 16u) / 255.0),
        srgb_to_linear(f32((color & 0x0000ff00u) >> 8u) / 255.0),
        srgb_to_linear(f32(color & 0x000000ffu) / 255.0),
        f32((color & 0xff000000u) >> 24u) / 255.0,
    );
}

// The page a glyph came from, in normalized coordinates.
fn atlas_uv(texel: vec2<u32>, content: u32) -> vec2<f32> {
    var dim = vec2<u32>(1u);
    if content == 0u {
        dim = textureDimensions(mask_atlas);
    } else {
        dim = textureDimensions(color_atlas);
    }
    return vec2<f32>(texel) / vec2<f32>(dim);
}

// A glyph corner in physical paint space.
//
// The same homography `Quad` applies, in logical space, per corner. A pure
// translation skips it: the run origin already carries the whole-pixel
// translation and the instance the sub-pixel remainder its bitmap was
// rasterized for, so touching the corner at all would only round it again.
//
// A projected run's instances are in the run's raster px, which a magnified
// entry makes finer than device px, so they are taken back to logical space
// by the raster scale and out again by the device scale.
fn text_world_position(run: TextRun, local: vec2<f32>) -> vec2<f32> {
    let paint = local + run.origin;
    if (run.flags & RUN_PROJECT) == 0u {
        return paint;
    }
    let presentation = text_presentations[run.presentation];
    let scale = presentation.clip_inv_ef.w;
    let p = paint / run.raster;
    let xp = presentation.affine.x * p.x + presentation.affine.z * p.y + presentation.project.x;
    let yp = presentation.affine.y * p.x + presentation.affine.w * p.y + presentation.project.y;
    let w = presentation.project.z * p.x + presentation.project.w * p.y + 1.0;
    var projected = vec2<f32>(xp, yp);
    if abs(w) >= 1e-8 {
        projected = projected / w;
    }
    return projected * scale;
}
