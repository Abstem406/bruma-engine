//! # bruma-platform
//!
//! Capa de plataforma: ventanas de fondo en Wayland vía `wlr-layer-shell`.
//!
//! Estado: **Fase 5**. Implementación con `smithay-client-toolkit` 0.20
//! (NO winit: no soporta layer-shell), igual que swww. El banco de pruebas
//! de referencia es **niri** (+ DankMaterialShell), con el resto de
//! compositors wlroots/KWin como best-effort.
//!
//! Hay **una superficie de fondo por salida conectada** (Fase 5): una
//! superficie sin output concreto solo cubre una pantalla en la mayoría
//! de compositors. Las superficies nacen con el hotplug de salidas
//! (`new_output`), mueren con él (`output_destroyed`/`closed`) y el
//! renderizador de cada una lo decide una **factory** inyectada desde la
//! CLI (la plataforma no conoce GPU: frontera D3/D6).
//!
//! El bucle de eventos tiene dos modos: `run` (solo eventos Wayland; la
//! CPU queda idle con contenido estático) y `run_with_runtime` (conduce
//! además la animación al ritmo que pida el `WallpaperRuntime`, durmiendo
//! en `poll` sobre el socket — nunca spin) pintando TODAS las salidas.
//!
//! NON-GOALS (ver DECISIONS.md): GNOME/Mutter en v1 (sin layer-shell);
//! vídeo y audio en v1.

#![forbid(unsafe_code)]

mod notify;
mod pause;
mod toplevel;

pub use notify::DesktopNotifier;

use bruma_core::Color;
use bruma_renderer::{FrameDecision, FrameRenderer, WallpaperRuntime};
use smithay_client_toolkit::reexports::client as wayland_client;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::ptr::NonNull;
use std::time::Instant;
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle, event_created_child,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// Una ventana de fondo detrás de todas las ventanas, **una por salida**.
///
/// Tipo de alto nivel para el CLI: construye una superficie por cada
/// salida conocida (y por las que se conecten después), y el bucle de
/// eventos las mantiene vivas, redibujando en cada re-configuración del
/// compositor (recarga de config, cambio de resolución, etc.).
pub struct BackgroundWindow {
    /// Manija de la conexión (Arc interno): necesaria para `display_ptr`,
    /// el fd del socket y roundtrips del CLI.
    conn: Connection,
    event_queue: EventQueue<BackgroundState>,
    state: BackgroundState,
}

/// Estado delegado de eventos Wayland. Es el dueño de todo lo necesario
/// para dibujar, así los handlers pueden redibujar directamente.
struct BackgroundState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    color: Color,
    closed: bool,
    /// Una entrada por salida conectada (Fase 5). Cada una con su propia
    /// superficie layer-shell, tamaño y renderizador.
    outputs: Vec<OutputEntry>,
    /// Factory de renderers por superficie (Fase 5). La inyecta el CLI;
    /// la plataforma solo la llama con los punteros crudos de la nueva
    /// superficie — no sabe nada de GPU (D3/D6).
    factory: Option<SurfaceRendererFactory>,
    /// Rastreo de ventanas fullscreen por salida (pausa D12). El bind
    /// del manager es opcional: sin protocolo, no hay pausa y todo
    /// sigue como siempre.
    toplevel: toplevel::ToplevelTracker,
    /// El manager ligado (si el protocolo existe). Hay que conservarlo
    /// vivo: al dropearlo el compositor deja de anunciar toplevels.
    _toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    /// Último estado de pausa por salida (índice del Vec outputs): para
    /// loggear solo transiciones, no cada frame.
    last_pause: Vec<bool>,
    /// Pausa global (D12): bloqueo de sesión y batería, best-effort vía
    /// D-Bus de sistema. Sin bus, nunca pausa.
    pause: pause::SessionPauseWatcher,
    /// Último estado de la pausa global, para log de transiciones.
    last_global_pause: bool,
}

