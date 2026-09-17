//! Configuración persistente del servicio (Fase 5): qué corre en cada
//! pantalla y con qué parámetros, sin flags a mano cada vez.
//!
//! Ubicación: `$XDG_CONFIG_HOME/bruma/config.json`
//! (`~/.config/bruma/config.json` por defecto).
//!
//! Forma (ejemplo):
//!
//! ```json
//! {
//!   "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
//!   "outputs": {
//!     "eDP-1":    { "color": "#1d2021" },
//!     "HDMI-A-1": { "package": "onda-bruma-demo", "params": { "intensidad": 0.3 } }
//!   },
//!   "fps": 30
//! }
//! ```
//!
//! Resolución: cada salida usa SU entrada; si no tiene, `default`; si no
//! hay nada, el color por defecto del motor (comportamiento previo).
//!
//! Errores estrictos: campo desconocido → error (los typos en nombres de
//! salida/param deben doler, no ignorarse), params fuera de 0..1 → error.

use serde::Deserialize;

/// Qué poner en una salida (o en todas, si es `default`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// Paquete instalado (nombre o `nombre:versión`).
    pub package: Option<String>,
    /// Overrides de parámetros por nombre (contra el manifiesto).
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    /// Ruta a un shader .wgsl suelto (alternativa a paquete).
    pub shader: Option<String>,
    /// Ruta a una imagen (alternativa a paquete).
    pub image: Option<String>,
    /// Color de fallback si la GPU falla, o contenido si no hay nada más.
    pub color: Option<String>,
}

impl OutputConfig {
    /// ¿Declara contenido de verdad (no solo color de fallback)?
    pub fn has_content(&self) -> bool {
        self.package.is_some() || self.shader.is_some() || self.image.is_some()
    }
}

/// Configuración completa del servicio.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Qué corre en las salidas SIN entrada propia.
    pub default: Option<OutputConfig>,
    /// Entradas por nombre de salida (ej. "eDP-1", "HDMI-A-1").
    #[serde(default)]
    pub outputs: std::collections::BTreeMap<String, OutputConfig>,
    /// FPS global del runtime animado (el manifiesto puede proponer
    /// otro; gana el de la config si está presente).
    pub fps: Option<u32>,
}

impl Config {
    /// Carga y valida el archivo por defecto. Errores claros con ruta:
    /// el usuario edita esto a mano.
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

    /// Ruta por defecto de la config (XDG).
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

    /// Escribe una config de ejemplo (solo si el archivo NO existe).
    pub fn write_example() -> Result<std::path::PathBuf, String> {
        let path = Self::default_path().ok_or("sin HOME ni XDG_CONFIG_HOME")?;
        if path.exists() {
            return Err(format!(
                "{}, no lo toco — edítalo a mano o bórralo para regenerar",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("no se pudo crear {}: {e}", parent.display()))?;
        }
        std::fs::write(
            &path,
            r##"{
  "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
  "outputs": {
    "eDP-1": { "color": "#1d2021" }
  },
  "fps": 30
}
"##,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Validaciones tempranas (formato de color, fps razonable).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(fps) = self.fps
            && (fps == 0 || fps > 240)
        {
            return Err(format!("fps={fps} fuera de rango (1..240)"));
        }
        for oc in self.all_outputs() {
            if let Some(c) = &oc.color {
                parse_color_hex(c)?;
            }
        }
        Ok(())
    }

    /// Qué usa una salida concreta: SU entrada, o `default`. (La usan
    /// los tests; la resolución real por salida ocurre en la factory del
    /// CLI con el nombre que reporta Wayland.)
    #[allow(dead_code)]
    pub fn for_output(&self, name: Option<&str>) -> Option<&OutputConfig> {
        if let Some(n) = name
            && let Some(oc) = self.outputs.get(n)
        {
            return Some(oc);
        }
        self.default.as_ref()
    }

    /// Itera todas las entradas (default + outputs) para validación.
    fn all_outputs(&self) -> impl Iterator<Item = &OutputConfig> {
        self.default.iter().chain(self.outputs.values())
    }
}

/// Parsea `#RRGGBB` / `RRGGBB` / `0xRRGGBB` a u32. Exige EXACTAMENTE 6
/// dígitos hex (RGB): `#12345` numéricamente parsea, pero como color es
/// un typo — mejor que duela. Función pura, testeada.
pub fn parse_color_hex(s: &str) -> Result<u32, String> {
    let v = s.trim().trim_start_matches("0x").trim_start_matches('#');
    if v.len() != 6 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "color inválido: '{s}' (usa hex RRGGBB, ej: #3B4252)"
        ));
    }
    u32::from_str_radix(v, 16).map_err(|_| format!("color inválido: '{s}'"))
}

