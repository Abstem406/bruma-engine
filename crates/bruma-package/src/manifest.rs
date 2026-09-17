//! El manifiesto `wallpaper.json`: schema v1, parseo de campos y
//! validación.
//!
//! El schema es deliberadamente mínimo (ver D5): lo que un wallpaper
//! necesita para declararse y nada más. Los campos `type: video|web`
//! están **reservados** en el schema (D10) pero este motor los rechaza
//! con un error claro hasta que haya una implementación.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::error::PackError;

/// Versión del schema que habla este motor.
pub const SCHEMA_VERSION: u32 = 1;

/// Campos requeridos por el schema v1 (PLAN, Fase 4).
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Versión del schema (1).
    pub format: u32,
    /// Tipo de wallpaper: `shader` (implementado) o `video`/`web`
    /// (reservados, D10).
    pub wallpaper_type: String,
    /// Título legible por humanos.
    pub title: String,
    /// Versión del paquete ("major.minor.patch"). Ausente: "0.0.0".
    /// Sustenta el layout de instalación versionado.
    pub version: String,
    /// Ruta del shader de entrada, relativa a la raíz del paquete.
    pub entry: String,
    /// Ruta de la imagen de preview, relativa a la raíz del paquete.
    pub preview: String,
    /// Capacidades que declara el wallpaper. Hoy solo se reconocen
    /// `params` y `mouse`; cualquier otra es un error de validación.
    pub permissions: Vec<String>,
    /// Versión mínima del motor (semver: "0.1.0").
    pub min_engine: Option<String>,
    /// Límite de FPS por defecto (1..=120). Ausente: 30.
    pub fps: Option<u32>,
    /// Parámetros con nombre: los sliders que generará la UI (Fase 6).
    /// Máx 4 (mapean 1:1 a `u_params0..3` del uniform block).
    pub params: Vec<Param>,
}

/// Un parámetro ajustable declarado por el wallpaper.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// Nombre (identidad en la CLI/UI), p. ej. `"velocidad"`.
    pub name: String,
    /// Etiqueta legible para la UI. Ausente: el nombre.
    pub label: Option<String>,
    /// Valor por defecto 0..=1. Ausente: 0.0.
    pub default: f32,
}

