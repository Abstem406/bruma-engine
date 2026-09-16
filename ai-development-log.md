# Registro de desarrollo con IA — bruma

> Bitácora del trabajo con asistentes de IA (modelo de trabajo:
> IA-implementa / humano-revisa demos — ver D8 en DECISIONS.md).
> Una entrada por sesión de trabajo. Reglas: una tarea = una demo,
> commits pequeños, nunca aceptar código que no corra.

---

## 2026-09-16 — Fase 0: cimientos del proyecto

- **Sesión:** arranque del proyecto en Freebuff (CLI), tras destilar las
  decisiones de la conversación de chat en `PLAN.md` y `DECISIONS.md`.
- **Hecho:**
  - Workspace Cargo con 6 crates: `bruma-core`, `bruma-package`,
    `bruma-runtime`, `bruma-renderer`, `bruma-platform`, CLI `bruma`.
  - `bruma-core` con tests iniciales (versión del motor, errores).
  - README, PLAN, DECISIONS, licencias MIT+Apache-2.0, CI en GitHub Actions.
  - Marcadores de crates futuros documentando su frontera de dependencias.
- **Demo:** `cargo test` y `cargo run -p bruma` en verde (ver CI).
- **Decisiones tomadas hoy:** D9 (nombre bruma), D6 aplicado desde el
  primer commit, D4 reflejado en docs (niri banco de pruebas).
- **Siguiente paso:** Fase 1 — ventana de fondo en niri con
  smithay-client-toolkit; demo de color sólido detrás de todo.

---

## 2026-09-16 — Fase 1: ventana de fondo Wayland

- **Sesión:** continuación en Freebuff, implementación completa de la
  Fase 1 en la sesión real de niri del autor (WAYLAND_DISPLAY disponible).
- **Hecho:**
  - `bruma-platform`: `BackgroundWindow` — capa `Background` de
    wlr-layer-shell, anclada a 4 bordes, sin foco de teclado, buffer shm
    ARGB8888 con `SlotPool`, redibujo en cada configure (recargas de
    config y cambios de resolución).
  - `bruma-core`: tipo `Color` (RGBA, conversiones 0xRRGGBB) con tests.
  - CLI `bruma run [color]` con logger mínimo propio (env_logger
    descartado: arrastra regex/27 crates; se elegirá facade cuando haga
    falta, según D8).
  - Descubrimiento de API: sctk 0.21 usa el modelo dispatch2 (`
    delegate_registry!` + `delegate_dispatch2!`); verificado contra
    fuente antes de escribir código.
- **Demo:** `bruma run 0x1D2021` en vivo sobre niri: superficie en capa
  Background de HDMI-A-1 (2560x1440), color exacto verificado por
  histograma de `grim`, sobrevive a `niri msg action load-config-file`
  con el proceso intacto y el fondo re-dibujado. Evidencia en
  `demos/fase1/` (logs + capturas).
- **Verificación:** fmt/clippy/tests/WASM-check en verde (mismos gates
  que la CI).
- **Notas:** niri mapea la superficie sin output en una sola salida
  (multi-monitor por pantalla: Fase 5, como estaba previsto). El proceso
  lanzado desde el runner de terminal muere al cerrar la terminal: para
  uso real, `setsid` o generar como servicio.
- **Siguiente paso:** Fase 2 — renderer con wgpu: triángulo → quad →
  imagen a pantalla completa; nuevo crate `bruma-renderer-wgpu`.

---

## 2026-09-16 — Fase 2 (paso 1): renderer wgpu, triángulo en el fondo

- **Sesión:** continuación en Freebuff, misma sesión real de niri.
- **Hecho:**
  - Nuevo crate `bruma-renderer-wgpu` (wgpu 30.0.1, features mínimas:
    wgsl+vulkan+gles, sin winit). El único `unsafe` del motor está ahí:
    crear la `wgpu::Surface` desde punteros crudos wl_display/wl_surface
    vía raw-window-handle 0.6.
  - Contrato `FrameRenderer` en `bruma-renderer` (cero dependencias, D6);
    `bruma-platform` lo consume con fallback al color sólido de Fase 1.
  - `bruma-platform` expone `display_ptr()`/`surface_ptr()`; en libwayland
    `wl_display` es un `wl_proxy`, cast verificado contra fuente de
    wayland-backend sys.
  - CLI: `bruma run --gpu` compone plataforma+renderer; el renderer se
    instala ANTES del primer configure para pintar el primer frame.
  - Pipeline WGSL propio con `include_str!`; shader "full-screen
    triangle" sin buffers de vértices.
- **Demo:** `bruma run --gpu` en niri: adaptador AMD Radeon 660M (RADV),
  backend Vulkan, 2560x1440. Gradiente del triángulo verificado por
  muestreo de píxeles (grim+ImageMagick) y sobrevive a recarga de
  config de niri con píxeles idénticos pre/post. Evidencia en
  `demos/fase2/`.
- **Verificación:** fmt/clippy/tests/WASM en verde.
- **Notas de API wgpu 30 (drift fuerte vs 0.20):** `request_device`
  devuelve tupla; `CurrentSurfaceTexture` enum; `present` se movió a
  `Queue::present(frame)`; `InstanceDescriptor` sin Default;
  `Trace::Off`; campos nuevos (`experimental_features`, `color_space`,
  `depth_slice`, `multiview_mask`, `immediate_size`). Toda firma
  verificada contra fuente antes de usar.
- **Pendiente de la fase:** quad → textura → imagen PNG/JPEG a pantalla
  completa (hito de la fase). Criterio de FPS/CPU aún por medir.
- **Siguiente paso:** paso 2 de Fase 2 — quad y carga de imagen a
  textura; `bruma run --image foto.png`.
