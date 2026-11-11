// Custom-paint path triangles (Issue #217). Tessellated on the CPU and kept
// on the GPU node-local; the vertex stage places them, and the only
// per-pixel work is the clip test, an optional gradient and turning coverage
// into alpha. Shares the stroke module's globals and clip palette.

const NO_GRADIENT: u32 = 0xffffffffu;
const GRADIENT_STOPS: u32 = 16u;

// Matches `GpuGradient` in `mesh.rs`.
struct GpuGradient {
    // kind (0 linear, 1 radial, 2 conic), extend (0 pad, 1 repeat, 2 reflect),
    // stop count, the space stops interpolate in (0 sRGB, 1 linear scRGB).
    header: vec4<u32>,
    geometry: vec4<f32>,
    offsets: array<vec4<f32>, 4>,
    // Premultiplied, in the space `header.w` names.
    colors: array<vec4<f32>, 16>,
    // A mask's map from scene px into the space of `geometry`: a, b, c, d
    // and e, f. A paint gradient leaves it unused.
    space: array<vec4<f32>, 2>,
}

struct GradientPalette {
    items: array<GpuGradient>,
}

@group(0) @binding(2)
var<storage, read> gradient_palette: GradientPalette;

// A mesh's vertices stay on the GPU node-local, as the mesh was built; what a
// frame changes comes per draw in `PathInstanceInput`.
struct PathVertexInput {
    // Node-local px.
    @location(0) position: vec2<f32>,
    // Node-space miter along which the vertex moves half a physical px of AA
    // fringe; zero off a fringe. `path_screen_extrude` carries it to screen.
    @location(1) extrude: vec2<f32>,
    // Where the vertex sits in the gradient's space.
    @location(2) paint_pos: vec2<f32>,
    @location(3) coverage: f32,
    // Linear RGBA, straight alpha, before the draw's opacity. Under a
    // gradient only its alpha is used.
    @location(4) color: vec4<f32>,
}

// Matches `GpuPathInstance` in `mesh.rs`.
struct PathInstanceInput {
    // The node-to-scene affine's linear part.
    @location(5) abcd: vec4<f32>,
    // Its translation, then the mesh's origin in the node's layout space.
    @location(6) ef_origin: vec4<f32>,
    @location(7) opacity: f32,
    @location(8) clip_index: u32,
    @location(9) gradient: u32,
    // The node's `mask-image`, a gradient of the palette, or `NO_GRADIENT`.
    @location(10) mask: u32,
}

struct PathVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) coverage: f32,
    @location(3) @interpolate(flat) clip_index: u32,
    @location(4) paint_pos: vec2<f32>,
    @location(5) @interpolate(flat) gradient: u32,
    // `globals.viewport_scale`: the uniform is bound to the vertex stage only.
    @location(6) @interpolate(flat) pixel_scale: f32,
    // 1 on an AA fringe or the interior it bounds, 0 on a shadow's band.
    @location(7) fringe: f32,
    // `globals.corner_exponent`, bound to the vertex stage as the scale is.
    @location(8) @interpolate(flat) corner_exponent: f32,
    @location(9) @interpolate(flat) mask: u32,
}

// One of a miter's two edge normals (`bisector` turned by ±the half angle),
// mapped by the inverse transpose of `m` and normalised; z is 0 when the map
// leaves it no length.
fn path_screen_normal(m: vec4<f32>, det: f32, bisector: vec2<f32>, c: f32, s: f32) -> vec3<f32> {
    let n = vec2<f32>(bisector.x * c - bisector.y * s, bisector.x * s + bisector.y * c);
    let mapped = vec2<f32>(m.w * n.x - m.y * n.y, -m.z * n.x + m.x * n.y) / det;
    let len = length(mapped);
    if !(len > 1e-12) {
        return vec3<f32>(0.0);
    }
    return vec3<f32>(mapped / len, 1.0);
}

