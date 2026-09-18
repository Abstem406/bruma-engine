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

---

## 2026-09-16 — Fase 2 (paso 2): imagen a pantalla completa — hito

- **Sesión:** continuación en Freebuff; el autor confirma el entorno
  híbrido: iGPU AMD 660M (conectores HDMI-A-1 + eDP-1) + dGPU NVIDIA
  (DP-*); salidas 2560x1440@144 (externa) y 1920x1200@165 (laptop).
- **Hecho:**
  - Refactor: `GpuContext` compartido (instancia, adaptador, device,
    cola, superficie) + renderers independientes. Elección de adaptador
    con `LowPower` deliberada para que la dGPU duerma; selección
    explícita diferida a Fase 5.
  - `ImageRenderer`: quad + textura RGBA8 sRGB subida con
    `Queue::write_texture` (una sola vez) + shader de muestreo. La
    imagen se ESTIRA (aspect ratio sin corregir: decisión de
    cover/contain diferida al manifiesto .wallpaper, Fase 4).
  - CLI: `bruma run --image IMG` (además de --gpu y color).
  - `image` 0.25 con features mínimas (png+jpeg, sin rayon).
- **Demo:** imagen rojo/azul de prueba: colores puros #FF2828 y
  #2850FF verificados por histograma de grim en pantalla (detrás del
  tinte translúcido del shell). Sobrevive a recarga de config de niri
  con el pipeline activo.
- **Eficiencia (criterio de la fase):** CPU 0 ticks en 5 s (idle total:
  renderiza solo en configure; el compositor retiene el buffer), RAM
  ~137 MB (driver Vulkan + wgpu). FPS: n/a por diseño en contenido
  estático; se medirá en Fase 3 con shaders animados.
- **Pendiente conocido:** multi-monitor — la superficie única solo
  cubre una salida; eDP-1 del autor queda sin wallpaper (Fase 5, caso
  urgente por ser su uso diario).
- **Siguiente paso:** Fase 3 — contrato `WallpaperRuntime`, uniforms,
  hot-reload WGSL, límite de FPS; primer shader animado.

## 2026-09-16 — Fase 3: runtime, shader animado y hot-reload — hito

- **Sesión:** continuación en Freebuff, con dos cortes de conexión a
  mitad de camino; el estado quedó coherente en ambos casos gracias al
  orden todo-verificado-antes-de-escribir.
- **Hecho:**
  - `bruma-runtime` (puro, D6): contrato `WallpaperRuntime`
    (`begin_frame` → `FrameDecision` Draw/Skip{deadline}, `target_fps`,
    `state()`, params) + `BasicRuntime` (reloj, límite de FPS, pausa que
    congela el tiempo, mouse/resolución, params). 6 tests.
  - `bruma-renderer`: `FrameRenderer` ampliado con `wants_animation()`
    y `render_animated(&FrameState)` (default impls: los renderers de
    las fases anteriores no cambian). Re-exports del runtime.
  - `bruma-renderer-wgpu`: `AnimatedRenderer` — uniform block de 48
    bytes (`u_time/u_params0/u_mouse/u_params/u_res`; el padding lo
    fija un `const assert!`), hot-reload por mtime con validación
    SÍNCRONA del WGSL usando `naga` directo (la misma naga de wgpu: el
    mensaje de error es tipado, no depende de features de wgpu). Shader
    roto → se conserva el pipeline anterior y el error queda en el log.
  - `bruma-platform`: `run_with_runtime()` — réplica del bucle interno
    de `blocking_dispatch` de wayland-client pero con `rustix::poll` y
    timeout: duerme hasta el deadline del runtime o hasta que el
    compositor mande algo. La resolución del `FrameState` la impone la
    superficie (uniforms siempre coherentes con el target).
  - CLI: `bruma run --shader F.wgsl [--fps N] [--param=VALOR]`.
  - naga 30.0.1 añadido (wgsl-in) — ya estaba en el árbol por wgpu:
    coste real de dependencias: cero.
- **Demo verificada en niri:** animación confirmada por píxeles (el
  fondo visible cambia entre capturas; antes, en Fase 2, era estático),
  hot-reload en vivo completo: vino → verde → shader roto (rechazado,
  wallpaper sigue vivo) → arreglado (recuperado). `--param=0.9` brilla
  más (verificado por píxel). CPU: **20 ticks en 5 s ≈ 4% de un núcleo**
  a 30 fps en 2560x1440 (vs 0% estático: el coste es el render de 75 MB
  de píxeles/s por GPU, la CPU solo duerme y sube uniforms). RAM
  ~141 MB. Sobrevive a `load-config-file` de niri con animación activa.
- **Notas de API verificadas contra fuente:** wgpu 30 `push_error_scope`
  devuelve guard; naga 30: `Validator::new(ValidationFlags, Capabilities)`,
  `Capabilities` en `naga::valid`; structs uniform en WGSL redondean su
  tamaño a múltiplo de 16 (40 → 48); `wayland-backend::Backend::poll_fd()`.
- **Pendiente:** mouse real (el cursor del compositor no llega a un
  layer-shell sin passthrough; se evaluará `--mouse` con la Fase 5 o
  quedará para wallpapers que lo pidan), pausa por visibilidad
  (salidas apagadas), audio (se activará con el primer caso real).
- **Siguiente paso:** Fase 4 — formato `.wallpaper` (manifiesto +
  zip): `bruma validate` / `install`, y ahí los params ganan nombres.

## 2026-09-16 — Fase 3, extra: aviso de shader rechazado como notificación de escritorio (D11)

- **Motivación:** en la sesión de hot-reload en vivo quedó claro que el
  aviso de "shader rechazado" solo iba al log: invisible si bruma corre
  como servicio sin terminal. El creador guardaría su shader roto y no
  sabría por qué no cambia nada.
