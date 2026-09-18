// Calm water following the cursor — template (feedback + mouse).
//
// A real water SIMULATION: a height field that ripples where the pointer
// moves, propagates on its own and settles back to calm in a few
// seconds. The photo is re-drawn with DIFFUSE REFLECTION — broad, soft
// bands of light that smear across the wake (the "calm water" look) —
// plus a gentle specular sheen on the steepest ripples.
//
// HOW IT WORKS (the only template family with a second entry point):
//   fs_main(uv)           — SIM pass, offscreen, runs every frame. Reads
//                           the previous state from group 1 and returns
//                           the new water state. Nothing here reaches
//                           the screen. The target is fp16: the state
//                           must stay continuous frame to frame.
//   display(uv, frame, u) — DISPLAY pass. `frame` is what fs_main just
//                           produced, `u` the uniform block: use the
//                           state to light the photo and return the
//                           SCREEN color.
//
// State, per texel (fp16, around 0.5):
//   R = water height   G = vertical velocity
// Update: classic wave equation — velocity pulls toward the neighbors'
// average height (that is what makes rings PROPAGATE), height follows
// its own velocity, then a light damping calms everything down.
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
    // Cursor speed in buffer px/s (0 = still or unknown): the wake's
    // strength follows it. The clock's 16-byte alignment (vec3 at
    // offset 48) leaves exactly this gap free at 40.
    mouse_speed: f32,
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
    // Engine orientation contract (same as image.wgsl): uv.y = 0 at the
    // TOP of the target. Nothing flips here — ever.
    out.uv = uvs[idx];
    return out;
}

// One texel of the previous state (uv in texel centers).
fn prev_at(uv: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(prev_tex, prev_samp, uv, 0.0).rg;
}

// ===== SIM PASS (offscreen, per frame) =====
@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    let e = 1.0 / max(U.u_res, vec2<f32>(1.0));
    let c = prev_at(in.uv) - vec2<f32>(0.5);
    let sum = (prev_at(in.uv + vec2<f32>(e.x, 0.0)).r - 0.5)
            + (prev_at(in.uv - vec2<f32>(e.x, 0.0)).r - 0.5)
            + (prev_at(in.uv + vec2<f32>(0.0, e.y)).r - 0.5)
            + (prev_at(in.uv - vec2<f32>(0.0, e.y)).r - 0.5);

    // Wave equation: velocity toward the neighborhood, height follows.
    // Courant-safe for this stencil (c^2 <= 0.5); 0.4 makes the rings
    // cross the screen in a couple of seconds.
    var v = c.g + (sum - 4.0 * c.r) * 0.4;
    v *= 0.995 - 0.01 * U.u_params.y; // damping: the wake fades in seconds
    var h = c.r + v;

    // The pointer injects energy proportional to its SPEED (px/s, the
    // engine supplies it in the free uniform slot at offset 52): a still
    // cursor stirs nothing (a resting finger in water), a moving one
    // leaves a CONTINUOUS wake — not a chain of separate plops that cut
    // the trail's continuity. One impulse per texel per frame, spread
    // along the movement: the wave equation turns the dent into rings
    // on its own.
    if (U.u_mouse.x >= 0.0) {
        let m = U.u_mouse * e;
        let d = distance(in.uv * U.u_res, m * U.u_res);
        let gauss = exp(-d * d / 1400.0);
        // 2000 px/s => full strength; scaled by the intensity param.
        let push = min(U.mouse_speed / 2000.0, 1.0);
        h += gauss * -0.22 * push * (0.3 + 0.7 * U.u_params.x);
    }

    // Clamp: a runaway value (driver hiccup) can never poison the
    // field — NaN/Inf would otherwise persist forever.
    h = clamp(h, -1.0, 1.0);
    v = clamp(v, -1.0, 1.0);

    return vec4<f32>(h + 0.5, v + 0.5, 0.5, 1.0);
}

// ===== DISPLAY PASS (what the screen shows) =====
fn display(uv: vec2<f32>, frame: vec4<f32>, u: Uniforms) -> vec4<f32> {
    let e = 1.0 / max(u.u_res, vec2<f32>(1.0));
    // Water slope from the height field, sampled 3 texels apart:
    // broad, soft light bands (the wake reads as smooth light, not
    // per-pixel speckle). Gain ×48: a single-frame cursor drop tilts
    // the normal ~25° — the light bands are clearly visible.
    let e3 = e * 3.0;
    let hL = prev_at(uv - vec2<f32>(e3.x, 0.0)).r;
    let hR = prev_at(uv + vec2<f32>(e3.x, 0.0)).r;
    let hB = prev_at(uv - vec2<f32>(0.0, e3.y)).r;
    let hU = prev_at(uv + vec2<f32>(0.0, e3.y)).r;
    let grad = vec2<f32>(hR - hL, hU - hB) / 6.0;

    // Cover-fit: fill the screen without distorting the photo.
    let img = textureDimensions(tex0);
    let screenAsp = u.u_res.x / u.u_res.y;
    let imgAsp = f32(img.x) / f32(img.y);
    var scale = vec2<f32>(1.0);
    if (screenAsp > imgAsp) {
        scale = vec2<f32>(1.0, imgAsp / screenAsp);
    } else {
        scale = vec2<f32>(screenAsp / imgAsp, 1.0);
    }

    // SPREAD of the disturbance (wide 4-tap blur of the height field):
    // light scatters on disturbed water, so the reflection must extend
    // BEYOND the ring itself — this term lights the whole wake area,
    // decaying with distance from it.
    let r = e * 5.0;
    let spread = (
        prev_at(uv + vec2<f32>(r.x, 0.0)).r
            + prev_at(uv - vec2<f32>(r.x, 0.0)).r
            + prev_at(uv + vec2<f32>(0.0, r.y)).r
            + prev_at(uv - vec2<f32>(0.0, r.y)).r
    ) * 0.25 - 0.5;

    // Refraction: a visible pull along the slope (~2-4 px at 1080p).
    // The photo stays sharp; the water reads through light + wobble.
    let base = (uv - 0.5) * scale + 0.5;
    let bent = clamp(base + grad * 0.12, vec2<f32>(0.0), vec2<f32>(1.0));
    var col = textureSampleLevel(tex0, samp0, bent, 0.0).rgb;

    // DIFFUSE REFLECTION: soft light on a slope-derived normal. The
    // lambert term MULTIPLIES the photo (0.80..1.50): broad light/shadow
    // bands slide across the wake and the image keeps all its detail.
    let n = normalize(vec3<f32>(grad * 48.0, 1.0));
    let l = normalize(vec3<f32>(-0.4, -0.55, 0.73));
    let diff = clamp(dot(n, l), 0.0, 1.0);
    col *= 0.80 + 0.70 * diff * (0.4 + 0.6 * u.u_params.x);

    // Scattered light across the disturbed area (the SPREAD term): a
    // gentle cool lift that reaches well past the ring — the "light
    // plays over the water" feeling.
    let scatter = clamp(abs(spread) * 8.0, 0.0, 1.0);
    col *= 1.0 + scatter * 0.45 * (0.4 + 0.6 * u.u_params.x);

    // A sheen on the steepest crests, for sparkle (bounded mix: the
    // rims can never clip to pure white).
    let spec = pow(clamp(dot(reflect(-l, n), vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0), 20.0);
    col = mix(col, vec3<f32>(0.9, 0.94, 1.0), spec * 0.25);

    return vec4<f32>(col, 1.0);
}
