//! # bruma-core
//!
//! Base types and shared contracts of the bruma engine.
//!
//! This crate is **pure**: no wgpu, no Wayland, no Steam, no network I/O.
//! Everything defined here must compile to WASM unchanged, so the future
//! web gallery can reuse the same core.
//!
//! Plan and design decisions: see `PLAN.md` and `DECISIONS.md` at the
//! repository root.

#![forbid(unsafe_code)]

/// Engine version, for the manifest's `min_engine` field.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Common core errors, with no I/O dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrumaError {
    /// The package format is not recognized or is incomplete.
    UnknownFormat,
    /// The manifest does not conform to the schema.
    InvalidManifest(String),
}

impl core::fmt::Display for BrumaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BrumaError::UnknownFormat => write!(f, "unrecognized package format"),
            BrumaError::InvalidManifest(d) => write!(f, "invalid manifest: {d}"),
        }
    }
}

impl std::error::Error for BrumaError {}

/// Standard result of the core.
pub type Result<T> = std::result::Result<T, BrumaError>;

/// 8-bit-per-channel RGBA color, plain bytes layout.
///
/// Defined here in the core (dependency-free crates) so platform and
/// renderer share the same type without coupling to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for Color {
    /// Opaque black (not transparent: an invisible background is no background).
    fn default() -> Self {
        Color::rgb(0, 0, 0)
    }
}

impl Color {
    /// Opaque color from RGB components.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 0xFF }
    }

    /// Converts to `0xRRGGBB` (handy for logs and the CLI).
    pub const fn as_rgb_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Interprets a `0xRRGGBB` integer as an opaque color.
    pub const fn from_rgb_u32(v: u32) -> Self {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

/// Reserved: wallpaper types supported by the manifest.
///
/// Only `shader` and `image` exist in the near-term roadmap;
/// `video` and `web` are reserved without implementation (see DECISIONS.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperType {
    /// Static image (Phase 2).
    Image,
    /// WGSL fragment shader (Phases 2-3).
    Shader,
    /// Video — reserved, not implemented.
    Video,
    /// Web (HTML/CSS/JS) — reserved, not implemented.
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
            "invalid manifest: missing title"
        );
    }

    #[test]
    fn color_rgb_converts() {
        let c = Color::rgb(0x2E, 0x34, 0x40);
        assert_eq!(c.as_rgb_u32(), 0x2E3440);
        assert_eq!(Color::from_rgb_u32(0x3B4252), Color::rgb(0x3B, 0x42, 0x52));
        assert_eq!(Color::default(), Color::rgb(0, 0, 0));
    }
}
