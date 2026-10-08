@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;

// Dynamic display metadata.  This is deliberately separate from the
// ScenePresentationProfile pipeline key: moving a window between displays or
// changing EDR headroom updates this small uniform without rebuilding the
// painter's destination pipelines.
// x = tone-map headroom above SDR reference white, y = SDR reference white
// in nits, z = SDR white in an extended-linear target's units (Windows scRGB
// puts 1.0 at 80 nits).  The host normalizes all to finite, conservative
// bounds.
@group(0) @binding(2)
var<uniform> presentation_params: vec4<f32>;

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
        dot(rgb, vec3<f32>(0.8224621, 0.1775380, 0.0000000)),
        dot(rgb, vec3<f32>(0.0331941, 0.9668058, 0.0000000)),
        dot(rgb, vec3<f32>(0.0170827, 0.0723974, 0.9105199)),
    );
}

// BT.709/sRGB linear -> BT.2020 linear, D65.  WGSL matrices are column
// major, so these columns are the rows of the CPU reference matrix.
fn sc_to_bt2020(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(rgb, vec3<f32>(0.627402, 0.329292, 0.043306)),
        dot(rgb, vec3<f32>(0.069095, 0.919544, 0.011360)),
        dot(rgb, vec3<f32>(0.016394, 0.088028, 0.895578)),
    );
}

// Apply one highlight scale to channels above reference white, preserving
// neutral highlight chroma without lifting unrelated SDR midtones. Reference
// white remains an exact 1.0 for every headroom value.
fn tone_map_headroom3(rgb: vec3<f32>, requested: f32) -> vec3<f32> {
    // Keep malformed custom-node values from poisoning the shared scale. WGSL
    // has no portable `isFinite` builtin; x == x rejects NaN and the magnitude
    // bound rejects infinities on all backends.
    let finite_rgb = vec3<f32>(
        select(0.0, rgb.x, rgb.x == rgb.x && abs(rgb.x) < 1.0e20),
        select(0.0, rgb.y, rgb.y == rgb.y && abs(rgb.y) < 1.0e20),
        select(0.0, rgb.z, rgb.z == rgb.z && abs(rgb.z) < 1.0e20),
    );
    let peak = max(finite_rgb.x, max(finite_rgb.y, finite_rgb.z));
    if (!(peak > 1.0)) {
        return finite_rgb;
    }
    // The host normalizes this uniform. The NaN self-comparison keeps a
    // malformed external update fail closed without relying on removed WGSL
    // `isFinite` builtins.
    let finite = requested == requested && abs(requested) < 1.0e20;
    let headroom = clamp(select(1.0, requested, finite), 1.0, 125.0);
    if (!(headroom > 1.0)) {
        // No room above white: scale the colour as a whole until its
        // brightest channel is white, so its hue survives.
        return finite_rgb / peak;
    }
    let mapped_peak =
        1.0 + (headroom - 1.0) * (1.0 - exp(-(peak - 1.0) / (headroom - 1.0)));
    // Anchor the shared highlight scale at reference white so a channel
    // crossing 1.0 stays continuous even alongside a much brighter channel.
    let scaled = vec3<f32>(1.0) + (finite_rgb - vec3<f32>(1.0)) * ((mapped_peak - 1.0) / (peak - 1.0));
    return select(scaled, finite_rgb, finite_rgb <= vec3<f32>(1.0));
}

fn pq_oetf3(normalized_nits: vec3<f32>) -> vec3<f32> {
    let m1 = vec3<f32>(0.1593017578125);
    let m2 = vec3<f32>(78.84375);
    let c1 = vec3<f32>(0.8359375);
    let c2 = vec3<f32>(18.8515625);
    let c3 = vec3<f32>(18.6875);
    // ST 2084's code range ends at 10,000 nits. CPU helpers clamp the
    // normalized input too, so keeping this clamp here makes clear and blit
    // paths agree even when reference white/headroom metadata is extreme.
    let clamped = clamp(normalized_nits, vec3<f32>(0.0), vec3<f32>(1.0));
    let yp = pow(clamped, m1);
    let encoded = pow((c1 + c2 * yp) / (vec3<f32>(1.0) + c3 * yp), m2);
    return select(encoded, vec3<f32>(0.0), normalized_nits <= vec3<f32>(0.0));
}

