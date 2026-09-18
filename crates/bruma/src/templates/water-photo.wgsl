// Water over an image — template.
//
// The Phase 6 texture-binding showcase: the manifest's `textures` array
// exposes package assets to the shader as bindable textures. This one
// distorts the sampling of `assets/water-photo.png` with a sum-of-sines
// ripple field and adds specular glints — the classic "image behind
// moving water" effect.
//
// Replace assets/water-photo.png with your own photo (keep the name or
// update wallpaper.json) and it becomes YOUR image behind the water.
//
// Parameters (rename freely in wallpaper.json):
//   waves — ripple strength (default 0.4)
//   speed — surface drift speed (default 0.5)
//
// Texture bindings (from wallpaper.json `textures`, in order):
//   @binding(1) texture_2d  — assets/water-photo.png
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
    // UVs with Y pointing up, shadertoy-style.
    // Engine orientation contract (same as image.wgsl): uv.y = 0 at the top.
    out.uv = uvs[idx];
    return out;
}

// Sum-of-sines ripple field (same recipe as the water template).
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
    let strength = 0.1 + 0.6 * U.u_params.x;
    let t = U.u_time * (0.3 + 1.4 * U.u_params.y);

    // Refraction: wobble the texture lookup with the ripple field. Two
    // offset taps averaged by the wave height fake the light bending.
    let h = ripples(in.uv * vec2<f32>(1.0, 1.6), t, strength);
    let dir = vec2<f32>(h * 0.04, h * 0.06);
    let a = textureSample(tex0, samp0, in.uv + dir).rgb;
    let b = textureSample(tex0, samp0, in.uv - dir * 0.6).rgb;
    var col = mix(a, b, 0.5 + 0.5 * h);

    // Specular glints where the wave crests align.
    let glint = smoothstep(0.45, 0.95, h);
    col += vec3<f32>(1.0, 0.95, 0.85) * glint * 0.35;

    return vec4<f32>(col, 1.0);
}
