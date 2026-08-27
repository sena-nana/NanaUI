// Articulated-line stroke: one instance per segment, covering quad in the
// vertex shader, vanilla-disc SDF in local space (ellipse under non-uniform
// affine). Independent WGSL. `STROKE_AA_FRINGE` is the physical-pixel pad;
// identity + viewport 1 keeps 1 logical px (GraphCanvas 1.6px covering).
const STROKE_AA_FRINGE: f32 = 1.0;

// Unique clips interned on the CPU; instances store an index.
// `inv_ef_radius.w` is polygon vertex count; `poly0..3` pack ≤8 clip-path
// vertices in rect-local space (same as dest `FragmentClip`).
struct GpuClip {
    rect: vec4<f32>,
    inv_abcd: vec4<f32>,
    inv_ef_radius: vec4<f32>,
    poly0: vec4<f32>,
    poly1: vec4<f32>,
    poly2: vec4<f32>,
    poly3: vec4<f32>,
}

struct ClipPalette {
    items: array<GpuClip>,
}

@group(0) @binding(1)
var<storage, read> clip_palette: ClipPalette;

struct SolidInstanceInput {
    @location(0) color: vec4<f32>,
    @location(1) p0: vec2<f32>,
    @location(2) p1: vec2<f32>,
    @location(3) radii: vec2<f32>,
    @location(4) clip_index: u32,
    @location(5) packed_caps: f32,
    @location(6) affine_abcd: vec4<f32>,
    @location(7) affine_ef: vec2<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) @interpolate(flat) clip_index: u32,
    @location(3) @interpolate(flat) p0: vec2<f32>,
    @location(4) @interpolate(flat) p1: vec2<f32>,
    @location(5) @interpolate(flat) radii_caps: vec4<f32>,
    @location(6) @interpolate(flat) affine_abcd: vec4<f32>,
    @location(7) @interpolate(flat) affine_ef: vec2<f32>,
}

fn unit_corner(vertex_index: u32) -> vec2<f32> {
    let id = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u)[vertex_index];
    return vec2<f32>(
        select(-1.0, 1.0, (id & 2u) != 0u),
        select(-1.0, 1.0, id == 1u || id == 2u),
    );
}

fn unpack_cap0(packed: f32) -> f32 {
    return select(0.0, 1.0, packed == 1.0 || packed == 3.0);
}

fn unpack_cap1(packed: f32) -> f32 {
    return select(0.0, 1.0, packed >= 2.0);
}

// External-tangent hull half-width. Nested discs use the larger pad.
fn hull_half_width(along: f32, pad0: f32, pad1: f32, seg_len: f32) -> f32 {
    let dr = pad0 - pad1;
    if abs(dr) >= seg_len {
        return max(pad0, pad1);
    }
    let sin_a = dr / seg_len;
    let cos_a = sqrt(max(1.0 - sin_a * sin_a, 0.0));
    return (pad0 - sin_a * along) / max(cos_a, 1e-8);
}

fn covering_corner(
    p0: vec2<f32>,
    p1: vec2<f32>,
    r0: f32,
    r1: f32,
    cap0: f32,
    cap1: f32,
    fringe: f32,
    corner: vec2<f32>,
) -> vec2<f32> {
    let delta = p1 - p0;
    let seg_len = max(length(delta), 1e-8);
    let tangent = delta / seg_len;
    let normal = vec2<f32>(-tangent.y, tangent.x);
    let pad0 = r0 + fringe;
    let pad1 = r1 + fringe;
    var end0 = select(pad0, fringe, cap0 > 0.5);
    var end1 = select(pad1, fringe, cap1 > 0.5);
    // Nested discs: the hull is the larger disc. Extend a round far end so
    // the covering quad still contains it; a butt cut keeps the end plane.
    if pad0 >= pad1 + seg_len && cap1 < 0.5 {
        end1 = max(end1, pad0 - seg_len);
    }
    if pad1 >= pad0 + seg_len && cap0 < 0.5 {
        end0 = max(end0, pad1 - seg_len);
    }
    let along = select(-end0, seg_len + end1, corner.x > 0.0);
    let side = hull_half_width(along, pad0, pad1, seg_len);
    return p0 + tangent * along + normal * (corner.y * side);
}

// CSS/Canvas `matrix(a, b, c, d, e, f)`: x' = ax + cy + e, y' = bx + dy + f.
// `abcd` is (a, b, c, d). Identity skips the multiply.
fn is_identity_affine(abcd: vec4<f32>, ef: vec2<f32>) -> bool {
    return all(abcd == vec4<f32>(1.0, 0.0, 0.0, 1.0)) && all(ef == vec2<f32>(0.0));
}

fn apply_affine(abcd: vec4<f32>, ef: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        abcd.x * p.x + abcd.z * p.y + ef.x,
        abcd.y * p.x + abcd.w * p.y + ef.y,
    );
}

fn local_to_world(abcd: vec4<f32>, ef: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    if is_identity_affine(abcd, ef) {
        return p;
    }
    return apply_affine(abcd, ef, p);
}

fn world_to_local(abcd: vec4<f32>, ef: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    if is_identity_affine(abcd, ef) {
        return p;
    }
    let det = abcd.x * abcd.w - abcd.y * abcd.z;
    if abs(det) < 1e-12 {
        return p;
    }
    let inv_det = 1.0 / det;
    let ia = abcd.w * inv_det;
    let ib = -abcd.y * inv_det;
    let ic = -abcd.z * inv_det;
    let id = abcd.x * inv_det;
    let ie = -(ia * ef.x + ic * ef.y);
    let i_f = -(ib * ef.x + id * ef.y);
    return vec2<f32>(ia * p.x + ic * p.y + ie, ib * p.x + id * p.y + i_f);
}