/// Una superficie de fondo en una salida concreta.
struct OutputEntry {
    /// Proxy de la salida (para casar `update_output`/`output_destroyed`
    /// con su entrada, y para consultar el fullscreen del toplevel).
    output: Option<wl_output::WlOutput>,
    /// Info declarativa (nombre, tamaño lógico) para logs y API.
    info: OutputInfo,
    layer: LayerSurface,
    width: u32,
    height: u32,
    configured: bool,
    /// Renderer de ESTA salida (construido por la factory). `None` si la
    /// factory falló o no hay factory: fallback de color sólido.
    renderer: Option<Box<dyn FrameRenderer>>,
}

/// Información mínima de una salida conectada.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub name: Option<String>,
    pub logical_size: Option<(i32, i32)>,
}

/// Punteros crudos + identidad de una superficie de fondo recién creada.
/// Es lo único que la plataforma le pasa a la factory: con esto (y el
/// nombre de la salida) el CLI puede construir su renderer.
pub struct OutputSurfaceHandles {
    pub display_ptr: NonNull<std::ffi::c_void>,
    pub surface_ptr: NonNull<std::ffi::c_void>,
    pub output_name: Option<String>,
}

/// Factory de renderers por superficie. Devuelve `Err` con el motivo si
/// no se pudo construir; esa salida degrada a color sólido (con log) y
/// las demás siguen — un escritorio con N pantallas no muere por una.
pub type SurfaceRendererFactory =
    Box<dyn FnMut(&OutputSurfaceHandles) -> Result<Box<dyn FrameRenderer>, String>>;

/// Reporte de una salida tras el arranque (para el log del CLI).
#[derive(Debug, Clone)]
pub struct OutputReport {
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    /// ¿Tiene renderer de la factory (true) o fallback de color (false)?
    pub gpu: bool,
}

/// Errores de la capa de plataforma.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// No se pudo conectar al compositor Wayland.
    #[error("no se pudo conectar al compositor Wayland: {0}")]
    Connect(String),
    /// El compositor no soporta un protocolo necesario.
    #[error(
        "el compositor no soporta {0} (¿Wayland con wlr-layer-shell? niri, sway, Hyprland y KWin lo soportan; GNOME no)"
    )]
    MissingProtocol(&'static str),
    /// Error de memoria compartida (wl_shm).
    #[error("error de wl_shm: {0}")]
    Shm(String),
    /// El compositor cerró la superficie.
    #[error("el compositor cerró la superficie de fondo")]
    SurfaceClosed,
    /// El compositor nunca configuró la superficie.
    #[error("el compositor nunca configuró la superficie de fondo")]
    NotConfigured,
    /// No hay ninguna salida conectada.
    #[error("no hay ninguna salida conectada")]
    NoOutputs,
    /// Error de despacho de eventos.
    #[error("error en el bucle de eventos: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    /// Error de E/S esperando eventos (flush/poll/lectura del socket).
    #[error("error esperando eventos: {0}")]
    Poll(String),
}

impl BackgroundWindow {
    /// Conecta al compositor (vía entorno) y prepara el fondo. Las
    /// superficies por salida se crean solas en el primer roundtrip
    /// (`new_output`), ya con la factory instalada si se instala ANTES
    /// del primer `present_once`.
    pub fn new(color: Color) -> Result<Self, PlatformError> {
        let conn =
            Connection::connect_to_env().map_err(|e| PlatformError::Connect(e.to_string()))?;
        Self::for_connection(conn, color)
    }

