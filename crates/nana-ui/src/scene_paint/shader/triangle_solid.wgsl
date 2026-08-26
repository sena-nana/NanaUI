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
    @location(7) radii_cap: vec3<f32>,
}

fn unit_corner(vertex_index: u32) -> vec2<f32> {
    let id = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u)[vertex_index];
    return vec2<f32>(
        select(-1.0, 1.0, (id & 2u) != 0u),
        select(-1.0, 1.0, id == 1u || id == 2u),
    );
}

fn covering_corner(
    p0: vec2<f32>,
    p1: vec2<f32>,
    r0: f32,
    r1: f32,
    cap: f32,
    corner: vec2<f32>,
) -> vec2<f32> {
    let delta = p1 - p0;
    let seg_len = max(length(delta), 1e-8);
    let tangent = delta / seg_len;
    let normal = vec2<f32>(-tangent.y, tangent.x);
    let end0 = select(r0 + STROKE_AA_FRINGE, STROKE_AA_FRINGE, cap > 0.5);
    let end1 = select(r1 + STROKE_AA_FRINGE, STROKE_AA_FRINGE, cap > 0.5);
    let side = max(r0, r1) + STROKE_AA_FRINGE;
    let along = select(-end0, seg_len + end1, corner.x > 0.0);
    return p0 + tangent * along + normal * (corner.y * side);
}

@vertex
fn solid_vs_main(
    @builtin(vertex_index) vertex_index: u32,
    input: SolidInstanceInput,
) -> SolidVertexOutput {
    var out: SolidVertexOutput;
    let cap = input.clip_inv_ef_cap.w;
    let position = covering_corner(
        input.p0,
        input.p1,
        input.radii.x,
        input.radii.y,
        cap,
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
    out.radii_cap = vec3<f32>(input.radii, cap);
    return out;
}

fn capsule_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = saturate(dot(pa, ba) / max(dot(ba, ba), 1e-8));
    return length(pa - ba * h);
}

fn sd_uneven_capsule(p: vec2<f32>, pa: vec2<f32>, pb: vec2<f32>, ra: f32, rb: f32) -> f32 {
    var point = p - pa;
    let span = pb - pa;
    let h = dot(span, span);
    if h < 1e-8 {
        return length(point) - max(ra, rb);
    }
    let b = ra - rb;
    if b * b >= h {
        return min(length(point) - ra, length(p - pb) - rb);
    }
    let q = vec2<f32>(dot(point, vec2<f32>(span.y, -span.x)), dot(point, span)) / h;
    let qx = abs(q.x);
    let qy = q.y;
    let c = vec2<f32>(sqrt(h - b * b), b);
    let k = c.x * qy - c.y * qx;
    let n = qx * qx + qy * qy;
    if k < 0.0 {
        return sqrt(h) * n - ra;
    }
    if k > c.x {
        return sqrt(h) * (n + 1.0 - 2.0 * qy) - rb;
    }
    return dot(c, vec2<f32>(qx, qy)) - ra;
}

fn sd_butt(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r0: f32, r1: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let seg_len = max(length(ba), 1e-8);
    let tangent = ba / seg_len;
    let local = vec2<f32>(dot(pa, tangent), dot(pa, vec2<f32>(-tangent.y, tangent.x)));
    let radius = mix(r0, r1, saturate(local.x / seg_len));
    let d = vec2<f32>(max(local.x - seg_len, -local.x), abs(local.y) - radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

fn stroke_signed_distance(
    p: vec2<f32>,
    p0: vec2<f32>,
    p1: vec2<f32>,
    r0: f32,
    r1: f32,
    cap: f32,
) -> f32 {
    if cap > 0.5 {
        return sd_butt(p, p0, p1, r0, r1);
    }
    if abs(r0 - r1) < 1e-5 {
        return capsule_distance(p, p0, p1) - r0;
    }
    return sd_uneven_capsule(p, p0, p1, r0, r1);
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
        input.radii_cap.x,
        input.radii_cap.y,
        input.radii_cap.z,
    );
    let pixel = max(fwidth(distance_to_path), 1e-5);
    let alpha = 1.0 - smoothstep(-pixel * 0.5, pixel * 0.5, distance_to_path);
    if alpha <= 0.0 {
        discard;
    }
    return input.color * alpha;
}
