//! # bruma-renderer
//!
//! Contrato de renderizado del motor bruma, sin implementación concreta.
//!
//! Estado: **Fase 2**. Aquí vive solo el contrato (`FrameRenderer`); la
//! implementación con wgpu está en `bruma-renderer-wgpu` (crate aparte)
//! para que este contrato no arrastre dependencias gráficas.
//!
//! FUTURO: demos progresivas — triángulo → quad → textura → imagen
//! PNG/JPEG a pantalla completa. WGSL será el único lenguaje de shaders
//! (idéntico en escritorio y navegador vía WebGPU, ver DECISIONS.md D3).

#![forbid(unsafe_code)]

/// Objeto capaz de pintar un frame del wallpaper en la superficie que la
/// plataforma le haya asignado.
///
/// El contrato es deliberadamente mínimo: la plataforma le dice el tamaño
/// del frame y el renderer decide cómo pintarlo (color, triángulo, imagen,
/// shader...). Cuando el wallpaper sea animado, el renderer pedirá frames
/// por su cuenta; la plataforma solo reparte eventos.
pub trait FrameRenderer {
    /// Pinta un frame al tamaño indicado (en píxeles de buffer).
    ///
    /// Se llama en el primer configure y en cada re-configuración de la
    /// superficie (recarga de config del compositor, cambio de
    /// resolución...). Con contenido estático basta con repintar aquí;
    /// con contenido animado (Fase 3) el renderer solicitará frame
    /// callbacks por su cuenta.
    fn render_frame(&mut self, width: u32, height: u32);
}
