// Adapted from historical Iced (MIT).
struct SolidVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
    @location(0) color: vec4<f32>,
    @location(1) pos: vec2<f32>,
    @location(2) scale: vec2<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_radius: vec4<f32>,
    @location(5) border_widths: vec4<f32>,
    @location(6) shadow_color: vec4<f32>,
    @location(7) shadow_offset: vec2<f32>,
    @location(8) shadow_blur_radius: f32,
    @location(9) shadow_spread_radius: f32,
    @location(10) snap: u32,
    @location(11) affine_abcd: vec4<f32>,
    @location(12) affine_ef: vec4<f32>,
    @location(13) clip_rect: vec4<f32>,
    @location(14) clip_inv_abcd: vec4<f32>,
    @location(15) clip_inv_ef: vec3<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) border_color: vec4<f32>,
    @location(2) pos: vec2<f32>,
    @location(3) scale: vec2<f32>,
    @location(4) border_radius: vec4<f32>,
    @location(5) border_widths: vec4<f32>,
    @location(6) shadow_color: vec4<f32>,
    @location(7) shadow_offset: vec2<f32>,
    @location(8) shadow_blur_radius: f32,
    @location(9) shadow_spread_radius: f32,
    @location(10) local_pos: vec2<f32>,
    @location(11) world_pos: vec2<f32>,
    @location(12) clip_rect: vec4<f32>,
    @location(13) clip_inv_abcd: vec4<f32>,
    @location(14) clip_inv_ef: vec3<f32>,
    @location(15) @interpolate(flat) instance_index: u32,
}

fn apply_affine(abcd: vec4<f32>, ef: vec4<f32>, p: vec2<f32>) -> vec2<f32> {
    let x = abcd.x * p.x + abcd.z * p.y + ef.x;
    let y = abcd.y * p.x + abcd.w * p.y + ef.y;
    let w = ef.z * p.x + ef.w * p.y + 1.0;
    if (abs(w) < 1.0e-8) {
        return vec2<f32>(x, y);
    }
    return vec2<f32>(x, y) / w;
}

fn border_dash_mask(along: f32, across: f32, width: f32, style: u32) -> f32 {
    if (style == 0u) {
        return 1.0;
    }
    let w = max(width, 1.0);
    if (style == 2u) {
        let period = w * 2.0;
        let nearest = round(along / period) * period;
        let d = length(vec2(along - nearest, across));
        return clamp(0.5 - (d - w * 0.5), 0.0, 1.0);
    }
    let dash = w * 3.0;
    let period = dash + w * 2.0;
    let t = fract(along / period) * period;
    let sd = select(min(t - dash, period - t), -min(t, dash - t), t < dash);
    return clamp(0.5 - sd, 0.0, 1.0);
}

@vertex
fn solid_vs_main(input: SolidVertexInput) -> SolidVertexOutput {
    var out: SolidVertexOutput;

    // Outline/inset expansion is packed into instance shadow radii on the CPU so
    // this stage never reads storage (VERTEX_STORAGE is not guaranteed).
    let shadow_outset = input.shadow_blur_radius + max(input.shadow_spread_radius, 0.0);
    var pos: vec2<f32> = (input.pos + min(input.shadow_offset, vec2<f32>(0.0, 0.0)) - shadow_outset) * globals.scale;
    var scale: vec2<f32> = (input.scale + vec2<f32>(abs(input.shadow_offset.x), abs(input.shadow_offset.y)) + shadow_outset * 2.0) * globals.scale;

    var pos_snap = vec2<f32>(0.0, 0.0);
    var scale_snap = vec2<f32>(0.0, 0.0);

    if bool(input.snap) {
        pos_snap = round(pos + vec2(0.001, 0.001)) - pos;
        scale_snap = round(pos + scale + vec2(0.001, 0.001)) - pos - pos_snap - scale;
    }

    let border_radius = min(input.border_radius, vec4(min(input.scale.x, input.scale.y) / 2.0));
    let unit = vertex_position(input.vertex_index);
    let local = pos + pos_snap - vec2<f32>(0.5, 0.5) + unit * (scale + scale_snap + 1.0);
    let logical = local / globals.scale;
    let world = apply_affine(input.affine_abcd, input.affine_ef, logical);

    out.position = globals.transform * vec4<f32>(world * globals.scale, 0.0, 1.0);
    out.color = premultiply(input.color);
    out.border_color = premultiply(input.border_color);
    out.pos = input.pos * globals.scale + pos_snap;
    out.scale = input.scale * globals.scale + scale_snap;
    out.border_radius = border_radius * globals.scale;
    out.border_widths = input.border_widths * globals.scale;
    out.shadow_color = premultiply(input.shadow_color);
    out.shadow_offset = input.shadow_offset * globals.scale;
    out.shadow_blur_radius = input.shadow_blur_radius * globals.scale;
    out.shadow_spread_radius = input.shadow_spread_radius * globals.scale;
    out.local_pos = local;
    out.world_pos = world;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;
    out.instance_index = input.instance_index;

    return out;
}