- **Hecho:**
  - `bruma-platform/src/notify.rs`: `DesktopNotifier` sobre
    `org.freedesktop.Notifications` (crate `dbus` 0.9.12, dlopen de
    libdbus — presente en todo sistema con D-Bus; sin bindgen ni dep de
    build). Best-effort estricto: sin bus de sesión degrada a no-op,
    timeout de 300 ms, errores deglutidos con log de debug. 2 tests.
  - `AnimatedRenderer`: callback opcional con eventos tipados
    `ReloadEvent` (Applied / Rejected{error} / Recovered) y dedup de
    errores idénticos (los autosaves reescriben el mismo archivo roto).
    La capa gráfica no conoce D-Bus (frontera D6/D11).
  - CLI: convierte Rejected/Recovered en notificaciones (primera línea
    del error de naga, recorte a 140 chars, debounce de 2 s).
- **Demo verificada en niri (con quickshell/DMS como daemon):**
  dbus-monitor capturó los mensajes `Notify` con app_name "bruma" para
  "shader rechazado" (cuerpo: primera línea del error de naga recortada
  con …) y "shader recuperado"; la burbuja fue visible en pantalla
  (RMSE 0.46 entre captura con y sin popup en la región superior). Al
  arreglar el shader llegó la segunda burbuja. Procesos y escritorio
  restaurados al terminar.
- **Incidentes de sesión (honestos):** dos guardadas rotas mías al
  construir el edit de prueba (el wallpaper sobrevivió ambas, como debe
  ser); un `pkill -f dbus-monitor` que se atrapó a sí mismo vía el
  wrapper bash (usar `pkill -x`); y un alias de tipo colocado dentro del
  doc-comment del struct que clippy rechazó con razón.
- **Siguiente paso:** Fase 4 — formato `.wallpaper` (manifiesto + zip):
  `bruma validate` / `install`; los parámetros ganan nombres y la
  imagen gana cover/contain.

## 2026-09-16 — Fase 4: formato .wallpaper (manifiesto, seguridad, CLI completa)

**Objetivo del hito:** el corazón del proyecto — un formato de paquete
abierto y seguro, con herramientas para validar/instalar/ejecutar, y los
parámetros de la Fase 3 ganando nombres reales.

**Nuevos módulos en `bruma-package`:**
- `manifest.rs`: schema `wallpaper.json` con `deny_unknown_fields`
  (`format/type/title/entry/preview/permissions/min_engine/version/fps/
  params`). `Param { name, label?, default }` — 16 máx. Validación:
  claves sin espacios vacíos, defaults 0..=1, fps 1..=120, min_engine
  "x.y.z" numérico, entry/preview dentro del paquete, `video|web`
  reservados. `install_name()` deriva el slug desde el título.
- `store.rs`: `validate()` (todas las entradas escaneadas), `install()`
  con **staging + rename atómico** en
  `~/.local/share/bruma/wallpapers/NOMBRE/VERSION`, `installed()`,
  `pack()`. Seguridad: `enclosed_name` anti-traversal, symlinks
  rechazados, límites 50 MB zip / 20 MB archivo / 96 MB descomprimido,
  permisos 0o644 siempre.
- `error.rs`: `PackError` con `thiserror`.

**Runtime:** `ParamValue { name, value }` + `set_params()` en
`BasicRuntime` (por defecto: valores del manifiesto). Los 4 primeros
parámetros siguen llegando al shader como `u_params0..3`.

**CLI:** subcomandos `validate` (salida legible), `install`, `list`,
`pack` (con `-o`), y `run --package nombre[:version]` que resuelve el
manifiesto (min_engine, fps por defecto, entry) y mapea
`--param=nombre=valor` → posición. Corrección de bugs durante la fase:
`pack` no parseaba `-o` (creó un archivo llamado literalmente "-o"), y el
parseo de `--param` era ambiguo con `=nombre=valor` (separo prefijo
`--param=` antes de parsear; `--param` desnudo = error de uso).

**Demo verificada en niri** (`demos/fase4/`):
- Ciclo completo: pack → validate → install → list → run --package.
- Animación viva: píxel (5,5) cambia entre capturas (152,130,141 →
  187,151,159).
- Parámetros: `--param=intensidad=0.9` cambia el píxel; nombre
  inexistente → `error: el paquete no declara el parámetro 'x'`;
  `intensidad=1.5` se clampea a 1.0.
- Seguridad con paquetes maliciosos fabricados con python zipfile:
  - traversal (`../../.bashrc`): rechazado en validate E install;
    ~/.bashrc intacto.
  - symlink (`link.wgsl` → /etc/passwd): rechazado.
  - bomba (40 entradas × 4 MB): rechazada por límite descomprimido.
- Eficiencia: 18 ticks CPU / 5 s (~3.6% de un núcleo a 30 fps en
  2560×1440), ~135 MB RAM.

**Paquete demo:** `demos/fase4/paquetes/demo-src/` (manifiesto con param
`intensidad`, shader de la Fase 3, preview generada) →
`onda-bruma.wallpaper`.

## 2026-09-16 — Lección: un test envió notificaciones reales al escritorio