// Local pad covering `STROKE_AA_FRINGE` physical px after affine + viewport.
fn local_aa_fringe(abcd: vec4<f32>) -> f32 {
    var sigma = 1.0;
    if !all(abcd == vec4<f32>(1.0, 0.0, 0.0, 1.0)) {
        let a = abcd.x;
        let b = abcd.y;
        let c = abcd.z;
        let d = abcd.w;
        let det = a * d - b * c;
        let fro2 = a * a + b * b + c * c + d * d;
        let disc = max(fro2 * fro2 - 4.0 * det * det, 0.0);
        sigma = sqrt(max((fro2 - sqrt(disc)) * 0.5, 0.0));
    }
    let viewport = max(globals.viewport_scale, 1e-4);
    return STROKE_AA_FRINGE / max(sigma * viewport, 1e-4);
}

@vertex
fn solid_vs_main(
    @builtin(vertex_index) vertex_index: u32,
    input: SolidInstanceInput,
) -> SolidVertexOutput {
    var out: SolidVertexOutput;
    let packed = input.packed_caps;
    let cap0 = unpack_cap0(packed);
    let cap1 = unpack_cap1(packed);
    let local = covering_corner(
        input.p0,
        input.p1,
        input.radii.x,
        input.radii.y,
        cap0,
        cap1,
        local_aa_fringe(input.affine_abcd),
        unit_corner(vertex_index),
    );
    let world = local_to_world(input.affine_abcd, input.affine_ef, local);
    out.color = premultiply(input.color);
    out.position = globals.transform * vec4<f32>(world, 0.0, 1.0);
    out.world_pos = world;
    out.clip_index = input.clip_index;
    out.p0 = input.p0;
    out.p1 = input.p1;
    out.radii_caps = vec4<f32>(input.radii, cap0, cap1);
    out.affine_abcd = input.affine_abcd;
    out.affine_ef = input.affine_ef;
    return out;
}

// Convex hull of discs (a, r0) and (b, r1). Nested: the larger disc is the shape.
fn sd_variable_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r0: f32, r1: f32) -> f32 {
    let ba = b - a;
    let l = length(ba);
    if l < 1e-8 {
        return length(p - a) - max(r0, r1);
    }
    let tangent = ba / l;
    let normal = vec2<f32>(-tangent.y, tangent.x);
    let pa = p - a;
    let along = dot(pa, tangent);
    let perp = abs(dot(pa, normal));
    let dr = r0 - r1;
    if abs(dr) >= l {
        if r0 > r1 {
            return length(p - a) - r0;
        }
        return length(p - b) - r1;
    }
    let sin_a = dr / l;
    let cos_a = sqrt(max(1.0 - sin_a * sin_a, 0.0));
    let k = along * cos_a - perp * sin_a;
    if k < 0.0 {
        return length(p - a) - r0;
    }
    if k > cos_a * l {
        return length(p - b) - r1;
    }
    return along * sin_a + perp * cos_a - r0;
}

fn stroke_signed_distance(
    p: vec2<f32>,
    p0: vec2<f32>,
    p1: vec2<f32>,
    r0: f32,
    r1: f32,
    cap0: f32,
    cap1: f32,
) -> f32 {
    var distance_to_path = sd_variable_capsule(p, p0, p1, r0, r1);
    if cap0 > 0.5 || cap1 > 0.5 {
        let ba = p1 - p0;
        let seg_len = max(length(ba), 1e-8);
        let local_x = dot(p - p0, ba / seg_len);
        if cap0 > 0.5 {
            distance_to_path = max(distance_to_path, -local_x);
        }
        if cap1 > 0.5 {
            distance_to_path = max(distance_to_path, local_x - seg_len);
        }
    }
    return distance_to_path;
}

fn stroke_clip_and_distance(input: SolidVertexOutput) -> f32 {
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
    let local_p = world_to_local(input.affine_abcd, input.affine_ef, input.world_pos);
    return stroke_signed_distance(
        local_p,
        input.p0,
        input.p1,
        input.radii_caps.x,
        input.radii_caps.y,
        input.radii_caps.z,
        input.radii_caps.w,
    );
}

@fragment
fn solid_fs_main(input: SolidVertexOutput) -> @location(0) vec4<f32> {
    let distance_to_path = stroke_clip_and_distance(input);
    // Gradient length, not isotropic `fwidth`, so anisotropic ellipses AA evenly.
    let pixel = max(
        length(vec2<f32>(dpdx(distance_to_path), dpdy(distance_to_path))),
        1e-5,
    );
    let alpha = 1.0 - smoothstep(-pixel * 0.5, pixel * 0.5, distance_to_path);
    if alpha <= 0.0 {
        discard;
    }
    return input.color * alpha;
}

// MSAA dest (`pipeline_msaa`): hard SDF coverage so the sample mask owns the
// edge. sample_count=1 keeps `solid_fs_main` anisotropic screen-space AA.
@fragment
fn solid_fs_msaa(input: SolidVertexOutput) -> @location(0) vec4<f32> {
    let distance_to_path = stroke_clip_and_distance(input);
    let alpha = 1.0 - step(0.0, distance_to_path);
    if alpha <= 0.0 {
        discard;
    }
    return input.color * alpha;
}