// A vertex's node-space AA miter carried through the linear part `m` of the
// draw's affine so the edges still move one pixel on screen: each edge
// normal maps by the inverse transpose and the miter is rebuilt from the two
// (limit 4). A degenerate map gives no fringe. `screen_extrude` in `mesh.rs`
// is the reference.
fn path_screen_extrude(m: vec4<f32>, extrude: vec2<f32>) -> vec2<f32> {
    let det = m.x * m.w - m.y * m.z;
    let len = length(extrude);
    if !(abs(det) >= 1e-12 && abs(det) <= 3.0e38) || !(len > 1e-12) {
        return vec2<f32>(0.0);
    }
    if m.x == m.w && m.y == -m.z {
        // A rotation and uniform scale: the miter just turns with it.
        let k = sqrt(m.x * m.x + m.y * m.y);
        return vec2<f32>(m.x * extrude.x + m.z * extrude.y, m.y * extrude.x + m.w * extrude.y) / k;
    }
    let bisector = extrude / len;
    let c = min(1.0 / len, 1.0);
    let s = sqrt(max(1.0 - c * c, 0.0));
    let n1 = path_screen_normal(m, det, bisector, c, s);
    let n2 = path_screen_normal(m, det, bisector, c, -s);
    if n1.z == 0.0 || n2.z == 0.0 {
        return vec2<f32>(0.0);
    }
    let denominator = 1.0 + dot(n1.xy, n2.xy);
    var miter = n1.xy;
    if denominator > 1e-3 {
        miter = (n1.xy + n2.xy) / denominator;
    }
    let reach = length(miter);
    if reach > 4.0 {
        return miter * (4.0 / reach);
    }
    return miter;
}

@vertex
fn path_vs_main(input: PathVertexInput, instance: PathInstanceInput) -> PathVertexOutput {
    var out: PathVertexOutput;
    let m = instance.abcd;
    let local = input.position + instance.ef_origin.zw;
    let scene = vec2<f32>(
        m.x * local.x + m.z * local.y + instance.ef_origin.x,
        m.y * local.x + m.w * local.y + instance.ef_origin.y,
    );
    let extrude = path_screen_extrude(m, input.extrude);
    let world = scene + extrude * 0.5 / max(globals.viewport_scale, 1e-4);
    out.fringe = select(0.0, 1.0, any(extrude != vec2<f32>(0.0)));
    out.position = globals.transform * vec4<f32>(world, 0.0, 1.0);
    out.world_pos = world;
    out.color = vec4<f32>(input.color.rgb, input.color.a * instance.opacity);
    out.coverage = input.coverage;
    out.clip_index = instance.clip_index;
    out.pixel_scale = globals.viewport_scale;
    out.corner_exponent = globals.corner_exponent;
    out.paint_pos = input.paint_pos;
    out.gradient = instance.gradient;
    out.mask = instance.mask;
    return out;
}

fn gradient_offset(g: GpuGradient, i: u32) -> f32 {
    return g.offsets[i / 4u][i % 4u];
}

// Premultiplied linear colour of gradient `index` at `p`.
fn gradient_color(index: u32, p: vec2<f32>) -> vec4<f32> {
    let g = gradient_palette.items[index];
    var t = 0.0;
    switch g.header.x {
        case 0u: {
            let axis = g.geometry.zw - g.geometry.xy;
            let length2 = dot(axis, axis);
            t = select(0.0, dot(p - g.geometry.xy, axis) / length2, length2 > 1e-12);
        }
        case 1u: {
            t = length(p - g.geometry.xy) / max(g.geometry.z, 1e-6);
        }
        default: {
            let d = p - g.geometry.xy;
            let turn = 6.283185307179586;
            let angle = atan2(d.y, d.x) - g.geometry.z;
            t = angle / turn - floor(angle / turn);
        }
    }
    switch g.header.y {
        case 1u: {
            t = t - floor(t);
        }
        case 2u: {
            let r = t - 2.0 * floor(t * 0.5);
            t = select(r, 2.0 - r, r > 1.0);
        }
        default: {
            t = clamp(t, 0.0, 1.0);
        }
    }
    let count = min(g.header.z, GRADIENT_STOPS);
    var srgb = g.colors[0];
    if t >= gradient_offset(g, 0u) {
        // A gradient with no stops packs `count == 0`; its colours are all
        // zero, so stop 0 stands in for the missing last one.
        srgb = g.colors[max(count, 1u) - 1u];
        for (var i = 1u; i < count; i = i + 1u) {
            let o1 = gradient_offset(g, i);
            if t <= o1 {
                let o0 = gradient_offset(g, i - 1u);
                let k = select(1.0, (t - o0) / (o1 - o0), o1 > o0);
                srgb = mix(g.colors[i - 1u], g.colors[i], k);
                break;
            }
        }
    }
    // Plain sRGB stops interpolated in sRGB, as a browser blends them, and
    // decode once here. Stops in a colour space of their own arrive linear
    // and keep extended values until the presentation transform.
    if srgb.a <= 0.0 {
        return vec4<f32>(0.0);
    }
    var straight = srgb.rgb / srgb.a;
    if g.header.w == 0u {
        straight = vec3<f32>(
            srgb_channel_to_linear(straight.r),
            srgb_channel_to_linear(straight.g),
            srgb_channel_to_linear(straight.b),
        );
    }
    return vec4<f32>(straight * srgb.a, srgb.a);
}