**Síntoma:** el usuario recibió dos burbujas ("shader rechazado — error de
prueba" y "shader recuperado") sin estar editando nada.

**Causa raíz:** el test `sin_bus_degrada_a_noop` de `notify.rs` llamaba a
`DesktopNotifier::new()` asumiendo que en los tests no hay bus de sesión.
En CI no lo hay; **en el escritorio del usuario sí** (quickshell/DMS).
`cargo test` de la verificación de la Fase 4 conectó de verdad y envió
dos `Notify` reales. Un test unitario no debe tener efectos en el
mundo físico del usuario.

**Corrección (3 piezas):**
1. `DesktopNotifier::disabled()` — camino no-op construible sin bus,
   para tests. Regla anotada en el doc: los tests jamás llaman a `new()`.
2. Construcción del mensaje extraída a `build_notify_message()` (pura,
   sin conexión): el spec del mensaje ahora se audita byte a byte en el
   test (`get_items()`), sin tocar D-Bus.
3. Test no-op reescrito sobre `disabled()` con aserciones de retorno
   (`assert!(!...)`) en vez de "que no paniquee y ya".

**Generalización para el resto del proyecto:** ninguna prueba debe
depender del entorno del usuario (bus, sesión, GPU activa, pantallas).
Todo efecto observable pasa por una frontera construible-en-disabled.

## 2026-09-16 — Bug de geometría: mitad de pantalla sin dibujar (desde Fase 2)

**Síntoma:** el usuario ve un corte diagonal exacto de esquina a esquina:
shader solo en la mitad superior-izquierda, negro en el resto.

**Causa raíz:** quads con `draw(0..4)` en topología TriangleList → un
solo triángulo (0,1,2); el vértice 3 se descarta. El comentario del
shader describía dos triángulos estilo indexado que nunca existió como
index buffer. El clear transparente dejaba ver el escritorio detrás.

**Por qué sobrevivió dos fases de "verificación":** todas las mediciones
usaban el píxel (5,5) — dentro del triángulo que sí funcionaba. El
histograma de la Fase 2 confirmó colores "asomando" sin verificar
cobertura completa. Lección registrada: verificar las 4 esquinas +
centro, nunca un punto de la región sabida-buena.

**Corrección:** `topology: TriangleStrip` en los 3 pipelines (los 4
vértices ya estaban en orden strip; `draw(0..3)` del demo no cambia).
Verificado: 4 esquinas con el shader, centro brillante con
intensidad=0.9, animación viva entre capturas.

## 2026-09-16 — Test de cobertura de quad (el test del test)

**Motivación:** el bug de la mitad negra sobrevivió dos fases porque la
verificación en vivo solo medía (5,5). Ahora existe
`crates/bruma-renderer-wgpu/tests/quad_coverage.rs`: render offline a
textura (sin Wayland, sin superficie), readback de píxeles y aserciones.

**Diseño — probar producción, no una copia:**
- `build_quad_pipeline()` extraída a función pura (device + formato +
  módulo): la usa el AnimatedRenderer y el test por igual.
- `compile_wgsl()` pública: el test valida con naga por el mismo camino.
- `hello.wgsl` via `include_str!`: el shader real del binario.
- El draw es idéntico (`draw(0..4)` sobre el mismo pipeline).

**Aserciones (la geometría como contrato):**
1. Cobertura: TL/TR/BL/BR/centro (8px hacia adentro) ≠ clear rojo.
2. Simetría: las 4 esquinas idénticas entre sí (la onda solo depende
   de r) — delata quad espejado o desplazado.
3. Contraste: centro ≠ esquinas (con t=0, w centro=0, w esquina≈0.73,
   determinista) — delata uniforms/uv sin conectar.

**Validación del propio test (mutación):** reintroduje TriangleList
temporalmente → el test FALLÓ en la aserción de cobertura; restaurado
→ pasa. Un test que nunca vio el bug no vale nada; este lo vio.

**Sin GPU (CI):** request_adapter sin superficie y skip con aviso —
mismo contrato best-effort del proyecto: el CI corre en runners sin
GPU; en dev corre de lleno.

## 2026-09-16 — Fase 5 (paso 1): multi-monitor, una superficie por salida

**Arquitectura (los tres movimientos):**
1. `bruma-renderer-wgpu`: `GpuContext` (device+surface acoplados) se
   divide en `GpuShared` (instance/adapter/device/queue, todos Clone —
   una sola GPU para N salidas) + `SurfaceCtx` (superficie por salida,
   formato de swapchain elegido por salida). Los tres renderers ganan
   `on_shared()`; `new_wayland()` queda como envoltorio de una salida.
2. `bruma-platform`: `BackgroundState` guarda un `Vec<OutputEntry>`
   (salida + superficie layer-shell + tamaño + renderer). `new_output`
   crea superficie con output concreto (la clave: sin output, la
   superficie solo cubre una pantalla); `output_destroyed`/`closed` la
   retiran; sin salidas, el fondo termina limpio. La creación de
   renderers es una **factory inyectada** (`set_renderer_factory`):
   la plataforma pasa punteros crudos y nombre de salida, no sabe nada
   de GPU (frontera D3/D6 intacta).
3. CLI: descubre la GPU una vez (`GpuShared::new()`), la factory
   construye el renderer por salida sobre ella; el callback de
   notificaciones de hot-reload se comparte entre salidas.

**Bug latente cazado por la medición honesta:** la CPU de la demo dio
1 tick/5s — demasiado bajo incluso para dos salidas. Causa: tras el
Draw, el bucle hacía `wait_and_dispatch(None)` — espera indefinida; el
avance de la animación dependía de eventos del compositor (un escritorio
en uso genera miles; uno idle, ninguno → congelación). Existía desde la
Fase 3. Fix: tras dibujar, un segundo `begin_frame` (que no toca el
tiempo) entrega el deadline del próximo tick y el poll SIEMPRE queda
acotado. CPU post-fix con escritorio quieto: 53 ticks/10 s ≈ 5.3% de un
núcleo por DOS salidas a 30 fps (~2.6% por salida; la Fase 3 medía 4%
con una).

**Demo verificada en niri** (`demos/fase5/`):
- Dos superficies: eDP-1 1920x1200 + HDMI-A-1 2560x1440, un solo
  adaptador (AMD 660M, "compartido entre salidas" en el log).
- Animación por píxeles en ambas salidas (TL cambia entre capturas;
  RMSE 0.069 en 0.4 s).
- Recarga de config de niri: proceso vivo y pintando.
- Ciclo DPMS completo (power-off/power-on): sobrevive, reconfigura y
  sigue animando (RMSE 0.023).
- RAM ~134 MB (una GPU compartida, no dos devices).

## 2026-09-16 — Pausa en fullscreen por salida (Fase 5, D12)

**Qué:** bind de `wlr-foreign-toplevel-management` (opcional): el
compositor anuncia cada ventana con sus estados y salidas; si una
ventana fullscreen está sobre una salida, su fondo deja de repintarse.
La pausa es por salida; el runtime NO se pausa (el tiempo global sigue:
al salir del fullscreen la animación retoma sin salto). Sin protocolo →
`ToplevelTracker::disabled()`, nunca pausa.

**Detalles de implementación:**
- `event_created_child!` es obligatorio en el Dispatch del manager: el
  evento `toplevel` crea un objeto hijo y wayland-client paniquea sin
  user-data declarado (segundo panic de este tipo; el patrón ya está en
  el log de la Fase 1 con otro protocolo).
- El enum `state` del protocolo: maximized=0, minimized=1, activated=2,
  fullscreen=3 — confirmado contra el XML, no contra memoria.
- Logging de transiciones: fullscreen de toplevels en info, decision
  por salida en info (solo cambia), `output_enter` en debug (ruido).

**Verificación empírica en niri (la que valió):** CPU por ticks
(/proc/PID/stat) + contador de transiciones en el log para garantizar
estado estable durante cada medición:
- Ambas animando (juego fullscreen en HDMI): 43 ticks/10s (solo eDP
  trabajaba — consistente con ~4.3%/salida de la Fase 5).
- Chat fullscreen en eDP → "salida eDP-1: pausa = true": 1 tick/8s.
- Quitar fullscreen → "pausa = false": 33 ticks/8s (reanudado).
RMSE de capturas resultó inútil aquí: el propio fullscreen y las
ventanas cambian la imagen; la CPU del proceso es la señal limpia.

**Falsas alarmes que descarté con datos:** (1) pensé que "ambas
pausadas" era bug de contagio entre salidas — el log mostró que kitty
TAMBIÉN estaba fullscreen (mi propio toggle de pruebas lo dejó así);
(2) pensé que el toggle al juego fallaba — el juego se re-aplica
fullscreen él solo al recibir foco (líneas true/false alternadas en el
log). Lección repetida: antes de cazar bugs, loguear la verdad del
sistema y leerla completa.

**Pendiente del criterio original:** bloqueo de sesión y batería.

## 2026-09-16 — Pausa global: bloqueo de sesión y batería (D12, parte 2)

**Qué:** `SessionPauseWatcher` (módulo `pause.rs`): D-Bus de sistema,
sin hilos. logind (señales `Lock`/`Unlock` + `LockedHint` vía
`PropertiesChanged` como segunda fuente) y UPower (`DisplayDevice.State
== 2`). El bucle llama `poll()` (drenaje `process(ZERO)` con cota de 32)
y consulta `paused()` cada frame; con pausa, ninguna salida repinta.

**Dos descubrimientos que solo el sistema real enseñaba:**
1. `GetSessionByPID` responde `NoSessionForPID` para cualquier PID: los
   compositors Wayland corren como servicios de usuario (systemd
   --user), fuera del alcance de sesión de logind. Camino correcto:
   `ListSessions` + filtrar `Type="wayland"`.
2. logind escapa los IDs en object paths: la sesión "4" vive en
   `/org/freedesktop/login1/session/_34` y las señales se emiten por el
   path ESCAPADO. Mi primer match por `/session/4` (sacado del método
   roto) nunca recibió nada. Cazado con dbus-monitor: las señales
   estaban ahí todo el tiempo, en otro path.

**Verificado en vivo:**
- Arranque en batería → pausa global ON desde el primer frame
  (estado inicial sincrónico, no espera primera señal): 35 → 1-2
  ticks/8s. El fondo de la demo quedó congelado un rato: era tu laptop
  sin cargador, no un bug.
- `loginctl lock-session` → doble transición registrada (Lock +
  LockedHint); unlock → desbloqueada por ambas fuentes.
- Degradación: sin bus de sistema o sin sesión wayland, la fuente
  desaparece con log y el motor nunca pausa (mismo contrato que D11).

**Tests:** 4 nuevos en pause.rs (semántica de flags, State==2, extracción
pura de Variant bool/u32/String, watcher disabled). Total workspace: 33.

**Nota de diseño:** el estado inicial se consulta sincrónico (Get) y las
señales solo avisan de cambios; suscribirse solo si el Get respondió (si
logind no habla, no insistimos).

## 2026-09-16 — Wallpaper por pantalla + config persistente + servicio (Fase 5)

**Qué:**
1. `FactoryRenderer` (plataforma): la factory puede devolver renderer,
   color por salida, o ambos — `draw_entry_solid` usa el color de la
   entrada si lo hay (antes el color era global único).
2. `AnimatedRenderer::set_param_overrides` (renderer-wgpu): overrides
   (posición, valor) aplicados sobre FrameState en cada frame. La CLI
   resuelve nombre→posición contra el manifiesto; el renderer no sabe
   nada de manifiestos. Un solo runtime → ambos monitores animan
   sincronizados aunque tengan params distintos.
3. `bruma/src/config.rs`: config JSON estricta (deny_unknown_fields,
   params 0..1 validados, color RRGGBB exacto — `#12345` parseaba como
   número y era un typo: ahora duele). Resolución: salida → default →
   nada. `bruma run` sin flags la carga sola; `--config RUTA` explícita
   (excluyente con flags de fuente); con flags, legacy intacto.
4. `bruma config init|show` y `bruma service install|remove`.

**Bug de la unidad systemd cazado por prueba de vida:** con
`Restart=on-failure`, un SIGTERM se registra `Result=success` y systemd
NO reviva el proceso. Un wallpaper "revive si muere" quiere
`Restart=always`. Verificado: kill -TERM → nuevo PID en 2 s.

**Demo en tu máquina:** config real con eDP-1 = onda intensidad 0.15,
HDMI-A-1 = color #1d2021 → log confirma `eDP-1 (gpu)` + `HDMI-A-1
(color)` y el píxel de HDMI mide exactamente srgb(29,32,33). El servicio
quedó instalado y activo: el fondo sobrevive al cierre de la terminal.

**Limitación honesta:** tu laptop está en batería, así que la pausa
global (D12) congela la animación — la verificación del shader con dos
params distintos por pantalla queda pendiente de tener cargador a mano
(el camino de overrides está testeado y el pipeline de color/animado
mixto quedó demostrado).

## 2026-09-17 — Decisión: sin autoarranque; bruma solo manual

- **Contexto:** la config actual usa color sólido en el monitor externo
  (HDMI-A-1) y aún no hay fondos definitivos, así que el fondo de
  momento solo se lanza cuando se usa de forma directa.
- **Hecho:** `systemctl --user disable --now bruma.service` → la unidad
  queda escrita pero `disabled` (sin enlace en
  `graphical-session.target.wants`): ya no arranca con la sesión.
  Reversible con `bruma service install` (rehabilita y arranca).
- **Uso manual:** `bruma run` (carga la config persistente igual que el
  servicio; para dejarlo vivo tras cerrar la terminal, el servicio o
  `setsid`).

## 2026-09-17 — Traducción completa del código al español

- **Contexto:** las fases 0–2 se escribieron con comentarios en
  español, pero desde la Fase 3 los crates puros
  (`bruma-core`, `bruma-runtime`, `bruma-renderer`, `bruma-package`)
  quedaron mayormente en inglés.
- **Hecho:** traducidos doc-comments, comentarios internos y
  **mensajes de error visibles al usuario** (todos los `#[error(...)]`
  de `PackError`, `BrumaError` y las validaciones del manifiesto).
