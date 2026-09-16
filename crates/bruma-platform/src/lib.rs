//! # bruma-platform
//!
//! Capa de plataforma: ventana de fondo en Wayland vía `wlr-layer-shell`.
//!
//! Estado: **Fase 1**. Implementación con `smithay-client-toolkit` 0.21
//! (NO winit: no soporta layer-shell), igual que swww. El banco de pruebas
//! de referencia es **niri** (+ DankMaterialShell), con el resto de
//! compositors wlroots/KWin como best-effort.
//!
//! Demo de la fase: un rectángulo de color sólido detrás de todas las
//! ventanas, anclado a los cuatro bordes, que sobrevive a recargas de
//! configuración y a reconexiones de salida (el compositor re-configura
//! la superficie y aquí se redibuja).
//!
//! NON-GOALS (ver DECISIONS.md): GNOME/Mutter en v1 (sin layer-shell);
//! vídeo y audio en v1.

#![forbid(unsafe_code)]

use bruma_core::Color;
use bruma_renderer::FrameRenderer;
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
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

/// Una ventana de fondo a pantalla completa detrás de todas las ventanas.
///
/// Tipo de alto nivel para el CLI: se construye con un color y el bucle de
/// eventos mantiene la ventana viva, redibujando en cada re-configuración
/// del compositor (recarga de config, cambio de resolución, etc.).
pub struct BackgroundWindow {
    conn: Connection,
    event_queue: EventQueue<BackgroundState>,
    state: BackgroundState,
}

/// Estado delegado de eventos Wayland. Es el dueño de todo lo necesario
/// para dibujar, así los handlers pueden redibujar directamente.
struct BackgroundState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    color: Color,
    width: u32,
    height: u32,
    configure_seen: bool,
    closed: bool,
    /// Renderizador de frames (Fase 2). Si hay uno, pinta él; si no,
    /// se usa el fallback de color sólido de la Fase 1.
    renderer: Option<Box<dyn FrameRenderer>>,
    /// Salidas conocidas; una entrada por pantalla conectada (Fase 5).
    outputs: Vec<OutputInfo>,
}

/// Información mínima de una salida conectada (Fase 1: solo logging;
/// Fase 5: wallpaper por pantalla con posición y escala).
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub name: Option<String>,
    pub logical_size: Option<(i32, i32)>,
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
    /// Error de despacho de eventos.
    #[error("error en el bucle de eventos: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
}

impl BackgroundWindow {
    /// Conecta al compositor (vía entorno) y crea la ventana de fondo en
    /// la capa `Background` de Wayland.
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

        let compositor_state = CompositorState::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("wl_compositor"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("zwlr_layer_shell_v1"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|_| PlatformError::MissingProtocol("wl_shm"))?;

        let surface = compositor_state.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Background,
            Some("bruma"),
            None, // sin output concreto: cubre todas las pantallas (Fase 5 lo refinará)
        );

        // Anclada a los cuatro bordes => ocupa la pantalla completa; el
        // tamaño lo impone el compositor en el configure (se pide 0,0).
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_size(0, 0);
        // Un wallpaper no debe robar el teclado ni el puntero.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        // Commit inicial sin buffer: el compositor responde con un
        // configure que nos da el tamaño real de pantalla.
        layer.commit();

        let pool =
            SlotPool::new(64 * 64 * 4, &shm).map_err(|e| PlatformError::Shm(e.to_string()))?;

