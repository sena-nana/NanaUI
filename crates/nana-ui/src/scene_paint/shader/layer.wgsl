@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

struct LayerUniforms {
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    clip_rect: vec4<f32>,
    clip_inv_abcd: vec4<f32>,
    clip_inv_ef: vec2<f32>,
    _pad3: vec2<f32>,
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
    if !inside_transformed_rect(
        input.position.xy,
        layer.clip_rect,
        layer.clip_inv_abcd,
        layer.clip_inv_ef
    ) {
        discard;
    }
    return textureSample(source, source_sampler, input.uv) * layer.opacity;
}
