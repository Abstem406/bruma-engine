# PLAN — Motor de Wallpapers Libre para Linux ("bruma")

> Roadmap operativo del proyecto. Las decisiones de diseño viven en
> [DECISIONS.md](DECISIONS.md); el registro de trabajo con IA en
> [ai-development-log.md](ai-development-log.md).

## Principios de trabajo con IA

1. **Una tarea = una demo ejecutable.** Nunca dar una fase por terminada
   sin captura, vídeo o log que lo demuestre.
2. **Commits pequeños** y verificables: cada uno compila y pasa tests.
3. **Nunca aceptar código que no se pueda correr.** Antes la duda:
   reproducir o descartar.
4. **Auditar dependencias** al añadir cualquiera (`cargo audit`,
   mirar mantenimiento y peso).
5. **Guardar evidencia** (capturas/logs) junto a la demo en cada fase.

## Hito definitorio

> "Un extraño descarga un archivo `.wallpaper`, lo arrastra y ve un fondo
> animado." — el proyecto no está hecho hasta que esto ocurra.

## Fases

### Fase 0 — Cimientos (1-2 semanas) ✅
- Repo Git, workspace Cargo con crates: `core`, `package`, `runtime`,
  `renderer`, `platform`, CLI `bruma`.
- Licencias MIT + Apache-2.0, CI (GitHub Actions), `.gitignore`.
- `PLAN.md`, `DECISIONS.md`, `ai-development-log.md`.
- Demo: `cargo run -p bruma` compila y saluda; tests del core en verde.

### Fase 1 — Ventana de fondo en Wayland (2-5 semanas) ✅
- `wlr-layer-shell` vía `smithay-client-toolkit` (NO winit).
- Banco de pruebas de referencia: **niri** (+ DankMaterialShell).
- Sin wgpu todavía: buffer de color sólido.
- Demo: un rectángulo de color detrás de todas las ventanas.
  (`bruma run [color]`; evidencia en `demos/fase1/`).
- Criterio de aceptación: sobrevive a recarga de config de niri ✅
  (verificado) y a desconexión/reconexión de salida (pendiente de
  verificar con hardware real; la re-configuración de superficie ya
  está cubierta).

### Fase 2 — Renderizado con wgpu (3-6 semanas)
- Triángulo → quad → textura → imagen PNG/JPEG a pantalla completa.
- Crates: `bruma-renderer` (contrato) + `bruma-renderer-wgpu` (impl).
- Demo: imagen estática como wallpaper.
- Criterio: FPS estable, CPU casi idle, memoria contenida.

### Fase 3 — Contrato de runtime + hot-reload (3-6 semanas)
- Trait `WallpaperRuntime`: tiempo, delta, resolución, mouse,
  parámetros del manifiesto, audio opcional (off por defecto).
- Uniform buffers, hot-reload de WGSL, límite de FPS configurable.
- Demo: shader animado que se edita en vivo sin reiniciar.

### Fase 4 — Formato .wallpaper (2-4 semanas)
- ZIP con `wallpaper.json`, `preview.png`, shaders, assets.
- Schema: `format/type/title/entry/preview/permissions/min_engine`;
  `type: video|web` reservados, no implementados.
- Validación: anti path-traversal, rechazo de symlinks, límites de tamaño.
- Comandos: `bruma validate`, `bruma install`.
- Demo: instalar un paquete de ejemplo y verlo como fondo.

### Fase 5 — Escritorio completo (4-8 semanas)
- Multi-monitor con wallpaper por pantalla, DPI por salida.
- Config persistente; pausa en fullscreen/bloqueo/batería.
- Compatibilidad verificada: niri (referencia), Hyprland, sway, KWin.
- GNOME/Mutter sigue siendo non-goal en v1.

### Fase 6 — Herramientas para creadores (3-6 semanas)
- Plantillas WGSL: gradiente, partículas, ondas, reacción al mouse.
- Parámetros declarados en el manifiesto con UI generada.
- Comandos: `bruma new`, `bruma pack`.
- Demo/hito: una persona que no programa crea, empaqueta y comparte.

### Fase 7 — Presencia pública (3-6 semanas)
- Releases en GitHub Releases (binarios por target).
- Docs en GitHub Pages; galería estática con descarga directa.
- Previews vivos: el mismo WGSL corre en el navegador vía
  WASM/WebGPU (reutiliza `core`/`runtime` compilados a WASM).
- Decisión diferida: Steam (con SDK aislado del core, `steamworks-rs`).

## Non-goals de la v1

- GNOME/Mutter (sin wlr-layer-shell; se reevaluará con extensiones o el
  futuro soporte de layer-shell en Mutter).
- Vídeo (mpv/GStreamer) y audio (PipeWire): fases futuras, no v1.
- Compatibilidad con assets de Wallpaper Engine (`.pkg`): el proyecto es
  una reimaginación con formato propio, no un emulador.
- Widgets web de escritorio: experimento futuro detrás del `type: web`
  reservado, no v1.

## Expectativas temporales

- Prototipo visible: semanas-meses.
- Herramienta personal fiable: 6-18 meses.
- Ecosistema con comunidad: años.
