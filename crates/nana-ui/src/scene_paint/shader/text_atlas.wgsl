// Shared by both text programs: the projection, the two atlas pages, the two
// samplers, and the sRGB unpack. Keeping it in one file is what makes "an
// upright label and a rotated one sample the same page" true by construction
// rather than by two declarations agreeing.
struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

@group(1) @binding(0)
var mask_atlas: texture_2d<f32>;

@group(1) @binding(1)
var color_atlas: texture_2d<f32>;

@group(1) @binding(2)
var atlas_nearest: sampler;

@group(1) @binding(3)
var atlas_linear: sampler;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// Scene colors are sRGB-encoded and the target is linear, so the conversion is
// the same one every other pipeline applies. Doing it here rather than on the
// CPU is what lets an axis instance carry four bytes of color.
fn unpack_srgb(color: u32) -> vec4<f32> {
    return vec4<f32>(
        srgb_to_linear(f32((color & 0x00ff0000u) >> 16u) / 255.0),
        srgb_to_linear(f32((color & 0x0000ff00u) >> 8u) / 255.0),
        srgb_to_linear(f32(color & 0x000000ffu) / 255.0),
        f32((color & 0xff000000u) >> 24u) / 255.0,
    );
}

// The page a glyph came from, in normalized coordinates.
fn atlas_uv(texel: vec2<u32>, content: u32) -> vec2<f32> {
    var dim = vec2<u32>(1u);
    if content == 0u {
        dim = textureDimensions(mask_atlas);
    } else {
        dim = textureDimensions(color_atlas);
    }
    return vec2<f32>(texel) / vec2<f32>(dim);
}
