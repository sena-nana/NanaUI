struct SceneUniform {
    parameters: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: SceneUniform;

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

@fragment
fn fragment_main() -> @location(0) vec4<f32> {
    let seed = scene.parameters.x;
    return vec4<f32>(0.12 + seed * 0.01, 0.28, 0.46, 1.0);
}