    /// Igual que [`Self::new`] pero sobre una conexión ya establecida.
    pub fn for_connection(conn: Connection, color: Color) -> Result<Self, PlatformError> {
        let (globals, event_queue) =
            registry_queue_init(&conn).map_err(|e| PlatformError::Connect(e.to_string()))?;
        let qh = event_queue.handle();

        // Los estados de protocolo viven en el estado: `new_output` los
        // necesita para crear superficies en caliente (hotplug).
        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("wl_compositor"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("zwlr_layer_shell_v1"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|_| PlatformError::MissingProtocol("wl_shm"))?;

        // Pausa en fullscreen (D12): opcional por protocolo.
        let toplevel_mgr = globals
            .bind::<
                wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
                BackgroundState,
                (),
            >(&qh, 1..=3, ())
            .ok();
        if toplevel_mgr.is_none() {
            log::info!("sin wlr-foreign-toplevel: la pausa en fullscreen no está disponible");
        }

        let pool =
            SlotPool::new(64 * 64 * 4, &shm).map_err(|e| PlatformError::Shm(e.to_string()))?;

        Ok(Self {
            conn,
            event_queue,
            state: BackgroundState {
                registry_state: RegistryState::new(&globals),
                output_state: OutputState::new(&globals, &qh),
                compositor,
                layer_shell,
                shm,
                pool,
                color,
                closed: false,
                outputs: Vec::new(),
                factory: None,
                toplevel: toplevel::ToplevelTracker::disabled(),
                _toplevel_manager: toplevel_mgr,
                last_pause: Vec::new(),
                pause: pause::SessionPauseWatcher::new(),
                last_global_pause: false,
            },
        })
    }

    /// Instala la factory de renderers por superficie (Fase 5). Debe
    /// llamarse ANTES del primer `present_once` para que las superficies
    /// del arranque nazcan ya con su renderer.
    pub fn set_renderer_factory(&mut self, factory: SurfaceRendererFactory) {
        self.state.factory = Some(factory);
    }

    /// Instala UN renderer compartido por todas las salidas... no se puede:
    /// cada salida necesita su propia superficie GPU. Este método es el
    /// reemplazo del antiguo `set_frame_renderer`: envuelve al renderer
    /// dado en una factory que clona el modelo de construcción.
    ///
    /// NOTA: `FrameRenderer` no es `Clone`; la factory solo tiene sentido
    /// construyendo un renderer NUEVO por superficie (por eso existe
    /// [`Self::set_renderer_factory`]).
    pub fn set_frame_renderer(&mut self, renderer: Box<dyn FrameRenderer>) {
        // Degradación honesta: el primer output recibe el renderer tal
        // cual (backward compat con las fases 1-4); los demás quedan en
        // color sólido con un warning claro.
        let mut taken = Some(renderer);
        self.state.factory = Some(Box::new(move |_handles| {
            if let Some(r) = taken.take() {
                Ok(r)
            } else {
                Err("set_frame_renderer solo provee un renderer: \
                     usa set_renderer_factory para multi-salida"
                    .to_owned())
            }
        }));
    }

    /// ¿Algún renderer instalado produce contenido animado? Lo consulta
    /// el CLI para elegir entre `run` y `run_with_runtime`.
    pub fn wants_animation(&self) -> bool {
        self.state
            .outputs
            .iter()
            .any(|o| o.renderer.as_ref().is_some_and(|r| r.wants_animation()))
    }

    /// Puntero crudo al `wl_display` de la conexión.
    pub fn display_ptr(&self) -> NonNull<std::ffi::c_void> {
        // En libwayland `wl_display` ES un `wl_proxy` (el mismo puntero);
        // el cast a c_void es lo que esperan wgpu/Vulkan.
        let ptr = self.conn.backend().display_id().as_ptr();
        NonNull::new(ptr).expect("wl_display vivo").cast()
    }

    /// Cambia el color de fondo; surte efecto en el próximo redibujo.
    pub fn set_color(&mut self, color: Color) {
        self.state.color = color;
    }

    /// Pantallas detectadas hasta ahora.
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.state.outputs.iter().map(|o| o.info.clone()).collect()
    }

    /// Procesa eventos hasta que TODAS las salidas conocidas estén
    /// configuradas. Devuelve un reporte por salida (tamaño y si pinta
    /// con GPU o con el fallback de color).
    pub fn present_once(&mut self) -> Result<Vec<OutputReport>, PlatformError> {
        // Primer roundtrip: globals + creación de superficies + configures.
        self.event_queue.roundtrip(&mut self.state)?;
        if self.state.closed {
            return Err(PlatformError::SurfaceClosed);
        }
        // Los outputs pueden seguir llegando (hotplug temprano); los
        // recién llegados necesitan su configure. Un roundtrip extra
        // drena los pendientes.
        self.event_queue.roundtrip(&mut self.state)?;
        if self.state.outputs.is_empty() {
            return Err(PlatformError::NoOutputs);
        }
        let reports: Vec<OutputReport> = self
            .state
            .outputs
            .iter()
            .map(|o| OutputReport {
                name: o.info.name.clone(),
                width: o.width,
                height: o.height,
                gpu: o.renderer.is_some(),
            })
            .collect();
        if reports.iter().any(|r| r.width == 0) {
            return Err(PlatformError::NotConfigured);
        }
        Ok(reports)
    }

