// Triángulo de bienvenida de bruma — Fase 2, paso 1.
//
// Sin vértices: el triángulo se genera en el vertex shader a partir de
// `vertex_index` (técnica "full-screen triangle"). Los colores llegan por
// interpolación de la coordenada del triángulo.
//
// WGSL es el único lenguaje de shaders del motor (D3): este mismo archivo
// correrá sin cambios en la galería web (WebGPU) en la Fase 7.

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    // Triángulo que cubre más que la pantalla (clip space).
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let pos = positions[idx];

    // Colores fijos por vértice, estilo "hola mundo" de gráficas.
    let colors = array<vec3<f32>, 3>(
        vec3<f32>(0.18, 0.20, 0.29), // índigo oscuro
        vec3<f32>(0.55, 0.30, 0.35), // vino
        vec3<f32>(0.83, 0.69, 0.44), // arena
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