/// Convierte los params de la config (`{ "intensidad": 0.9 }`) a pares
/// (nombre, f32) validando rango y tipo. Función pura, testeada.
pub fn params_a_pares(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(String, f32)>, String> {
    params
        .iter()
        .map(|(name, v)| {
            let n = v
                .as_f64()
                .ok_or_else(|| format!("param '{name}': debe ser un número (0..1), es {v}"))?;
            if !(0.0..=1.0).contains(&n) {
                return Err(format!("param '{name}': {n} fuera de rango (0..1)"));
            }
            Ok((name.clone(), n as f32))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_config_completa() {
        let text = r##"{
            "default": { "package": "onda", "params": { "intensidad": 0.9 } },
            "outputs": {
                "eDP-1": { "color": "#1d2021" },
                "HDMI-A-1": { "package": "onda:1.2.0", "fps_global": null }
            },
            "fps": 30
        }"##;
        // Este JSON tiene un campo desconocido (fps_global) → debe fallar.
        let r: Result<Config, _> = serde_json::from_str(text);
        assert!(r.is_err(), "campo desconocido debe ser error");
    }

    #[test]
    fn parsea_config_valida() {
        let text = r##"{
            "default": { "package": "onda", "params": { "intensidad": 0.9 } },
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
        // Salida sin entrada → default.
        assert!(cfg.for_output(Some("DP-3")).unwrap().package.is_some());
        // Sin nombre → default.
        assert!(cfg.for_output(None).unwrap().package.is_some());
    }

    #[test]
    fn salida_sin_default_ya_conocida() {
        let cfg: Config =
            serde_json::from_str(r##"{"outputs": {"eDP-1": {"color": "#000000"}}}"##).unwrap();
        assert!(cfg.for_output(Some("eDP-1")).is_some());
        assert!(cfg.for_output(Some("DP-9")).is_none());
        assert!(cfg.for_output(None).is_none());
    }

    #[test]
    fn colores_validos_e_invalidos() {
        assert_eq!(parse_color_hex("#1d2021").unwrap(), 0x1d_20_21);
        assert_eq!(parse_color_hex("0x3B4252").unwrap(), 0x3b_42_52);
        assert_eq!(parse_color_hex("ffffff").unwrap(), 0xff_ff_ff);
        assert!(parse_color_hex("rojo").is_err());
        assert!(parse_color_hex("#12345").is_err());
    }

    #[test]
    fn params_validos_y_fuera_de_rango() {
        let mut m = serde_json::Map::new();
        m.insert("a".into(), serde_json::json!(0.5));
        m.insert("b".into(), serde_json::json!(1));
        m.insert("c".into(), serde_json::json!(0));
        let pares = params_a_pares(&m).unwrap();
        assert_eq!(pares.len(), 3);

        let mut mal = serde_json::Map::new();
        mal.insert("x".into(), serde_json::json!(1.5));
        assert!(params_a_pares(&mal).is_err());

        let mut no_num = serde_json::Map::new();
        no_num.insert("y".into(), serde_json::json!("alto"));
        assert!(params_a_pares(&no_num).is_err());
    }

    #[test]
    fn fps_fuera_de_rango_rechaza() {
        let cfg: Config = serde_json::from_str(r##"{"fps": 1000}"##).unwrap();
        assert!(cfg.validate().is_err());
        let cfg: Config = serde_json::from_str(r##"{"fps": 0}"##).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn has_content_distingue_fallback_de_contenido() {
        let solo_color: OutputConfig = serde_json::from_str(r##"{"color": "#000000"}"##).unwrap();
        assert!(!solo_color.has_content());
        let con_pkg: OutputConfig =
            serde_json::from_str(r##"{"package": "onda", "color": "#111111"}"##).unwrap();
        assert!(con_pkg.has_content());
    }
}
