// Adapted from historical Iced (MIT).
struct Globals {
    transform: mat4x4<f32>,
    scale: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

fn rounded_box_sdf(p: vec2<f32>, size: vec2<f32>, corners: vec4<f32>) -> f32 {
    var box_half = select(corners.yz, corners.xw, p.x > 0.0);
    var corner = select(box_half.y, box_half.x, p.y > 0.0);
    var q = abs(p) - size + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner;
}

// `rounded_box_sdf` units per device pixel at `p`, where `dx`/`dy` are the
// screen derivatives of the position `p` was measured in. Dividing by it keeps
// an edge ramp one device pixel wide under any transform's scale. Along the
// analytic normal, not `dpdx` of the distance: that finite difference halves
// across a thin box's medial axis.
fn rounded_box_pixel(p: vec2<f32>, size: vec2<f32>, corners: vec4<f32>, dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    let box_half = select(corners.yz, corners.xw, p.x > 0.0);
    let corner = select(box_half.y, box_half.x, p.y > 0.0);
    let q = abs(p) - size + corner;
    let side = select(vec2(0.0, 1.0), vec2(1.0, 0.0), q.x > q.y);
    let normal = select(side, normalize(q), all(q > vec2(0.0)))
        * select(vec2(-1.0), vec2(1.0), p >= vec2(0.0));
    return max(length(vec2(dot(normal, dx), dot(normal, dy))), 1.0e-6);
}
