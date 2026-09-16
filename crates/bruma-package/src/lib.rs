//! # bruma-package
//!
//! Formato abierto `.wallpaper` (D5): un ZIP con `wallpaper.json`,
//! `preview.png`, shaders y assets.
//!
//! Estado: **Fase 4**. Este crate implementa el schema del manifiesto
//! (`format/type/title/entry/preview/permissions/min_engine`), la
//! validación de seguridad (anti path-traversal, rechazo de symlinks,
//! límites de tamaño) y las operaciones `pack` / `validate` / `install`.
//!
//! Frontera de dependencias (D6): nunca dependerá de wgpu, Wayland ni
//! Steam. Solo formato: zip + serde_json.

#![forbid(unsafe_code)]

mod error;
mod manifest;
mod store;

pub use error::PackError;
pub use manifest::{Manifest, Param, SCHEMA_VERSION};
pub use store::Store;
