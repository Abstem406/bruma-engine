//! # bruma-package
//!
//! The open `.wallpaper` format (D5): a ZIP with `wallpaper.json`,
//! `preview.png`, shaders and assets.
//!
//! Status: **Phase 4**. This crate implements the manifest schema
//! (`format/type/title/entry/preview/permissions/min_engine`), the
//! security validation (anti path-traversal, symlink rejection, size
//! limits) and the `pack` / `validate` / `install` operations.
//!
//! Dependency boundary (D6): it will never depend on wgpu, Wayland or
//! Steam. Format only: zip + serde_json.

#![forbid(unsafe_code)]

mod error;
mod manifest;
mod store;

pub use error::PackError;
pub use manifest::{Manifest, Param, SCHEMA_VERSION, TextureFit, TextureSpec};
pub use store::Store;
