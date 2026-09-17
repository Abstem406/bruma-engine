// bruma's welcome triangle — Phase 2, step 1.
//
// No vertices: the triangle is generated in the vertex shader from
// `vertex_index` ("full-screen triangle" technique). Colors arrive by
// interpolation of the triangle coordinate.
//
// WGSL is the engine's only shader language (D3): this same file will
// run unchanged in the web gallery (WebGPU) in Phase 7.

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    // Triangle covering more than the screen (clip space).
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let pos = positions[idx];

    // Fixed per-vertex colors, graphics "hello world" style.
    let colors = array<vec3<f32>, 3>(
        vec3<f32>(0.18, 0.20, 0.29), // dark indigo
        vec3<f32>(0.55, 0.30, 0.35), // wine
        vec3<f32>(0.83, 0.69, 0.44), // sand
    );
    let color = colors[idx];

    var out: VsOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