- **Tests actualizados en consecuencia:** las aserciones que
  verificaban texto de errores (`contains("symlink")`,
  `contains("reserved")`, `contains("too large")`, `contains("unsafe")`,
  `contains("duplicate")`) ahora esperan el texto traducido. Los
  mensajes son contrato observable: el test del mensaje cambia con el
  mensaje.
- **Verificación:** fmt/clippy/tests del workspace en verde (40 tests).

## 2026-09-17 — bruma se independiza: proyecto propio en ~/Documents/DEV

- **Decisión:** bruma ya no vive dentro de `niri-tui-tools`; se mueve
  a `~/Documents/DEV/bruma-engine` como proyecto independiente (tiene
  su propio repo git, licencias, CI y roadmap — nunca fue parte real
  de las herramientas del compositor).
- **Movimiento:** `mv` simple, sin historial que reescribir: en el
  repo exterior `bruma-engine/` era untracked; el repo propio viaja
  intacto con todos sus cambios.
- **Rutas afectadas:** la unidad de usuario `bruma.service` apuntaba
  al `target/debug/bruma` bajo la ruta vieja → se reescribe `ExecStart`
  con la ruta nueva (la unidad sigue `disabled`: bruma sigue siendo
  de ejecución manual según la decisión de hoy).

## 2026-09-17 — Fix latente: rustix::runtime (API experimental) → libc::sigaction

