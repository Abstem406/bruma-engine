// Drifting fog over mountain silhouettes — template.
//
// The two "ambient" techniques creators combine the most:
//   - value-noise FBM for fog layers (no textures, cheap on iGPUs)
//   - day/night tint driven by u_clock: the engine feeds the REAL local
//     time, so the wallpaper follows the day on its own
//
// Parameters (rename freely in wallpaper.json):
//   speed   — fog drift speed (default 0.5)
//   density — fog opacity (default 0.6)
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
    out.uv = vec2<f32>(uvs[idx].x, 1.0 - uvs[idx].y);
    return out;
}

// --- classic value noise + FBM (no textures needed) ---

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

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        v += amp * value_noise(q);
        q = q * 2.03 + vec2<f32>(11.0, 7.0);
        amp *= 0.5;
    }
    return v;
}

// Day factor from the REAL local clock: 1.0 at solar noon (~13:30),
// 0.0 at midnight. Feeds the sky and fog tints.
fn daylight() -> f32 {
    let h = U.u_clock.x + U.u_clock.y / 60.0;
    let angle = (h - 13.5) / 24.0 * 6.2831853;
    return clamp(cos(angle) * 0.5 + 0.5, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let day = daylight();
    let speed = 0.2 + 0.8 * U.u_params.x;
    let density = U.u_params.y;

    // Sky gradient tinted by the real clock: cold night, warm dusk,
    // clear noon.
    let night = vec3<f32>(0.05, 0.07, 0.12);
    let dusk  = vec3<f32>(0.35, 0.22, 0.28);
    let noon  = vec3<f32>(0.55, 0.68, 0.82);
    var sky = mix(night, dusk, smoothstep(0.0, 0.35, day));
    sky = mix(sky, noon, smoothstep(0.35, 1.0, day));

    // Mountain silhouettes: two ridges from cheap FBM cuts.
    let r1 = 0.42 + 0.08 * fbm(vec2<f32>(uv.x * 3.1, 1.7));
    let r2 = 0.30 + 0.06 * fbm(vec2<f32>(uv.x * 4.7, 9.2));
    var col = sky;
    col = mix(col, vec3<f32>(0.08, 0.09, 0.13), smoothstep(r1 + 0.002, r1 - 0.002, uv.y) * 0.9);
    col = mix(col, vec3<f32>(0.04, 0.05, 0.08), smoothstep(r2 + 0.002, r2 - 0.002, uv.y));

    // Two drifting fog layers (far one slower: depth by speed).
    let t = U.u_time * speed;
    let fog_far  = fbm(vec2<f32>(uv.x * 2.0 + t * 0.15, uv.y * 3.0 - t * 0.03));
    let fog_near = fbm(vec2<f32>(uv.x * 3.0 - t * 0.30, uv.y * 4.0 + t * 0.05));
    let fog = clamp(fog_far * 0.7 + fog_near * 0.6 - 0.25, 0.0, 1.0);
    let fog_col = mix(vec3<f32>(0.75, 0.78, 0.85), vec3<f32>(0.95, 0.93, 0.88), day);
    col = mix(col, fog_col, fog * density * (0.35 + 0.65 * uv.y));

    return vec4<f32>(col, 1.0);
}