    /// Ejecuta el bucle de eventos hasta que el compositor cierre el
    /// fondo. Ctrl-C termina el proceso (comportamiento por defecto).
    ///
    /// Modo estático: solo atiende eventos Wayland (configure, frame
    /// callbacks...); entre eventos, `blocking_dispatch` duerme sin girar
    /// la CPU.
    pub fn run(&mut self) -> Result<(), PlatformError> {
        while !self.state.closed {
            self.event_queue.blocking_dispatch(&mut self.state)?;
        }
        Ok(())
    }

    /// Igual que [`Self::run`], pero conduciendo además la animación con
    /// el runtime dado (Fase 3) en TODAS las salidas (Fase 5).
    ///
    /// El ritmo lo dicta el runtime: en cada iteración se le pregunta si
    /// toca dibujar ([`WallpaperRuntime::begin_frame`]); si no, el bucle
    /// duerme en `poll` hasta el deadline del runtime o hasta que llegue
    /// un evento del compositor — lo que ocurra primero. Sin spin.
    pub fn run_with_runtime(
        &mut self,
        runtime: &mut dyn WallpaperRuntime,
    ) -> Result<(), PlatformError> {
        while !self.state.closed {
            // Pausa global (D12): drena señales de bloqueo/batería y,
            // si hay alguna, no se repinta NINGUNA salida (el fondo está
            // tapado por la pantalla de bloqueo, o ahorramos batería).
            // Igual que el fullscreen: el runtime NO se pausa — el
            // tiempo global sigue y al despausar retoma sin salto.
            self.state.pause.poll();
            let global_pause = self.state.pause.paused();
            if global_pause != self.state.last_global_pause {
                log::info!(
                    "pausa global {}: el motor {} repintar (bloqueo/batería)",
                    if global_pause { "ON" } else { "OFF" },
                    if global_pause { "deja de" } else { "vuelve a" }
                );
                self.state.last_global_pause = global_pause;
            }

            // Deadline del despertar: SIEMPRE acotado. Tras dibujar, el
            // segundo begin_frame devuelve Skip con el deadline del
            // próximo tick (no toca el tiempo: now-last < interval).
            // Sin esto, el poll tras el Draw sería indefinido y la
            // animación solo avanzaría con eventos del compositor — bug
            // latente desde la Fase 3, enmascarado por el tráfico
            // constante de un escritorio en uso.
            let deadline = match runtime.begin_frame(Instant::now()) {
                FrameDecision::Draw => {
                    // La resolución real la impone cada superficie (su
                    // último configure); el runtime aporta tiempo, mouse
                    // y parámetros. Un frame de runtime pinta todas las
                    // salidas con el MISMO instante: la animación va
                    // sincronizada entre monitores.
                    //
                    // Pausa por salida (D12): si hay una ventana
                    // fullscreen sobre la salida, su fondo se congela
                    // (no se repinta) — el último buffer sigue en
                    // pantalla a cargo del compositor. Las demás salidas
                    // siguen animando. El runtime NO se pausa: el tiempo
                    // global sigue corriendo y al salir del fullscreen
                    // la animación retoma por donde iba (sin salto).
                    let mut st = runtime.state();
                    // Pausa global activa: no se repinta NINGUNA salida
                    // (el último buffer queda en pantalla a cargo del
                    // compositor, como en el fullscreen por salida).
                    for idx in 0..self.state.outputs.len() {
                        if global_pause {
                            break;
                        }
                        let (w, h) = (
                            self.state.outputs[idx].width,
                            self.state.outputs[idx].height,
                        );
                        if w == 0 || h == 0 {
                            continue;
                        }
                        st.width = w;
                        st.height = h;
                        let fullscreened = self.state.outputs[idx]
                            .output
                            .as_ref()
                            .is_some_and(|o| self.state.toplevel.is_fullscreen_on(o));
                        // Diagnóstico: log solo en transición (no por frame).
                        if self
                            .state
                            .last_pause
                            .get(idx)
                            .is_none_or(|prev| *prev != fullscreened)
                        {
                            log::info!(
                                "salida {:?}: pausa fullscreen = {}",
                                self.state.outputs[idx].info.name,
                                fullscreened
                            );
                            if idx < self.state.last_pause.len() {
                                self.state.last_pause[idx] = fullscreened;
                            } else {
                                self.state.last_pause.resize(idx + 1, false);
                                self.state.last_pause[idx] = fullscreened;
                            }
                        }
                        if fullscreened {
                            continue;
                        }
                        match self.state.outputs[idx].renderer.as_mut() {
                            Some(renderer) => renderer.render_animated(&st),
                            None => self.state.draw_entry_solid(idx),
                        }
                    }
                    match runtime.begin_frame(Instant::now()) {
                        FrameDecision::Skip { deadline } => Some(deadline),
                        // Sin límite de fps (runtime exótico): cede el
                        // hilo con timeout 0 en vez de girar en seco.
                        FrameDecision::Draw => Some(Instant::now()),
                    }
                }
                FrameDecision::Skip { deadline } => Some(deadline),
            };
            self.wait_and_dispatch(deadline)?;
        }
        Ok(())
    }

