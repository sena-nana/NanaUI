// NanaUI articulated-line stroke (constant radius). Independent WGSL; not
// derived from third-party shader sources.
struct SolidVertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_inv_abcd: vec4<f32>,
    @location(4) clip_inv_ef: vec3<f32>,
    @location(5) p0_radius: vec3<f32>,
    @location(6) p1: vec2<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_inv_abcd: vec4<f32>,
    @location(4) clip_inv_ef: vec3<f32>,
    @location(5) p0_radius: vec3<f32>,
    @location(6) p1: vec2<f32>,
}

@vertex
fn solid_vs_main(input: SolidVertexInput) -> SolidVertexOutput {
    var out: SolidVertexOutput;

    out.color = premultiply(input.color);
    out.position = globals.transform * vec4<f32>(input.position, 0.0, 1.0);
    out.world_pos = input.position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;
    out.p0_radius = input.p0_radius;
    out.p1 = input.p1;

    return out;
}

fn capsule_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = saturate(dot(pa, ba) / max(dot(ba, ba), 1e-8));
    return length(pa - ba * h);
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
    let p0 = input.p0_radius.xy;
    let radius = input.p0_radius.z;
    let distance_to_path = capsule_distance(input.world_pos, p0, input.p1);
    let pixel = max(fwidth(distance_to_path), 1e-5);
    let alpha = 1.0 - smoothstep(
        radius - pixel * 0.5,
        radius + pixel * 0.5,
        distance_to_path
    );
    if alpha <= 0.0 {
        discard;
    }
    return input.color * alpha;
}
