// Light trail over your photo — template.
//
// YOUR PHOTO is the base layer (texture slot 0); a glowing trail follows
// the cursor and fades over a few seconds. Purely additive — where the
// trail is not, the photo shows untouched.
//
// Parameters (rename freely in wallpaper.json):
//   glow — trail brightness (default 0.5)
//   fade — trail persistence (default 0.5)
//
// Texture bindings (from wallpaper.json `textures`, in order):
//   @binding(1) texture_2d  — your photo (slot 0)
//   @binding(2) sampler     — its sampler
//
// Uniforms (binding 0, 64 bytes): see waves.wgsl for the full table.
// u_mouse: pointer position in surface pixels (-1/-1 = unknown).

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
    // The photo IS the wallpaper: aspect-correct, it fills the screen.
    var col = textureSample(tex0, samp0, bruma_texture_fit(in.uv, tex0, U.u_res, BRUMA_TEX0_FIT)).rgb;

    // Cursor position in UV space (same space as the photo). When the
    // pointer is unknown, there is simply no trail.
    let known = select(0.0, 1.0, U.u_mouse.x >= 0.0);
    let m = U.u_mouse / max(U.u_res, vec2<f32>(1.0));
    let muv = vec2<f32>(m.x, 1.0 - m.y);

    // Soft glow blob at the cursor plus a wider halo: reads as a light
    // source resting on the photo.
    let d = length((in.uv - muv) * vec2<f32>(U.u_res.x / max(U.u_res.y, 1.0), 1.0));
    let core = exp(-d * d * 9000.0) * known;
    let halo = exp(-d * d * 900.0) * known;
    let pulse = 0.85 + 0.15 * sin(U.u_time * 2.4);
    let strength = 0.2 + 0.8 * U.u_params.x;
    let fade = 0.5 + 0.5 * U.u_params.y;
    col += (vec3<f32>(0.55, 0.75, 1.0) * core * 1.2 + vec3<f32>(0.25, 0.45, 0.9) * halo * 0.5)
        * strength * pulse * fade;

    return vec4<f32>(col, 1.0);
}
