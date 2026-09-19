//! Layer stacks: the data model behind the studio's layer editor and the
//! WGSL generator that compiles a stack into one shader.
//!
//! A wallpaper is a BASE (the user's photo, aspect-fit into the screen)
//! plus an ordered stack of EFFECTS composited transparently on top —
//! the Wallpaper Engine model. Each layer carries its own parallax
//! `depth` (how much it shifts against the cursor) and `opacity`. The
//! generator emits a single `fs_main`: distorter layers (heat haze)
//! compute their masks before the base sample; compositor layers mix or
//! add over the accumulated color in stack order.
//!
//! Pure module: parse → validate → generate, with the naga compile
//! asserted in tests, exactly like production assembles it (prelude +
//! generated code).

/// One tunable of an effect: name (within the effect), label, default.
pub struct EffectParam {
    pub name: &'static str,
    pub label: &'static str,
    pub default: f32,
}

/// One effect of the catalog.
pub struct Effect {
    pub id: &'static str,
    pub label: &'static str,
    pub params: &'static [EffectParam],
    /// WGSL helper functions the layer block needs (deduped by name).
    pub helpers: &'static [&'static str],
    /// Needs the cursor (manifest `mouse` permission).
    pub mouse: bool,
    /// Needs a photo base (re-samples tex0 for refraction).
    pub needs_photo: bool,
    /// Distorter: contributes a UV offset to everything behind instead
    /// of color (heat haze).
    pub distorter: bool,
    /// Emits the layer's WGSL block. `depth`/`opacity` arrive as
    /// formatted constants; `p` maps the effect's param names to GPU
    /// expressions (`u.u_params…`).
    pub emit: fn(args: &EmitArgs) -> String,
}

/// What `Effect::emit` gets.
pub struct EmitArgs<'a> {
    /// Parallax depth, formatted (e.g. "0.10").
    pub depth: &'a str,
    /// Opacity, formatted (e.g. "1.00").
    pub opacity: &'a str,
    /// Param name → WGSL expression (e.g. "density" → "u.u_params[2]").
    pub p: &'a dyn Fn(&str) -> String,
}

// ---------------------------------------------------------------- //
// WGSL helpers (emitted once per used name, before fs_main).

const HELPER_HASH: &str = r#"
fn bruma_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}"#;

const HELPER_NOISE: &str = r#"
fn bruma_vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = bruma_hash(i);
    let b = bruma_hash(i + vec2<f32>(1.0, 0.0));
    let c = bruma_hash(i + vec2<f32>(0.0, 1.0));
    let d = bruma_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}"#;

const HELPER_FBM: &str = r#"
fn bruma_fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += amp * bruma_vnoise(q);
        q = q * 2.1 + vec2<f32>(17.3, 9.1);
        amp *= 0.5;
    }
    return v;
}"#;

// ---------------------------------------------------------------- //
// Catalog.

const FOG_PARAMS: &[EffectParam] = &[
    EffectParam {
        name: "density",
        label: "Density",
        default: 0.6,
    },
    EffectParam {
        name: "speed",
        label: "Speed",
        default: 0.5,
    },
];
const WAVES_PARAMS: &[EffectParam] = &[
    EffectParam {
        name: "waves",
        label: "Waves",
        default: 0.5,
    },
    EffectParam {
        name: "speed",
        label: "Speed",
        default: 0.5,
    },
];
const AURORA_PARAMS: &[EffectParam] = &[
    EffectParam {
        name: "glow",
        label: "Glow",
        default: 0.35,
    },
    EffectParam {
        name: "drift",
        label: "Drift",
        default: 0.5,
    },
];
const STARS_PARAMS: &[EffectParam] = &[
    EffectParam {
        name: "density",
        label: "Density",
        default: 0.4,
    },
    EffectParam {
        name: "twinkle",
        label: "Twinkle",
        default: 0.5,
    },
];
const TRAIL_PARAMS: &[EffectParam] = &[EffectParam {
    name: "glow",
    label: "Glow",
    default: 0.5,
}];
const HEAT_PARAMS: &[EffectParam] = &[
    EffectParam {
        name: "strength",
        label: "Strength",
        default: 0.5,
    },
    EffectParam {
        name: "speed",
        label: "Speed",
        default: 0.5,
    },
];

