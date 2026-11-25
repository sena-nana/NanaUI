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
    filter_hue: f32,
    filter_blur: f32,
    mix_blend: u32,
    // The scene's corner curve exponent: 2 is a circular arc (`corner_norm`).
    corner_exponent: f32,
    drop_shadow_offset: vec2<f32>,
    drop_shadow_blur: f32,
    filter_invert: f32,
    drop_shadow_color: vec4<f32>,
    // Scroll edge fade in dest pixels: top edge, bottom edge, top band,
    // bottom band (`GroupSlot::edge_fade`).
    edge_fade: vec4<f32>,
    // An open shutter's streak in dest pixels: where the content was at the
    // start of the exposure, relative to now (`GroupSlot::motion_streak`).
    motion_streak: vec2<f32>,
    // Element blur taps either side of the centre (`element_blur_taps`):
    // 2 up to the 16 px cap, more for a group that opted into 64 px.
    filter_blur_taps: f32,
    filter_motion_id: u32,
    opacity_motion_id: u32,
    dest_scale: f32,
    max_filter_blur: f32,
    filter_opacity: f32,
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

fn layer_srgb_channel_to_linear(u: f32) -> f32 {
    if (u < 0.04045) {
        return u / 12.92;
    }
    return pow((u + 0.055) / 1.055, 2.4);
}

fn layer_srgb_to_linear3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        layer_srgb_channel_to_linear(c.x),
        layer_srgb_channel_to_linear(c.y),
        layer_srgb_channel_to_linear(c.z),
    );
}

// A gaussian of standard deviation `radius / 2` reaching out to `radius`,
// sampled `n` taps either side of the centre per axis. With `n == 2` it is
// the 5×5 kernel the 16 px cap was tuned for.
fn element_filter_blur(uv: vec2<f32>, radius: f32, blur_taps: f32) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source));
    let texel = 1.0 / max(dims, vec2<f32>(1.0));
    let n = clamp(i32(blur_taps), 2, 8);
    let nf = f32(n);
    let step = radius / nf;
    let falloff = 2.0 / (nf * nf);
    let span = 2 * n + 1;
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var row = 0; row < span; row = row + 1) {
        for (var column = 0; column < span; column = column + 1) {
            let d = vec2<f32>(f32(column - n), f32(row - n));
            let w = exp(-dot(d, d) * falloff);
            acc += textureSample(source, source_sampler, uv + d * step * texel) * w;
            wsum += w;
        }
    }
    return acc / max(wsum, 0.0001);
}

// The group's content at `uv`, blurred when its filter blurs.
fn element_sample(uv: vec2<f32>, filter_blur: f32, blur_taps: f32) -> vec4<f32> {
    if (filter_blur > 0.5) {
        return element_filter_blur(uv, filter_blur, blur_taps);
    }
    return textureSample(source, source_sampler, uv);
}

// What an open shutter exposes at `uv`: the content that passed over it
// during the exposure, averaged along the streak (Issue #323). Content now
// at `uv - k * streak` stood at `uv` a fraction `k` of the exposure ago.
fn shutter_sample(uv: vec2<f32>, filter_blur: f32, blur_taps: f32) -> vec4<f32> {
    let streak = layer.motion_streak;
    let length_px = length(streak);
    if (length_px < 0.5) {
        return element_sample(uv, filter_blur, blur_taps);
    }
    let dims = vec2<f32>(textureDimensions(source));
    let streak_uv = streak / max(dims, vec2<f32>(1.0));
    // About one tap per 2 dest pixels, fewer when each tap is itself a blur.
    var taps = clamp(i32(ceil(length_px * 0.5)), 2, 32);
    if (filter_blur > 0.5) {
        taps = min(taps, 8);
    }
    let samples = taps + 1;
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < samples; i = i + 1) {
        acc += element_sample(uv - streak_uv * (f32(i) / f32(taps)), filter_blur, blur_taps);
    }
    return acc / f32(samples);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return layer_source(input);
}

