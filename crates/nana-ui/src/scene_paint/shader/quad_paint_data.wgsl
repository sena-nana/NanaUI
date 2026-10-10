// Shared QuadPaintData + stop/mask sampling. Bindings stay in the including shader.

const PAINT_GRADIENT: u32 = 1u;
const PAINT_MASK: u32 = 2u;
const PAINT_URL: u32 = 4u;
const PAINT_FILTER: u32 = 8u;
const PAINT_POLYGON: u32 = 16u;
const PAINT_RADIAL: u32 = 32u;
const PAINT_MASK_RADIAL: u32 = 64u;
const PAINT_SHADOW_INSET: u32 = 128u;
const PAINT_MASK_URL: u32 = 256u;
// Generated border-image gradients are already linear-premultiplied fp16.
const PAINT_URL_PREMULT: u32 = 512u;
// Gradient stops are premultiplied sRGB: decode after interpolating.
const PAINT_GRADIENT_SRGB: u32 = 1024u;

struct QuadPaintData {
    flags: u32,
    grad_angle: f32,
    grad_stop_count: u32,
    mask_stop_count: u32,
    mask_angle: f32,
    polygon_count: u32,
    url_tex_index: u32,
    url_fit: u32,
    filter_b: f32,
    filter_s: f32,
    filter_c: f32,
    filter_hue: f32,
    grad_stops0: vec4<f32>,
    grad_stops1: vec4<f32>,
    grad_stops2: vec4<f32>,
    grad_stops3: vec4<f32>,
    grad_pos: vec4<f32>,
    grad_pos2: vec4<f32>,
    // The mask stops' alpha, which is all a gradient mask reads.
    mask_alpha: vec4<f32>,
    mask_alpha2: vec4<f32>,
    mask_pos: vec4<f32>,
    poly0: vec4<f32>,
    poly1: vec4<f32>,
    poly2: vec4<f32>,
    poly3: vec4<f32>,
    grad_stops4: vec4<f32>,
    grad_stops5: vec4<f32>,
    grad_stops6: vec4<f32>,
    grad_stops7: vec4<f32>,
    mask_pos2: vec4<f32>,
    grad_center_x: f32,
    grad_center_y: f32,
    grad_radial_shape: u32,
    _pad_tail0: u32,
    mask_center_x: f32,
    mask_center_y: f32,
    mask_radial_shape: u32,
    _pad_tail1: u32,
    url_dest: vec4<f32>,
    // Four scalars, not vec3: vec3 in a storage struct is 16-byte aligned and
    // would inflate the stride past the CPU QuadPaintData (464).
    outline_width: f32,
    // Packed T/R/B/L 2-bit styles: 0 solid, 1 dashed, 2 dotted.
    border_styles: u32,
    filter_invert: f32,
    filter_opacity: f32,
    outline_color: vec4<f32>,
    border_color_right: vec4<f32>,
    border_color_bottom: vec4<f32>,
    border_color_left: vec4<f32>,
}

fn gradient_axis(angle_deg: f32) -> vec2<f32> {
    let rad = angle_deg * 0.017453292;
    return vec2(sin(rad), -cos(rad));
}

fn gradient_t(local: vec2<f32>, angle_deg: f32) -> f32 {
    let axis = gradient_axis(angle_deg);
    let p = local - vec2(0.5);
    let denom = abs(axis.x) + abs(axis.y);
    if (denom <= 0.0001) {
        return 0.5;
    }
    return clamp(p.x * axis.x / denom + p.y * axis.y / denom + 0.5, 0.0, 1.0);
}

fn radial_max_distance(center: vec2<f32>) -> f32 {
    let c0 = length(center);
    let c1 = length(vec2(1.0 - center.x, center.y));
    let c2 = length(vec2(center.x, 1.0 - center.y));
    let c3 = length(vec2(1.0 - center.x, 1.0 - center.y));
    return max(c0, max(c1, max(c2, c3)));
}

