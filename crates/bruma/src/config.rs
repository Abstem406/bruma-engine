//! Persistent service configuration (Phase 5): what runs on each screen
//! and with which parameters, without hand-typed flags every time.
//!
//! Location: `$XDG_CONFIG_HOME/bruma/config.json`
//! (`~/.config/bruma/config.json` by default).
//!
//! Shape (example):
//!
//! ```json
//! {
//!   "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
//!   "outputs": {
//!     "eDP-1":    { "color": "#1d2021" },
//!     "HDMI-A-1": { "package": "onda-bruma-demo", "params": { "intensidad": 0.3 } }
//!   },
//!   "fps": 30,
//!   "fullscreen_pause": true
//! }
//! ```
//!
//! Resolution: each output uses ITS entry; if it has none, `default`; if
//! there is nothing, the engine's default color (previous behavior).
//!
//! Strict errors: unknown field → error (typos in output/param names must
//! hurt, not be ignored), params out of 0..1 → error.

use serde::{Deserialize, Serialize};

/// What to put on an output (or on all of them, if it's `default`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// Installed package (name or `name:version`).
    pub package: Option<String>,
    /// Parameter overrides by name (against the manifest).
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    /// Path to a loose .wgsl shader (alternative to a package).
    pub shader: Option<String>,
    /// Path to an image (alternative to a package).
    pub image: Option<String>,
    /// Fallback color if the GPU fails, or the content if there is
    /// nothing else.
    pub color: Option<String>,
}

impl OutputConfig {
    /// Does it declare real content (not just a fallback color)?
    pub fn has_content(&self) -> bool {
        self.package.is_some() || self.shader.is_some() || self.image.is_some()
    }
}

/// Full service configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// What runs on outputs WITHOUT their own entry.
    pub default: Option<OutputConfig>,
    /// Entries by output name (e.g. "eDP-1", "HDMI-A-1").
    #[serde(default)]
    pub outputs: std::collections::BTreeMap<String, OutputConfig>,
    /// Global FPS of the animated runtime (the manifest may propose
    /// another; config wins when present).
    pub fps: Option<u32>,
    /// Pause rendering on an output while a fullscreen window covers it
    /// (D12's power saving). Default true. Set to false to keep the
    /// wallpaper alive under fullscreen windows (a fullscreen window on
    /// ANOTHER workspace also counts as covering — this switch is the
    /// escape hatch).
    #[serde(default = "default_true")]
    pub fullscreen_pause: bool,
}

fn default_true() -> bool {
    true
}

