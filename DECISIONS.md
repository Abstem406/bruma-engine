# DECISIONS — bruma

> Registro de decisiones de diseño, con su porqué. Orden: primero las
> fundamentales. Si una decisión cambia, se añade una nueva entrada al
> final y se marca la antigua como superada (nunca se borra la historia).

## D1 — Proyecto, no producto
Es un proyecto personal abierto que otros podrán usar; no un producto con
usuarios a los que servir. Esto libera de promesas de soporte y ritmo de
release. **Consecuencia:** no publicar anuncios hasta tener releases
mantenibles.

## D2 — Rust nativo primero, web después
La implementación es nativa (Rust). El experimento web (widgets HTML/CSS/JS)
queda **diferido**: los webviews en Linux (WebKitGTK/CEF) son pesados para
24/7 y diluyen la ventaja de rendimiento, que es el motivo de existir del
proyecto. **Decisiones baratas tomadas hoy para no reescribir:** (a)
`type: web` reservado en el manifiesto, (b) núcleo separado del renderizador,
(c) API de runtime como trait, (d) WGSL ya corre en navegador vía WebGPU.

## D3 — wgpu + WGSL, nada de OpenGL
Render con **wgpu** (Vulkan/Metal/DX12/GL fallback) y shaders en **WGSL**
exclusivamente. Motivos: WGSL es idéntico en escritorio y navegador (la
galería web reutiliza los shaders tal cual), wgpu es el estándar del
ecosistema Rust y evita el OpenGL heredado. Nota: Wallpaper Engine original
usa DirectX 10/11 (no OpenGL, que es solo su fallback); linux-wallpaperengine
emula con OpenGL, pero este proyecto NO emula — es una reimaginación.

## D4 — Solo Wayland; niri primero
v1 solo Wayland vía `wlr-layer-shell` con **smithay-client-toolkit**
(winit no soporta layer-shell; swww usa sctk por eso). **niri es el banco
de pruebas de referencia** (el entorno del autor, con DankMaterialShell);
Hyprland/sway/KWin best-effort. **GNOME/Mutter non-goal en v1** (no soporta
layer-shell) — se documentará como requisito para evitar issues.

## D5 — Formato abierto .wallpaper, sin marca
El paquete se llama `.wallpaper` (zip con `wallpaper.json`, `preview.png`,
shaders, assets) — nombre genérico de ecosistema, como `.lively`. El nombre
de marca (bruma) queda para el motor. Precedente: Lively demostró que el
contenido comunitario no necesita Steam Workshop (drag-and-drop, foros).

## D6 — Frontera de dependencias del core
`bruma-core`, `bruma-package` y `bruma-runtime` **nunca** dependerán de
wgpu, Wayland ni Steam. Motivos: (1) compilar a WASM para la galería,
(2) poder testear sin GPU ni compositor, (3) mantener el SDK de Steam fuera
del FOSS si algún día hay versión Steam (`steamworks-rs` en crate opcional).
En Fase 0 los crates `renderer` y `platform` también nacen sin dependencias
(contratos vacíos); sus implementaciones pesadas irán en crates separados.

## D7 — GitHub primero, Steam quizá después
Distribución inicial solo GitHub. Steam (tarifa ~$100/app, SDK propietario)
es un acelerador posiblemente futuro, no un objetivo. Si llega: mismo
formato `.wallpaper` en ambos canales para no dividir la comunidad.

## D8 — Modelo de trabajo IA-implementa / humano-revisa
El autor no revisa línea a línea: revisa demos. Reglas: una tarea = una
demo, commits pequeños, nunca aceptar código que no corra, guardar
capturas/logs, auditar dependencias. El detalle de cada sesión queda en
[ai-development-log.md](ai-development-log.md).

## D9 — Nombre: bruma
- Repo/proyecto: **bruma-engine** (libre en crates.io y GitHub).
- Binario/CLI: **bruma** (`bruma run`, `bruma install`, `bruma new`).
- Crates: `bruma-core`, `bruma-package`, `bruma-runtime`,
  `bruma-renderer(-wgpu)`, `bruma-platform`.
- Formato: `.wallpaper` (sin marca, ver D5).
- Tagline: "Motor libre de wallpapers animados para Wayland".
- Evoca: niebla/atmósfera — encaja con fondos ambientados con shaders.

## D10 — Vídeo y audio fuera de la v1
Vídeo (mpv/GStreamer) y captura de audio (PipeWire) arrastran dependencias
enormes y casos de uso distintos. Reservados en el manifiesto (`type: video`),
implementación futura. Audio como input de shaders: opcional y apagado por
defecto (privacidad) cuando llegue.

## D11 — Notificaciones de escritorio vía D-Bus, best-effort
- Qué: los avisos al usuario (shader rechazado/recuperado en hot-reload)
  van por `org.freedesktop.Notifications` (mako, dunst, quickshell/DMS...)
  con el crate `dbus` 0.9 (dlopen de libdbus: sin bindgen ni dep de build).
- Política: **best-effort estricto**. Sin bus de sesión, el canal se
  degrada a no-op; el envío tiene timeout de 300 ms; ningún error de
  notificación puede afectar al render ni al proceso. Un wallpaper no
  falla por su canal de avisos.
- Arquitectura: el renderer emite eventos tipados (`ReloadEvent`:
  Applied/Rejected/Recovered con dedup de autosaves idénticos); la CLI
  decide qué hacer con ellos (hoy: notificar con debounce de 2 s). Así
  la capa gráfica no conoce D-Bus y la futura UI (Fase 6) podrá
  consumir los mismos eventos.
- Urgencia "normal" siempre: los avisos de bruma nunca son críticos.
