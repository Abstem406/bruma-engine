//! Manifest `wallpaper.json`: schema v1, parsing and field validation.
//!
//! The schema is deliberately minimal (see D5): what a wallpaper needs to
//! declare itself and nothing more. The `type: video|web` fields are
//! **reserved** in the schema (D10) but this engine rejects them with a
//! clear error until there is an implementation.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::error::PackError;

/// Schema version this engine speaks.
pub const SCHEMA_VERSION: u32 = 1;

/// Fields the v1 schema requires (PLAN Phase 4).
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Schema version (1).
    pub format: u32,
    /// Wallpaper type: `shader` (implemented) or `video`/`web`
    /// (reserved, D10).
    pub wallpaper_type: String,
    /// Human-readable title.
    pub title: String,
    /// Package version ("major.minor.patch"). Missing: "0.0.0".
    /// Backs the versioned install layout.
    pub version: String,
    /// Entry shader path, relative to the package root.
    pub entry: String,
    /// Preview image path, relative to the package root.
    pub preview: String,
    /// Capabilities the wallpaper declares. Only `params` and `mouse` are
    /// recognized today; anything else is a validation error.
    pub permissions: Vec<String>,
    /// Minimum engine version (semver: "0.1.0").
    pub min_engine: Option<String>,
    /// Default FPS cap (1..=120). Missing: 30.
    pub fps: Option<u32>,
    /// Named parameters: the sliders the UI will generate (Phase 6).
    /// Maximum 4 (they map 1:1 to `u_params0..3` of the uniform block).
    pub params: Vec<Param>,
}

/// An adjustable parameter declared by the wallpaper.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// Name (identity in the CLI/UI), e.g. `"speed"`.
    pub name: String,
    /// Human-readable label for the UI. Missing: the name.
    pub label: Option<String>,
    /// Default value 0..=1. Missing: 0.0.
    pub default: f32,
}

