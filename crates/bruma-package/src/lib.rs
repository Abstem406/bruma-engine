//! # bruma-package
//!
//! Formato abierto `.wallpaper`: un ZIP con `wallpaper.json`,
//! `preview.png`, shaders y assets.
//!
//! Estado: **Fase 4 (pendiente)**. Este crate se crea ahora como marcador
//! de la frontera de dependencias: nunca dependerá de wgpu, Wayland ni Steam.
//!
//! FUTURO: `validate`, `install`, `new`, `pack`; schema del manifiesto con
//! `format/type/title/entry/preview/permissions/min_engine`; validación
//! anti path-traversal y rechazo de symlinks dentro del ZIP.

#![forbid(unsafe_code)]