    /// Despacha lo pendiente y, si no había nada, espera en el socket de
    /// Wayland hasta `timeout` o hasta que lleguen eventos (lo que ocurra
    /// antes). Es la réplica del bucle interno de
    /// `EventQueue::blocking_dispatch`, con timeout añadido:
    ///
    /// 1. `dispatch_pending`: despacha lo ya leído.
    /// 2. `flush`: envía las peticiones pendientes (p. ej. frame callbacks).
    /// 3. `prepare_read` + `poll` + `read`: arma la lectura y duerme.
    ///    El `prepare_read` es obligatorio para no perder eventos que
    ///    lleguen entre el `dispatch_pending` y el `poll`.
    /// 4. `dispatch_pending` final: reparte lo recién leído.
    fn wait_and_dispatch(&mut self, timeout: Option<Instant>) -> Result<(), PlatformError> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};

        if self.event_queue.dispatch_pending(&mut self.state)? > 0 {
            return Ok(());
        }
        self.event_queue
            .flush()
            .map_err(|e| PlatformError::Poll(e.to_string()))?;

        let wait = timeout.map(|deadline| {
            let d = deadline.saturating_duration_since(Instant::now());
            Timespec {
                tv_sec: d.as_secs() as _,
                tv_nsec: d.subsec_nanos() as _,
            }
        });

        if let Some(guard) = self.event_queue.prepare_read() {
            // El handle Backend es un Arc barato; el BorrowedFd presta de
            // él, así que el handle vive en este bloque.
            let backend = self.conn.backend();
            let fd = backend.poll_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            match poll(&mut fds, wait.as_ref()) {
                // ready == 0: venció el timeout (toque de animación).
                // ready > 0: llegaron datos; se leen y despachan abajo.
                Ok(_) => {}
                // EINTR (señales como SIGINT): no es un error para el bucle.
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(PlatformError::Poll(e.to_string())),
            }
            // `read` consume el guard; si devolvió 0 eventos (p. ej. solo
            // EINTR), el dispatch_pending de abajo no tendrá nada nuevo.
            guard
                .read()
                .map_err(|e| PlatformError::Poll(e.to_string()))?;
        }

        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}

