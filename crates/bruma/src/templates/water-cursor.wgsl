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
//   damping   — how fast the wake calms down (default 0.15)
//   ambient   — strength of the permanent ambient swell (default 0.5;
//               0 = still pond, only your strokes move the water)

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
        // BOAT-STYLE WAKE. The stem covers the WHOLE segment moved this
        // frame (prev -> mouse) — a point blob makes disconnected
        // circles that cut the trail's continuity. The closest point is
        // a true PROJECTION onto the segment: clamping per-component to
        // the bounding box painted SQUARES on diagonal moves.
        let px = in.uv * U.u_res;
        let ab = U.u_mouse - U.u_mouse_prev;
        let t = clamp(dot(px - U.u_mouse_prev, ab) / max(dot(ab, ab), 1.0), 0.0, 1.0);
        let p = U.u_mouse_prev + t * ab;
        let rel_stem = px - p;
        let q = dot(rel_stem, rel_stem) / 1000.0;
        let hat = (q - 1.0) * exp(-q);
        // Directional weighting — comet wake anchored at the cursor:
        // full strength right behind the motion (1.65), neutral at the
        // sides (1.0), calm ahead (0.35). The hat is compact (its support
        // is ~5 sigma wide) while the weight varies over screen scale, so
        // over the hat w is nearly constant: the stroke mass stays
        // w(center) · 0 = 0 up to a second-order residue. The level
        // healing below removes that residue every frame.
        let back = -normalize(vec2<f32>(0.0001) + ab);
        let rel = px - U.u_mouse;
        let cosb = dot(rel / max(length(rel), 0.0001), back);
        let hat_d = (1.0 + 0.65 * cosb * cosb * cosb) * hat;
        // 800 px/s => full strength; scaled by the intensity param.
        let push = min(U.mouse_speed / 800.0, 1.0);
        h += hat_d * 0.30 * push * (0.3 + 0.7 * U.u_params.x);
    }

    // The pond always returns to calm: a slow pull of the height
    // toward rest (0) heals any residual level drift long before the
    // clamp could ever pin the field. A uniform level shift makes no
    // gradient, so while it acts the surface looks still — invisible
    // as motion, decisive for forever-alive water.
    h -= h * 0.002;

    // Clamp: a runaway value (driver hiccup) can never poison the
    // field — NaN/Inf would otherwise persist forever.
    h = clamp(h, -1.0, 1.0);
    v = clamp(v, -1.0, 1.0);

    return vec4<f32>(h + 0.5, v + 0.5, 0.5, 1.0);
}

// AMBIENT WATER — a permanent, STANDING ripple field (three broad
// crossed waves, barely drifting over tens of seconds): a calm pond
// breathing, not running water. Its gradient feeds the SAME normal the
// wake uses, so the surface always reads as water. The `ambient`
// parameter scales it (0 = still pond between strokes).
fn swell(p: vec2<f32>, t: f32) -> f32 {
    let w1 = sin(p.x * 0.028 + sin(p.y * 0.041) * 2.4 + t * 0.15);
    let w2 = sin(p.y * 0.037 + sin(p.x * 0.033) * 2.1 - t * 0.12);
    let w3 = sin((p.x + p.y) * 0.072 + sin(p.x * 0.021 - p.y * 0.017) * 1.5 + t * 0.18);
    return (w1 + w2) * 0.030 + w3 * 0.016;
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
    let grad_wake = (
        vec2<f32>(
            prev_at(uv + vec2<f32>(e3.x, 0.0)).r - prev_at(uv - vec2<f32>(e3.x, 0.0)).r,
            prev_at(uv + vec2<f32>(0.0, e3.y)).r - prev_at(uv - vec2<f32>(0.0, e3.y)).r,
        ) / 6.0
            + vec2<f32>(
                prev_at(uv + vec2<f32>(e9.x, 0.0)).r - prev_at(uv - vec2<f32>(e9.x, 0.0)).r,
                prev_at(uv + vec2<f32>(0.0, e9.y)).r - prev_at(uv - vec2<f32>(0.0, e9.y)).r,
            ) / 18.0
    ) * 0.5;
    // The ambient ripple field enters through the SAME normal: its
    // gradient is finite-differenced at the same 3-texel scale as the
    // fine wake stencil, so both slope sources shade identically.
    let px = uv * u.u_res;
    let ga = vec2<f32>(
        (swell(px + vec2<f32>(3.0, 0.0), u.u_time) - swell(px - vec2<f32>(3.0, 0.0), u.u_time)) / 6.0,
        (swell(px + vec2<f32>(0.0, 3.0), u.u_time) - swell(px - vec2<f32>(0.0, 3.0), u.u_time)) / 6.0,
    );
    // `ambient` (u_params.z) scales the pond's breathing; 0 = still.
    let grad = grad_wake + ga * (0.1 + 1.1 * u.u_params.z);

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
    let spec = pow(clamp(dot(reflect(-l, n), vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0), 34.0);
    col = mix(col, vec3<f32>(0.9, 0.94, 1.0), spec * 0.25);

    return vec4<f32>(col, 1.0);
}