        Ok(Self {
            conn,
            event_queue,
            state: BackgroundState {
                registry_state: RegistryState::new(&globals),
                output_state: OutputState::new(&globals, &qh),
                shm,
                pool,
                layer,
                color,
                width: 0,
                height: 0,
                configure_seen: false,
                closed: false,
                renderer: None,
                outputs: Vec::new(),
            },
        })
    }

    /// Instala el renderizador de frames (contrato de
    /// `bruma-renderer`). A partir de entonces pinta él en cada
    /// re-configuración, en vez del color sólido de la Fase 1.
    pub fn set_frame_renderer(&mut self, renderer: Box<dyn FrameRenderer>) {
        self.state.renderer = Some(renderer);
    }

    /// Puntero crudo al `wl_display` de la conexión.
    ///
    /// Para crear la superficie de wgpu (`WgpuRenderer::new_wayland`). El
    /// puntero es válido mientras `BackgroundWindow` viva.
    pub fn display_ptr(&self) -> NonNull<std::ffi::c_void> {
        // En libwayland `wl_display` ES un `wl_proxy` (el mismo puntero);
        // el cast a c_void es lo que esperan wgpu/Vulkan.
        let ptr = self.conn.backend().display_id().as_ptr();
        NonNull::new(ptr).expect("wl_display vivo").cast()
    }

    /// Puntero crudo al `wl_surface` de la ventana de fondo.
    pub fn surface_ptr(&self) -> NonNull<std::ffi::c_void> {
        let ptr = self.state.layer.wl_surface().id().as_ptr();
        NonNull::new(ptr).expect("wl_surface vivo").cast()
    }

    /// Conexión Wayland subyacente (para roundtrips del CLI o tests).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Cambia el color de fondo; surte efecto en el próximo redibujo.
    pub fn set_color(&mut self, color: Color) {
        self.state.color = color;
    }

    /// Tamaño (buffer) del último configure recibido.
    pub fn size(&self) -> (u32, u32) {
        (self.state.width, self.state.height)
    }

    /// Pantallas detectadas hasta ahora.
    pub fn outputs(&self) -> &[OutputInfo] {
        &self.state.outputs
    }

    /// Procesa eventos hasta dibujar el primer frame. Pensado para tests y
    /// verificación: espera el configure inicial y confirma que hay un
    /// buffer committeado, sin bloquearse indefinidamente.
    pub fn present_once(&mut self) -> Result<(u32, u32), PlatformError> {
        self.event_queue.roundtrip(&mut self.state)?;
        if self.state.closed {
            return Err(PlatformError::SurfaceClosed);
        }
        if !self.state.configure_seen || self.state.width == 0 {
            return Err(PlatformError::NotConfigured);
        }
        // Segundo roundtrip: drena enteros/frame events pendientes.
        self.event_queue.roundtrip(&mut self.state)?;
        Ok((self.state.width, self.state.height))
    }

    /// Ejecuta el bucle de eventos hasta que el compositor cierre la
    /// ventana. Ctrl-C termina el proceso (comportamiento por defecto).
    pub fn run(mut self) -> Result<(), PlatformError> {
        while !self.state.closed {
            self.event_queue.blocking_dispatch(&mut self.state)?;
        }
        Ok(())
    }
}

impl BackgroundState {
    /// Fallback de la Fase 1: rellena un buffer ARGB8888 del color
    /// actual, lo daña y lo committea. Se usa cuando no hay renderer.
    fn draw_solid(&mut self) {
        let (width, height) = (self.width, self.height);
        if width == 0 || height == 0 {
            return;
        }
        let stride = width as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            log::error!("no se pudo crear buffer {width}x{height}");
            return;
        }; // ARGB8888 en memoria nativa little-endian => bytes B,G,R,A.
        let px = [self.color.b, self.color.g, self.color.r, self.color.a];
        canvas.as_chunks_mut::<4>().0.iter_mut().for_each(|chunk| {
            *chunk = px;
        });

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(surface).expect("attach buffer");
        self.layer.commit();
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
        // Fase 5: redimensionar buffers por DPI. Con shm el factor no
        // afecta al tamaño en píxeles del buffer lógico completo.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Fase 5: rotación de salida.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Fase 1: color estático, no se anima; los frame callbacks solo
        // llegarían si los solicitáramos (llegará con wgpu en Fase 2).
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        let info = self.output_state.info(output);
        let entry = OutputInfo {
            name: info.as_ref().and_then(|i| i.name.clone()),
            logical_size: info.as_ref().and_then(|i| i.logical_size),
        };
        log::info!("ventana de fondo entra en salida {entry:?}");
        if !self.outputs.iter().any(|o| o.name == entry.name) {
            self.outputs.push(entry);
        }
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
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        // Fase 5: crear una ventana de fondo por salida.
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for BackgroundState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::info!("el compositor cerró la ventana de fondo");
        self.closed = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        // Con anchor a 4 bordes el tamaño lo impone el compositor; si
        // llegara 0 (no debería), usamos un mínimo digno para no morir.
        self.width = w.max(1);
        self.height = h.max(1);
        self.configure_seen = true;
        // Redibujar en CADA configure: así sobrevivimos a recargas de
        // configuración de niri y a cambios de resolución de salida.
        match &mut self.renderer {
            Some(renderer) => renderer.render_frame(self.width, self.height),
            None => self.draw_solid(),
        }
    }
}

impl ShmHandler for BackgroundState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
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
