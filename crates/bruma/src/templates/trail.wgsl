// Phosphor trails — template (feedback permission).
//
// The shader reads its OWN previous frame (group 1) and fades it while
// drawing fresh light: the classic "everything leaves a trail" look.
// This is impossible for a plain shader — the state lives on the GPU,
// frame after frame — and it is what the `feedback` permission buys.
//
// Parameters (rename freely in wallpaper.json):
//   fade  — how fast old light dies (default 0.35; 0 = never fades)
//   speed — motion of the light sources (default 0.5)
//
// Group 0 (binding 0): the standard 64-byte uniform block.
// Group 1: the PREVIOUS frame — @binding(0) texture_2d, @binding(1) sampler.

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

@group(1) @binding(0)
var prev_tex: texture_2d<f32>;

@group(1) @binding(1)
var prev_samp: sampler;

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
    out.uv = vec2<f32>(uvs[idx].x, 1.0 - uvs[idx].y);
    return out;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    // Fade whatever the previous frame left behind. param0 = fade.
    let prev = textureSample(prev_tex, prev_samp, in.uv).rgb;
    let fade = 0.01 + 0.9 * U.u_params.x;
    var col = prev * (1.0 - fade);

    // Two roaming light sources inject fresh energy each frame.
    let t = U.u_time * (0.2 + 1.5 * U.u_params.y);
    let asp = U.u_res.x / max(U.u_res.y, 1.0);
    let p = vec2<f32>(in.uv.x * asp, in.uv.y);
    var energy = 0.0;
    energy += exp(-12.0 * length(p - vec2<f32>(
        asp * (0.5 + 0.42 * sin(t * 1.3)),
        0.5 + 0.4 * sin(t * 0.9),
    )));
    energy += exp(-9.0 * length(p - vec2<f32>(
        asp * (0.5 + 0.4 * cos(t * 1.7)),
        0.5 + 0.42 * cos(t * 0.7),
    )));
    col += vec3<f32>(0.35, 0.55, 1.0) * energy * 0.5;

    return vec4<f32>(col, 1.0);
}
