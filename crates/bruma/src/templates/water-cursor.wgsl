// Calm water following the cursor — template (feedback + mouse).
//
// A real water SIMULATION: a height field that ripples where the pointer
// moves and keeps propagating on its own, drawn as refraction and
// specular light over a fixed photo (the manifest's `textures`). The
// wake fades in a couple of seconds — the classic "calm water" effect.
//
// HOW IT WORKS (the only template family with a second entry point):
//   fs_main(uv)           — SIM pass, offscreen, runs every frame. Reads
//                           the previous state from group 1 and returns
//                           the new water state. Nothing here reaches
//                           the screen.
//   display(uv, frame, u) — DISPLAY pass. `frame` is what fs_main just
//                           produced, `u` the uniform block: use the
//                           state to bend and light the photo, and
//                           return the SCREEN color.
//
// The wave math: frame.r encodes the water height around 0.5. Each frame
// the height relaxes toward its neighborhood average (waves propagate)
// and a bit of energy is absorbed (damping). Moving the pointer injects
// a drop. Zero per-pixel branching, iGPU-friendly at 30 fps.
//
// PARAMETERS (wallpaper.json):
//   intensity — ripple strength of the cursor's wake (default 0.6)
//   damping   — how fast the wake calms down (default 0.35)

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

// One texel of the previous state (uv in texel centers).
fn prev_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(prev_tex, prev_samp, uv, 0.0).r;
}

// ===== SIM PASS (offscreen, per frame) =====
@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    let px = 1.0 / max(U.u_res, vec2<f32>(1.0));
    let h = prev_at(in.uv);
    let avg = 0.25 * (prev_at(in.uv + vec2<f32>(px.x, 0.0))
                    + prev_at(in.uv - vec2<f32>(px.x, 0.0))
                    + prev_at(in.uv + vec2<f32>(0.0, px.y))
                    + prev_at(in.uv - vec2<f32>(0.0, px.y)));

    // Propagate toward the neighborhood and damp toward calm (0.5).
    let damping = 0.05 + 0.5 * U.u_params.y;
    var height = h + (avg - h) * 0.42;
    height = mix(height, 0.5, damping * 0.08);

    // The pointer drips: a Gaussian drop wherever the cursor is.
    if (U.u_mouse.x >= 0.0) {
        let m = U.u_mouse * px;
        let d = length((in.uv - m) * U.u_res) / max(U.u_res.y, 1.0);
        height += exp(-d * d * 2200.0) * (0.08 + 0.5 * U.u_params.x);
    }

    return vec4<f32>(clamp(height, 0.0, 1.0), 0.5, 0.5, 1.0);
}

// ===== DISPLAY PASS (what the screen shows) =====
fn display(uv: vec2<f32>, frame: vec4<f32>, u: Uniforms) -> vec4<f32> {
    let px = 1.0 / max(u.u_res, vec2<f32>(1.0));
    let hx = frame.r - textureSampleLevel(prev_tex, prev_samp, uv + vec2<f32>(px.x, 0.0), 0.0).r;
    let hy = frame.r - textureSampleLevel(prev_tex, prev_samp, uv + vec2<f32>(0.0, px.y), 0.0).r;

    // Refraction: bend the photo lookup along the water's slope.
    let bend = 0.01 + 0.06 * u.u_params.x;
    var col = textureSample(tex0, samp0, uv + vec2<f32>(hx, hy) * 6.0 * bend).rgb;

    // Sun glints where the wave slopes catch the light.
    let glint = clamp(length(vec2<f32>(hx, hy)) * 30.0 * (0.4 + u.u_params.x), 0.0, 1.0);
    col += vec3<f32>(1.0, 0.97, 0.9) * glint * glint * 0.35;

    return vec4<f32>(col, 1.0);
}
