// Demo animated wallpaper — Phase 3.
//
// Concentric waves expanding from the center with a Nord-ish gradient.
// Everything time-dependent arrives via uniforms, so the file is ready
// for hot-reload: edit and save is enough, no wallpaper restart needed.
//
// Uniforms (binding 0, 48 bytes):
//   u_time    f32  — seconds since startup
//   u_params0 f32  — alias of u_params.x (first parameter)
//   u_mouse   vec2 — cursor position (px; -1,-1 = unknown)
//   u_params  vec4 — flat parameters 0..3 (names in the manifest)
//   u_res     vec2 — buffer resolution (px)

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
    // UVs with Y pointing up, shadertoy-style.
    out.uv = vec2<f32>(uvs[idx].x, 1.0 - uvs[idx].y);
    return out;
}

// Damped waves: sin(t*k - r*speed) * exp(-r*decay)
fn waves(uv: vec2<f32>, t: f32) -> f32 {
    let p = uv * U.u_res;
    let r = length(p) / 200.0;
    return sin(t * 1.4 - r * 4.5) * exp(-r * 0.10);
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // Nord palette: flint blue and wine, blended by the wave.
    let base = vec3<f32>(0x2e, 0x34, 0x40) / 255.0;
    let wine = vec3<f32>(0xbf, 0x61, 0x6a) / 255.0;

    let uv = in.uv - vec2<f32>(0.5, 0.5);
    let w = waves(uv, U.u_time);
    var col = mix(base, wine, 0.5 + 0.5 * w);

    // u_params0 shifts brightness; animatable with:
    //   bruma run --shader hello.wgsl --fps 30 --param param0=0.5
    col += U.u_params0 * 0.15 * w;

    return vec4<f32>(col, 1.0);
}
