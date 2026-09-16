//! # bruma-platform
//!
//! Capa de plataforma: ventana de fondo en Wayland vía `wlr-layer-shell`.
//!
//! Estado: **Fase 1 (pendiente)**. La implementación usará
//! `smithay-client-toolkit` (NO winit: no soporta layer-shell), igual que
//! swww. El banco de pruebas de referencia es **niri** (+ DankMaterialShell),
//! con el resto de compositors wlroots/KWin como best-effort.
//!
//! NON-GOALS (ver DECISIONS.md): GNOME/Mutter en v1 (sin layer-shell);
//! vídeo y audio en v1.
//!
//! FUTURO: demo "color sólido detrás de todo", multi-monitor por pantalla,
//! DPI, pausa en fullscreen/bloqueo/batería (Fase 5).

#![forbid(unsafe_code)]