impl BackgroundState {
    /// Ciclo completo de alta de una salida: superficie layer-shell,
    /// renderer vía factory (con los punteros reales ya creados) y
    /// registro. El renderer se construye ANTES del primer configure
    /// para que pinte él el primer frame (patrón de las fases 2-4).
    fn create_surface_for_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: &wl_output::WlOutput,
    ) {
        let info = self.output_state.info(output);
        let entry_info = OutputInfo {
            name: info.as_ref().and_then(|i| i.name.clone()),
            logical_size: info.as_ref().and_then(|i| i.logical_size),
        };

        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("bruma"),
            Some(output), // << LA salida concreta (clave de la Fase 5)
        );
        // Anclada a los cuatro bordes => ocupa la pantalla completa; el
        // tamaño lo impone el compositor en el configure (se pide 0,0).
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_size(0, 0);
        // Un wallpaper no debe robar el teclado ni el puntero.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        // Renderer por salida, con los punteros crudos de LA superficie
        // recién creada. Error de factory = color sólido (con log): un
        // escritorio de N pantallas no muere por una.
        let renderer = self.build_renderer(conn, &layer, output);

        log::info!("superficie de fondo creada en salida {:?}", entry_info.name);
        self.outputs.push(OutputEntry {
            output: Some(output.clone()),
            info: entry_info,
            layer,
            width: 0,
            height: 0,
            configured: false,
            renderer,
        });
    }

    /// Construye el renderer de una salida vía factory (si hay).
    fn build_renderer(
        &mut self,
        conn: &Connection,
        layer: &LayerSurface,
        output: &wl_output::WlOutput,
    ) -> Option<Box<dyn FrameRenderer>> {
        let factory = self.factory.as_mut()?;
        let display_ptr = {
            let ptr = conn.backend().display_id().as_ptr();
            NonNull::new(ptr).expect("wl_display vivo").cast()
        };
        let surface_ptr = {
            let ptr = layer.wl_surface().id().as_ptr();
            NonNull::new(ptr).expect("wl_surface vivo").cast()
        };
        let info = self.output_state.info(output);
        let handles = OutputSurfaceHandles {
            display_ptr,
            surface_ptr,
            output_name: info.as_ref().and_then(|i| i.name.clone()),
        };
        match factory(&handles) {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!(
                    "salida {:?} sin renderer GPU (fallback a color): {e}",
                    handles.output_name
                );
                None
            }
        }
    }

    /// Busca la entrada dueña de una superficie layer-shell.
    fn entry_by_surface(&mut self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.outputs
            .iter()
            .position(|o| o.layer.wl_surface().id() == surface.id())
    }

    /// Fallback de la Fase 1: rellena un buffer ARGB8888 del color dado,
    /// lo daña y lo committea, para la entrada indicada.
    fn draw_solid_entry(
        entry: &mut OutputEntry,
        pool: &mut SlotPool,
        width: u32,
        height: u32,
        color: &Color,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let stride = width as i32 * 4;
        let Ok((buffer, canvas)) = pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            log::error!("no se pudo crear buffer {width}x{height}");
            return;
        }; // ARGB8888 en memoria nativa little-endian => bytes B,G,R,A.
        let px = [color.b, color.g, color.r, color.a];
        canvas.as_chunks_mut::<4>().0.iter_mut().for_each(|chunk| {
            *chunk = px;
        });

        let surface = entry.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(surface).expect("attach buffer");
        entry.layer.commit();
    }

    /// Dibuja una entrada (renderer si hay; color si no).
    fn draw_entry(&mut self, idx: usize) {
        let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
        if w == 0 || h == 0 {
            return;
        }
        if self.outputs[idx].renderer.is_some() {
            let renderer = self.outputs[idx].renderer.as_mut().unwrap();
            renderer.render_frame(w, h);
        } else {
            self.draw_entry_solid(idx);
        }
    }

    /// Fallback de color para la entrada idx (usar cuando NO hay renderer).
    fn draw_entry_solid(&mut self, idx: usize) {
        let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
        let color = self.color;
        let entry = &mut self.outputs[idx];
        Self::draw_solid_entry(entry, &mut self.pool, w, h, &color);
    }
}

impl CompositorHandler for BackgroundState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Fase 5 (futuro): redimensionar buffers por DPI. Con shm el
        // factor no afecta al tamaño en píxeles del buffer lógico.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Fase 5 (futuro): rotación de salida.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // El color estático no anima; el renderer wgpu presenta por su
        // cuenta (no pide frame callbacks del compositor).
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Con una superficie por salida, `enter` es trivialmente la suya.
        // El log de salidas vive en `create_surface_for_output`.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for BackgroundState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // HOTPLUG (Fase 5): nueva salida => nueva superficie de fondo.
        self.create_surface_for_output(conn, qh, &output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // La info de la salida (nombre/tamaño) llegó o cambió.
        let info = self.output_state.info(&output);
        let new_info = OutputInfo {
            name: info.as_ref().and_then(|i| i.name.clone()),
            logical_size: info.as_ref().and_then(|i| i.logical_size),
        };
        if let Some(entry) = self
            .outputs
            .iter_mut()
            .find(|o| o.output.as_ref().is_some_and(|x| x.id() == output.id()))
        {
            if entry.info.name != new_info.name {
                log::info!(
                    "salida renombrada: {:?} → {:?}",
                    entry.info.name,
                    new_info.name
                );
            }
            entry.info = new_info;
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // HOTPLUG: la salida se fue => su superficie se destruye con ella.
        let before = self.outputs.len();
        self.outputs
            .retain(|o| o.output.as_ref().is_none_or(|x| x.id() != output.id()));
        if self.outputs.len() != before {
            log::info!(
                "salida retirada; superficies restantes: {}",
                self.outputs.len()
            );
        }
        if self.outputs.is_empty() {
            log::info!("sin salidas: el fondo termina");
            self.closed = true;
        }
    }
}

