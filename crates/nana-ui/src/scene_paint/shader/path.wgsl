// Custom-paint path triangles (Issue #217). Tessellated on the CPU; the only
// per-pixel work is the clip test, an optional gradient and turning coverage
// into alpha. Shares the stroke module's globals and clip palette.

const NO_GRADIENT: u32 = 0xffffffffu;
const GRADIENT_STOPS: u32 = 16u;

// Matches `GpuGradient` in `mesh.rs`.
struct GpuGradient {
    // kind (0 linear, 1 radial, 2 conic), extend (0 pad, 1 repeat, 2 reflect),
    // stop count, unused.
    header: vec4<u32>,
    geometry: vec4<f32>,
    offsets: array<vec4<f32>, 4>,
    // Premultiplied sRGB.
    colors: array<vec4<f32>, 16>,
}

struct GradientPalette {
    items: array<GpuGradient>,
}

@group(0) @binding(2)
var<storage, read> gradient_palette: GradientPalette;

struct PathVertexInput {
    // Logical scene px after the primitive's affine.
    @location(0) position: vec2<f32>,
    // Logical px this vertex moves per physical px of AA fringe.
    @location(1) extrude: vec2<f32>,
    @location(2) coverage: f32,
    @location(3) clip_index: u32,
    // Linear RGBA, straight alpha, primitive opacity applied. Under a
    // gradient only its alpha is used.
    @location(4) color: vec4<f32>,
    // Where the vertex sits in the gradient's space.
    @location(5) paint_pos: vec2<f32>,
    @location(6) gradient: u32,
}

struct PathVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) coverage: f32,
    @location(3) @interpolate(flat) clip_index: u32,
    @location(4) paint_pos: vec2<f32>,
    @location(5) @interpolate(flat) gradient: u32,
}

@vertex
fn path_vs_main(input: PathVertexInput) -> PathVertexOutput {
    var out: PathVertexOutput;
    let world = input.position + input.extrude / max(globals.viewport_scale, 1e-4);
    out.position = globals.transform * vec4<f32>(world, 0.0, 1.0);
    out.world_pos = world;
    out.color = input.color;
    out.coverage = input.coverage;
    out.clip_index = input.clip_index;
    out.paint_pos = input.paint_pos;
    out.gradient = input.gradient;
    return out;
}

fn gradient_offset(g: GpuGradient, i: u32) -> f32 {
    return g.offsets[i / 4u][i % 4u];
}

fn srgb_channel_to_linear(u: f32) -> f32 {
    if u < 0.04045 {
        return u / 12.92;
    }
    return pow((u + 0.055) / 1.055, 2.4);
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
        srgb = g.colors[count - 1u];
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
    // Stops interpolate premultiplied in sRGB, as CSS does; the target
    // blends linear.
    if srgb.a <= 0.0 {
        return vec4<f32>(0.0);
    }
    let straight = srgb.rgb / srgb.a;
    let linear = vec3<f32>(
        srgb_channel_to_linear(straight.r),
        srgb_channel_to_linear(straight.g),
        srgb_channel_to_linear(straight.b),
    );
    return vec4<f32>(linear * srgb.a, srgb.a);
}

@fragment
fn path_fs_main(input: PathVertexOutput) -> @location(0) vec4<f32> {
    let clip = clip_palette.items[input.clip_index];
    if !inside_fragment_clip(
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
    ) {
        discard;
    }
    // A linear ramp shaped by smoothstep: a one-pixel AA fringe, or across a
    // shadow band of ±2σ, a close fit of the Gaussian edge.
    let alpha = smoothstep(0.0, 1.0, clamp(input.coverage, 0.0, 1.0));
    if alpha <= 0.0 {
        discard;
    }
    if input.gradient != NO_GRADIENT {
        return gradient_color(input.gradient, input.paint_pos) * (input.color.a * alpha);
    }
    return premultiply(input.color) * alpha;
}
