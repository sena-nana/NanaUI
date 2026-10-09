// Copy the backdrop under a frosted panel into the blur's working texture,
// shrunk by `downsample`. A texel of the copy is the average of the
// `downsample` × `downsample` block of device pixels it stands for: a box
// filter, read as one bilinear tap per 2 × 2 quad, so no pixel of the
// backdrop is skipped and a thin line cannot flicker in and out of a wide
// blur.

@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

struct CopyUniforms {
    src_origin: vec2<f32>,
    src_size: vec2<f32>,
    dest_size: vec2<f32>,
    // Device pixels per texel of the copy, along each axis: 1, 2, 4, …
    downsample: f32,
    _pad: f32,
}

@group(0) @binding(2)
var<uniform> copy: CopyUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
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
    output.local = positions[index] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let downsample = max(copy.downsample, 1.0);
    let block = floor(input.position.xy) * downsample;
    if downsample < 2.0 {
        return textureSampleLevel(source, source_sampler, (block + 0.5) / copy.dest_size, 0.0);
    }
    let taps = i32(downsample * 0.5);
    var sum = vec4(0.0);
    for (var y = 0; y < taps; y = y + 1) {
        for (var x = 0; x < taps; x = x + 1) {
            let corner = block + vec2(f32(x), f32(y)) * 2.0 + 1.0;
            sum += textureSampleLevel(source, source_sampler, corner / copy.dest_size, 0.0);
        }
    }
    return sum / f32(taps * taps);
}
