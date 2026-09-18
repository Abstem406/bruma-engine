// Calm water with a boat-style V wake — template (feedback + mouse).
//
// A real water SIMULATION: a height field that ripples where the pointer
// moves, propagates on its own and settles back to calm in a few
// seconds. The wake opens BEHIND the motion (a boat's V), lighting up
// the surface. The photo is re-drawn with DIFFUSE REFLECTION — broad, soft
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
    // vec2 needs 8-byte alignment: the clock ends at 60, so this pad
    // holds u_mouse_prev at 64 (the engine writes it there).
    _pad60: f32,
    // Previous frame's cursor position: the wake's stem follows the
    // moved segment (prev -> mouse), one continuous V trail.
    u_mouse_prev: vec2<f32>,
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
    // Courant limit for this stencil is c^2 <= 0.5; 0.49 sends the
    // rings across the whole screen in ~1.5 s and they reach far
    // before dying.
    var v = c.g + (sum - 4.0 * c.r) * 0.49;
    // Damping: the wake lingers for a long time (0.05%/frame of
    // velocity loss at damping 0 — rings cross the screen and back
    // before they die); the param shortens it on demand.
    v *= 0.9995 - 0.0045 * U.u_params.y;
    var h = c.r + v;

    // The pointer injects energy proportional to its SPEED (px/s, the
    // engine supplies it in the free uniform slot at offset 40): a
    // still cursor stirs nothing (a resting finger in water), a moving
    // one leaves a CONTINUOUS wake.
    //
    // MASS-NEUTRAL profile (Mexican hat): pressing water DOWN in the
    // center raises a ring around it. A one-sided push would lower the
    // field's mean with every stroke — and a wave equation never gives
    // the mean back — so after minutes of play the pond would be pinned
    // at the clamp and the wake would die. The hat keeps the level
    // forever stable.
    if (U.u_mouse.x >= 0.0) {
        // BOAT-STYLE V WAKE. The stem covers the WHOLE segment moved
        // this frame (prev -> mouse) — a point blob makes disconnected
        // circles that cut the trail's continuity; the segment keeps it
        // ONE continuous trail. The mass-neutral hat (integrates to
        // exactly 0) keeps the pond's level forever stable.
        let a = min(U.u_mouse_prev, U.u_mouse);
        let b = max(U.u_mouse_prev, U.u_mouse);
        let ab = max(b - a, vec2<f32>(0.0001));
        let p = clamp(in.uv * U.u_res, a, b);
        let d = distance(in.uv * U.u_res, p);
        let q = d * d / 1000.0;
        let hat = (q - 1.0) * exp(-q);
        // Directional weighting — the V is ANCHORED AT THE CURSOR and
        // opens behind the motion. Weight 1 + a*(2*cos^3 b - 1), with b
        // the angle from the backward axis:
        //   exactly behind the cursor (b = 0) -> 1 + a   (the apex)
        //   decaying through the arms        -> the V wedge
        //   straight ahead (b = 180)         -> 1 - a   (calm water)
        // The angular mean of the weight is exactly 1 (cosine terms
        // average to zero over the circle), so the pond's mass balance
        // survives the anisotropy.
        let back = -normalize(vec2<f32>(0.0001) + U.u_mouse - U.u_mouse_prev);
        let rel = in.uv * U.u_res - U.u_mouse;
        let cosb = dot(rel / max(length(rel), 0.0001), back);
        let hat_d = hat * (1.0 + 0.65 * (2.0 * cosb * cosb * cosb - 1.0));
        // 800 px/s => full strength; scaled by the intensity param.
        let push = min(U.mouse_speed / 800.0, 1.0);
        h += hat_d * 0.30 * push * (0.3 + 0.7 * U.u_params.x);
    }

    // The pond always returns to calm: a very slow pull of the height
    // toward rest (0) heals any residual level drift. A uniform level
    // shift makes no gradient, so while it acts the surface looks
    // still — invisible as motion, decisive for forever-alive water.
    h -= h * 0.0008;

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
    // per-pixel speckle). Gain ×48 tilts the normal hard enough for
    // clearly visible light bands.
    // Two scales of slope (3 and 9 texels): the fine one keeps the
    // liquid detail, the broad one the smooth viscous flow of light.
    let e3 = e * 3.0;
    let e9 = e * 9.0;
    let grad = (
        vec2<f32>(
            prev_at(uv + vec2<f32>(e3.x, 0.0)).r - prev_at(uv - vec2<f32>(e3.x, 0.0)).r,
            prev_at(uv + vec2<f32>(0.0, e3.y)).r - prev_at(uv - vec2<f32>(0.0, e3.y)).r,
        ) / 6.0
            + vec2<f32>(
                prev_at(uv + vec2<f32>(e9.x, 0.0)).r - prev_at(uv - vec2<f32>(e9.x, 0.0)).r,
                prev_at(uv + vec2<f32>(0.0, e9.y)).r - prev_at(uv - vec2<f32>(0.0, e9.y)).r,
            ) / 18.0
    ) * 0.5;

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

    // PERMANENT WATER: a slow large-scale swell (two crossing,
    // time-animated sine waves) keeps the whole surface living — the
    // screen always reads as water, even between strokes. Subtle by
    // design; the wake rides on top of it.
    {
        let p = uv * u.u_res;
        let s1 = sin(p.x * 0.011 + u.u_time * 0.9 + sin(p.y * 0.017 + u.u_time * 0.6) * 1.8);
        let s2 = sin(p.y * 0.009 - u.u_time * 0.7 + sin(p.x * 0.013 - u.u_time * 0.5) * 1.6);
        col *= 1.0 + (s1 + s2) * 0.012 * (0.4 + 0.6 * u.u_params.x);
    }

    // Scattered light across the disturbed area (the SPREAD term): a
    // gentle cool lift that reaches well past the ring — the "light
    // plays over the water" feeling.
    let scatter = clamp(abs(spread) * 8.0, 0.0, 1.0);
    col *= 1.0 + scatter * 0.45 * (0.4 + 0.6 * u.u_params.x);

    // A sheen on the steepest crests, for sparkle (bounded mix: the
    // rims can never clip to pure white).
    let spec = pow(clamp(dot(reflect(-l, n), vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0), 34.0);
    col = mix(col, vec3<f32>(0.9, 0.94, 1.0), spec * 0.25);

    return vec4<f32>(col, 1.0);
}
