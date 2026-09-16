//! # bruma-runtime
//!
//! Contrato de runtime: la API que un wallpaper puede consumir
//! (tiempo, delta, resolución, mouse, parámetros, audio opcional).
//!
//! Estado: **Fase 3 (pendiente)**. Se define ahora como trait para congelar
//! el contrato cuanto antes; la implementación llegará con renderer-wgpu.
//!
//! FUTURO: trait `WallpaperRuntime` + uniform buffers + hot-reload WGSL
//! + límite de FPS. La demo de la fase: un shader animado editado en vivo.

#![forbid(unsafe_code)]
