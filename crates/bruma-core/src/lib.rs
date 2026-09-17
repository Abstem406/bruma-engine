//! # bruma-core
//!
//! Tipos base y contratos compartidos del motor bruma.
//!
//! Este crate es **puro**: sin wgpu, sin Wayland, sin Steam, sin E/S de
//! red. Todo lo que se define aquí debe compilar a WASM sin cambios,
//! para que la futura galería web reutilice el mismo core.
//!
//! Plan y decisiones de diseño: ver `PLAN.md` y `DECISIONS.md` en la
//! raíz del repositorio.

#![forbid(unsafe_code)]

/// Versión del motor, para el campo `min_engine` del manifiesto.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errores comunes del core, sin dependencias de E/S.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrumaError {
    /// El formato del paquete es irreconocible o incompleto.
    UnknownFormat,
    /// El manifiesto no cumple el schema.
    InvalidManifest(String),
}

impl core::fmt::Display for BrumaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BrumaError::UnknownFormat => write!(f, "formato de paquete irreconocible"),
            BrumaError::InvalidManifest(d) => write!(f, "manifiesto inválido: {d}"),
        }
    }
}

impl std::error::Error for BrumaError {}

/// Result estándar del core.
pub type Result<T> = std::result::Result<T, BrumaError>;

/// Color RGBA de 8 bits por canal, lineal en bytes.
///
/// Definido aquí en el core (crates sin dependencias) para que
/// plataforma y renderer compartan el mismo tipo sin acoplarse entre
/// sí.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for Color {
    /// Negro opaco (no transparente: un fondo invisible no es un
    /// fondo).
    fn default() -> Self {
        Color::rgb(0, 0, 0)
    }
}

impl Color {
    /// Color opaco a partir de componentes RGB.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 0xFF }
    }

    /// Convierte a `0xRRGGBB` (cómodo para logs y la CLI).
    pub const fn as_rgb_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Interpreta un entero `0xRRGGBB` como color opaco.
    pub const fn from_rgb_u32(v: u32) -> Self {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

/// Reservados: tipos de wallpaper que admite el manifiesto.
///
/// Solo `shader` y `image` existen en el roadmap cercano; `video` y
/// `web` quedan reservados sin implementación (ver DECISIONS.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperType {
    /// Imagen estática (Fase 2).
    Image,
    /// Shader de fragmentos WGSL (Fases 2-3).
    Shader,
    /// Video — reservado, sin implementar.
    Video,
    /// Web (HTML/CSS/JS) — reservado, sin implementar.
    Web,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_defined() {
        assert!(!ENGINE_VERSION.is_empty());
    }

    #[test]
    fn error_formats() {
        assert_eq!(
            BrumaError::InvalidManifest("missing title".into()).to_string(),
            "manifiesto inválido: missing title"
        );
    }

    #[test]
    fn rgb_color_converts() {
        let c = Color::rgb(0x2E, 0x34, 0x40);
        assert_eq!(c.as_rgb_u32(), 0x2E3440);
        assert_eq!(Color::from_rgb_u32(0x3B4252), Color::rgb(0x3B, 0x42, 0x52));
        assert_eq!(Color::default(), Color::rgb(0, 0, 0));
    }
}
