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
}
