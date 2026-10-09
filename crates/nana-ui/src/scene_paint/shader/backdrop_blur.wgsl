// CSS `blur(r)`: a separable gaussian whose standard deviation is `r`.
//
// One pass blurs along `direction`. `sigma` is in texels of the source,
// which a wide blur has already shrunk (see backdrop.rs): a texel then
// stands for a block of device pixels, so the kernel still reads every
// texel within three standard deviations. Reading every texel is what keeps
// a wide blur smooth; a kernel that stepped over texels would sum shifted
// copies of the backdrop and leave bands.

@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

struct BlurUniforms {
    direction: vec2<f32>,
    // Standard deviation, in source texels.
    sigma: f32,
    _pad0: f32,
    texel_size: vec2<f32>,
    // The texels this pass writes and may read, in source texels.
    region_origin: vec2<f32>,
    region_size: vec2<f32>,
    dest_size: vec2<f32>,
}

@group(0) @binding(2)
var<uniform> blur: BlurUniforms;

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

fn gaussian_weight(offset: f32, sigma: f32) -> f32 {
    return exp(-0.5 * (offset * offset) / (sigma * sigma));
}

// The texel at `at` (a texel centre, or between two), with the region's
// edge texels standing in for everything past them: CSS blurs a backdrop
// with its edges duplicated, not faded into transparent black.
fn read(at: vec2<f32>) -> vec4<f32> {
    let lo = blur.region_origin + 0.5;
    let hi = blur.region_origin + blur.region_size - 0.5;
    return textureSampleLevel(source, source_sampler, clamp(at, lo, hi) * blur.texel_size, 0.0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let center = floor(input.position.xy) + 0.5;
    let sigma = blur.sigma;
    if sigma < 0.01 {
        return read(center);
    }
    let reach = i32(ceil(3.0 * sigma));
    var accum = read(center);
    var total = 1.0;
    // Texels k and k + 1 on each side in one bilinear read, placed between
    // them by their weights, so their sum is exact with half the reads.
    for (var k = 1; k <= reach; k = k + 2) {
        let near = gaussian_weight(f32(k), sigma);
        let far = select(0.0, gaussian_weight(f32(k + 1), sigma), k + 1 <= reach);
        let weight = near + far;
        let offset = (f32(k) * near + f32(k + 1) * far) / weight;
        let step = blur.direction * offset;
        accum += (read(center + step) + read(center - step)) * weight;
        total += 2.0 * weight;
    }
    return accum / total;
}
