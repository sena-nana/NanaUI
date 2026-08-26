// NanaUI articulated-line stroke. Covering quads are expanded in the vertex
// shader from one instance per segment (p0, p1, r0, r1). Independent WGSL;
// not derived from third-party shader sources.
const STROKE_AA_FRINGE: f32 = 1.0;

struct SolidInstanceInput {
    @location(0) color: vec4<f32>,
    @location(1) clip_rect: vec4<f32>,
    @location(2) clip_inv_abcd: vec4<f32>,
    @location(3) clip_inv_ef_cap: vec4<f32>,
    @location(4) p0: vec2<f32>,
    @location(5) p1: vec2<f32>,
    @location(6) radii: vec2<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_inv_abcd: vec4<f32>,
    @location(4) clip_inv_ef: vec3<f32>,
    @location(5) p0: vec2<f32>,
    @location(6) p1: vec2<f32>,
    @location(7) radii_caps: vec4<f32>,
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

fn covering_corner(
    p0: vec2<f32>,
    p1: vec2<f32>,
    r0: f32,
    r1: f32,
    cap0: f32,
    cap1: f32,
    corner: vec2<f32>,
) -> vec2<f32> {
    let delta = p1 - p0;
    let seg_len = max(length(delta), 1e-8);
    let tangent = delta / seg_len;
    let normal = vec2<f32>(-tangent.y, tangent.x);
    let end0 = select(r0 + STROKE_AA_FRINGE, STROKE_AA_FRINGE, cap0 > 0.5);
    let end1 = select(r1 + STROKE_AA_FRINGE, STROKE_AA_FRINGE, cap1 > 0.5);
    let side = select(r0 + STROKE_AA_FRINGE, r1 + STROKE_AA_FRINGE, corner.x > 0.0);
    let along = select(-end0, seg_len + end1, corner.x > 0.0);
    return p0 + tangent * along + normal * (corner.y * side);
}

@vertex
fn solid_vs_main(
    @builtin(vertex_index) vertex_index: u32,
    input: SolidInstanceInput,
) -> SolidVertexOutput {
    var out: SolidVertexOutput;
    let packed = input.clip_inv_ef_cap.w;
    let cap0 = unpack_cap0(packed);
    let cap1 = unpack_cap1(packed);
    let position = covering_corner(
        input.p0,
        input.p1,
        input.radii.x,
        input.radii.y,
        cap0,
        cap1,
        unit_corner(vertex_index),
    );
    out.color = premultiply(input.color);
    out.position = globals.transform * vec4<f32>(position, 0.0, 1.0);
    out.world_pos = position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef_cap.xyz;
    out.p0 = input.p0;
    out.p1 = input.p1;
    out.radii_caps = vec4<f32>(input.radii, cap0, cap1);
    return out;
}

fn sd_variable_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r0: f32, r1: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = saturate(dot(pa, ba) / max(dot(ba, ba), 1e-8));
    return length(pa - ba * h) - mix(r0, r1, h);
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

@fragment
fn solid_fs_main(input: SolidVertexOutput) -> @location(0) vec4<f32> {
    if !inside_fragment_clip(
        input.world_pos,
        input.clip_rect,
        input.clip_inv_abcd,
        input.clip_inv_ef.xy,
        input.clip_inv_ef.z,
        0u,
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    ) {
        discard;
    }
    let distance_to_path = stroke_signed_distance(
        input.world_pos,
        input.p0,
        input.p1,
        input.radii_caps.x,
        input.radii_caps.y,
        input.radii_caps.z,
        input.radii_caps.w,
    );
    let pixel = max(fwidth(distance_to_path), 1e-5);
    let alpha = 1.0 - smoothstep(-pixel * 0.5, pixel * 0.5, distance_to_path);
    if alpha <= 0.0 {
        discard;
    }
    return input.color * alpha;
}
