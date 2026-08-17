//! sRGB packing matching in-tree Iced (`engine/iced/graphics/src/color.rs`).

/// Converts an sRGB Scene color to the linear RGBA Iced packs into quad/mesh
/// instances when gamma correction is enabled.
pub(super) fn pack_linear([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    [
        linear_component(r),
        linear_component(g),
        linear_component(b),
        a,
    ]
}

pub(super) fn with_opacity([r, g, b, a]: [f32; 4], opacity: f32) -> [f32; 4] {
    [r, g, b, a * opacity]
}

pub(super) fn to_rgba8([r, g, b, a]: [f32; 4]) -> [u8; 4] {
    [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        (a * 255.0).round() as u8,
    ]
}

fn linear_component(u: f32) -> f32 {
    if u < 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

/// OpenGL-style ortho matching Iced: (0,0) top-left, Y-down, physical pixels.
pub(super) fn orthographic(width: u32, height: u32) -> [f32; 16] {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    [
        2.0 / width,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / height,
        0.0,
        0.0,
        0.0,
        0.0,
        -1.0,
        0.0,
        -1.0,
        1.0,
        0.0,
        1.0,
    ]
}

/// `orthographic(physical) * scale(scale_factor)` for logical-space meshes.
pub(super) fn orthographic_scaled(width: u32, height: u32, scale: f32) -> [f32; 16] {
    let mut matrix = orthographic(width, height);
    matrix[0] *= scale;
    matrix[5] *= scale;
    matrix
}
