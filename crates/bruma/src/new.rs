//! `bruma new`: scaffold a wallpaper package (Phase 6).
//!
//! One command from zero to an installable package:
//! `bruma new my-fog --template fog` writes the manifest, the shader and
//! a preview placeholder, all valid from the first `bruma pack` /
//! `bruma validate`. The manifest is checked with the real parser before
//! anything is written, so a scaffold cannot ship broken (the manifest
//! schema is the contract). Templates are embedded in the binary.

use std::path::{Path, PathBuf};

/// Built-in templates (Phase 6). Each one compiles standalone against the
/// engine's uniform block and declares its tunable params in the
/// generated manifest.
const TEMPLATES: &[(&str, &str)] = &[
    ("waves", include_str!("templates/waves.wgsl")),
    ("fog", include_str!("templates/fog.wgsl")),
    ("water", include_str!("templates/water.wgsl")),
];

fn template_source(name: &str) -> Option<&'static str> {
    TEMPLATES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

pub fn template_list() -> String {
    TEMPLATES
        .iter()
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Params each template declares, in the order the shader expects them
/// in `u_params`.
fn template_params(name: &str) -> &[(&str, &str, f32)] {
    match name {
        // (name, label, default)
        "waves" => &[("speed", "Speed", 0.5), ("glow", "Glow", 0.3)],
        "fog" => &[("speed", "Speed", 0.5), ("density", "Density", 0.6)],
        "water" => &[("waves", "Waves", 0.4), ("speed", "Speed", 0.5)],
        _ => &[],
    }
}

/// 1x1 opaque PNG, so `wallpaper.json`'s `preview` is real from minute
/// zero. The creator replaces it with a proper capture later.
const PREVIEW_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0xF8, 0xCF, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Builds the manifest JSON for a new package (schema v1, Phase 4).
fn manifest_json(title: &str, template: &str) -> String {
    let params = template_params(template)
        .iter()
        .map(|(name, label, default)| {
            format!(
                r#"    {{ "name": "{}", "label": "{}", "default": {} }}"#,
                name, label, default
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    format!(
        r#"{{
  "format": 1,
  "type": "shader",
  "title": "{title}",
  "version": "0.1.0",
  "entry": "main.wgsl",
  "preview": "preview.png",
  "permissions": ["params"],
  "fps": 30,
  "params": [
{params}
  ]
}}"#,
    )
}

/// Creates the package directory and returns the written paths.
/// Refuses to touch an existing directory (never clobbers work).
pub fn create(name: &str, template: &str, dir: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let source = template_source(template).ok_or_else(|| {
        format!(
            "unknown template '{template}' (available: {})",
            template_list()
        )
    })?;

    let root: PathBuf = dir
        .map(|d| d.join(name))
        .unwrap_or_else(|| PathBuf::from(name));
    if root.exists() {
        return Err(format!(
            "{}, refusing to overwrite — remove it or choose another name",
            root.display()
        ));
    }
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("could not create {}: {e}", root.display()))?;

    let json = manifest_json(name, template);
    // Self-check with the REAL parser: a scaffold can never ship a broken
    // manifest (guards against schema drift with the templates).
    bruma_package::Manifest::parse(&json)
        .map_err(|e| format!("internal error: generated manifest is invalid: {e}"))?;

    let files = [
        ("wallpaper.json", json.into_bytes()),
        ("main.wgsl", source.as_bytes().to_vec()),
        ("preview.png", PREVIEW_PNG.to_vec()),
    ];
    let mut written = Vec::with_capacity(files.len());
    for (fname, data) in files {
        let path = root.join(fname);
        std::fs::write(&path, &data)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_manifest_is_valid() {
        for (tpl, _) in TEMPLATES {
            let json = manifest_json("Test", tpl);
            let m = bruma_package::Manifest::parse(&json)
                .unwrap_or_else(|e| panic!("template {tpl}: {e}"));
            assert_eq!(m.entry, "main.wgsl");
            assert_eq!(m.params.len(), 2, "template {tpl}");
        }
    }

    #[test]
    fn every_template_compiles_with_naga() {
        // Same validator the engine runs before touching the GPU: a
        // template that lands broken in a release fails here first.
        let failures: Vec<String> = TEMPLATES
            .iter()
            .filter_map(|(name, src)| {
                let module = match naga::front::wgsl::parse_str(src) {
                    Ok(m) => m,
                    Err(e) => return Some(format!("{name}: parse: {}", e.emit_to_string(src))),
                };
                let mut validator = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::empty(),
                );
                validator
                    .validate(&module)
                    .err()
                    .map(|e| format!("{name}: validate: {e}"))
            })
            .collect();
        assert!(failures.is_empty(), "template errors: {failures:?}");
    }

    #[test]
    fn create_scaffolds_all_files_and_refuses_overwrite() {
        let base = std::env::temp_dir().join(format!("bruma-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let files = create("demo", "waves", Some(&base)).unwrap();
        assert_eq!(files.len(), 3);
        assert!(base.join("demo/wallpaper.json").is_file());
        assert!(base.join("demo/main.wgsl").is_file());
        assert!(base.join("demo/preview.png").is_file());
        // PNG magic intact.
        let png = std::fs::read(base.join("demo/preview.png")).unwrap();
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);

        let err = create("demo", "waves", Some(&base)).unwrap_err();
        assert!(err.contains("refusing to overwrite"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unknown_template_lists_available() {
        let err = create("x", "vortice", None).unwrap_err();
        assert!(err.contains("unknown template"));
        assert!(err.contains("waves, fog, water"));
    }
}