/// Tolerant deserialization shape (optional fields).
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct ManifestRaw {
    format: u32,
    #[serde(rename = "type")]
    wallpaper_type: String,
    title: String,
    #[serde(default)]
    version: Option<String>,
    entry: String,
    preview: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    min_engine: Option<String>,
    #[serde(default)]
    fps: Option<u32>,
    #[serde(default)]
    params: Vec<ParamRaw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct ParamRaw {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    default: Option<f32>,
}

impl Manifest {
    /// Parses and validates a manifest from its JSON. Applies every rule
    /// of the v1 schema: versions, types, safe paths, known permissions,
    /// parameter limits and fps/default ranges.
    pub fn parse(json: &str) -> Result<Self, PackError> {
        let raw: ManifestRaw = serde_json::from_str(json)
            .map_err(|e| PackError::Json("wallpaper.json".to_owned(), e))?;
        Self::from_raw(raw)
    }

    fn from_raw(raw: ManifestRaw) -> Result<Self, PackError> {
        if raw.format != SCHEMA_VERSION {
            return Err(PackError::Format(raw.format, SCHEMA_VERSION));
        }

        match raw.wallpaper_type.as_str() {
            "shader" => {}
            "video" | "web" => return Err(PackError::ReservedType(raw.wallpaper_type)),
            other => return Err(PackError::BadType(other.to_owned())),
        }

        let title = raw.title.trim().to_owned();
        if title.is_empty() || title.len() > 80 {
            return Err(PackError::BadTitle(raw.title));
        }
        let version = raw.version.unwrap_or_else(|| "0.0.0".to_owned());
        if !is_semver_triple(&version) {
            return Err(PackError::BadParam(format!("invalid version: '{version}'")));
        }

        if !is_safe_relative(&raw.entry)
            || Path::new(&raw.entry).extension() != Some("wgsl".as_ref())
        {
            return Err(PackError::BadEntry(raw.entry));
        }
        let ext = Path::new(&raw.preview)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !is_safe_relative(&raw.preview) || !matches!(ext, "png" | "jpg" | "jpeg") {
            return Err(PackError::BadPreview(raw.preview));
        }

        for p in &raw.permissions {
            if !matches!(p.as_str(), "params" | "mouse") {
                return Err(PackError::BadParam(format!(
                    "unknown permission '{p}' (known: params, mouse)"
                )));
            }
        }

        if let Some(fps) = raw.fps
            && !(1..=120).contains(&fps)
        {
            return Err(PackError::BadParam(format!("fps={fps} out of 1..=120")));
        }

        if raw.params.len() > 4 {
            return Err(PackError::BadParam(format!(
                "the engine exposes 4 parameters (u_params0..3), the manifest declares {}",
                raw.params.len()
            )));
        }
        let mut params = Vec::with_capacity(raw.params.len());
        for p in raw.params {
            let name = p.name.trim().to_owned();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(PackError::BadParam(format!(
                    "invalid parameter name: '{name}' (use [a-zA-Z0-9_])"
                )));
            }
            let default = p.default.unwrap_or(0.0);
            if !(0.0..=1.0).contains(&default) {
                return Err(PackError::BadParam(format!(
                    "default of '{}' out of 0..=1",
                    p.name
                )));
            }
            params.push(Param {
                name,
                label: p.label,
                default,
            });
        }
        // No repeated names.
        for (i, a) in params.iter().enumerate() {
            if params[i + 1..].iter().any(|b| b.name == a.name) {
                return Err(PackError::BadParam(format!(
                    "duplicate parameter: '{}'",
                    a.name
                )));
            }
        }

        if let Some(m) = &raw.min_engine
            && !is_semver_triple(m)
        {
            return Err(PackError::BadParam(format!("invalid min_engine: '{m}'")));
        }

        Ok(Manifest {
            format: raw.format,
            wallpaper_type: raw.wallpaper_type,
            title,
            version,
            entry: raw.entry,
            preview: raw.preview,
            permissions: raw.permissions,
            min_engine: raw.min_engine,
            fps: raw.fps,
            params,
        })
    }

    /// Install name: derived from the title (lowercase, non-alphanumerics
    /// → `-`). Deterministic and stable across versions.
    pub fn install_name(&self) -> String {
        let mut out = String::new();
        for c in self.title.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
        while out.ends_with('-') {
            out.pop();
        }
        out
    }
}

/// Is this a plain numeric "major.minor.patch" semver?
fn is_semver_triple(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
}

/// Is this a safe relative path? Rejects absolute paths, `..`, empty/odd
/// components and Windows prefixes. zip's `enclosed_name` does the strong
/// check when reading each entry; this is the first line of defense for
/// manifest fields.
fn is_safe_relative(path: &str) -> bool {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|c| c == ".." || c.is_empty() || c == ".")
    {
        return false;
    }
    Path::new(path).components().count() == path.split('/').count()
}

