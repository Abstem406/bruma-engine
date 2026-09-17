//! # bruma-renderer
//!
//! Contrato de renderizado del motor bruma, sin implementación concreta.
//!
//! Estado: **Fase 3**. Aquí vive solo el contrato (`FrameRenderer`); la
//! implementación con wgpu vive en `bruma-renderer-wgpu` (crate aparte)
//! para que este contrato no arrastre dependencias gráficas.
//!
//! La plataforma pregunta [`FrameRenderer::wants_animation`] una vez: si
//! `false` (color sólido, imagen) pinta solo en cada re-configure; si
//! `true` (shader animado) la plataforma conduce el bucle: pide el
//! ritmo a [`WallpaperRuntime::target_fps`], duerme en `poll` y llama a
//! [`FrameRenderer::render_animated`] en cada frame.

#![forbid(unsafe_code)]

/// Estado de un frame que consume el renderer (tiempo, resolución,
/// mouse, parámetros). Re-exportado del contrato puro de
/// `bruma-runtime` para que plataforma y renderers compartan el mismo
/// tipo.
pub use bruma_runtime::{FrameDecision, FrameState, WallpaperRuntime};

/// Algo capaz de pintar un frame de wallpaper sobre la superficie que
/// la plataforma le asignó.
///
/// El contrato es deliberadamente mínimo: la plataforma le dice el
/// tamaño del frame y el renderer decide cómo pintarlo (color,
/// triángulo, imagen, shader...).
pub trait FrameRenderer {
    /// Pinta un frame al tamaño dado (píxeles de buffer).
    ///
    /// Se llama en el primer configure y en cada re-configure de la
    /// superficie (recarga de config del compositor, cambio de
    /// resolución...). Con contenido estático, repintar aquí basta.
    fn render_frame(&mut self, width: u32, height: u32);

    /// ¿Este renderer produce contenido animado?
    ///
    /// `false` (color sólido, imagen): la plataforma repinta solo en
    /// cada configure y la CPU queda idle entre eventos. `true` (shader
    /// animado): la plataforma conduce el bucle de frames vía
    /// [`Self::render_animated`].
    ///
    /// Se consulta una vez, al arrancar: el modo de un renderer nunca
    /// cambia en caliente.
    fn wants_animation(&self) -> bool {
        false
    }

    /// Pinta el frame correspondiente al `state` dado (solo si
    /// [`Self::wants_animation`] es `true`).
    ///
    /// El estado (tiempo, delta, mouse, parámetros) lo lleva el
    /// [`WallpaperRuntime`]; el renderer no guarda estado respecto del
    /// tiempo — así el mismo shader funciona idéntico con el runtime
    /// nativo y el futuro runtime WASM de la Fase 7.
    fn render_animated(&mut self, _state: &FrameState) {
        // Por defecto: sin animación. Los renderers estáticos no lo usan.
    }
}
