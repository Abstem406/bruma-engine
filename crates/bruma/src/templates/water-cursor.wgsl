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
    // SPACED STENCIL (4 texels): the classic 5-point Laplacian propagates
    // sqrt(c²) ≈ 0.7 TEXELS per frame — at full-res that is ~21 px/s: a
    // ring needs ~90 s to cross the screen, so waves died where they were
    // born ("it tries to radiate and fades before spreading"). Sampling
    // the neighbors 4 texels out scales the cell size ×4: ~84 px/s, a
    // ring visibly expands and crosses the screen in ~20 s. The Laplacian
    // divides by the squared spacing (4² = 16) and the Courant number
    // stays at 0.49 (same stability bound).
    let s = e * 4.0;
    let c = prev_at(in.uv) - vec2<f32>(0.5);
    let sum = (prev_at(in.uv + vec2<f32>(s.x, 0.0)).r - 0.5)
            + (prev_at(in.uv - vec2<f32>(s.x, 0.0)).r - 0.5)
            + (prev_at(in.uv + vec2<f32>(0.0, s.y)).r - 0.5)
            + (prev_at(in.uv - vec2<f32>(0.0, s.y)).r - 0.5);

    // Wave equation: velocity toward the neighborhood, height follows.
    // Courant limit for this stencil is c^2 <= 0.5; 0.49 keeps it stable.
    var v = c.g + (sum - 4.0 * c.r) * (0.49 / 16.0);
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
    // GATED ON REAL MOVEMENT, not just presence: a resting cursor on
    // the desktop used to keep relaxing h toward its (shrunken) furrow
    // — a local sink that ate every ring passing near the cursor, so
    // the pond looked frozen while the pointer hovered it ("the effect
    // only plays when I'm in another window": there the background is
    // pointer-free and evolves free). With the gate, a still cursor is
    // exactly that: nothing. Movement (speed EMA > 1 px/s) re-opens
    // the dig.
    if (U.u_mouse.x >= 0.0 && U.mouse_speed > 1.0) {
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
        // Directional weighting — comet wake, full strength right
        // behind the motion (1.65), neutral at the sides (1.0), calm
        // ahead (0.35). ANCHORED AT THE SEGMENT PROJECTION p, not the
        // cursor: the weight must vary smoothly ALONG the frame's path,
        // or a fast stroke (a long segment) gets a discontinuous weight
        // — periodic seams down the wake (user-measured: interruptions
        // every few dozen px at medium speed).
        let back = -normalize(vec2<f32>(0.0001) + ab);
        let rel = px - p;
        let cosb = dot(rel / max(length(rel), 0.0001), back);
        // 450 px/s => full strength; scaled by the intensity param.
        // Floor at 35% while there is REAL movement: slow drifts must
        // stay clearly VISIBLE, not just present. The gate is on the
        // ENGINE-SMOOTHED speed (an EMA over ~1/8 s) and the threshold
        // is 1 px/s: raw per-frame speed dips at every inflection of a
        // drawn curve (the wrist slows to turn), which cut the carve
        // OFF mid-stroke — periodic gaps exactly where the user drew
        // curves. A truly still cursor (EMA -> 0) injects nothing.
        let push = select(
            0.0,
            clamp(U.mouse_speed / 450.0, 0.35, 1.0),
            U.mouse_speed > 1.0,
        );
        // FURROW FOLLOWS THE FINGER. Additive schemes pinned the clamp
        // (measured: 84% of the corridor frozen at |h|=1 after one
        // stroke — "only the first wave works"), so the pointer never
        // ADDS: the height RELAXES toward a bounded, mass-zero furrow.
        // Depth scales with REAL speed (a fast finger displaces more
        // water per second) and delivery is prompt (rate 0.5): the dent
        // tracks the cursor instead of accumulating behind it over ~20
        // frames — that per-frame delta onto FRESH texels IS the shed
        // wave, rings break off continuously WHILE moving (the old
        // per-distance dose fed the furrow at 0.0025/frame: the
        // groove's bright edges buried the tiny shed rings — "waves
        // only when I stop"). At rest push decays with the speed EMA
        // and the furrow closes smoothly behind the stroke.
        let w_vortex = 1.0 + 0.65 * cosb * cosb * cosb;
        // Depth 0.35 (slow drag) .. 0.65 (fast swipe): shed rings must
        // read at the same scale the release blooms used to (the first
        // speed-scaled attempt left them 4x dimmer than the visible
        // release rings of earlier builds — mid-drag energy was present
        // but under perception). The intensity param still scales it.
        let furrow = hat * w_vortex * (0.35 + 0.30 * push) * (0.3 + 0.7 * U.u_params.x);
        // ASYMMETRIC RELAXATION — dig fast, fill slow. Water piles
        // beside a moving finger instantly but fills the hole over
        // seconds. Digging toward the furrow (target below h) runs at
        // 0.5 so the dent tracks the cursor; REFILLING (target above
        // h — the dent closing, e.g. after the finger stops or moves
        // on) runs at 0.04: the depression LINGERS ~1 s and the wave
        // equation turns its rebound into the big release ring — the
        // visible bloom the speed-scaled furrow had smoothed away
        // ("waves only show when I'm in another window": there the
        // free pond finally radiated what the quick close had
        // suppressed). Both phases are contractions toward bounded
        // targets: nothing can accumulate.
        let r = select(0.04, 0.5, furrow < h);
        let dh = (furrow - h) * r;
        h += dh;
        v += dh * 2.0;
        // MOMENTUM = the carve's own delta (gain 2). dh is nonzero
        // only where the finger is actively changing the field: it
        // delivers the impulse while the stroke passes, stops the
        // instant the furrow is reached (no accumulation, no fighting
        // — h at target means dh = 0), and hits hardest when
        // re-carving an old ribbon (h far from target). (Two fancier
        // couplings were measured and rejected: fixed-kick targets
        // pinned the clamp on slow strokes; rate=dig delivered too
        // little mid-drag.)
    }

    // The pond always returns to calm: a LINEAR pull of the height
    // toward rest (0) — neutral by construction (a uniform level shift
    // makes no gradient and heals at the same rate everywhere). A
    // NON-linear pull was tried and measured poisonous: it extracts
    // more from crests than from valleys, the pond's mean sinks every
    // frame, and the whole field settles pinned near the pull's
    // equilibrium — every stroke dead (the exact bug this file
    // chased).
    h -= h * 0.002;

    // Clamp: a runaway value (driver hiccup) can never poison the
    // field — NaN/Inf would otherwise persist forever. On v this is a
    // soft LIMITER (smooth compression, no kink at ±1): it bounds the
    // wave amplitude so repeated carving can never ramp h into the
    // clamp plateau (a flat mesa neither radiates nor heals — seen as
    // 78-97% of the re-stroke corridor pinned at |h| = 1.000 with
    // every purely-additive scheme). Mass-neutral; fades to nothing as
    // |v| -> 0, so small waves pass untouched.
    v = v * (1.0 + (v * v) * 0.5) / (1.0 + (v * v) * 1.5);
    // Same protection on h, as a SOFT KNEE that is exact identity
    // below 0.85 (the carve furrow lives at ~0.4-0.66, untouched) and
    // smoothly compresses only the transients that would otherwise
    // hit the clamp during a re-carve over an old ribbon (the edge
    // Laplacian pumps h via v; measured: 78-97% of the corridor
    // pinned at 1.000 without it). Odd function — mass-neutral.
    let ah = abs(h);
    h = sign(h) * select(
        ah,
        0.85 + (ah - 0.85) / (1.0 + (ah - 0.85) * 2.0),
        ah > 0.85,
    );
    h = clamp(h, -1.0, 1.0);
    v = clamp(v, -1.0, 1.0);

    // NAN EXORCISM. clamp(NaN) is NaN in WGSL, and one NaN texel
    // infects its neighbors through the stencil every frame, forever:
    // growing dead islands where no wave can ever exist again (seen
    // live as "the water layer dies" and "no new wave over here"). A
    // non-finite texel (NaN fails every comparison, including with
    // itself) resets to calm; an infection can then never outgrow one
    // frame of spread around its seed.
    if (h == h) {
        h = clamp(h, -1.0, 1.0);
    } else {
        h = 0.0;
    }
    if (v == v) {
        v = clamp(v, -1.0, 1.0);
    } else {
        v = 0.0;
    }

    return vec4<f32>(h + 0.5, v + 0.5, 0.5, 1.0);
}