impl Config {
    /// Loads and validates the default file. Clear errors with the path:
    /// the user edits this by hand.
    pub fn load() -> Result<Option<Config>, String> {
        let Some(path) = Self::default_path() else {
            return Ok(None);
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let cfg: Config =
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                cfg.validate()
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Some(cfg))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Default config path (XDG).
    pub fn default_path() -> Option<std::path::PathBuf> {
        let base = match std::env::var("XDG_CONFIG_HOME") {
            Ok(s) if !s.is_empty() => std::path::PathBuf::from(s),
            _ => {
                let home = std::env::var("HOME").ok()?;
                std::path::PathBuf::from(home).join(".config")
            }
        };
        Some(base.join("bruma/config.json"))
    }

    /// Writes an example config (only if the file does NOT exist).
    pub fn write_example() -> Result<std::path::PathBuf, String> {
        let path = Self::default_path().ok_or("neither HOME nor XDG_CONFIG_HOME set")?;
        if path.exists() {
            return Err(format!(
                "{}, leaving it alone — edit it by hand or delete it to regenerate",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        // Package and param names below are the real names of the demo
        // package this repo ships, so the example works as copied.
        std::fs::write(
            &path,
            r##"{
  "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
  "outputs": {
    "eDP-1": { "color": "#1d2021" }
  },
  "fps": 30,
  "fullscreen_pause": true
}
"##,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Early validations (color format, sane fps).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(fps) = self.fps
            && (fps == 0 || fps > 240)
        {
            return Err(format!("fps={fps} out of range (1..240)"));
        }
        for oc in self.all_outputs() {
            if let Some(c) = &oc.color {
                parse_color_hex(c)?;
            }
        }
        Ok(())
    }

    /// What a concrete output uses: ITS entry, or `default`. (Tests use
    /// it; the real per-output resolution happens in the CLI's factory
    /// with the name Wayland reports.)
    #[allow(dead_code)]
    pub fn for_output(&self, name: Option<&str>) -> Option<&OutputConfig> {
        if let Some(n) = name
            && let Some(oc) = self.outputs.get(n)
        {
            return Some(oc);
        }
        self.default.as_ref()
    }

    /// Iterates all entries (default + outputs) for validation.
    fn all_outputs(&self) -> impl Iterator<Item = &OutputConfig> {
        self.default.iter().chain(self.outputs.values())
    }
}

/// Parses `#RRGGBB` / `RRGGBB` / `0xRRGGBB` into a u32. Requires EXACTLY
/// 6 hex digits (RGB): `#12345` parses numerically, but as a color it is
/// a typo — better that it hurts. Pure function, tested.
pub fn parse_color_hex(s: &str) -> Result<u32, String> {
    let v = s.trim().trim_start_matches("0x").trim_start_matches('#');
    if v.len() != 6 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "invalid color: '{s}' (use RRGGBB hex, e.g. #3B4252)"
        ));
    }
    u32::from_str_radix(v, 16).map_err(|_| format!("invalid color: '{s}'"))
}

/// Converts config params (`{ "intensidad": 0.9 }`) into (name, f32)
/// pairs, validating range and type. Pure function, tested.
pub fn params_to_pairs(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(String, f32)>, String> {
    params
        .iter()
        .map(|(name, v)| {
            let n = v
                .as_f64()
                .ok_or_else(|| format!("param '{name}': must be a number (0..1), got {v}"))?;
            if !(0.0..=1.0).contains(&n) {
                return Err(format!("param '{name}': {n} out of range (0..1)"));
            }
            Ok((name.clone(), n as f32))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_field() {
        let text = r##"{
            "default": { "package": "onda", "params": { "brightness": 0.9 } },
            "outputs": {
                "eDP-1": { "color": "#1d2021" },
                "HDMI-A-1": { "package": "onda:1.2.0", "global_fps": null }
            },
            "fps": 30
        }"##;
        // This JSON has an unknown field (global_fps) → it must fail.
        let r: Result<Config, _> = serde_json::from_str(text);
        assert!(r.is_err(), "unknown field must be an error");
    }

    #[test]
    fn parses_valid_config() {
        let text = r##"{
            "default": { "package": "onda", "params": { "brightness": 0.9 } },
            "outputs": {
                "eDP-1": { "color": "#1d2021" },
                "HDMI-A-1": { "package": "onda:1.2.0" }
            },
            "fps": 30
        }"##;
        let cfg: Config = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.fps, Some(30));
        assert!(cfg.for_output(Some("eDP-1")).unwrap().color.is_some());
        assert!(cfg.for_output(Some("HDMI-A-1")).unwrap().package.is_some());
        // Output without its own entry → default.
        assert!(cfg.for_output(Some("DP-3")).unwrap().package.is_some());
        // No name → default.
        assert!(cfg.for_output(None).unwrap().package.is_some());
    }

    #[test]
    fn outputs_without_default() {
        let cfg: Config =
            serde_json::from_str(r##"{"outputs": {"eDP-1": {"color": "#000000"}}}"##).unwrap();
        assert!(cfg.for_output(Some("eDP-1")).is_some());
        assert!(cfg.for_output(Some("DP-9")).is_none());
        assert!(cfg.for_output(None).is_none());
    }

    #[test]
    fn valid_and_invalid_colors() {
        assert_eq!(parse_color_hex("#1d2021").unwrap(), 0x1d_20_21);
        assert_eq!(parse_color_hex("0x3B4252").unwrap(), 0x3b_42_52);
        assert_eq!(parse_color_hex("ffffff").unwrap(), 0xff_ff_ff);
        assert!(parse_color_hex("red").is_err());
        assert!(parse_color_hex("#12345").is_err());
    }

    #[test]
    fn params_valid_and_out_of_range() {
        let mut m = serde_json::Map::new();
        m.insert("a".into(), serde_json::json!(0.5));
        m.insert("b".into(), serde_json::json!(1));
        m.insert("c".into(), serde_json::json!(0));
        let pairs = params_to_pairs(&m).unwrap();
        assert_eq!(pairs.len(), 3);

        let mut bad = serde_json::Map::new();
        bad.insert("x".into(), serde_json::json!(1.5));
        assert!(params_to_pairs(&bad).is_err());

        let mut not_num = serde_json::Map::new();
        not_num.insert("y".into(), serde_json::json!("tall"));
        assert!(params_to_pairs(&not_num).is_err());
    }

    #[test]
    fn fps_out_of_range_rejected() {
        let cfg: Config = serde_json::from_str(r##"{"fps": 1000}"##).unwrap();
        assert!(cfg.validate().is_err());
        let cfg: Config = serde_json::from_str(r##"{"fps": 0}"##).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn has_content_distinguishes_fallback_from_content() {
        let color_only: OutputConfig = serde_json::from_str(r##"{"color": "#000000"}"##).unwrap();
        assert!(!color_only.has_content());
        let with_pkg: OutputConfig =
            serde_json::from_str(r##"{"package": "onda", "color": "#111111"}"##).unwrap();
        assert!(with_pkg.has_content());
    }
}
