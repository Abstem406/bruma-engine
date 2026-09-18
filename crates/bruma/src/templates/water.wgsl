// Procedural water — template.
//
// Reflective pool: a sum-of-sines ripple field wobbles the reflection of
// a dusk sky, with specular glints along the sun column. Pure math, no
// textures — light enough on integrated GPUs to run 24/7.
//
// Parameters (rename freely in wallpaper.json):
//   waves — ripple strength (default 0.4)
//   speed — surface drift speed (default 0.5)
//
// Uniforms (binding 0, 64 bytes): see waves.wgsl for the full table.

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
    // Engine orientation contract (same as image.wgsl): uv.y = 0 at the top.
    out.uv = uvs[idx];
    return out;
}

// Sum-of-sines ripple field: surface height at the given point.
fn ripples(p: vec2<f32>, t: f32, amp: f32) -> f32 {
    var h = 0.0;
    h += sin(p.x * 9.0 + t * 1.7) * 0.35;
    h += sin(p.y * 7.0 - t * 1.1 + p.x * 3.0) * 0.30;
    h += sin((p.x + p.y) * 13.0 + t * 2.3) * 0.20;
    h += sin(length(p - vec2<f32>(0.35, 0.2)) * 24.0 - t * 3.1) * 0.15;
    return h * amp;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // uv.y = 0 at the BOTTOM (look composed pre-contract; one flip keeps
    // the far shore at the top of the pool).
    let uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let strength = 0.1 + 0.6 * U.u_params.x;
    let t = U.u_time * (0.3 + 1.4 * U.u_params.y);

    // Dusk sky the pool reflects (screen top = far shore).
    let sky_near = vec3<f32>(0.10, 0.12, 0.20);
    let sky_far  = vec3<f32>(0.85, 0.55, 0.40);

    // Fake refraction: wobble the reflection's sampling line with the
    // ripple field.
    let h = ripples(uv * vec2<f32>(1.0, 1.6), t, strength);
    let refl_uv = clamp(uv.y + h * 0.35, 0.0, 1.0);
    var col = mix(sky_far, sky_near, smoothstep(1.0, 0.2, refl_uv));

    // Sun column and its glints, broken up by the ripples.
    let band = smoothstep(0.55, 0.95, 1.0 - abs(uv.x - 0.5) * 2.0)
             * smoothstep(0.2, 0.9, uv.y);
    col += vec3<f32>(1.0, 0.75, 0.45) * band * 0.25;
    let glint = smoothstep(0.55, 0.95, h + band * 0.4);
    col += vec3<f32>(1.0, 0.9, 0.7) * glint * band * 0.5;

    return vec4<f32>(col, 1.0);
}
