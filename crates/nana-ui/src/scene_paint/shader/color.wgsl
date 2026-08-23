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

fn unpack_color(data: vec2<u32>) -> vec4<f32> {
    return premultiply(unpack_u32(data));
}

fn unpack_u32(data: vec2<u32>) -> vec4<f32> {
    let rg: vec2<f32> = unpack2x16float(data.x);
    let ba: vec2<f32> = unpack2x16float(data.y);

    return vec4<f32>(rg.y, rg.x, ba.y, ba.x);
}
