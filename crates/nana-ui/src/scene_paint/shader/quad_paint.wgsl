// Quad fill bindings and compose. Shared data lives in quad_paint_data.wgsl.

struct PaintBuffer {
    items: array<QuadPaintData>,
}

@group(0) @binding(1)
var<storage, read> paint_buffer: PaintBuffer;

@group(0) @binding(2)
var url_tex: texture_2d<f32>;
@group(0) @binding(3)
var url_sampler: sampler;

fn source_over_premult(base: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    let inv = 1.0 - src.a;
    return vec4(base.rgb * inv + src.rgb, base.a * inv + src.a);
}

fn apply_color_filter(color: vec4<f32>, paint: QuadPaintData) -> vec4<f32> {
    var rgb = color.xyz * paint.filter_b;
    let lum = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3(lum), rgb, paint.filter_s);
    rgb = (rgb - 0.5) * paint.filter_c + 0.5;
    return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), color.a);
}

fn point_in_polygon(local: vec2<f32>, paint: QuadPaintData) -> bool {
    if (paint.polygon_count < 3u) {
        return true;
    }
    var verts = array<vec2<f32>, 8>(
        paint.poly0.xy, paint.poly0.zw, paint.poly1.xy, paint.poly1.zw,
        paint.poly2.xy, paint.poly2.zw, paint.poly3.xy, paint.poly3.zw,
    );
    var inside = false;
    var j = paint.polygon_count - 1u;
    for (var i: u32 = 0u; i < paint.polygon_count; i = i + 1u) {
        let vi = verts[i];
        let vj = verts[j];
        if ((vi.y > local.y) != (vj.y > local.y))
            && (local.x < (vj.x - vi.x) * (local.y - vi.y) / max(vj.y - vi.y, 0.0001) + vi.x) {
            inside = !inside;
        }
        j = i;
    }
    return inside;
}

fn sample_url(local: vec2<f32>, paint: QuadPaintData) -> vec4<f32> {
    var uv = local;
    let dims = textureDimensions(url_tex);
    let tex_aspect = f32(dims.x) / max(f32(dims.y), 1.0);
    let box_aspect = 1.0;
    if (paint.url_fit == 0u) {
        if (tex_aspect > box_aspect) {
            let scale = box_aspect / tex_aspect;
            uv.x = (local.x - 0.5) / scale + 0.5;
        } else {
            let scale = tex_aspect / box_aspect;
            uv.y = (local.y - 0.5) / scale + 0.5;
        }
    } else if (paint.url_fit == 1u) {
        if (tex_aspect > box_aspect) {
            let scale = tex_aspect / box_aspect;
            uv.y = (local.y - 0.5) / scale + 0.5;
        } else {
            let scale = box_aspect / tex_aspect;
            uv.x = (local.x - 0.5) / scale + 0.5;
        }
    }
    return textureSample(url_tex, url_sampler, uv);
}

fn compose_quad_fill(base: vec4<f32>, local: vec2<f32>, paint: QuadPaintData) -> vec4<f32> {
    var color = base;
    if ((paint.flags & PAINT_GRADIENT) != 0u) {
        var t: f32;
        if ((paint.flags & PAINT_RADIAL) != 0u) {
            t = radial_gradient_t(
                local,
                vec2(paint.grad_center_x, paint.grad_center_y),
                paint.grad_radial_shape == 0u,
            );
        } else {
            t = gradient_t(local, paint.grad_angle);
        }
        let grad = sample_stops(
            t,
            paint.grad_stop_count,
            paint.grad_stops0,
            paint.grad_stops1,
            paint.grad_stops2,
            paint.grad_stops3,
            paint.grad_stops4,
            paint.grad_stops5,
            paint.grad_stops6,
            paint.grad_stops7,
            paint.grad_pos,
            paint.grad_pos2,
        );
        let grad_premult = vec4(grad.rgb * grad.a, grad.a);
        color = source_over_premult(color, grad_premult);
    }
    if ((paint.flags & PAINT_URL) != 0u) {
        let sampled = sample_url(local, paint);
        let sampled_premult = vec4(sampled.rgb * sampled.a, sampled.a);
        color = source_over_premult(color, sampled_premult);
    }
    if ((paint.flags & PAINT_MASK) != 0u) {
        let m = mask_alpha(local, paint);
        color = vec4(color.rgb * m, color.a * m);
    }
    if ((paint.flags & PAINT_FILTER) != 0u) {
        color = apply_color_filter(color, paint);
    }
    return color;
}
