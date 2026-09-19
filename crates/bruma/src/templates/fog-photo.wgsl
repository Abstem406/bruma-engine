// Drifting fog over your photo — template.
//
// YOUR PHOTO is the base layer (texture slot 0); two FBM fog layers
// drift across it with depth (the far one slower). Where there is no
// fog the photo shows untouched — the fog is the transparent overlay.
//
// Parameters (rename freely in wallpaper.json):
//   speed   — fog drift speed (default 0.5)
//   density — fog opacity (default 0.6)
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

// Cheap value noise (hash based) for the fog layers.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += amp * value_noise(q);
        q = q * 2.1 + vec2<f32>(17.3, 9.1);
        amp *= 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // The photo IS the wallpaper: aspect-correct, it fills the screen.
    var col = textureSample(tex0, samp0, bruma_texture_fit(in.uv, tex0, U.u_res, BRUMA_TEX0_FIT)).rgb;

    // Two drifting fog layers (far one slower: depth by speed).
    let speed = 0.2 + 0.8 * U.u_params.x;
    let density = U.u_params.y;
    let t = U.u_time * speed;
    let fog_far  = fbm(vec2<f32>(in.uv.x * 2.0 + t * 0.15, in.uv.y * 3.0 - t * 0.03));
    let fog_near = fbm(vec2<f32>(in.uv.x * 3.0 - t * 0.30, in.uv.y * 4.0 + t * 0.05));
    let fog = clamp(fog_far * 0.7 + fog_near * 0.6 - 0.25, 0.0, 1.0);

    // Soft daylight fog; ground fog pools at the bottom of the screen.
    let fog_col = vec3<f32>(0.82, 0.85, 0.90);
    col = mix(col, fog_col, fog * density * (0.35 + 0.65 * in.uv.y));

    return vec4<f32>(col, 1.0);
}
