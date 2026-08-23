// Adapted from historical Iced (MIT).
struct SolidVertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_inv_abcd: vec4<f32>,
    @location(4) clip_inv_ef: vec2<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec2<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) clip_inv_abcd: vec4<f32>,
    @location(4) clip_inv_ef: vec2<f32>,
}

@vertex
fn solid_vs_main(input: SolidVertexInput) -> SolidVertexOutput {
    var out: SolidVertexOutput;

    out.color = premultiply(input.color);
    out.position = globals.transform * vec4<f32>(input.position, 0.0, 1.0);
    out.world_pos = input.position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;

    return out;
}

@fragment
fn solid_fs_main(input: SolidVertexOutput) -> @location(0) vec4<f32> {
    if !inside_transformed_rect(
        input.world_pos,
        input.clip_rect,
        input.clip_inv_abcd,
        input.clip_inv_ef
    ) {
        discard;
    }
    return input.color;
}
