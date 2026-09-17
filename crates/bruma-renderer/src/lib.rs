//! # bruma-renderer
//!
//! Rendering contract of the bruma engine, with no concrete implementation.
//!
//! Status: **Phase 3**. Only the contract lives here (`FrameRenderer`); the
//! wgpu implementation is in `bruma-renderer-wgpu` (a separate crate) so
//! this contract carries no graphics dependencies.
//!
//! The platform asks [`FrameRenderer::wants_animation`] once: if it is
//! `false` (solid color, image) it paints only on every re-configure; if
//! it is `true` (animated shader) the platform drives the loop: it queries
//! the pace from [`WallpaperRuntime::target_fps`], sleeps in `poll` and
//! calls [`FrameRenderer::render_animated`] on every frame.

#![forbid(unsafe_code)]

/// State of a frame consumed by the renderer (time, resolution, mouse,
/// parameters). Re-exported from the pure `bruma-runtime` contract so
/// platform and renderers share the same type.
pub use bruma_runtime::{FrameDecision, FrameState, WallpaperRuntime};

/// Something able to paint a wallpaper frame onto the surface the platform
/// assigned to it.
///
/// The contract is deliberately minimal: the platform tells it the frame
/// size and the renderer decides how to paint it (color, triangle, image,
/// shader...).
pub trait FrameRenderer {
    /// Paints a frame at the given size (in buffer pixels).
    ///
    /// Called on the first configure and on every re-configuration of the
    /// surface (compositor config reload, resolution change...). Static
    /// content only needs a repaint here.
    fn render_frame(&mut self, width: u32, height: u32);

    /// Does this renderer produce animated content?
    ///
    /// `false` (solid color, image): the platform repaints only on every
    /// configure and the CPU stays idle between events. `true` (animated
    /// shader): the platform drives the frame loop via
    /// [`Self::render_animated`].
    ///
    /// Queried once, at startup: a renderer's mode does not change at
    /// runtime.
    fn wants_animation(&self) -> bool {
        false
    }

    /// Paints the frame for the given `state` (only if
    /// [`Self::wants_animation`] is `true`).
    ///
    /// The state (time, delta, mouse, parameters) is carried by the
    /// [`WallpaperRuntime`]; the renderer is stateless with respect to
    /// time — so the same shader behaves identically on the native runtime
    /// and the future WASM runtime of Phase 7.
    fn render_animated(&mut self, _state: &FrameState) {
        // Default: no animation. Static renderers never use it.
    }
}
