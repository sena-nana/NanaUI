// Adapted from historical Iced (MIT).
fn premultiply(color: vec4<f32>) -> vec4<f32> {
    return vec4(color.xyz * color.a, color.a);
}

fn clip_apply_affine(abcd: vec4<f32>, ef: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        abcd.x * p.x + abcd.z * p.y + ef.x,
        abcd.y * p.x + abcd.w * p.y + ef.y
    );
}

fn inside_transformed_rect(
    world: vec2<f32>,
    rect: vec4<f32>,
    inv_abcd: vec4<f32>,
    inv_ef: vec2<f32>
) -> bool {
    let local = clip_apply_affine(inv_abcd, inv_ef, world);
    return all(local >= rect.xy) && all(local <= rect.xy + rect.zw);
}

fn clip_rounded_box_sdf(p: vec2<f32>, size: vec2<f32>, corner: f32) -> f32 {
    let q = abs(p) - size + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner;
}

fn clip_polygon_point(index: u32, poly0: vec4<f32>, poly1: vec4<f32>, poly2: vec4<f32>, poly3: vec4<f32>) -> vec2<f32> {
    switch index {
        case 0u: { return poly0.xy; }
        case 1u: { return poly0.zw; }
        case 2u: { return poly1.xy; }
        case 3u: { return poly1.zw; }
        case 4u: { return poly2.xy; }
        case 5u: { return poly2.zw; }
        case 6u: { return poly3.xy; }
        default: { return poly3.zw; }
    }
}

fn point_in_clip_polygon(local: vec2<f32>, count: u32, poly0: vec4<f32>, poly1: vec4<f32>, poly2: vec4<f32>, poly3: vec4<f32>) -> bool {
    if (count < 3u) {
        return true;
    }
    var winding = 0;
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let j = (i + 1u) % count;
        let vi = clip_polygon_point(i, poly0, poly1, poly2, poly3);
        let vj = clip_polygon_point(j, poly0, poly1, poly2, poly3);
        if (vi.y <= local.y) {
            if (vj.y > local.y) {
                let cross = (vj.x - vi.x) * (local.y - vi.y) - (local.x - vi.x) * (vj.y - vi.y);
                if (cross > 0.0) {
                    winding = winding + 1;
                }
            }
        } else if (vj.y <= local.y) {
            let cross = (vj.x - vi.x) * (local.y - vi.y) - (local.x - vi.x) * (vj.y - vi.y);
            if (cross < 0.0) {
                winding = winding - 1;
            }
        }
    }
    return winding != 0;
}

fn inside_fragment_clip(
    world: vec2<f32>,
    rect: vec4<f32>,
    inv_abcd: vec4<f32>,
    inv_ef: vec2<f32>,
    corner_radius: f32,
    polygon_count: u32,
    poly0: vec4<f32>,
    poly1: vec4<f32>,
    poly2: vec4<f32>,
    poly3: vec4<f32>,
) -> bool {
    if !inside_transformed_rect(world, rect, inv_abcd, inv_ef) {
        return false;
    }
    let local = clip_apply_affine(inv_abcd, inv_ef, world);
    let rel = local - rect.xy;
    if (polygon_count >= 3u) && !point_in_clip_polygon(rel, polygon_count, poly0, poly1, poly2, poly3) {
        return false;
    }
    if (corner_radius <= 0.0) {
        return true;
    }
    let half = rect.zw * 0.5;
    let center = rel - half;
    let radius = min(corner_radius, min(half.x, half.y));
    return clip_rounded_box_sdf(center, half, radius) <= 0.0;
}

fn unpack_color(data: vec2<u32>) -> vec4<f32> {
    return premultiply(unpack_u32(data));
}

fn unpack_u32(data: vec2<u32>) -> vec4<f32> {
    let rg: vec2<f32> = unpack2x16float(data.x);
    let ba: vec2<f32> = unpack2x16float(data.y);

    return vec4<f32>(rg.y, rg.x, ba.y, ba.x);
}