// AMBIENT WATER — a permanent, ORGANIC ripple field: value-noise FBM
// with domain warping, drifting slowly. A calm pond breathing, not
// running water. (Three crossed sines were tried first: over a dark
// background they read as a regular diagonal lattice of dots — the
// math showing through. Warped noise cannot form a lattice.) Its
// gradient feeds the SAME normal the wake uses, so both shade
// identically. The `ambient` parameter scales it (0 = still pond
// between strokes).
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn swell(p: vec2<f32>, t: f32) -> f32 {
    // Domain warp: the sample position is displaced by two other noise
    // lookups, so the ripples curl and pinch like real water instead of
    // following a wave vector. Drifts are tuned to tens of seconds —
    // a pond, not a stream.
    let q = vec2<f32>(
        value_noise(p * 0.004 + vec2<f32>(0.0, t * 0.03)),
        value_noise(p * 0.004 + vec2<f32>(5.2, -t * 0.026)),
    );
    let w = value_noise(p * 0.011 + 3.5 * q + vec2<f32>(t * 0.05, -t * 0.04));
    return (w - 0.5) * 0.12;
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
    // `ambient` (u_params.z) scales the pond's breathing; 0 = fully
    // still (a true lock for A/B testing the ambient in isolation).
    let grad = grad_wake + ga * (1.1 * u.u_params.z);

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

    // ADDITIVE light on disturbed water: reflection ADDS light instead
    // of only modulating the photo — multiplication alone can never
    // brighten pure black, and on a dark photo the wake would vanish.
    // COMPRESSIVE response (12s/(1+12s)) so slope never hard-caps, and
    // BLOOM-STYLE HIGHLIGHT PROTECTION: the addition fades as the pixel
    // approaches white (1 - luminance). Without it, an old ribbon plus
    // its light stacked past the framebuffer's white clip — the whole
    // wake area SATURATED to white and a re-stroke over the same path
    // could add nothing visible ("passing again makes no wave", even
    // though the physics below was radiating fine). With the (1-lum)
    // factor the lit water plateaus just under the clip: every new
    // kick keeps screen headroom.
    let slope = length(grad);
    let lum = clamp(dot(col, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);
    col += vec3<f32>(0.45, 0.55, 0.65)
        * (slope * 12.0) / (1.0 + slope * 12.0)
        * (1.0 - lum)
        * (0.4 + 0.6 * u.u_params.x);

    // Scattered light across the disturbed area (the SPREAD term): a
    // gentle cool lift that reaches well past the ring — the "light
    // plays over the water" feeling.
    let scatter = clamp(abs(spread) * 8.0, 0.0, 1.0);
    col *= 1.0 + scatter * 0.22 * (0.4 + 0.6 * u.u_params.x);

    // A sheen on the steepest crests, for sparkle (bounded mix: the
    // rims can never clip to pure white).
    let spec = pow(clamp(dot(reflect(-l, n), vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0), 34.0);
    col = mix(col, vec3<f32>(0.9, 0.94, 1.0), spec * 0.12);

    return vec4<f32>(col, 1.0);
}
