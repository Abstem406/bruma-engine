// Quad con imagen a pantalla completa — Fase 2, paso 2.
//
// El quad cubre el viewport completo (posiciones en clip space, generadas
// por vértice_index) y las UV llegan al fragment shader para muestrear la
// textura de la imagen. La imagen se ESTIRA al tamaño de la pantalla;
// cover/contain llegarán con el manifiesto de .wallpaper (Fase 4).

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    // 4 esquinas en clip space; el orden forma dos triángulos:
    // (0,1,2) y (2,1,3).
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0), // arriba-izquierda
        vec2<f32>( 1.0,  1.0), // arriba-derecha
        vec2<f32>(-1.0, -1.0), // abajo-izquierda
        vec2<f32>( 1.0, -1.0), // abajo-derecha
    );
    let uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    var out: VsOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = uvs[idx];
    return out;
}

@group(0) @binding(0)
var tex: texture_2d<f32>;

@group(0) @binding(1)
var samp: sampler;

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
