// Premultiplied linear scRGB shader for `nana-ui-viewport3d`.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_color: vec4<f32>,
    ground_color: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
}

struct Instance {
    model: mat4x4<f32>,
    normal_0: vec4<f32>,
    normal_1: vec4<f32>,
    normal_2: vec4<f32>,
    color: vec4<f32>,
    emissive_roughness: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var albedo_tex: texture_2d<f32>;
@group(1) @binding(1) var albedo_sampler: sampler;
@group(2) @binding(0) var<uniform> inst: Instance;

struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn bg_vertex(@builtin(vertex_index) idx: u32) -> BgOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = pos[idx];
    var out: BgOut;
    out.clip = vec4<f32>(p, 0.9999, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

@fragment
fn bg_fragment(input: BgOut) -> @location(0) vec4<f32> {
    let rgb = mix(globals.bg_top.rgb, globals.bg_bottom.rgb, clamp(input.uv.y, 0.0, 1.0));
    return vec4<f32>(rgb, 1.0);
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world: vec3<f32>,
}

@vertex
fn mesh_vertex(input: VertexIn) -> VertexOut {
    let world = inst.model * vec4<f32>(input.position, 1.0);
    let n_mat = mat3x3<f32>(inst.normal_0.xyz, inst.normal_1.xyz, inst.normal_2.xyz);
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    out.world = world.xyz;
    out.normal = n_mat * input.normal;
    out.uv = input.uv;
    return out;
}

@fragment
fn mesh_fragment(input: VertexOut) -> @location(0) vec4<f32> {
    let tex = textureSample(albedo_tex, albedo_sampler, input.uv);
    let albedo = tex.rgb * inst.color.rgb;
    let n = normalize(input.normal);
    let v = normalize(globals.eye.xyz - input.world);
    let l = normalize(globals.sun_dir.xyz);
    let ndotl = max(dot(n, l), 0.0);
    let wrap = pow(dot(n, l) * 0.5 + 0.5, 1.6);
    let ambient = mix(globals.ground_color.rgb, globals.sky_color.rgb, n.y * 0.5 + 0.5);
    let h = normalize(l + v);
    let roughness = clamp(inst.emissive_roughness.w, 0.08, 1.0);
    let shininess = mix(96.0, 8.0, roughness);
    let spec = pow(max(dot(n, h), 0.0), shininess) * (1.0 - roughness) * 0.35;
    let rim = pow(1.0 - clamp(abs(dot(n, v)), 0.0, 1.0), 3.0) * 0.08;
    let rgb = albedo * (ambient + globals.sun_color.rgb * (0.25 * wrap + 0.75 * ndotl))
        + globals.sun_color.rgb * spec
        + vec3<f32>(rim)
        + inst.emissive_roughness.rgb;
    let alpha = tex.a * inst.color.a;
    return vec4<f32>(rgb * alpha, alpha);
}
