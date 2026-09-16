//! # bruma-renderer
//!
//! Contrato de renderizado del motor bruma, sin implementación concreta.
//!
//! Estado: **Fase 2 (pendiente)**. La implementación con wgpu vivirá en
//! `bruma-renderer-wgpu` (crate aparte, aún sin crear) para que este
//! contrato no arrastre dependencias gráficas.
//!
//! FUTURO: trait `Renderer` con demos progresivas: triángulo → quad →
//! imagen PNG/JPEG a pantalla completa. WGSL será el único lenguaje de
//! shaders (idéntico en escritorio y navegador vía WebGPU).

#![forbid(unsafe_code)]