impl LayerShellHandler for BackgroundState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        // El compositor cerró UNA superficie (p. ej. la salida se fue).
        if let Some(idx) = self.entry_by_surface(layer.wl_surface()) {
            let name = self.outputs[idx].info.name.clone();
            self.outputs.remove(idx);
            log::info!(
                "superficie cerrada por el compositor ({name:?}); restantes: {}",
                self.outputs.len()
            );
        }
        if self.outputs.is_empty() {
            log::info!("el compositor cerró el fondo completo");
            self.closed = true;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(idx) = self.entry_by_surface(layer.wl_surface()) else {
            return;
        };
        let (w, h) = configure.new_size;
        // Con anchor a 4 bordes el tamaño lo impone el compositor; si
        // llegara 0 (no debería), usamos un mínimo digno para no morir.
        let entry = &mut self.outputs[idx];
        entry.width = w.max(1);
        entry.height = h.max(1);
        entry.configured = true;
        // Redibujar en CADA configure: así sobrevivimos a recargas de
        // configuración de niri y a cambios de resolución de salida.
        self.draw_entry(idx);
    }
}

impl ShmHandler for BackgroundState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

// Despacho del protocolo foreign-toplevel (pausa en fullscreen, D12).
// wayland-client exige los impls sobre el estado dueño de la cola.
impl wayland_client::Dispatch<ZwlrForeignToplevelManagerV1, ()> for BackgroundState {
    fn event(
        state: &mut Self,
        _manager: &ZwlrForeignToplevelManagerV1,
        event: <ZwlrForeignToplevelManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::Event as MgrEvent;
        match event {
            MgrEvent::Toplevel { toplevel } => state.toplevel.toplevel_created(toplevel),
            MgrEvent::Finished => {
                state.toplevel.reset();
                log::info!(
                    "foreign-toplevel retirado por el compositor; pausa fullscreen desactivada"
                );
            }
            _ => {}
        }
    }

    // El evento `toplevel` (opcode 0) CREA un objeto hijo (el handle de
    // la ventana): wayland-client exige declarar su user-data aquí, o
    // el dispatcher paniquea en runtime. Los handles usan () como
    // user-data, igual que el manager.
    event_created_child!(BackgroundState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl wayland_client::Dispatch<ZwlrForeignToplevelHandleV1, ()> for BackgroundState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: <ZwlrForeignToplevelHandleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::Event as HEvent;
        match event {
            HEvent::State { state: states } => {
                // El array del protocolo es una lista de u32 crudos
                // (enum `state`); wayland-client lo entrega como Vec<u8>.
                state.toplevel.toplevel_state(handle, &states);
            }
            HEvent::Title { title } => state.toplevel.toplevel_title(handle, title),
            HEvent::OutputEnter { output } => state.toplevel.output_enter(handle, &output),
            HEvent::OutputLeave { output } => state.toplevel.output_leave(handle, &output),
            HEvent::Closed => state.toplevel.toplevel_closed(handle),
            HEvent::Done => {}
            _ => {}
        }
    }
}

delegate_registry!(BackgroundState);

impl ProvidesRegistryState for BackgroundState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

// Dispatch de wl_surface/wl_callback/wl_buffer/layer-shell: los user-data
// de sctk implementan Dispatch2 contra nuestros traits handler; esta macro
// genera los impls Dispatch requeridos por wayland-client.
delegate_dispatch2!(BackgroundState);