fn srgb_channel_to_linear(u: f32) -> f32 {
    if u < 0.04045 {
        return u / 12.92;
    }
    return pow((u + 0.055) / 1.055, 2.4);
}

// The pass's sample count, and a 4× pass's sample positions relative to the
// pixel centre: the standard pattern WebGPU fixes.
// Keep the specialization scalar as a float. Some GLES translators materialize
// integer overrides as an `int[1]`, which cannot initialize a WGSL `u32`.
override PATH_SAMPLES: f32 = 1.0;
const PATH_SAMPLE_OFFSETS = array<vec2<f32>, 4>(
    vec2<f32>(-0.125, -0.375),
    vec2<f32>(0.375, -0.125),
    vec2<f32>(-0.375, 0.125),
    vec2<f32>(0.125, 0.375),
);

// An AA fringe ramps linearly over its one device pixel, as every other edge;
// across a shadow band of ±2σ, smoothstep shapes the ramp into a close fit of
// the Gaussian edge.
fn path_alpha(coverage: f32, fringe: bool) -> f32 {
    let linear = clamp(coverage, 0.0, 1.0);
    return select(smoothstep(0.0, 1.0, linear), linear, fringe);
}

// The clip coverage of `input`'s fragment.
fn path_clip_cover(input: PathVertexOutput) -> f32 {
    let clip = clip_palette.items[input.clip_index];
    return fragment_clip_coverage(
        input.world_pos,
        clip.rect,
        clip.inv_abcd,
        clip.inv_ef_radius.xy,
        clip.inv_ef_radius.z,
        u32(clip.inv_ef_radius.w),
        clip.poly0,
        clip.poly1,
        clip.poly2,
        clip.poly3,
        input.pixel_scale,
        input.corner_exponent,
    );
}

// The node's `mask-image` at `input`'s fragment, as `mask_alpha` in
// quad_paint_data.wgsl takes it over the node's own quad: the stops' alpha.
fn path_node_mask(input: PathVertexOutput) -> f32 {
    if input.mask == NO_GRADIENT {
        return 1.0;
    }
    let space = gradient_palette.items[input.mask].space;
    let p = input.world_pos;
    let at = vec2<f32>(
        space[0].x * p.x + space[0].z * p.y + space[1].x,
        space[0].y * p.x + space[0].w * p.y + space[1].y,
    );
    return gradient_color(input.mask, at).a;
}

// The premultiplied colour of `input`'s fragment at coverage `alpha`.
fn path_shade(input: PathVertexOutput, alpha: f32) -> vec4<f32> {
    if input.gradient != NO_GRADIENT {
        return gradient_color(input.gradient, input.paint_pos) * (input.color.a * alpha);
    }
    return premultiply(input.color) * alpha;
}

// Keep the clip and the shading in their own functions and the one
// `discard` at the end. With them written inline and the clip's discard
// first, D3D12's FXC build of this shader drew nothing under 4x MSAA
// (WARP: every path black), while the same code split this way and Metal,
// Vulkan and GL were correct.
@fragment
fn path_fs_main(
    input: PathVertexOutput,
    @builtin(sample_index) sample_index: u32,
) -> @location(0) vec4<f32> {
    // Coverage is linear across a triangle; its screen gradient, taken while
    // every invocation of the quad still runs, reaches any sample from here.
    let ramp = vec2<f32>(dpdx(input.coverage), dpdy(input.coverage));
    let fringe = input.fringe > 0.5;
    var coverage = path_alpha(input.coverage, fringe);
    if PATH_SAMPLES == 4.0 {
        // Sample-index shading makes the MSAA resolve average the four
        // coverage values. It also avoids relying on the backend's
        // `sample_mask` array builtin, which is not representable as a scalar
        // u32 by every GLSL translation backend.
        coverage = path_alpha(
            input.coverage + dot(ramp, PATH_SAMPLE_OFFSETS[min(sample_index, 3u)]),
            fringe,
        );
    }
    let alpha = coverage * path_clip_cover(input) * path_node_mask(input);
    if alpha <= 0.0 {
        discard;
    }
    return path_shade(input, alpha);
}

// A painter layer keeping its node's mask (`LayerMaskMode::Keep`), drawn
// with the erase blend: the layer keeps `mask` of itself where the mesh
// covers it. One device pixel ramps the mesh's own edge, as `path_fs_main`.
@fragment
fn path_keep_fs_main(input: PathVertexOutput) -> @location(0) vec4<f32> {
    let coverage = path_alpha(input.coverage, input.fringe > 0.5);
    let erased = coverage * path_clip_cover(input) * (1.0 - path_node_mask(input));
    if erased <= 0.0 {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, erased);
}
