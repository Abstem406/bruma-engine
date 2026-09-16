//! # bruma-core
//!
//! Tipos base y contratos compartidos del motor bruma.
//!
//! Este crate es **puro**: sin wgpu, sin Wayland, sin Steam, sin I/O de red.
//! Todo lo que aquí se define debe poder compilarse a WASM sin cambios,
//! para que la futura galería web reutilice el mismo núcleo.
//!
//! Plan y decisiones de diseño: ver `PLAN.md` y `DECISIONS.md` en la raíz
//! del repositorio.

#![forbid(unsafe_code)]

/// Versión del motor, para el campo `min_engine` del manifiesto.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errores comunes del núcleo, sin dependencias de I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrumaError {
    /// El formato del paquete no es reconocido o está incompleto.
    FormatoDesconocido,
    /// El manifiesto no cumple el esquema.
    ManifiestoInvalido(String),
}

impl core::fmt::Display for BrumaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BrumaError::FormatoDesconocido => write!(f, "formato de paquete no reconocido"),
            BrumaError::ManifiestoInvalido(d) => write!(f, "manifiesto inválido: {d}"),
        }
    }
}

impl std::error::Error for BrumaError {}

/// Resultado estándar del núcleo.
pub type Result<T> = std::result::Result<T, BrumaError>;

/// Color RGBA de 8 bits por canal, lineal en bytes.
///
/// Lo define aquí el núcleo (crates sin dependencias) para que
/// plataforma y renderizador compartan el mismo tipo sin acoplarse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for Color {
    /// Negro opaco (ni transparente: un fondo invisible no es un fondo).
    fn default() -> Self {
        Color::rgb(0, 0, 0)
    }
}

impl Color {
    /// Color opaco a partir de componentes RGB.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 0xFF }
    }

    /// Convierte a `0xRRGGBB` (útil para logs y CLI).
    pub const fn as_rgb_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Interpreta un entero `0xRRGGBB` como color opaco.
    pub const fn from_rgb_u32(v: u32) -> Self {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

/// Reservado: tipos de fondo soportados por el manifiesto.
///
/// Solo `shader` e `image` existen en la hoja de ruta cercana;
/// `video` y `web` están reservados sin implementación (ver DECISIONS.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperType {
    /// Imagen estática (Fase 2).
    Image,
    /// Fragment shader WGSL (Fases 2-3).
    Shader,
    /// Vídeo — reservado, no implementado.
    Video,
    /// Web (HTML/CSS/JS) — reservado, no implementado.
    Web,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_esta_definida() {
        assert!(!ENGINE_VERSION.is_empty());
    }

    #[test]
    fn error_se_formatea() {
        assert_eq!(
            BrumaError::ManifiestoInvalido("falta title".into()).to_string(),
            "manifiesto inválido: falta title"
        );
    }

    #[test]
    fn color_rgb_convierte() {
        let c = Color::rgb(0x2E, 0x34, 0x40);
        assert_eq!(c.as_rgb_u32(), 0x2E3440);
        assert_eq!(Color::from_rgb_u32(0x3B4252), Color::rgb(0x3B, 0x42, 0x52));
        assert_eq!(Color::default(), Color::rgb(0, 0, 0));
    }
}