fn hlg_oetf3(scene_luminance: vec3<f32>) -> vec3<f32> {
    let a = vec3<f32>(0.17883277);
    let b = vec3<f32>(0.28466892);
    let c = vec3<f32>(0.55991073);
    let value = clamp(scene_luminance, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = sqrt(3.0 * value);
    let hi = a * log(max(12.0 * value - b, vec3<f32>(1.0e-6))) + c;
    return select(hi, lo, value <= vec3<f32>(1.0 / 12.0));
}

// `mode`: SDR sRGB/P3, extended linear/sRGB/P3, HDR-to-SDR sRGB/P3, or
// BT.2100 PQ/HLG (7/8).
fn presentation_rgb(straight: vec3<f32>, mode: u32) -> vec3<f32> {
    switch mode {
        case 0u: { return gamut_map(straight); }
        case 1u: { return gamut_map(sc_to_p3(straight)); }
        case 2u: { return tone_map_headroom3(straight, presentation_params.x) * presentation_params.z; }
        case 3u: { return tone_map_headroom3(straight, presentation_params.x); }
        case 4u: { return sc_to_p3(tone_map_headroom3(straight, presentation_params.x)); }
        case 5u: { return gamut_map(tone_map_headroom3(straight, 1.0)); }
        case 6u: { return gamut_map(sc_to_p3(tone_map_headroom3(straight, 1.0))); }
        case 7u: {
            let headroom = presentation_params.x;
            let white_nits = max(presentation_params.y, 1.0);
            return pq_oetf3(sc_to_bt2020(tone_map_headroom3(straight, headroom)) * white_nits / 10000.0);
        }
        case 8u: {
            let headroom = presentation_params.x;
            let white_nits = max(presentation_params.y, 1.0);
            let display = clamp(
                sc_to_bt2020(tone_map_headroom3(straight, headroom)) * white_nits / 1000.0,
                vec3<f32>(0.0),
                vec3<f32>(1.0),
            );
            // HLG encodes scene light: undo the reference display's OOTF
            // (system gamma 1.2) before the OETF; see `hlg_encode_display`.
            let luminance = dot(display, vec3<f32>(0.2627, 0.6780, 0.0593));
            if (!(luminance > 0.0)) {
                return vec3<f32>(0.0);
            }
            return hlg_oetf3(display * pow(luminance, (1.0 - 1.2) / 1.2));
        }
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
    if (mode == 7u || mode == 8u) {
        // PQ/HLG values already are the encoded BT.2100 signal.  Do not run
        // them through sRGB's OETF. A typed-sRGB target is not a valid
        // negotiated BT.2100 surface, but pre-decode it here so a manually
        // constructed offscreen profile still has deterministic output.
        if (srgb_target) {
            return vec4<f32>(srgb_to_linear3(mapped * color.a), color.a);
        }
        return vec4<f32>(mapped * color.a, color.a);
    }
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

@fragment
fn fs_present_bt2100_pq(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 7u, false, false);
}

@fragment
fn fs_present_bt2100_pq_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 7u, true, false);
}

@fragment
fn fs_present_bt2100_pq_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 7u, false, true);
}

@fragment
fn fs_present_bt2100_pq_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 7u, true, true);
}

@fragment
fn fs_present_bt2100_hlg(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 8u, false, false);
}

@fragment
fn fs_present_bt2100_hlg_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 8u, true, false);
}

@fragment
fn fs_present_bt2100_hlg_typed(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 8u, false, true);
}

@fragment
fn fs_present_bt2100_hlg_typed_gamma(input: VertexOutput) -> @location(0) vec4<f32> {
    return present_sample(input, 8u, true, true);
}