pub fn catalog() -> &'static [Effect] {
    &[
        Effect {
            id: "fog",
            label: "Fog",
            params: FOG_PARAMS,
            helpers: &[HELPER_HASH, HELPER_NOISE, HELPER_FBM],
            mouse: false,
            needs_photo: false,
            distorter: false,
            emit: |a| {
                let density = (a.p)("density");
                let speed = (a.p)("speed");
                format!(
                    r#"    // == fog ==
    {{
        let p = (uv + mouse_off * {depth}) * vec2<f32>(2.4, 3.2);
        let f = clamp(
            bruma_fbm(p + vec2<f32>(t * {speed} * 0.12, -t * {speed} * 0.03)) * 0.7
                + bruma_fbm(p * 1.6 - vec2<f32>(t * {speed} * 0.22, 0.0)) * 0.6
                - 0.25,
            0.0,
            1.0,
        );
        col = mix(col, vec3<f32>(0.82, 0.85, 0.90), f * {density} * {opacity});
    }}
"#,
                    depth = a.depth,
                    opacity = a.opacity,
                    density = density,
                    speed = speed,
                )
            },
        },
        Effect {
            id: "waves",
            label: "Waves",
            params: WAVES_PARAMS,
            helpers: &[],
            mouse: false,
            needs_photo: true,
            distorter: false,
            emit: |a| {
                let waves = (a.p)("waves");
                let speed = (a.p)("speed");
                format!(
                    r#"    // == waves == (refracts the photo)
    {{
        let r = length((uv - vec2<f32>(0.5)) * vec2<f32>(U.u_res.x / max(U.u_res.y, 1.0), 1.0));
        let w = sin(r * 28.0 - t * (0.3 + {speed} * 2.2)) * exp(-r * 2.2);
        let wob = vec2<f32>(w * {waves} * 0.05);
        let refr = textureSample(tex0, samp0, bruma_texture_fit(uv + wob, tex0, U.u_res, BRUMA_TEX0_FIT)).rgb;
        col = mix(col, refr + vec3<f32>(0.9, 0.95, 1.0) * smoothstep(0.0, 1.0, w) * 0.18, {opacity});
    }}
"#,
                    speed = speed,
                    waves = waves,
                    opacity = a.opacity,
                )
            },
        },
        Effect {
            id: "aurora",
            label: "Aurora (parallax)",
            params: AURORA_PARAMS,
            helpers: &[HELPER_HASH, HELPER_NOISE, HELPER_FBM],
            mouse: true,
            needs_photo: false,
            distorter: false,
            emit: |a| {
                let glow = (a.p)("glow");
                let drift = (a.p)("drift");
                format!(
                    r#"    // == aurora == (three depth bands over the base)
    {{
        var acc = vec3<f32>(0.0);
        for (var i = 0; i < 3; i++) {{
            let fi = f32(i);
            let par = {depth} * 0.10 * (1.0 - fi * 0.3);
            let pu = uv + mouse_off * par;
            let band = bruma_fbm(vec2<f32>(
                pu.x * (2.0 + fi) + t * (0.02 + 0.03 * fi) * (0.4 + {drift}),
                pu.y * (1.2 + fi * 0.6) - fi * 0.35,
            ) + vec2<f32>(0.0, t * 0.05));
            let curtain = smoothstep(0.45, 0.75, band);
            let tint = mix(vec3<f32>(0.1, 0.9, 0.55), vec3<f32>(0.4, 0.3, 0.9), fi * 0.5);
            acc += tint * curtain * (0.35 - fi * 0.09);
        }}
        col += acc * {glow} * {opacity};
    }}
"#,
                    depth = a.depth,
                    glow = glow,
                    drift = drift,
                    opacity = a.opacity,
                )
            },
        },
        Effect {
            id: "stars",
            label: "Stars",
            params: STARS_PARAMS,
            helpers: &[HELPER_HASH],
            mouse: false,
            needs_photo: false,
            distorter: false,
            emit: |a| {
                let density = (a.p)("density");
                let twinkle = (a.p)("twinkle");
                format!(
                    r#"    // == stars == (procedural grid field, per-star twinkle)
    {{
        let cell = uv * vec2<f32>(U.u_res.x / max(U.u_res.y, 1.0), 1.0) * 36.0;
        let id = floor(cell);
        let f = fract(cell) - 0.5;
        let rnd = bruma_hash(id);
        let on = select(0.0, 1.0, rnd < {density} * 0.6);
        let off = vec2<f32>(
            bruma_hash(id + vec2<f32>(0.4, 7.7)) - 0.5,
            bruma_hash(id + vec2<f32>(3.9, 1.1)) - 0.5,
        ) * 0.6;
        let tw = 0.55 + 0.45 * sin(t * (1.0 + rnd * 3.0) * (0.5 + {twinkle} * 2.0) + rnd * 6.28);
        let star = on * tw * smoothstep(0.22, 0.0, length(f - off));
        col += vec3<f32>(0.9, 0.95, 1.0) * star * {opacity};
    }}
"#,
                    density = density,
                    twinkle = twinkle,
                    opacity = a.opacity,
                )
            },
        },
        Effect {
            id: "trail",
            label: "Light trail",
            params: TRAIL_PARAMS,
            helpers: &[],
            mouse: true,
            needs_photo: false,
            distorter: false,
            emit: |a| {
                let glow = (a.p)("glow");
                format!(
                    r#"    // == trail == (glow resting on the base, follows the cursor)
    {{
        let m = U.u_mouse / max(U.u_res, vec2<f32>(1.0));
        let muv = vec2<f32>(m.x, 1.0 - m.y);
        let d = length((uv - muv) * vec2<f32>(U.u_res.x / max(U.u_res.y, 1.0), 1.0));
        let g = exp(-d * d * 1400.0) * known;
        col += vec3<f32>(0.55, 0.75, 1.0) * g * {glow} * {opacity};
    }}
"#,
                    glow = glow,
                    opacity = a.opacity,
                )
            },
        },
        Effect {
            id: "heat",
            label: "Heat haze",
            params: HEAT_PARAMS,
            helpers: &[HELPER_HASH, HELPER_NOISE, HELPER_FBM],
            mouse: false,
            needs_photo: true,
            distorter: true,
            emit: |a| {
                let strength = (a.p)("strength");
                let speed = (a.p)("speed");
                format!(
                    r#"    // == heat haze mask == (distorts everything behind it)
    let heat_p = (uv + mouse_off * {depth}) * vec2<f32>(6.0, 4.0) + vec2<f32>(0.0, -t * {speed} * 0.8);
    let heat_m = smoothstep(0.35, 0.75, bruma_fbm(heat_p));
    let heat_off = vec2<f32>(
        bruma_fbm(heat_p + vec2<f32>(4.7, 1.3)) - 0.5,
        bruma_fbm(heat_p + vec2<f32>(9.2, 3.7)) - 0.5,
    ) * ({strength} * 0.08) * heat_m;
"#,
                    depth = a.depth,
                    strength = strength,
                    speed = speed,
                )
            },
        },
    ]
}