// The group's premultiplied colour at this pixel, after its clip, blur, drop
// shadow, opacity and colour filter.
fn layer_source(input: VertexOutput) -> vec4<f32> {
    // Clip inverse is stored in dest pixels (`FragmentClip::for_physical_pixels`).
    let clip_cover = fragment_clip_coverage(
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
        1.0,
        layer.corner_exponent,
    );
    if clip_cover <= 0.0 {
        discard;
    }
    var fb = layer.filter_b;
    var fs = layer.filter_s;
    var fc = layer.filter_c;
    var fhue = layer.filter_hue;
    var finv = layer.filter_invert;
    var fblur = layer.filter_blur;
    var fshadow_offset = layer.drop_shadow_offset;
    var fshadow_blur = layer.drop_shadow_blur;
    var fshadow_color = layer.drop_shadow_color;
    var fblur_taps = layer.filter_blur_taps;
    var fopacity = layer.filter_opacity;
    if (layer.filter_motion_id != 0u) {
        let fsamp = motion_evaluate(layer.filter_motion_id);
        if (fsamp.applies != 0u && fsamp.value.kind == MOTION_KIND_FILTER) {
            let c = fsamp.value.channels;
            let e = fsamp.value.extra;
            let e2 = fsamp.value.extra2;
            let e3 = fsamp.value.extra3;
            let scale = max(layer.dest_scale, 0.0001);
            let max_blur = max(layer.max_filter_blur, 0.0);
            fb = max(c.x, 0.0);
            fs = max(c.y, 0.0);
            fc = max(c.z, 0.0);
            fhue = c.w;
            finv = clamp(e.x, 0.0, 1.0);
            fopacity = clamp(e.y, 0.0, 1.0);
            let logical_blur = clamp(e.z, 0.0, max_blur);
            let logical_shadow_blur = clamp(e2.y, 0.0, max_blur);
            fblur = logical_blur * scale;
            fshadow_offset = vec2<f32>(e.w, e2.x) * scale;
            fshadow_blur = logical_shadow_blur * scale;
            let shadow_a = clamp(e3.y, 0.0, 1.0);
            if (shadow_a > 1.0e-5) {
                fshadow_color = vec4<f32>(
                    layer_srgb_to_linear3(clamp(vec3<f32>(e2.z, e2.w, e3.x), vec3<f32>(0.0), vec3<f32>(1.0))),
                    shadow_a,
                );
            } else {
                fshadow_color = vec4<f32>(0.0);
            }
            let max_logical = max(logical_blur, logical_shadow_blur);
            fblur_taps = select(2.0, clamp(ceil(max_logical / 8.0), 2.0, 8.0), max_logical > 16.0);
        }
    }
    var group_opacity = layer.opacity;
    if (layer.opacity_motion_id != 0u) {
        group_opacity = clamp(motion_sample_scalar(motion_evaluate(layer.opacity_motion_id), group_opacity), 0.0, 1.0);
    }
    let total_opacity = group_opacity * fopacity;

    var sampled = shutter_sample(input.uv, fblur, fblur_taps);
    if (fshadow_color.a > 0.001) {
        let dims = vec2<f32>(textureDimensions(source));
        let offset_uv = fshadow_offset / max(dims, vec2<f32>(1.0));
        let shadow_uv = input.uv - offset_uv;
        var shadow_sample = textureSample(source, source_sampler, shadow_uv);
        if (fshadow_blur > 0.5) {
            shadow_sample = element_filter_blur(shadow_uv, fshadow_blur, fblur_taps);
        }
        let shadow_a = shadow_sample.a * fshadow_color.a;
        let shadow = vec4<f32>(fshadow_color.rgb * shadow_a, shadow_a);
        sampled = sampled + shadow * (1.0 - sampled.a);
    }
    sampled = sampled * total_opacity;
    if (fb != 1.0
        || fs != 1.0
        || fc != 1.0
        || abs(fhue) > 0.0001
        || finv > 0.0001)
    {
        sampled = apply_color_filter_channels(
            sampled,
            fb,
            fs,
            fc,
            fhue,
            finv,
            1.0,
        );
    }
    // After the filter, which must not see the faded edge.
    return sampled * (clip_cover * edge_fade_coverage(input.position.y));
}

// How much of the group shows at dest row `y` under its scroll edge fade:
// 0 at the scrollport's top and bottom edges, ramping linearly to 1 across
// each band. Rows outside the bands, and above or below the scrollport (a
// shadow the scroll container casts), are untouched.
fn edge_fade_coverage(y: f32) -> f32 {
    let fade = layer.edge_fade;
    var cover = 1.0;
    if fade.z > 0.0 && y >= fade.x {
        cover = min(cover, clamp((y - fade.x) / fade.z, 0.0, 1.0));
    }
    if fade.w > 0.0 && y <= fade.y {
        cover = min(cover, clamp((fade.y - y) / fade.w, 0.0, 1.0));
    }
    return cover;
}