/// Estructura de deserialización tolerante (campos opcionales).
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
    /// Parsea y valida un manifiesto desde su JSON. Aplica todas las
    /// reglas del schema v1: versiones, tipos, rutas seguras,
    /// permisos conocidos, límites de parámetros y rangos fps/default.
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
            return Err(PackError::BadParam(format!(
                "versión inválida: '{version}'"
            )));
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
                    "permiso desconocido '{p}' (conocidos: params, mouse)"
                )));
            }
        }

        if let Some(fps) = raw.fps
            && !(1..=120).contains(&fps)
        {
            return Err(PackError::BadParam(format!("fps={fps} fuera de 1..=120")));
        }

        if raw.params.len() > 4 {
            return Err(PackError::BadParam(format!(
                "el motor expone 4 parámetros (u_params0..3), el manifiesto declara {}",
                raw.params.len()
            )));
        }
        let mut params = Vec::with_capacity(raw.params.len());
        for p in raw.params {
            let name = p.name.trim().to_owned();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(PackError::BadParam(format!(
                    "nombre de parámetro inválido: '{name}' (usa [a-zA-Z0-9_])"
                )));
            }
            let default = p.default.unwrap_or(0.0);
            if !(0.0..=1.0).contains(&default) {
                return Err(PackError::BadParam(format!(
                    "default de '{}' fuera de 0..=1",
                    p.name
                )));
            }
            params.push(Param {
                name,
                label: p.label,
                default,
            });
        }
        // Sin nombres repetidos.
        for (i, a) in params.iter().enumerate() {
            if params[i + 1..].iter().any(|b| b.name == a.name) {
                return Err(PackError::BadParam(format!(
                    "parámetro duplicado: '{}'",
                    a.name
                )));
            }
        }

        if let Some(m) = &raw.min_engine
            && !is_semver_triple(m)
        {
            return Err(PackError::BadParam(format!("min_engine inválido: '{m}'")));
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

    /// Nombre de instalación: derivado del título (minúsculas, no
    /// alfanuméricos → `-`). Determinista y estable entre versiones.
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

/// ¿Es un semver "major.minor.patch" puramente numérico?
fn is_semver_triple(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
}

/// ¿Es una ruta relativa segura? Rechaza absolutas, `..`, componentes
/// vacíos/extraños y prefijos de Windows. El `enclosed_name` de zip
/// hace la comprobación fuerte al leer cada entrada; esta es la
/// primera línea de defensa para los campos del manifiesto.
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
            "title": "Ondas Nortie",
            "entry": "main.wgsl",
            "preview": "preview.png",
            "permissions": ["params"],
            "min_engine": "0.1.0",
            "fps": 30,
            "params": [{"name": "brillo", "label": "Brillo", "default": 0.2}]
        }"#
        .to_owned()
    }

    #[test]
    fn valid_manifest_parses() {
        let m = Manifest::parse(&base_json()).unwrap();
        assert_eq!(m.wallpaper_type, "shader");
        assert_eq!(m.install_name(), "ondas-nortie");
        assert_eq!(m.params[0].name, "brillo");
        assert_eq!(m.params[0].default, 0.2);
        assert_eq!(m.fps, Some(30));
    }

    #[test]
    fn reserved_type_rejected_with_clear_message() {
        let json = base_json().replace("\"shader\"", "\"video\"");
        let err = Manifest::parse(&json).unwrap_err().to_string();
        assert!(err.contains("reservado"), "{err}");
    }

    #[test]
    fn entry_with_traversal_is_rejected() {
        for bad in [
            "../main.wgsl",
            "/etc/passwd",
            "a/../../b.wgsl",
            "sub\\x.wgsl",
        ] {
            let json = base_json().replace("main.wgsl", bad);
            assert!(
                Manifest::parse(&json).is_err(),
                "entry '{bad}' debería haber sido rechazado"
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
        r#""params": [{"name": "brillo", "label": "Brillo", "default": 0.2}]"#;

    #[test]
    fn params_limits_and_duplicates() {
        // Más de 4: rechazado.
        let json = base_json().replace(
            PARAMS_BASE,
            r#""params": [
                {"name": "a"}, {"name": "b"}, {"name": "c"}, {"name": "d"}, {"name": "e"}
            ]"#,
        );
        assert!(Manifest::parse(&json).is_err());

        // Duplicado: rechazado.
        let json = base_json().replace(PARAMS_BASE, r#""params": [{"name": "x"}, {"name": "x"}]"#);
        let err = Manifest::parse(&json).unwrap_err().to_string();
        assert!(err.contains("duplicado"), "{err}");

        // Default fuera de rango: rechazado.
        let json = base_json().replace(PARAMS_BASE, r#""params": [{"name": "x", "default": 5.0}]"#);
        assert!(Manifest::parse(&json).is_err());
    }

    #[test]
    fn default_version_and_validation() {
        // Sin versión: "0.0.0".
        assert_eq!(Manifest::parse(&base_json()).unwrap().version, "0.0.0");
        // Con versión válida.
        let json = base_json().replace(
            r#""preview": "preview.png","#,
            r#""preview": "preview.png",
            "version": "1.2.3","#,
        );
        assert_eq!(Manifest::parse(&json).unwrap().version, "1.2.3");
        // Con versión rota.
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
            ("Ondas Nortie", "ondas-nortie"),
            ("  Raro  !! ", "raro"),
            ("Mi-Wallpaper_2", "mi-wallpaper-2"),
        ] {
            let json = base_json().replace("Ondas Nortie", title);
            assert_eq!(Manifest::parse(&json).unwrap().install_name(), expected);
        }
    }
}