pub fn effect_by_id(id: &str) -> Option<&'static Effect> {
    catalog().iter().find(|e| e.id == id)
}

// ---------------------------------------------------------------- //
// Data model (what the studio stores as `layers.json`).

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Layer {
    pub effect: String,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Parallax strength 0..1 (0 = glued to the base).
    #[serde(default)]
    pub depth: f32,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, f32>,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LayersDoc {
    /// The base layer: the user's photo (texture slot 0). Absent = dark
    /// base color (effects like stars on a night sky need nothing more).
    #[serde(default)]
    pub base: Option<BasePhoto>,
    #[serde(default)]
    pub layers: Vec<Layer>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BasePhoto {
    pub path: String,
    #[serde(default = "cover")]
    pub fit: String,
}

fn cover() -> String {
    "cover".to_owned()
}

// ---------------------------------------------------------------- //
// Generation.

pub struct Generated {
    /// The full shader source (creator-level; the engine adds its
    /// prelude on compile).
    pub wgsl: String,
    /// Flattened manifest params in `u_params` order.
    pub params: Vec<(String, String, f32)>,
    /// True when any layer needs the cursor.
    pub mouse: bool,
}

fn param_expr(index: usize) -> String {
    match index {
        0..=3 => format!("U.u_params[{index}]"),
        4..=7 => format!("U.u_params4[{}]", index - 4),
        8..=11 => format!("U.u_params8[{}]", index - 8),
        _ => format!("U.u_params12[{}]", index - 12),
    }
}

/// Compiles a layer stack into one shader.
pub fn generate(doc: &LayersDoc) -> Result<Generated, String> {
    if doc.layers.len() > 8 {
        return Err(format!(
            "too many layers ({}) — the engine composites up to 8",
            doc.layers.len()
        ));
    }
    let mut params: Vec<(String, String, f32)> = Vec::new();
    let mut used_names: std::collections::BTreeSet<String> = Default::default();
    let mut mouse = false;
    let mut helpers: Vec<&str> = Vec::new();
    let mut blocks = String::new();
    let mut masks = String::new();
    let mut mask_names: Vec<String> = Vec::new();
    let mut layer_no = 0usize;

    for layer in &doc.layers {
        layer_no += 1;
        let effect = effect_by_id(&layer.effect)
            .ok_or_else(|| format!("unknown effect '{}'", layer.effect))?;
        if effect.needs_photo && doc.base.is_none() {
            return Err(format!("effect '{}' needs a photo base", layer.effect));
        }
        mouse |= effect.mouse;
        for h in effect.helpers {
            if !helpers.contains(h) {
                helpers.push(h);
            }
        }
        // Params: explicit values or effect defaults, in catalog order,
        // flattened into the manifest's u_params (16 max).
        let mut exprs: Vec<(String, String)> = Vec::new();
        for ep in effect.params {
            let value = layer.params.get(ep.name).copied().unwrap_or(ep.default);
            if !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "layer {} ({}): param '{}' out of 0..1",
                    layer_no, layer.effect, ep.name
                ));
            }
            let mut name = format!("{}_{}", layer.effect, ep.name);
            let mut n = 1;
            while !used_names.insert(name.clone()) {
                n += 1;
                name = format!("{}{}_{}", layer.effect, n, ep.name);
            }
            if params.len() >= 16 {
                return Err(
                    "too many parameters across layers (max 16) — reduce effects".to_owned(),
                );
            }
            exprs.push((ep.name.to_owned(), param_expr(params.len())));
            params.push((
                name.clone(),
                format!("{} · {}", effect.label, ep.label),
                value,
            ));
        }
        let lookup = |name: &str| -> String {
            exprs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, e)| e.clone())
                .unwrap_or_else(|| "0.0".to_owned())
        };
        let args = EmitArgs {
            depth: &format!("{:.3}", layer.depth.clamp(0.0, 1.0)),
            opacity: &format!("{:.3}", layer.opacity.clamp(0.0, 1.0)),
            p: &lookup,
        };
        if effect.distorter {
            masks.push_str(&(effect.emit)(&args));
            mask_names.push("heat_off".to_owned());
        } else {
            blocks.push_str(&(effect.emit)(&args));
        }
    }

    // Base sample: photo (aspect-fit) or dark color, shifted by the
    // distorter masks and a small base parallax.
    let base = match &doc.base {
        Some(_) => format!(
            "    let base_uv = bruma_texture_fit(uv + mouse_off * 0.03{plus}, tex0, U.u_res, BRUMA_TEX0_FIT);\n    col = textureSample(tex0, samp0, base_uv).rgb;\n",
            plus = mask_names
                .iter()
                .map(|_| " + heat_off".to_owned())
                .collect::<String>(),
        ),
        None => "    col = vec3<f32>(0.05, 0.06, 0.08);\n".to_owned(),
    };

    let wgsl = format!(
        r#"// GENERATED by the bruma studio from the layer stack in layers.json.
// Edit layers in the studio's Design tab; edits to this file are
// overwritten on the next layers change.

struct Uniforms {{
    u_time: f32,
    u_params0: f32,
    u_mouse: vec2<f32>,
    u_params: vec4<f32>,
    u_res: vec2<f32>,
    u_clock: vec3<f32>,
    u_params4: vec4<f32>,
    u_params8: vec4<f32>,
    u_params12: vec4<f32>,
}}

@group(0) @binding(0)
var<uniform> U: Uniforms;

@group(0) @binding(1)
var tex0: texture_2d<f32>;

@group(0) @binding(2)
var samp0: sampler;

struct VsOutput {{
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {{
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
}}
{helpers}
@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {{
    let uv = in.uv;
    let t = U.u_time;
    // Parallax aim: the cursor when known, a slow drift otherwise.
    let known = select(0.0, 1.0, U.u_mouse.x >= 0.0);
    let aim = mix(
        vec2<f32>(0.5 * sin(t * 0.13), 0.5 * cos(t * 0.11)),
        U.u_mouse / max(U.u_res, vec2<f32>(1.0)),
        known,
    );
    let mouse_off = aim - vec2<f32>(0.5);
    var col: vec3<f32>;
{masks}{base}{blocks}    return vec4<f32>(col, 1.0);
}}
"#,
        helpers = helpers.join("\n"),
        masks = masks,
        base = base,
        blocks = blocks,
    );

    Ok(Generated {
        wgsl,
        params,
        mouse,
    })
}

// ---------------------------------------------------------------- //

#[cfg(test)]
mod tests {
    use super::*;

    fn naga_ok(source: &str) -> bool {
        let full = bruma_renderer_wgpu::with_prelude(
            source,
            &[bruma_renderer_wgpu::TextureFit::Cover; bruma_renderer_wgpu::TEXTURE_SLOTS],
        );
        let module = match naga::front::wgsl::parse_str(&full) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("parse:\n{}", e.emit_to_string(&full));
                return false;
            }
        };
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        match validator.validate(&module) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("validate: {e}");
                false
            }
        }
    }

    #[test]
    fn full_stack_compiles() {
        let doc = LayersDoc {
            base: Some(BasePhoto {
                path: "assets/photo.png".into(),
                fit: "cover".into(),
            }),
            layers: vec![
                Layer {
                    effect: "heat".into(),
                    opacity: 1.0,
                    depth: 0.2,
                    params: Default::default(),
                },
                Layer {
                    effect: "fog".into(),
                    opacity: 0.8,
                    depth: 0.1,
                    params: Default::default(),
                },
                Layer {
                    effect: "stars".into(),
                    opacity: 1.0,
                    depth: 0.3,
                    params: Default::default(),
                },
                Layer {
                    effect: "aurora".into(),
                    opacity: 1.0,
                    depth: 0.5,
                    params: Default::default(),
                },
                Layer {
                    effect: "waves".into(),
                    opacity: 1.0,
                    depth: 0.0,
                    params: Default::default(),
                },
                Layer {
                    effect: "trail".into(),
                    opacity: 1.0,
                    depth: 0.0,
                    params: Default::default(),
                },
            ],
        };
        let g = generate(&doc).expect("generate");
        assert!(naga_ok(&g.wgsl), "generated stack must compile");
        assert!(g.mouse, "aurora/trail require the mouse permission");
        // heat(2) + fog(2) + stars(2) + aurora(2) + waves(2) + trail(1) = 11 params
        assert_eq!(g.params.len(), 11);
        // Unique manifest names, in order.
        let names: Vec<_> = g.params.iter().map(|(n, _, _)| n.clone()).collect();
        assert_eq!(
            names.len(),
            names
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        assert_eq!(names[0], "heat_strength");
    }

    #[test]
    fn color_base_and_single_layer_compiles() {
        let doc = LayersDoc {
            base: None,
            layers: vec![Layer {
                effect: "stars".into(),
                opacity: 1.0,
                depth: 0.2,
                params: Default::default(),
            }],
        };
        let g = generate(&doc).expect("generate");
        assert!(naga_ok(&g.wgsl));
        assert!(!g.mouse);
    }

    #[test]
    fn duplicate_effects_get_unique_param_names() {
        let doc = LayersDoc {
            base: None,
            layers: vec![
                Layer {
                    effect: "stars".into(),
                    opacity: 1.0,
                    depth: 0.1,
                    params: Default::default(),
                },
                Layer {
                    effect: "stars".into(),
                    opacity: 1.0,
                    depth: 0.3,
                    params: Default::default(),
                },
            ],
        };
        let g = generate(&doc).expect("generate");
        assert!(naga_ok(&g.wgsl));
        assert_eq!(g.params[0].0, "stars_density");
        assert_eq!(g.params[2].0, "stars2_density");
    }

    #[test]
    fn rejects_bad_stacks() {
        // waves without a photo base
        let doc = LayersDoc {
            base: None,
            layers: vec![Layer {
                effect: "waves".into(),
                opacity: 1.0,
                depth: 0.0,
                params: Default::default(),
            }],
        };
        assert!(generate(&doc).is_err());
        // unknown effect
        let doc = LayersDoc {
            base: None,
            layers: vec![Layer {
                effect: "lava".into(),
                opacity: 1.0,
                depth: 0.0,
                params: Default::default(),
            }],
        };
        assert!(generate(&doc).is_err());
        // param out of range
        let mut params = std::collections::BTreeMap::new();
        params.insert("density".to_owned(), 7.0);
        let doc = LayersDoc {
            base: None,
            layers: vec![Layer {
                effect: "stars".into(),
                opacity: 1.0,
                depth: 0.0,
                params,
            }],
        };
        assert!(generate(&doc).is_err());
    }

    #[test]
    fn json_round_trip() {
        let doc = LayersDoc {
            base: Some(BasePhoto {
                path: "assets/p.jpg".into(),
                fit: "contain".into(),
            }),
            layers: vec![Layer {
                effect: "fog".into(),
                opacity: 0.7,
                depth: 0.15,
                params: [("speed".to_owned(), 0.9)].into_iter().collect(),
            }],
        };
        let json = serde_json::to_string(&doc).unwrap();
        let back: LayersDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
        assert_eq!(back.layers[0].params["speed"], 0.9);
        // Defaults apply on parse (opacity 1.0, depth 0.0 omitted).
        let minimal: LayersDoc =
            serde_json::from_str(r#"{"layers":[{"effect":"stars"}]}"#).unwrap();
        assert_eq!(minimal.layers[0].opacity, 1.0);
        assert_eq!(minimal.layers[0].depth, 0.0);
        assert!(minimal.base.is_none());
    }
}
