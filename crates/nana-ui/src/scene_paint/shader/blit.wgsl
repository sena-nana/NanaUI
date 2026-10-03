@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

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
    return textureSample(source, source_sampler, input.uv);
}

fn srgb_to_linear3(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    let decoded = select(pow((a + 0.055) / 1.055, vec3<f32>(2.4)), a / 12.92, a <= vec3<f32>(0.04045));
    return sign(c) * decoded;
}

fn linear_to_srgb3(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    let encoded = select(1.055 * pow(a, vec3<f32>(1.0 / 2.4)) - 0.055, a * 12.92, a <= vec3<f32>(0.0031308));
    return sign(c) * encoded;
}

fn extended_srgb_oetf3(c: vec3<f32>) -> vec3<f32> {
    // Extended sRGB is sign preserving; unlike ordinary SDR output it must
    // not clamp negative or above-white scRGB values before the transfer.
    return linear_to_srgb3(c);
}

fn shoulder3(c: vec3<f32>) -> vec3<f32> {
    // A deterministic SDR shoulder. Ordinary UI values retain their linear
    // value. Highlights roll off smoothly instead of hard-clipping every
    // value above one to the same white pixel.
    let knee = vec3<f32>(0.85);
    let excess = max(c - knee, vec3<f32>(0.0));
    let rolled = knee + vec3<f32>(0.15) * (vec3<f32>(1.0) - exp(-excess * vec3<f32>(0.8)));
    return select(rolled, c, c <= knee);
}

fn gamut_map(rgb: vec3<f32>) -> vec3<f32> {
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let lo = min(rgb.x, min(rgb.y, rgb.z));
    let hi = max(rgb.x, max(rgb.y, rgb.z));
    let hi_scale = select((1.0 - luma) / max(hi - luma, 1.0e-6), 1.0, hi <= 1.0);
    let lo_scale = select((0.0 - luma) / min(lo - luma, -1.0e-6), 1.0, lo >= 0.0);
    let scale = clamp(min(1.0, min(hi_scale, lo_scale)), 0.0, 1.0);
    // Compression above preserves hue and neutrals; the final clamp only
    // handles a negative neutral (or tiny floating-point residue) after the
    // controlled mapping, never authoring/intermediate composition.
    return clamp(luma + (rgb - vec3<f32>(luma)) * scale, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn sc_to_p3(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(rgb, vec3<f32>(0.8225927, 0.1775330, 0.0000000)),
        dot(rgb, vec3<f32>(0.0331995, 0.9667835, 0.0000000)),
        dot(rgb, vec3<f32>(0.0170853, 0.0723957, 0.9103015)),
    );
}

// `mode`: SDR sRGB/P3, extended linear/sRGB/P3, or HDR-to-SDR sRGB/P3.
fn presentation_rgb(straight: vec3<f32>, mode: u32) -> vec3<f32> {
    switch mode {
        case 0u: { return gamut_map(straight); }
        case 1u: { return gamut_map(sc_to_p3(straight)); }
        case 4u: { return sc_to_p3(straight); }
        case 5u: { return gamut_map(shoulder3(straight)); }
        case 6u: { return gamut_map(sc_to_p3(shoulder3(straight))); }
        default: { return straight; }
    }
}

fn present_sample(input: VertexOutput, mode: u32, gamma_alpha: bool, srgb_target: bool) -> vec4<f32> {
    let color = textureSample(source, source_sampler, input.uv);
    // Fail closed for zero, negative, or non-finite alpha from a custom
    // renderer.  A NaN comparison is false, so spelling this as `!(a > 0)`
    // prevents it from reaching the unpremultiplication below.
    if (!(color.a > 0.0)) {
        return vec4<f32>(0.0);
    }
    let mapped = presentation_rgb(color.rgb / color.a, mode);
    if (mode == 2u) {
        return vec4<f32>(mapped * color.a, color.a);
    }
    // Composition is linear and premultiplied. Only this output boundary
    // applies the requested transfer and compositor alpha representation.
    if (gamma_alpha) {
        let encoded_premult = linear_to_srgb3(mapped) * color.a;
        if (srgb_target) {
            return vec4<f32>(srgb_to_linear3(encoded_premult), color.a);
        }
        return vec4<f32>(encoded_premult, color.a);
    }
    let premult = mapped * color.a;
    if (srgb_target) {
        return vec4<f32>(premult, color.a);
    }
    return vec4<f32>(linear_to_srgb3(premult), color.a);
}

// Preserve the historic sRGB fast path, including exact SDR reference white.
@fragment
fn fs_gamma_premultiplied(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(source, source_sampler, input.uv);
    if (!(color.a > 0.0)) { return vec4<f32>(0.0); }
    return vec4<f32>(srgb_to_linear3(linear_to_srgb3(color.rgb / color.a) * color.a), color.a);
}

@fragment
fn fs_present_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 0u, false, false);
}

@fragment
fn fs_present_srgb_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 0u, true, false);
}

@fragment
fn fs_present_srgb_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 0u, false, true);
}

@fragment
fn fs_present_srgb_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 0u, true, true);
}

@fragment
fn fs_present_p3(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 1u, false, false);
}

@fragment
fn fs_present_p3_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 1u, true, false);
}

@fragment
fn fs_present_p3_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 1u, false, true);
}

@fragment
fn fs_present_p3_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 1u, true, true);
}

@fragment
fn fs_present_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 2u, false, false);
}

@fragment
fn fs_present_linear_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 2u, true, false);
}

@fragment
fn fs_present_linear_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 2u, false, true);
}

@fragment
fn fs_present_linear_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 2u, true, true);
}

@fragment
fn fs_present_extended_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 3u, false, false);
}

@fragment
fn fs_present_extended_srgb_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 3u, true, false);
}

@fragment
fn fs_present_extended_srgb_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 3u, false, true);
}

@fragment
fn fs_present_extended_srgb_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 3u, true, true);
}

@fragment
fn fs_present_extended_p3(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 4u, false, false);
}

@fragment
fn fs_present_extended_p3_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 4u, true, false);
}

@fragment
fn fs_present_extended_p3_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 4u, false, true);
}

@fragment
fn fs_present_extended_p3_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 4u, true, true);
}

@fragment
fn fs_present_hdr_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 5u, false, false);
}

@fragment
fn fs_present_hdr_srgb_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 5u, true, false);
}

@fragment
fn fs_present_hdr_srgb_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 5u, false, true);
}

@fragment
fn fs_present_hdr_srgb_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 5u, true, true);
}

@fragment
fn fs_present_hdr_p3(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 6u, false, false);
}

@fragment
fn fs_present_hdr_p3_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 6u, true, false);
}

@fragment
fn fs_present_hdr_p3_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 6u, false, true);
}

@fragment
fn fs_present_hdr_p3_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 6u, true, true);
}