@fragment
fn solid_fs_main(
    input: SolidVertexOutput
) -> @location(0) vec4<f32> {
    if !inside_fragment_clip(
        input.world_pos,
        input.clip_rect,
        input.clip_inv_abcd,
        input.clip_inv_ef.xy,
        input.clip_inv_ef.z,
        0u,
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    ) {
        discard;
    }

    let paint = paint_buffer.items[input.instance_index];
    let local_uv = (input.local_pos - input.pos) / max(input.scale, vec2(0.0001));

    if ((paint.flags & PAINT_POLYGON) != 0u) && !point_in_polygon(local_uv, paint) {
        discard;
    }

    var mixed_color: vec4<f32> = compose_quad_fill(input.color, local_uv, paint);

    var dist = rounded_box_sdf(
        -(input.local_pos - input.pos - input.scale * 0.5) * 2.0,
        input.scale,
        input.border_radius * 2.0
    ) / 2.0;

    if (max(max(input.border_widths.x, input.border_widths.y), max(input.border_widths.z, input.border_widths.w)) > 0.0) {
        let inner_shift = vec2(
            (input.border_widths.w - input.border_widths.y) * 0.5,
            (input.border_widths.x - input.border_widths.z) * 0.5,
        );
        let inner_size = max(
            input.scale - vec2(
                input.border_widths.w + input.border_widths.y,
                input.border_widths.x + input.border_widths.z,
            ),
            vec2(0.0),
        );
        var inner_radii = max(
            input.border_radius - vec4(
                min(input.border_widths.x, input.border_widths.w),
                min(input.border_widths.x, input.border_widths.y),
                min(input.border_widths.z, input.border_widths.y),
                min(input.border_widths.z, input.border_widths.w),
            ),
            vec4(0.0),
        );
        inner_radii = min(inner_radii, vec4(min(inner_size.x, inner_size.y) * 0.5));
        let inner_dist = rounded_box_sdf(
            -(input.local_pos - input.pos - input.scale * 0.5 - inner_shift) * 2.0,
            inner_size,
            inner_radii * 2.0
        ) / 2.0;
        let lp = input.local_pos - input.pos;
        let dt = select(1e8, lp.y / max(input.border_widths.x, 1e-4), input.border_widths.x > 0.0);
        let dr = select(1e8, (input.scale.x - lp.x) / max(input.border_widths.y, 1e-4), input.border_widths.y > 0.0);
        let db = select(1e8, (input.scale.y - lp.y) / max(input.border_widths.z, 1e-4), input.border_widths.z > 0.0);
        let dl = select(1e8, lp.x / max(input.border_widths.w, 1e-4), input.border_widths.w > 0.0);
        var edge_color = input.border_color;
        let nearest = min(min(dt, dr), min(db, dl));
        if (nearest == dr) {
            edge_color = premultiply(paint.border_color_right);
        } else if (nearest == db) {
            edge_color = premultiply(paint.border_color_bottom);
        } else if (nearest == dl) {
            edge_color = premultiply(paint.border_color_left);
        }
        var cover = clamp(0.5 + inner_dist, 0.0, 1.0);
        if (paint.border_styles != 0u) {
            var side: u32 = 0u;
            var along = lp.x;
            var across = lp.y - input.border_widths.x * 0.5;
            var bw = input.border_widths.x;
            if (nearest == dr) {
                side = 1u;
                along = lp.y;
                across = (input.scale.x - lp.x) - input.border_widths.y * 0.5;
                bw = input.border_widths.y;
            } else if (nearest == db) {
                side = 2u;
                along = lp.x;
                across = (input.scale.y - lp.y) - input.border_widths.z * 0.5;
                bw = input.border_widths.z;
            } else if (nearest == dl) {
                side = 3u;
                along = lp.y;
                across = lp.x - input.border_widths.w * 0.5;
                bw = input.border_widths.w;
            }
            cover *= border_dash_mask(along, across, bw, (paint.border_styles >> (side * 2u)) & 3u);
        }
        mixed_color = mix(mixed_color, edge_color, cover);
    }

    var quad_alpha: f32 = clamp(0.5-dist, 0.0, 1.0);

    // Storage keeps CSS px (same as instance spread before VS scale). `dist`
    // and interpolated `shadow_spread_radius` are already physical.
    let outline_px = paint.outline_width * globals.scale;
    if (outline_px > 0.0) {
        let cover = clamp(0.5 - dist + outline_px, 0.0, 1.0);
        let outline_premult = premultiply(paint.outline_color);
        mixed_color = mix(outline_premult, mixed_color, quad_alpha);
        quad_alpha = cover;
    }

    let quad_color = mixed_color * quad_alpha;

    if input.shadow_color.a > 0.0 {
        let css_spread = input.shadow_spread_radius - outline_px;
        let shadow_size = max(input.scale + vec2(css_spread * 2.0), vec2(0.0));
        let shadow_radius = max(input.border_radius + vec4(css_spread), vec4(0.0));
        var shadow_dist: f32 = rounded_box_sdf(
            -(input.local_pos - input.pos - input.shadow_offset - input.scale/2.0) * 2.0,
            shadow_size,
            shadow_radius * 2.0
        ) / 2.0;
        if ((paint.flags & PAINT_SHADOW_INSET) != 0u) {
            let shadow_alpha = 1.0 - smoothstep(
                -input.shadow_blur_radius,
                input.shadow_blur_radius,
                max(-shadow_dist, 0.0),
            );
            return mix(quad_color, input.shadow_color, clamp(0.5 - dist, 0.0, 1.0) * shadow_alpha);
        }
        let shadow_alpha = 1.0 - smoothstep(-input.shadow_blur_radius, input.shadow_blur_radius, max(shadow_dist, 0.0));

        return mix(quad_color, input.shadow_color, (1.0 - quad_alpha) * shadow_alpha);
    } else {
        return quad_color;
    }
}
