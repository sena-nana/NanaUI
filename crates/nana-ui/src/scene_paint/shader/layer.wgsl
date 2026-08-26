@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

struct LayerUniforms {
    opacity: f32,
    filter_b: f32,
    filter_s: f32,
    filter_c: f32,
    clip_rect: vec4<f32>,
    clip_inv_abcd: vec4<f32>,
    clip_inv_ef: vec2<f32>,
    clip_corner_radius: f32,
    clip_polygon_count: u32,
    clip_poly0: vec4<f32>,
    clip_poly1: vec4<f32>,
    clip_poly2: vec4<f32>,
    clip_poly3: vec4<f32>,
}

@group(0) @binding(2)
var<uniform> layer: LayerUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    output.uv = positions[index] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Clip inverse is stored in dest pixels (`FragmentClip::for_physical_pixels`).
    if !inside_fragment_clip(
        input.position.xy,
        layer.clip_rect,
        layer.clip_inv_abcd,
        layer.clip_inv_ef,
        layer.clip_corner_radius,
        layer.clip_polygon_count,
        layer.clip_poly0,
        layer.clip_poly1,
        layer.clip_poly2,
        layer.clip_poly3,
    ) {
        discard;
    }
    var sampled = textureSample(source, source_sampler, input.uv) * layer.opacity;
    if (layer.filter_b != 1.0 || layer.filter_s != 1.0 || layer.filter_c != 1.0) {
        var rgb = sampled.xyz * layer.filter_b;
        let lum = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
        rgb = mix(vec3(lum), rgb, layer.filter_s);
        rgb = (rgb - 0.5) * layer.filter_c + 0.5;
        sampled = vec4(clamp(rgb, vec3(0.0), vec3(1.0)), sampled.a);
    }
    return sampled;
}
