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
