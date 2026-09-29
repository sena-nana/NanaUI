//! Keyframed paint transforms, sampled through the adapter's motion path.
use crate::css_interactive::parse_keyframes_at_rule;
use crate::css_interactive_apply::keyframe_paint_at;

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

#[test]
fn keyframes_lerp_2d_rotate_affine() {
    let (rule, _) = parse_keyframes_at_rule(
        "@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(90deg); } }",
        0,
    )
    .expect("keyframes");
    let mid = keyframe_paint_at(&rule, 0.5).expect("sample");
    let transform = mid.transform.expect("transform");
    // Motion bucket lerps the 2×3, not the angle.
    approx(transform.a, 0.5);
    approx(transform.b, 0.5);
    approx(transform.c, -0.5);
    approx(transform.d, 0.5);
}