fn radial_gradient_t(local: vec2<f32>, center: vec2<f32>, circle: bool) -> f32 {
    let p = local - center;
    if (circle) {
        let dist = length(p);
        return clamp(dist / max(radial_max_distance(center), 0.0001), 0.0, 1.0);
    }
    let rx = max(center.x, 1.0 - center.x);
    let ry = max(center.y, 1.0 - center.y);
    let nx = p.x / max(rx, 0.0001);
    let ny = p.y / max(ry, 0.0001);
    return clamp(length(vec2(nx, ny)), 0.0, 1.0);
}

fn sample_stops(
    t: f32,
    count: u32,
    colors0: vec4<f32>,
    colors1: vec4<f32>,
    colors2: vec4<f32>,
    colors3: vec4<f32>,
    colors4: vec4<f32>,
    colors5: vec4<f32>,
    colors6: vec4<f32>,
    colors7: vec4<f32>,
    pos0: vec4<f32>,
    pos1: vec4<f32>,
) -> vec4<f32> {
    if (count <= 1u) {
        return colors0;
    }
    var colors = array<vec4<f32>, 8>(
        colors0, colors1, colors2, colors3, colors4, colors5, colors6, colors7,
    );
    var positions = array<f32, 8>(pos0.x, pos0.y, pos0.z, pos0.w, pos1.x, pos1.y, pos1.z, pos1.w);
    if (t <= positions[0]) {
        return colors[0];
    }
    let last = count - 1u;
    if (t >= positions[last]) {
        return colors[min(last, 7u)];
    }
    for (var i: u32 = 0u; i < min(count, 8u) - 1u; i = i + 1u) {
        let p0 = positions[i];
        let p1 = positions[i + 1u];
        if (t >= p0 && t <= p1) {
            let mix_t = (t - p0) / max(p1 - p0, 0.0001);
            return mix(colors[i], colors[min(i + 1u, 7u)], mix_t);
        }
    }
    return colors0;
}

// `sample_stops` over the stops' alpha alone.
fn sample_alpha_stops(
    t: f32,
    count: u32,
    alpha0: vec4<f32>,
    alpha1: vec4<f32>,
    pos0: vec4<f32>,
    pos1: vec4<f32>,
) -> f32 {
    var alpha = array<f32, 8>(alpha0.x, alpha0.y, alpha0.z, alpha0.w, alpha1.x, alpha1.y, alpha1.z, alpha1.w);
    var positions = array<f32, 8>(pos0.x, pos0.y, pos0.z, pos0.w, pos1.x, pos1.y, pos1.z, pos1.w);
    if (count <= 1u || t <= positions[0]) {
        return alpha[0];
    }
    let last = min(count, 8u) - 1u;
    if (t >= positions[last]) {
        return alpha[last];
    }
    for (var i: u32 = 0u; i < last; i = i + 1u) {
        let p0 = positions[i];
        let p1 = positions[i + 1u];
        if (t >= p0 && t <= p1) {
            return mix(alpha[i], alpha[i + 1u], (t - p0) / max(p1 - p0, 0.0001));
        }
    }
    return alpha[0];
}

// The node's gradient `mask-image` at `local`: the stops' alpha, as CSS
// `mask-mode: match-source` reads a gradient.
fn mask_alpha(local: vec2<f32>, paint: QuadPaintData) -> f32 {
    var t: f32;
    if ((paint.flags & PAINT_MASK_RADIAL) != 0u) {
        t = radial_gradient_t(
            local,
            vec2(paint.mask_center_x, paint.mask_center_y),
            paint.mask_radial_shape == 0u,
        );
    } else {
        t = gradient_t(local, paint.mask_angle);
    }
    return sample_alpha_stops(
        t,
        paint.mask_stop_count,
        paint.mask_alpha,
        paint.mask_alpha2,
        paint.mask_pos,
        paint.mask_pos2,
    );
}
