// Mouse parallax — template (mouse permission).
//
// The wallpaper reacts to the pointer: distant layers follow it slowly,
// near layers more strongly — the parallax depth trick. The engine feeds
// the pointer position in u_mouse (logical px, -1/-1 = unknown); when it
// is unknown the layers drift on their own so the wallpaper never looks
// dead.
//
// Parameters (rename freely in wallpaper.json):
//   depth — overall parallax strength (default 0.5)
//   glow  — aurora brightness (default 0.35)
//
// Uniforms (binding 0, 64 bytes): u_mouse = pointer position in
// surface pixels (x grows right, y grows DOWN like screen coordinates).

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

// Cheap value noise (hash based) — three drifting aurora bands.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let h = vec2<f32>(127.1, 311.7);
    let a = fract(sin(dot(i, h)) * 43758.5453);
    let b = fract(sin(dot(i + vec2<f32>(1.0, 0.0), h)) * 43758.5453);
    let c = fract(sin(dot(i + vec2<f32>(0.0, 1.0), h)) * 43758.5453);
    let d = fract(sin(dot(i + vec2<f32>(1.0, 1.0), h)) * 43758.5453);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += amp * noise(q);
        q = q * 2.1 + vec2<f32>(17.3, 9.1);
        amp *= 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    let t = U.u_time;
    let depth = 0.02 + 0.10 * U.u_params.x;

    // Parallax offset per layer: nearer layers move more. When the
    // pointer is unknown (-1,-1) the offset becomes a slow autonomous
    // drift (the wallpaper stays alive on an untouched desktop).
    let known = select(0.0, 1.0, U.u_mouse.x >= 0.0);
    let aim = mix(
        vec2<f32>(0.5 * sin(t * 0.13), 0.5 * cos(t * 0.11)),
        U.u_mouse / max(U.u_res, vec2<f32>(1.0)),
        known,
    );
    let off = aim - vec2<f32>(0.5);

    // Night-sky base gradient with the day/night tint from u_clock.
    let dayness = smoothstep(6.5, 9.0, U.u_clock.x) * (1.0 - smoothstep(17.5, 20.0, U.u_clock.x));
    let night = vec3<f32>(0.03, 0.05, 0.10);
    let day = vec3<f32>(0.35, 0.55, 0.75);
    var col = mix(night, day, dayness) * (0.35 + 0.5 * (1.0 - in.uv.y));

    // Three aurora bands at increasing depth. Deeper layers: slower
    // drift and LESS parallax (they are "far away").
    let glow = 0.2 + 0.6 * U.u_params.y;
    for (var i = 0; i < 3; i++) {
        let fi = f32(i);
        let par = depth * (1.0 - fi * 0.3);
        let p = vec2<f32>(
            in.uv.x * (2.0 + fi) + off.x * par * 8.0 + t * (0.02 + 0.03 * fi),
            in.uv.y * (1.2 + fi * 0.6) + off.y * par * 6.0 - fi * 0.35,
        );
        let band = fbm(p + vec2<f32>(0.0, t * 0.05));
        let curtain = smoothstep(0.45, 0.75, band);
        let tint = mix(vec3<f32>(0.1, 0.9, 0.55), vec3<f32>(0.4, 0.3, 0.9), fi * 0.5);
        col += tint * curtain * glow * (0.35 - fi * 0.09) * (1.0 - 0.4 * dayness);
    }

    // Vignette.
    let d = length(in.uv - vec2<f32>(0.5));
    col *= 1.0 - 0.35 * smoothstep(0.45, 0.85, d);

    return vec4<f32>(col, 1.0);
}
