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

### Fase 2 — Renderizado con wgpu (3-6 semanas) ✅
- Triángulo → quad → textura → imagen PNG/JPEG a pantalla completa.
- Crates: `bruma-renderer` (contrato) + `bruma-renderer-wgpu` (impl).
- Demo: imagen estática como wallpaper ✅ (`bruma run --image IMG`;
  evidencia en `demos/fase2/`).
- Criterio: FPS estable (n/a en estático), CPU casi idle ✅ (medido:
  idle total, renderiza solo en configure), memoria contenida ✅
  (~137 MB con driver Vulkan).

### Fase 3 — Contrato de runtime + hot-reload ✅ (2026-09-16)
- Trait `WallpaperRuntime`: tiempo, delta, resolución, mouse,
  parámetros planos (`[f32; 4]` → `u_params0..3`; los nombres llegarán
  con el manifiesto de la Fase 4). Audio opcional: pendiente (off por
  defecto, se evaluará con el primer wallpaper que lo necesite).
- ✅ Uniform buffers (48 bytes), hot-reload de WGSL (mtime + validación
  con naga; shader roto = se conserva el pipeline anterior), límite de
  FPS configurable (`--fps`, por defecto 30).
- ✅ Demo verificada en niri: animación por píxeles entre capturas,
  hot-reload en vivo (vino → verde → shader roto rechazado →
  recuperado), CPU 4% a 30 fps en 2560x1440, sobrevive a recarga de
  config de niri.
- Extra (D11): shader rechazado/recuperado avisa por notificación de
  escritorio vía D-Bus (verificado con quickshell/DMS en la demo;
  burbuja visible en captura y mensajes Notify capturados en el bus).

### Fase 4 — Formato .wallpaper (2-4 semanas) — ✅
- ✅ ZIP con `wallpaper.json`, `preview.png`, shaders, assets (crate `zip`
  8.6, features mínimas; `serde` para el manifiesto con `deny_unknown_fields`).
- ✅ Schema: `format/type/title/entry/preview/permissions/min_engine` +
  `version` del paquete (para el layout versionado de instalación) y
  `params` con nombre/label/default (los `u_params0..3` de la Fase 3
  ganan identidad). `type: video|web` reservados, no implementados.
- ✅ Validación: anti path-traversal (`enclosed_name`), rechazo de
  symlinks, límites de tamaño (paquete 50 MB, archivo 20 MB, descomprimido
  96 MB), entrada única e instalación con staging + rename atómico.
- ✅ Comandos: `bruma validate`, `install`, `list`, `pack` y
  `run --package nombre[:version]` con `--param=nombre=valor` resuelto
  contra el manifiesto (los params toman el default; los CLI los
  sobreescriben; nombre desconocido = error).
- ✅ Demo verificada en niri: pack → validate → install → list → run
  (animación viva por píxeles, `--param=intensidad=0.9` cambia el fondo,
  nombre inexistente rechazado con error).
- ✅ Seguridad demostrada: paquetes maliciosos (path traversal
  `../../.bashrc`, symlink, zip bomb 40×4 MB) rechazados en `validate`
  y en `install` con mensaje preciso; home intacto.
- Desviación menor del schema original: campo `version` añadido (es lo
  que ordena el directorio de instalación `~/.local/share/bruma/
  wallpapers/NOMBRE/VERSION`).

### Fase 5 — Escritorio completo (4-8 semanas) — ✅ (2026-09-17)
- ✅ Multi-monitor: UNA superficie layer-shell por salida (con output
  concreto en `create_layer_surface`), creada en hotplug (`new_output`),
  destruida al retirar la salida (`output_destroyed`/`closed`).
- ✅ GPU compartida (`GpuShared`, device/queue clonables) + `SurfaceCtx`
  por salida (formato de swapchain elegido por salida); factory de
  renderers inyectada desde la CLI (la plataforma no conoce GPU).
- ✅ Demo verificada en niri: eDP-1 1920x1200 + HDMI-A-1 2560x1440 con
  el mismo shader animado (AMD 660M por Vulkan), animación por píxeles
  en ambas, supervivencia a recarga de config y ciclo DPMS.
- ✅ FIX de bug latente (desde Fase 3): tras dibujar, el bucle esperaba
  eventos indefinidamente — la animación avanzaba solo con eventos del
  compositor (enmascarado por el tráfico del escritorio). Ahora el
  deadline SIEMPRE acota el poll; CPU medida con escritorio quieto:
  ~5.3% de un núcleo por dos salidas a 30 fps (~2.6% por salida).
- ✅ Pausa por salida en fullscreen (D12): bind opcional de
  `wlr-foreign-toplevel-management`; una ventana fullscreen sobre una
  salida congela SOLO su fondo (el último buffer queda en pantalla a
  cargo del compositor; el tiempo global no se pausa → sin salto al
  volver). Sin protocolo, degrada a "nunca pausar". Medido en niri:
  43→~4 ticks CPU/8-10s al pausar (por salida), ~1 tick con ambas
  pausadas, reanudación sin salto. `maximized` NO pausa (política D12).
- ✅ Pausa global: bloqueo de sesión (logind `Lock`/`Unlock` +
  `LockedHint` de respaldo) y batería (UPower `State==2`), vía D-Bus de
  sistema, best-effort (sin bus → nunca pausa). Sesión gráfica
  localizada con `ListSessions`+`Type=wayland` (GetSessionByPID
  inservible: compositors corren como servicios de usuario) y paths
  escapados (`_34`) usados tal cual. Medido: 35→1-2 ticks CPU/8s con la
  máquina en batería; lock/unlock verificado con transiciones en el
  log. Drenaje de señales sin hilos: `process(ZERO)` acotado por frame.
- ✅ Wallpaper distinto por pantalla + config persistente: config JSON
  estricta (`~/.config/bruma/config.json`, deny_unknown_fields, params
  validados) con `default` + entradas por nombre de salida. Factory
  extendida (`FactoryRenderer`: renderer y/o color por salida). Params
  POR SALIDA resueltos por nombre contra el manifiesto y aplicados en
  cada renderer (un solo runtime → animación sincronizada). CLI:
  `bruma config init|show`, `bruma run` sin flags carga la config.
- ✅ Servicio de usuario systemd (`bruma service install|remove`):
  unidad generada con el exe real, `Restart=always` (verificado: SIGTERM
  → revive en 2 s), arranca con `graphical-session.target`.
- ✅ DPI/escala por salida: `set_buffer_scale` con la escala anunciada
  por el compositor; los buffers (SHM y wgpu) se dibujan en píxeles
  físicos (lógico × escala), nítidos en HiDPI. Cambio de escala en
  caliente (`scale_factor_changed`) verificado en niri: eDP-1
  1920x1200 scale 1→2→1, animación continua y quad cubriendo las 4
  esquinas del buffer nuevo (CPU 8 ticks/5s en ambas escalas).
- ✅ Hotplug de salidas verificado en niri con IPC (`niri msg output
  X off/on`): superficie destruida al apagar ("restantes: 1") y nueva
  superficie con la fuente de su config al reconectar; la pausa
  fullscreen (D12) sigue correctamente a la ventana que migra de
  salida (CPU: 0 ticks con la única salida en fullscreen, 7 ticks al
  reconectar). SIGHUP tras hotplug estable.
- Límites conocidos (no bloquean el hito): escala fraccional
  (`wp_fractional_scale` — niri redondea a entero, el fondo queda
  ligeramente sobre-escaneado), posición real del cursor para
  `u_mouse` (los wallpapers reactivos lo pedirán; se evaluará con el
  primer caso real), FPS de config solo aplicado al reiniciar.
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