impl fmt::Display for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "format:       {}", self.format)?;
        writeln!(f, "type:         {}", self.wallpaper_type)?;
        writeln!(f, "title:        {}", self.title)?;
        writeln!(f, "version:      {}", self.version)?;
        writeln!(f, "entry:        {}", self.entry)?;
        writeln!(f, "preview:      {}", self.preview)?;
        writeln!(f, "permissions:  {:?}", self.permissions)?;
        if let Some(m) = &self.min_engine {
            writeln!(f, "min_engine:   {m}")?;
        }
        if let Some(fps) = self.fps {
            writeln!(f, "fps:          {fps}")?;
        }
        if !self.params.is_empty() {
            writeln!(f, "params:")?;
            for p in &self.params {
                writeln!(
                    f,
                    "  - {}: {} (default {})",
                    p.name,
                    p.label.as_deref().unwrap_or(&p.name),
                    p.default
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_json() -> String {
        r#"{
            "format": 1,
            "type": "shader",
            "title": "Night Waves",
            "entry": "main.wgsl",
            "preview": "preview.png",
            "permissions": ["params"],
            "min_engine": "0.1.0",
            "fps": 30,
            "params": [{"name": "brightness", "label": "Brightness", "default": 0.2}]
        }"#
        .to_owned()
    }

    #[test]
    fn valid_manifest_parses() {
        let m = Manifest::parse(&base_json()).unwrap();
        assert_eq!(m.wallpaper_type, "shader");
        assert_eq!(m.install_name(), "night-waves");
        assert_eq!(m.params[0].name, "brightness");
        assert_eq!(m.params[0].default, 0.2);
        assert_eq!(m.fps, Some(30));
    }

    #[test]
    fn reserved_type_rejected_with_clear_message() {
        let json = base_json().replace("\"shader\"", "\"video\"");
        let err = Manifest::parse(&json).unwrap_err().to_string();
        assert!(err.contains("reserved"), "{err}");
    }

    #[test]
    fn entry_with_traversal_fails() {
        for bad in [
            "../main.wgsl",
            "/etc/passwd",
            "a/../../b.wgsl",
            "sub\\x.wgsl",
        ] {
            let json = base_json().replace("main.wgsl", bad);
            assert!(
                Manifest::parse(&json).is_err(),
                "entry '{bad}' should have been rejected"
            );
        }
    }

    #[test]
    fn old_format_rejected() {
        let json = base_json().replace("\"format\": 1", "\"format\": 2");
        let err = Manifest::parse(&json).unwrap_err().to_string();
        assert!(err.contains("format=2"), "{err}");
    }

    #[test]
    fn unknown_field_rejected() {
        let json = base_json().replace("preview.png", "preview.png\", \"trap\": 1");
        assert!(Manifest::parse(&json).is_err());
    }

    const PARAMS_BASE: &str =
        r#""params": [{"name": "brightness", "label": "Brightness", "default": 0.2}]"#;

    #[test]
    fn params_limits_and_duplicates() {
        // More than 4: rejected.
        let json = base_json().replace(
            PARAMS_BASE,
            r#""params": [
                {"name": "a"}, {"name": "b"}, {"name": "c"}, {"name": "d"}, {"name": "e"}
            ]"#,
        );
        assert!(Manifest::parse(&json).is_err());

        // Duplicate: rejected.
        let json = base_json().replace(PARAMS_BASE, r#""params": [{"name": "x"}, {"name": "x"}]"#);
        let err = Manifest::parse(&json).unwrap_err().to_string();
        assert!(err.contains("duplicate"), "{err}");

        // Default out of range: rejected.
        let json = base_json().replace(PARAMS_BASE, r#""params": [{"name": "x", "default": 5.0}]"#);
        assert!(Manifest::parse(&json).is_err());
    }

    #[test]
    fn version_default_and_validation() {
        // No version: "0.0.0".
        assert_eq!(Manifest::parse(&base_json()).unwrap().version, "0.0.0");
        // Valid version.
        let json = base_json().replace(
            r#""preview": "preview.png","#,
            r#""preview": "preview.png",
            "version": "1.2.3","#,
        );
        assert_eq!(Manifest::parse(&json).unwrap().version, "1.2.3");
        // Broken version.
        let json = base_json().replace(
            r#""preview": "preview.png","#,
            r#""preview": "preview.png",
            "version": "one.two","#,
        );
        assert!(Manifest::parse(&json).is_err());
    }

    #[test]
    fn fps_out_of_range() {
        let json = base_json().replace("\"fps\": 30", "\"fps\": 300");
        assert!(Manifest::parse(&json).is_err());
    }

    #[test]
    fn install_name_is_deterministic() {
        for (title, expected) in [
            ("Night Waves", "night-waves"),
            ("  Weird  !! ", "weird"),
            ("My-Wallpaper_2", "my-wallpaper-2"),
        ] {
            let json = base_json().replace("Night Waves", title);
            assert_eq!(Manifest::parse(&json).unwrap().install_name(), expected);
        }
    }
}
