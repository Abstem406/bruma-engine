// Wallpaper animado de demostración — Fase 3.
//
// Ondas concéntricas que se expanden desde el centro con un gradiente
// Nortie-ish. Todo lo dependiente del tiempo llega por uniformes, de modo
// que el archivo queda listo para el hot-reload: editar y guardar es
// suficiente, no hay que reiniciar el wallpaper.
//
// Uniformes (binding 0, 48 bytes):
//   u_time    f32  — segundos desde el arranque
//   u_params0 f32  — alias de u_params.x (primer parámetro)
//   u_mouse   vec2 — posición del cursor (px; -1,-1 = desconocida)
//   u_params  vec4 — parámetros planos 0..3 (nombres en el manifiesto)
//   u_res     vec2 — resolución del buffer (px)

struct Uniforms {
    u_time: f32,
    u_params0: f32,
    u_mouse: vec2<f32>,
    u_params: vec4<f32>,
    u_res: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> U: Uniforms;

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
    );
    let uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    var out: VsOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    // UV con Y hacia arriba, a la usanza de shadertoy.
    out.uv = vec2<f32>(uvs[idx].x, 1.0 - uvs[idx].y);
    return out;
}

// Ondas amortiguadas: sin(t*k - r*velocidad) * exp(-r*caida)
fn waves(uv: vec2<f32>, t: f32) -> f32 {
    let p = uv * U.u_res;
    let r = length(p) / 200.0;
    return sin(t * 1.4 - r * 4.5) * exp(-r * 0.10);
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // Paleta Nortie: azul pedernal y vino, mezclados con la onda.
    let base = vec3<f32>(0x2e, 0x34, 0x40) / 255.0;
    let vino = vec3<f32>(0xbf, 0x61, 0x6a) / 255.0;

    let uv = in.uv - vec2<f32>(0.5, 0.5);
    let w = waves(uv, U.u_time);
    var col = mix(base, vino, 0.5 + 0.5 * w);

    // u_params0 desplaza el brillo; animable con:
    //   bruma run --shader hello.wgsl --fps 30 --param param0=0.5
    col += U.u_params0 * 0.15 * w;

    return vec4<f32>(col, 1.0);
}
