// Concentric waves over your photo — template.
//
// YOUR PHOTO is the base layer (texture slot 0); damped concentric sine
// waves expand from the center and gently wobble + brighten it — a calm
// "water surface" read without covering the image.
//
// Parameters (rename freely in wallpaper.json):
//   waves — ripple strength (default 0.5)
//   speed — wave pace (default 0.5)
//
// Texture bindings (from wallpaper.json `textures`, in order):
//   @binding(1) texture_2d  — your photo (slot 0)
//   @binding(2) sampler     — its sampler
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

@group(0) @binding(1)
var tex0: texture_2d<f32>;

@group(0) @binding(2)
var samp0: sampler;

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
    // Engine orientation contract (same as image.wgsl): uv.y = 0 at the top.
    out.uv = uvs[idx];
    return out;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    let amp = 0.05 + 0.25 * U.u_params.x;
    let t = U.u_time * (0.3 + 1.2 * U.u_params.y);

    // Damped concentric rings from the center.
    let r = length(in.uv - vec2<f32>(0.5)) * vec2<f32>(1.0, U.u_res.x / max(U.u_res.y, 1.0));
    let dist = length(r);
    let w = sin(dist * 28.0 - t * 2.2) * exp(-dist * 2.2) * amp;

    // The wave wobbles the photo sampling (refraction) and adds a soft
    // crest light — the overlay never covers the image.
    let dir = vec2<f32>(w * 0.05);
    let photo = textureSample(tex0, samp0, bruma_texture_fit(in.uv + dir, tex0, U.u_res, BRUMA_TEX0_FIT)).rgb;
    let crest = smoothstep(0.0, 1.0, w);
    return vec4<f32>(photo + vec3<f32>(0.9, 0.95, 1.0) * crest * 0.18, 1.0);
}
