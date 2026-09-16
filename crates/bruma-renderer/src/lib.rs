//! # bruma-renderer
//!
//! Contrato de renderizado del motor bruma, sin implementación concreta.
//!
//! Estado: **Fase 3**. Aquí vive solo el contrato (`FrameRenderer`); la
//! implementación con wgpu está en `bruma-renderer-wgpu` (crate aparte)
//! para que este contrato no arrastre dependencias gráficas.
//!
//! La plataforma interroga [`FrameRenderer::wants_animation`] una vez: si
//! es `false` (color sólido, imagen) pinta solo en cada re-configuración;
//! si es `true` (shader animado) la plataforma conduce el bucle: consulta
//! el ritmo con [`WallpaperRuntime::target_fps`], duerme en `poll` y llama
//! a [`FrameRenderer::render_animated`] en cada frame.

#![forbid(unsafe_code)]

/// Estado de un frame que el renderizador consume (tiempo, resolución,
/// mouse, parámetros). Re-exportado del contrato puro de `bruma-runtime`
/// para que platform y renderers compartan el mismo tipo.
pub use bruma_runtime::{FrameDecision, FrameState, WallpaperRuntime};

/// Objeto capaz de pintar un frame del wallpaper en la superficie que la
/// plataforma le haya asignado.
///
/// El contrato es deliberadamente mínimo: la plataforma le dice el tamaño
/// del frame y el renderer decide cómo pintarlo (color, triángulo, imagen,
/// shader...).
pub trait FrameRenderer {
    /// Pinta un frame al tamaño indicado (en píxeles de buffer).
    ///
    /// Se llama en el primer configure y en cada re-configuración de la
    /// superficie (recarga de config del compositor, cambio de
    /// resolución...). Con contenido estático basta con repintar aquí.
    fn render_frame(&mut self, width: u32, height: u32);

    /// ¿Este renderer produce contenido animado?
    ///
    /// `false` (color sólido, imagen): la plataforma repinta solo en cada
    /// configure y la CPU queda idle entre eventos. `true` (shader
    /// animado): la plataforma conduce el bucle de frames vía
    /// [`Self::render_animated`].
    ///
    /// Se consulta una vez, al arrancar: el modo de un renderer no cambia
    /// en caliente.
    fn wants_animation(&self) -> bool {
        false
    }

    /// Pinta el frame correspondiente al `state` dado (solo si
    /// [`Self::wants_animation`] es `true`).
    ///
    /// El estado (tiempo, delta, mouse, parámetros) lo lleva el runtime
    /// [`WallpaperRuntime`]; el renderizador es stateless respecto al
    /// tiempo — así el mismo shader funciona idéntico en el runtime
    /// nativo y en el futuro runtime WASM de la Fase 7.
    fn render_animated(&mut self, _state: &FrameState) {
        // Default: sin animación. Los renderers estáticos no lo usan.
    }
}
