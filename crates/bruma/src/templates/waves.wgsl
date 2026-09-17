// Concentric waves — template.
//
// Damped sine waves expanding from the center over a Nord palette.
// The simplest "alive" shader there is: two parameters, no textures.
// The u_clock uniform is declared but unused here — it exists for
// day/night variants (see the fog template).
//
// Parameters (rename freely in wallpaper.json):
//   speed — wave pace (default 0.5)
//   glow  — brightness of the wave crests (default 0.3)
//
// Uniforms (binding 0, 64 bytes):
//   u_time    f32  — seconds since startup
//   u_params0 f32  — alias of u_params.x (first parameter)
//   u_mouse   vec2 — cursor position (px; -1,-1 = unknown)
//   u_params  vec4 — flat parameters 0..3 (names in the manifest)
//   u_res     vec2 — buffer resolution (px)
//   u_clock   vec3 — local wall clock [h, m, s]

struct Uniforms {
    u_time: f32,
    u_params0: f32,
    u_mouse: vec2<f32>,
    u_params: vec4<f32>,
    u_res: vec2<f32>,
    u_clock: vec3<f32>,
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
    // UVs with Y pointing up, shadertoy-style.
    out.uv = vec2<f32>(uvs[idx].x, 1.0 - uvs[idx].y);
    return out;
}

// Damped waves: sin(t*pace - r*k) * exp(-r*decay)
fn waves(uv: vec2<f32>, t: f32) -> f32 {
    let p = uv * U.u_res;
    let r = length(p) / 200.0;
    let pace = 0.6 + 1.6 * U.u_params.x;
    return sin(t * pace - r * 4.5) * exp(-r * 0.10);
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // Nord palette: flint blue and wine, blended by the wave.
    let base = vec3<f32>(0x2e, 0x34, 0x40) / 255.0;
    let wine = vec3<f32>(0xbf, 0x61, 0x6a) / 255.0;

    let uv = in.uv - vec2<f32>(0.5, 0.5);
    let w = waves(uv, U.u_time);
    var col = mix(base, wine, 0.5 + 0.5 * w);

    // u_params.y ("glow") brightens the crests only.
    col += U.u_params.y * 0.3 * max(w, 0.0);

    return vec4<f32>(col, 1.0);
}