- **Síntoma:** `cargo build` del workspace en verde pero
  `cargo install --path crates/bruma` falla con E0603 (`module
  runtime is private`).
- **Causa raíz:** `hup.rs` (canal SIGHUP→eventfd de la Fase 5) usaba
  `rustix::runtime::kernel_sigaction`. La propia rustix lo documenta
  como API experimental "libc-like" con el módulo **mangled con una
  cadena aleatoria que rota entre versiones**: el lock del workspace
  tenía 1.1.4 (funciona) y `cargo install` re-resolvió a 1.1.5, donde
  el alias `runtime` pasó a `pub(crate)`. Bomba de tiempo de dos vías:
  cualquier build fuera del workspace y la próxima actualización del
  lock.
- **Corrección:** reemplazo por `libc::sigaction` (estable para
  siempre; libc ya estaba en el árbol vía wayland-backend/dbus, cero
  dependencias nuevas). `install()` conserva su firma
  (`io::Result<HupChannel>`). Detalle: en Linux `sighandler_t` es
  `usize`, así que el puntero de función se castea `as *const () as
  usize` (patrón recomendado por clippy) y `libc::SIG_IGN` entra como
  constante. En el Drop se restaura `SIG_IGN` (la disposición con la
  que Rust arranca el proceso; `SIG_DFL` mataría el proceso con el
  próximo HUP).
- **Limpieza:** `features` de rustix reducidos a `event`+`std`
  (elimina linux-raw-sys/prctl del árbol).
- **Verificación:** clippy del workspace con **0 warnings**, 40 tests
  en verde, y `cargo install` exitoso: el binario de `~/.cargo/bin`
  (el del PATH) se reconstruyó desde la nueva ubicación independiente
  con la traducción incluida (`config show`, `list` y subcomandos
  verificados).
- **Lección generalizada:** una dependencia solo está "en el lock"
  temporalmente; nunca construir sobre APIs marcadas experimentales u
  ocultas aunque compilen hoy. El gate del proyecto (`cargo install`
  como prueba de build de usuario final) entra al repertorio.

## 2026-09-17 — Cierre de Fase 5: DPI por salida, hotplug verificado y dos deadlocks del canal SIGHUP

- **Sesión:** cierre del hito de escritorio completo en Freebuff, con
  sesión real de niri (WAYLAND_DISPLAY disponible, laptop en carga).
- **DPI (el pendiente de la fase):**
  - `buffer_pixels()` (función pura, 3 tests): píxeles de buffer =
    tamaño lógico × escala. `OutputEntry` guarda la escala; la superficie
    pide `set_buffer_scale` ANTES del primer commit; los renderers (SHM
    y wgpu) reciben píxeles físicos en cada `draw_entry`/frame — nítido
    en HiDPI. `scale_factor_changed` (antes stub no-op) aplica la nueva
    escala con log de transición y repinta ya (no hay configure
    garantizado si el tamaño lógico no cambió). `OutputReport` y el log
    de la CLI ahora muestran la escala.
- **Dos deadlocks del canal SIGHUP, cazados con SIGABRT + coredumpctl**
  (ptrace bloqueado por yama=1 y el proceso `setsid` queda huérfano —
  el core dump fue el único inspector disponible, y sobró):
  1. **Deadlock de recarga:** `reload_renderers` corría DENTRO del
     bloque de `prepare_read()` — con el slot de lectura retenido. La
     factory construye el `AnimatedRenderer`, wgpu pregunta las
     capacidades de la superficie y **RADV hace un roundtrip de
     Wayland** (`wl_display_roundtrip_queue` → `wl_display_read_events`
     esperando el read que solo nuestro hilo puede hacer). Core en la
     mano. Fix: el callback y la recarga corren FUERA del guard; el
     `read()` solo se consume si el poll reportó datos (`revents`),
     que además evita bloquear tras un timeout de animación vencido.
  2. **`drain()` bloqueante:** un `read` directo sobre un eventfd en 0
     BLOQUEA hasta la próxima señal; el bucle llama `drain` tras cada
     poll (señal O timeout) → congelación total: 0 ticks crónicos, el
     fondo estático. Fue el primer síntoma de la sesión ("falso
     estático") y el fix intermedio (poll con timeout INFINITO) lo
     reprodujo exacto — la simulación mental de `Ok(0)` era falsa.
     Fix final: poll de timeout CERO dentro de `drain`, read solo si
     está listo. La CPU (0 ticks donde debía haber ~8) fue la señal
     que delató ambos; los píxeles mintieron porque el shell también
     se mueve.
- **Demo DPI verificada en niri** (`demos/fase5/demo-dpi.log` +
  capturas): eDP-1 1920x1200 scale 1→2→1 editando la config de DMS y
  recargando niri: transiciones `escala de buffer 1→2` y `2→1` en el
  log, proceso vivo, CPU 8 ticks/5s en ambas escalas, y cobertura de
  quad verificada en las 4 esquinas + centro del buffer nuevo (la
  lección del bug de la Fase 2 aplicada). Escritorio restaurado byte a
  byte (md5 del outputs.kdl de DMS verificado).
- **Demo hotplug verificada en niri** (`demos/fase5/demo-hotplug.log`):
  apagar/reconectar HDMI-A-1 con `niri msg output HDMI-A-1 off/on`:
  superficie cerrada por el compositor ("restantes: 1") y recreada en
  caliente con la fuente de su config (factory sobre el modelo vivo;
  sin warning de fallback). La pausa fullscreen (D12) siguió
  correctamente al juego que migró de salida: 0 ticks con la única
  salida en fullscreen, 7 ticks al reconectar. SIGHUP posterior
  estable (31 ticks/5s con sesión limpia, recarga incluida).
- **Descubrimientos de niri/DMS (verificados, no recordados):**
  `off true` en un bloque `output` NO es válido en este niri
  ("unexpected argument", recarga rechazada); apagar/reconectar es
  tarea del IPC (`niri msg output X off/on`). DMS reescribe su
  `outputs.kdl` durante el ciclo off/on (cambia el modo al preferido):
  restaurar desde backup y verificar md5. `niri msg outputs` sigue
  listando las salidas apagadas: no sirve como aserción de apagado.
- **Gates:** fmt/clippy 0 warnings, 43 tests (3 nuevos de DPI),
  `cargo install` OK. Commits: traducción, SIGHUP+rustix, DPI+hotplug.
- **Fase 5 cerrada en PLAN.md.** Límites documentados (no bloquean):
  escala fraccional (niri redondea a entero), posición real del cursor
  para `u_mouse`, fps de config solo al reiniciar. **Siguiente paso:**
  Fase 6 — herramientas para creadores (`bruma new`, plantillas WGSL).

## 2026-09-17 — Corrección: el proyecto se documenta en inglés (D13)

- **Contexto:** el usuario corrigió la convención del proyecto: código y
  documentación pública van en **inglés** (D1: proyecto abierto para
  otros). La entrada anterior de este día registró una "traducción al
  español" como decisión propia — era una deriva de sesiones pasadas
  commiteada sin cuestionarla contra la convención. Primera acción:
  **revertir ese commit** (`0c7e47f` revierte `9738d13`, 43 tests en
  verde) y luego traducir TODO lo que faltaba al inglés.
- **Alcance acordado con el usuario:** código (comentarios, doc-comments,
  mensajes de error, textos de CLI y logs) + docs públicas (README,
  PLAN, DECISIONS). La bitácora queda en español: es el diario de
  trabajo del autor, no una superficie pública.
- **Renombres de API en español → inglés** (breaking interno, sin
  callers externos): `BrumaError::{FormatoDesconocido,
  ManifiestoInvalido}` → `{UnknownFormat, InvalidManifest}`;
  `params_a_pares` → `params_to_pairs` (CLI); helpers de `pause.rs`
  (`flags_a_pausa`, `upower_state_a_bateria`, `*_de_variant`) →
  `flags_to_pause`, `upower_state_to_battery`, `*_from_variant`;
  `Fuente`/`resolver_fuente`/`ModeloFuentes` →
  `Source`/`resolve_source`/`SourceModel` (CLI); tests renombrados a
  inglés y aserciones de texto de error actualizadas (`reservado` →
  `reserved`, `insegura` → `unsafe`, `demasiado grande` → `too large`,
  `duplicado` → `duplicate`).
- **Decisiones de traducción tomadas (y por qué):** (a) los avisos de
  burbuja D-Bus van al inglés (`bruma: shader rejected`) — son
  visibles al usuario final de cualquier idioma; (b) los ejemplos JSON
  de `config.json` y README conservan verbatim `onda-bruma-demo` e
  `intensidad` — son los nombres REALES del paquete demo instalado;
  traducirlos rompería el ejemplo; los params de manifiesto son
  contenido del creador, no documentación del motor (D13 lo registra);
  (c) los logs de runtime van al inglés también (salida operativa de un
  proyecto abierto, mismo criterio que los errores).
- **Limpieza de paso en `main.rs`** (muertos detectados al traducir):
  un `push`+`pop` consecutivo que no hacía nada en `cli_overrides` y un
  `let _ = &mut gpu;` sin efecto — ambos eliminados; el flag `--gpu`
  sigue funcionando (se usa en `needs_gpu || gpu`).
- **16 archivos Rust + 3 docs traducidos** (≈6,6k líneas revisadas).
  **Gates:** fmt, clippy 0 warnings, 43 tests, `cargo install` OK.
  Commit único: la traducción es un cambio de texto sin lógica nueva.
- **Lección:** las convenciones viven en DECISIONS.md o no existen. D13
  queda registrada para que la próxima sesión no repita la deriva.

## 2026-09-17 — Fase 6: motor para creadores (u_clock, texturas, feedback, puntero)

Cuatro hitos commiteados, cada uno con su demo en la máquina real (niri,
iGPU AMD 660M/RADV) y gates verdes (fmt, clippy 0 warnings, 49 tests).

- **Paso 1 (`e853166`) — reloj real + `bruma new`.** El bloque de
  uniforms crece a 64 bytes con `u_clock` (hora local h/m/s vía
  `libc::localtime_r`; tintes día/noche sin tocar el motor). `bruma new
  <nombre> --template <t>` escribe el paquete completo, autoverificado
  con el parser real del manifiesto y con naga en tests. Plantillas
  iniciales: waves, fog (FBM), water (procedural).
- **Paso 2 (`f5b4cde`) — assets como texturas.** `textures` en el
  manifiesto (rutas bajo `assets/`, máx. 4, verificadas contra el zip);
  `AnimatedRenderer::set_textures` las sube con `write_texture` a slots
  fijos (bindings 2i+1/2i+2) con dummies 1×1 para los vacíos — el
  hot-reload sigue compartiendo UN layout. Plantilla water-photo: la foto
  del creador detrás de un agua procedural. Nota de la sesión: el
  `copy_external_image_to_texture` de wgpu 30 es solo-web; el camino
  nativo es `write_texture`.
- **Paso 3 (`35d7fa0`) — feedback (ping-pong).** Nuevo permiso
  `feedback`: el shader lee su fotograma anterior (grupo 1) y un blit
  interno lo copia al swapchain — estelas, reaction-diffusion,
  simulaciones. Plantilla trail. Dos lecciones caras: (1) el error
  "group 1 not available" fue culpa mía — el blit declara solo el grupo
  1 pero `build_quad_pipeline` lo mapea como grupo 0; con layout de dos
  grupos se arregla; (2) wgpu 30 **paniquea** en el primer error de
  validación no capturado (el daemon moría): ahora
  `Device::on_uncaptured_error` lo registra — coherente con la filosofía
  hot-reload ("un shader roto no tira el fondo"). El grupo 1 se bindea
  dummy también en la ruta de un pase: cualquier pipeline exige TODOS
  sus grupos bindeados en cada draw.
- **Paso 4 (`83975ea`) — puntero y parallax.** `wl_seat`/`wl_pointer`
  vía SCTK 0.21 (SeatState + PointerHandler; los Dispatch impls los
  genera `delegate_dispatch2`, no hay macros delegadas de seat/pointer
  en esta versión). `u_mouse` = posición lógica del cursor mientras
  sobrevuela nuestros fondos, (-1,-1) si no (el Wayland de verdad: una
  superficie background no recibe el puntero bajo ventanas). Plantilla
  parallax: aurora de 3 capas con profundidad que deriva sola cuando no
  hay cursor.
- **Verificación en vivo:** los 6 templates pasan naga en tests (un
  `target` reservado de WGSL lo cazó el test, no la GPU); demo-trail y
  demo-water corren 8 s con 0 errores; demo-parallax en sesión real con
  "pointer capability bound" y ambas salidas animando.
- **Pendiente de la fase:** UI de parámetros generada del manifiesto,
  cover/contain para texturas, asociación MIME del hito drag-and-drop.
- **Gates finales:** fmt, clippy 0 warnings, 49 tests, `cargo install`
  OK. Evidencia en `demos/fase6/`.

## 2026-09-17 — Agua que sigue al cursor (water-cursor): simulación con entrada de display

- **Efecto:** el "calm water" estilo Wallpaper Engine — el puntero deja
  ondulaciones sobre una foto fija que se propagan y se calman solas.
- **Motor:** tercera variante del pase de feedback. El shader del creador
  puede declarar `fs_main(uv)` como SIMULACIÓN pura (estado offscreen) y
  `display(uv, frame, u)` como presentación (refracción + brillos sobre
  la foto). El blit interno compila **el código del creador + un apéndice
  interno** (`BLIT_DISPLAY_APPEND`) porque un módulo WGSL es una unidad
  de compilación: `fs_display` necesita ver el `U` del creador y su
  `display()`. Si el módulo combinado no compila, cae al blit de copia
  (el fondo nunca muere por un display roto). Matiz de formatos que
  costó un test: el offscreen usa el MISMO formato sRGB que el
  swapchain — muestrear sRGB y re-codificar al escribir es un viaje
  idéntico byte a byte.
- **Precisión del mouse:** u_mouse pasó de lógico a PÍXELES DE BUFFER
  (posición × escala de la salida) para que los shaders hablen el mismo
  idioma que u_res en monitores HiDPI.
- **Plantilla `water-cursor`:** campo de alturas en frame.r alrededor de
  0.5; relajación hacia el vecindario (propagación) + amortiguación;
  gota Gaussiana donde está el cursor; display = refracción del lookup
  de la foto + glints especulares. Foto de la demo: **wallhaven yqxzqx**
  (API v1, búsqueda SFW con `+lake +mountain`, 5120×2880 → recortada a
  1920×1200/375 KB — límites del paquete 32 MB/96 MB sobran).
- **Test offline de la ruta completa** (`tests/display_blit.rs`, GPU
  real, saltos sin adaptador): (1) la pantalla NO es el estado crudo
  (display() fue llamado: la foto tiene varianza de color, el estado es
  gris plano); (2) la gota del cursor inyecta energía (la salida sim
  cambia con u_mouse en el centro). La primera aserción de "píxel ==
  foto" murió por el doble sRGB/lineal: el screenshot en vivo está
  contaminado por las ventanas del escritorio — la lección de las Fases
  2-5 otra vez: verificar con render offline, no con capturas.
- **Gates:** fmt, clippy 0 warnings, 50 tests, cargo install OK.
  Demo en vivo: 0 errores, puntero vinculado, ambas salidas activas.

## 2026-09-17 — water-cursor v4: physics and orientation fixed after live demo

The first live demo of `water-cursor` was broken in four ways the gates
had not caught. Root causes and fixes:

- **Upside-down photo (every feedback wallpaper).** The creator
  templates flipped `uv.y` (`1 - y`) while `image.wgsl` (phase 2,
  verified upright back then) does not — in Vulkan NDC, NDC.y=-1 is the
  TOP, so the flip turned every textured template upside down, and the
  blits' double correction hid it in the demo. Decision: the engine's
  orientation contract is uv.y=0 at the TOP, nothing ever flips.
  All 7 templates now match `image.wgsl`; procedural ones (fog, water,
  parallax) keep their look with a single `1 - uv.y` at the top of
  fs_main; parallax's mouse Y also comes out aligned.
- **Frozen rings + grain (the sim was diffusion in 8-bit sRGB).**
  The old fs_main averaged neighbors (no propagation, no momentum) and
  its target was the swapchain's sRGB format: quantization froze the
  rings, and a per-frame "shimmer" injected grain. Now: real wave
  equation (height + velocity in R/G, c²=0.4), offscreen targets are
  fp16 LINEAR (`SIM_FORMAT = Rgba16Float`), the creator pipeline is
  built for that format at construction (feedback is now a constructor
  argument, not a post-hoc setter — it decides the target format), and
  the shimmer is gone.
- **White rims + blur soup (v1 of the display look).** Refraction ×0.4
  displaced the lookup dozens of pixels (blur); additive glints and a
  wet-tint burned wave crests to white. Now: refraction ×0.03 (photo
  stays sharp), the diffuse-reflection lambert term MULTIPLIES the
  photo (0.78..1.33 — broad light bands that cannot clip), faint sheen
  only at pow>40. Slope is sampled 2 texels apart: smooth bands.
- **Tests that now pin the physics offline** (`display_blit.rs`): the
  blit preserves orientation (red top → red top); the ripple lands
  UNDER the cursor (>36 fp16 ULPs) and NOT in its vertical mirror
  (≤30) — "the effect appears in the opposite half" can never return;
  the screen shows the photo through display() (most-colorful-pixel
  probe; the old center-variance probe died on the gray lake center).
- **Lesson:** `bruma install` refuses to overwrite the same version —
  template edits never reach the desktop without wiping the store. The
  v2/v3 "still broken" reports were the old package still running.

Evidence: `demos/fase6/water-cursor-v4.{png,log}` (upright, sharp, 0
GPU errors, RADV/Vulkan). Gates: fmt, clippy 0 warnings, 15 suites OK.

## 2026-09-17 — water-cursor v5: wake-follows-cursor pinned at screen level, scattered light

Two follow-ups after the v4 demo:

- **The wake appears where the cursor is — proven on the SCREEN.** The
  new chain test (`wake_follows_the_cursor_on_screen_not_its_mirror`)
  runs the full production path offline — 15 frames of sim ping-pong
  (half a second of dripping at 30 fps) then the display blit — and
  compares the mean |per-pixel change| under the cursor vs. its
  vertical mirror: 3.09 vs. well under half that, asserted with a 2×
  margin. Absolute (not signed) deltas: the wake's light/dark bands
  average out to zero, only their magnitude is signal. Each run starts
  from fresh calm water (a shared state pool would let run 2 inherit
  run 1's wake — first version of the test compared identical frames).
- **The diffuse reflection spreads past the ring** (the "light plays
  over disturbed water" feel): a wide 4-tap SPREAD sample of the height
  field lifts brightness over the whole wake area (scatter ≤ +22%,
  still multiplicative — nothing can clip toward white); lambert bands
  sampled 3 texels apart; slope ×16.

Gates: fmt, clippy 0 warnings, 15 suites OK. Evidence:
`demos/fase6/water-cursor-v5.{png,log}`.

## 2026-09-17 — Fullscreen pause escape hatch wired (`--no-fullscreen-pause`)
User could not see the water-cursor effect while switching workspaces.
Diagnosis: the D12 fullscreen pause does not track workspaces —
wlr-foreign-toplevel reports a fullscreen window (Sniper3) on the output
even when it sits on another workspace, so that output stayed frozen by
design. Worse, the `--no-fullscreen-pause` flag was parsed but never
applied. Fix: `BackgroundWindow::set_fullscreen_pause` is now called from
`bruma run` (flag wins over config `fullscreen_pause`). 52 tests green.

## 2026-09-17 — The effect was invisible, not broken: probe + visible physics
Systematic diagnosis with a RED/BLUE diagnostic package (mouse-probe):
the compositor (niri) DOES deliver pointer events to the background
layer (15 Enters logged), and u_mouse reaches the shader — the whole
chain works. The real problem: v5's shading produced 1-4% brightness
deltas (mathematically present, invisible to the eye). Fixes:
- water-cursor: saturated drop (digs while h > -0.25, x6 stronger,
  sigma ~40 px), shader gain x3 (lambert 0.80..1.50, refract 0.12,
  scatter x0.45, spec pow 20 mix 0.25). Defaults intensity 0.8,
  damping 0.2.
- platform: pointer position is attributed to the output whose surface
  the event carries (multi-monitor: the drop no longer appears on both
  screens); pointer Enter/Leave logged at Info for diagnosis.
- tests: display_blit canvas 512x320 (the mirror row now sits 4+
  Gaussian sigmas from the cursor; at 192x120 the correct drop's tail
  leaked into the mirror assertion — fp16 ULP math confirmed it).
52 tests green. Proof of the working chain: probe screen turned RED
(2.3M px) exactly while the cursor was over the background.
