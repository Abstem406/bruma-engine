//! # bruma-platform
//!
//! Platform layer: background windows on Wayland via `wlr-layer-shell`.
//!
//! Status: **Phase 5**. Implemented with `smithay-client-toolkit` 0.20
//! (NOT winit: it lacks layer-shell support), same as swww. The reference
//! test bench is **niri** (+ DankMaterialShell), with the rest of the
//! wlroots/KWin compositors as best-effort.
//!
//! There is **one background surface per connected output** (Phase 5): a
//! surface without a concrete output only covers one screen on most
//! compositors. Surfaces are born with output hotplug (`new_output`), die
//! with it (`output_destroyed`/`closed`), and each one's renderer is
//! decided by a **factory** injected from the CLI (the platform knows
//! nothing about GPUs: D3/D6 boundary).
//!
//! The event loop has two modes: `run` (Wayland events only; the CPU goes
//! idle with static content) and `run_with_runtime` (also drives the
//! animation at the pace the `WallpaperRuntime` asks for, sleeping in
//! `poll` on the socket — never spinning) painting ALL outputs.
//!
//! NON-GOALS (see DECISIONS.md): GNOME/Mutter in v1 (no layer-shell);
//! video and audio in v1.

// deny and not forbid: the SIGHUP channel (hup.rs) needs exactly the
// unsafe signal/fd operations no safe API covers; they are isolated there
// with a targeted allow and the rest of the crate keeps it that way.
#![deny(unsafe_code)]

mod clock;
mod hup;
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
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
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

/// A background window behind every window, **one per output**.
///
/// High-level type for the CLI: builds a surface for each known output
/// (and for the ones connecting later), and the event loop keeps them
/// alive, repainting on every compositor re-configuration (config reload,
/// resolution change, etc.).
pub struct BackgroundWindow {
    /// Connection handle (inner Arc): needed for `display_ptr`, the
    /// socket fd and CLI roundtrips.
    conn: Connection,
    event_queue: EventQueue<BackgroundState>,
    state: BackgroundState,
}

/// Delegated Wayland event state. It owns everything needed to draw, so
/// handlers can repaint directly.
struct BackgroundState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    color: Color,
    closed: bool,
    /// One entry per connected output (Phase 5). Each with its own
    /// layer-shell surface, size and renderer.
    outputs: Vec<OutputEntry>,
    /// Renderer factory per surface (Phase 5). Injected by the CLI; the
    /// platform only calls it with the new surface's raw pointers — it
    /// knows nothing about GPUs (D3/D6).
    factory: Option<SurfaceRendererFactory>,
    /// Fullscreen window tracking per output (D12 pause). Binding the
    /// manager is optional: without the protocol there is no pause and
    /// everything goes on as usual.
    toplevel: toplevel::ToplevelTracker,
    /// The bound manager (if the protocol exists). It must be kept alive:
    /// dropping it makes the compositor stop announcing toplevels.
    _toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    /// Last pause state per output (index into the outputs Vec): so we
    /// log transitions only, not every frame.
    last_pause: Vec<bool>,
    /// Global pause (D12): session lock and battery, best-effort over the
    /// system D-Bus. Without a bus, never pauses.
    pause: pause::SessionPauseWatcher,
    /// Last global pause state, for transition logs.
    last_global_pause: bool,
    /// SIGHUP→eventfd channel (hot config reload). `None` if installation
    /// failed (rare: no eventfd would be an exotic kernel); then `bruma`
    /// does not reload on signal and nothing else changes.
    hup: Option<hup::HupChannel>,
    /// Config reload callback (installed by the CLI): `true` = there was
    /// a reload and the loop rebuilds renderers and repaints.
    on_hup: Option<Box<dyn FnMut() -> bool + 'static>>,
    /// Pointer position on the surface it currently hovers (LOGICAL
    /// coordinates), updated from wl_pointer events. `None` = the cursor
    /// is not over any of our surfaces (shaders get (-1, -1) = unknown).
    mouse: Option<(f64, f64)>,
    /// SCTK seat state: tracks seats and their capabilities; owner of
    /// the wl_pointer objects (bound on the pointer capability).
    seat_state: SeatState,
}

/// A background surface on one concrete output.
///
/// `width`/`height` are the LOGICAL size from the configure; buffer
/// pixels (what gets drawn) come from [`buffer_pixels`] with the scale.
struct OutputEntry {
    /// Output proxy (to match `update_output`/`output_destroyed` with its
    /// entry, and to query toplevel fullscreen).
    output: Option<wl_output::WlOutput>,
    /// Declarative info (name, logical size) for logs and API.
    info: OutputInfo,
    layer: LayerSurface,
    width: u32,
    height: u32,
    /// Integer buffer scale announced by the compositor (1 by default;
    /// with fractional scale niri rounds up).
    scale: u32,
    configured: bool,
    /// THIS output's renderer (built by the factory). `None` if the
    /// factory failed or there is none: solid color fallback.
    renderer: Option<Box<dyn FrameRenderer>>,
    /// THIS output's color for the SHM fallback (from the factory).
    /// `None` uses the window's global color (previous behavior).
    fallback_color: Option<Color>,
}

/// Buffer pixels for a logical size and an integer scale.
pub fn buffer_pixels(logical: (u32, u32), scale: u32) -> (u32, u32) {
    (
        logical.0.saturating_mul(scale),
        logical.1.saturating_mul(scale),
    )
}

/// Minimal info about a connected output.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub name: Option<String>,
    pub logical_size: Option<(i32, i32)>,
}

/// Raw pointers + identity of a freshly created background surface. It is
/// all the platform hands the factory: with this (and the output name)
/// the CLI can build its renderer.
pub struct OutputSurfaceHandles {
    pub display_ptr: NonNull<std::ffi::c_void>,
    pub surface_ptr: NonNull<std::ffi::c_void>,
    pub output_name: Option<String>,
}

/// Factory result for ONE output (Phase 5): renderer, fallback color, or
/// both.
///
/// - `Some(renderer)`: the output paints with the GPU.
/// - `None` + `Some(color)`: the output paints that color via SHM
///   (independent of `BackgroundWindow::new`'s global color).
/// - `Err`: the output falls back to the global color (previous behavior).
pub struct FactoryRenderer {
    pub renderer: Option<Box<dyn FrameRenderer>>,
    pub color: Option<Color>,
}

impl FactoryRenderer {
    pub fn renderer(r: Box<dyn FrameRenderer>) -> Self {
        Self {
            renderer: Some(r),
            color: None,
        }
    }

    pub fn color(c: Color) -> Self {
        Self {
            renderer: None,
            color: Some(c),
        }
    }

    pub fn with_color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

impl From<Box<dyn FrameRenderer>> for FactoryRenderer {
    fn from(r: Box<dyn FrameRenderer>) -> Self {
        Self::renderer(r)
    }
}

/// Renderer factory per surface. Returns `Err` with the reason when it
/// could not be built; that output degrades to solid color (with a log)
/// and the others go on — a desktop with N screens does not die for one.
pub type SurfaceRendererFactory =
    Box<dyn FnMut(&OutputSurfaceHandles) -> Result<FactoryRenderer, String>>;

/// Per-output report after startup (for the CLI's log).
#[derive(Debug, Clone)]
pub struct OutputReport {
    pub name: Option<String>,
    /// Drawn buffer size (logical × scale), in pixels.
    pub width: u32,
    pub height: u32,
    /// Output's integer scale (DPI).
    pub scale: u32,
    /// Does it have a factory renderer (true) or a color fallback (false)?
    pub gpu: bool,
}

/// Platform layer errors.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// Could not connect to the Wayland compositor.
    #[error("could not connect to the Wayland compositor: {0}")]
    Connect(String),
    /// The compositor does not support a required protocol.
    #[error(
        "the compositor does not support {0} (Wayland with wlr-layer-shell? niri, sway, Hyprland and KWin support it; GNOME does not)"
    )]
    MissingProtocol(&'static str),
    /// Shared memory error (wl_shm).
    #[error("wl_shm error: {0}")]
    Shm(String),
    /// The compositor closed the surface.
    #[error("the compositor closed the background surface")]
    SurfaceClosed,
    /// The compositor never configured the surface.
    #[error("the compositor never configured the background surface")]
    NotConfigured,
    /// There is no connected output.
    #[error("no connected outputs")]
    NoOutputs,
    /// Event dispatch error.
    #[error("event loop error: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    /// I/O error waiting for events (socket flush/poll/read).
    #[error("error waiting for events: {0}")]
    Poll(String),
}

impl BackgroundWindow {
    /// Connects to the compositor (via the environment) and prepares the
    /// background. Per-output surfaces create themselves on the first
    /// roundtrip (`new_output`), already with the factory installed if it
    /// was installed BEFORE the first `present_once`.
    pub fn new(color: Color) -> Result<Self, PlatformError> {
        let conn =
            Connection::connect_to_env().map_err(|e| PlatformError::Connect(e.to_string()))?;
        Self::for_connection(conn, color)
    }

    /// Same as [`Self::new`] but over an already established connection.
    pub fn for_connection(conn: Connection, color: Color) -> Result<Self, PlatformError> {
        let (globals, event_queue) =
            registry_queue_init(&conn).map_err(|e| PlatformError::Connect(e.to_string()))?;
        let qh = event_queue.handle();

        // Protocol states live in the state: `new_output` needs them to
        // create surfaces on the fly (hotplug).
        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("wl_compositor"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|_| PlatformError::MissingProtocol("zwlr_layer_shell_v1"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|_| PlatformError::MissingProtocol("wl_shm"))?;

        // Fullscreen pause (D12): optional per protocol.
        let toplevel_mgr = globals
            .bind::<
                wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
                BackgroundState,
                (),
            >(&qh, 1..=3, ())
            .ok();
        if toplevel_mgr.is_none() {
            log::info!("no wlr-foreign-toplevel: the fullscreen pause is unavailable");
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
                hup: hup::HupChannel::install().ok(),
                on_hup: None,
                mouse: None,
                seat_state: SeatState::new(&globals, &qh),
            },
        })
    }

    /// Installs the per-surface renderer factory (Phase 5). Must be called
    /// BEFORE the first `present_once` so startup surfaces are born with
    /// their renderer.
    pub fn set_renderer_factory(&mut self, factory: SurfaceRendererFactory) {
        self.state.factory = Some(factory);
    }

    /// Registers the config reload handler (SIGHUP): invoked on the loop's
    /// thread; returning `true` rebuilds the renderers via the factory and
    /// repaints. Without a handler, SIGHUP only wakes the loop.
    pub fn on_config_reload(&mut self, f: Box<dyn FnMut() -> bool + 'static>) {
        self.state.on_hup = Some(f);
    }

    /// Installs ONE renderer shared by all outputs... it can't be done:
    /// each output needs its own GPU surface. This method replaces the old
    /// `set_frame_renderer`: it wraps the given renderer in a factory that
    /// clones the build model.
    ///
    /// NOTE: `FrameRenderer` is not `Clone`; the factory only makes sense
    /// building a NEW renderer per surface (that's why
    /// [`Self::set_renderer_factory`] exists).
    pub fn set_frame_renderer(&mut self, renderer: Box<dyn FrameRenderer>) {
        // Honest degradation: the first output gets the renderer as is
        // (backward compat with phases 1-4); the rest stay on solid color
        // with a clear warning.
        let mut taken = Some(renderer);
        self.state.factory = Some(Box::new(move |_handles| {
            if let Some(r) = taken.take() {
                Ok(FactoryRenderer::renderer(r))
            } else {
                Err("set_frame_renderer provides a single renderer: \
                     use set_renderer_factory for multi-output"
                    .to_owned())
            }
        }));
    }

    /// Does any installed renderer produce animated content? The CLI
    /// queries it to choose between `run` and `run_with_runtime`.
    pub fn wants_animation(&self) -> bool {
        self.state
            .outputs
            .iter()
            .any(|o| o.renderer.as_ref().is_some_and(|r| r.wants_animation()))
    }

    /// Raw pointer to the connection's `wl_display`.
    pub fn display_ptr(&self) -> NonNull<std::ffi::c_void> {
        // In libwayland `wl_display` IS a `wl_proxy` (the same pointer);
        // the cast to c_void is what wgpu/Vulkan expect.
        let ptr = self.conn.backend().display_id().as_ptr();
        NonNull::new(ptr).expect("live wl_display").cast()
    }

    /// Changes the background color; takes effect on the next repaint.
    pub fn set_color(&mut self, color: Color) {
        self.state.color = color;
    }

    /// Screens detected so far.
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.state.outputs.iter().map(|o| o.info.clone()).collect()
    }

    /// Processes events until ALL known outputs are configured. Returns a
    /// report per output (size and whether it paints with the GPU or the
    /// color fallback).
    pub fn present_once(&mut self) -> Result<Vec<OutputReport>, PlatformError> {
        // First roundtrip: globals + surface creation + configures.
        self.event_queue.roundtrip(&mut self.state)?;
        if self.state.closed {
            return Err(PlatformError::SurfaceClosed);
        }
        // Outputs may keep arriving (early hotplug); newcomers need their
        // configure. An extra roundtrip drains the pending ones.
        self.event_queue.roundtrip(&mut self.state)?;
        if self.state.outputs.is_empty() {
            return Err(PlatformError::NoOutputs);
        }
        let reports: Vec<OutputReport> = self
            .state
            .outputs
            .iter()
            .map(|o| {
                let (bw, bh) = buffer_pixels((o.width, o.height), o.scale);
                OutputReport {
                    name: o.info.name.clone(),
                    width: bw,
                    height: bh,
                    scale: o.scale,
                    gpu: o.renderer.is_some(),
                }
            })
            .collect();
        if reports.iter().any(|r| r.width == 0) {
            return Err(PlatformError::NotConfigured);
        }
        Ok(reports)
    }

    /// Rebuilds each output's renderer/color via the factory and repaints
    /// them (hot config reload): the factory decides per output; if it
    /// fails, that output falls back to its fallback color.
    pub fn reload_renderers(&mut self) {
        let conn = self.conn.clone();
        for idx in 0..self.state.outputs.len() {
            let Some(output) = self.state.outputs[idx].output.clone() else {
                continue;
            };
            let layer = self.state.outputs[idx].layer.clone();
            let (renderer, fallback_color) = self.state.build_renderer(&conn, &layer, &output);
            self.state.outputs[idx].renderer = renderer;
            self.state.outputs[idx].fallback_color = fallback_color;
            // Repaint NOW (static config: with no frame callback the pixel
            // would not change until the next configure/event).
            self.state.draw_entry(idx);
        }
    }

    /// Runs the event loop until the compositor closes the background.
    /// Ctrl-C ends the process (default behavior).
    ///
    /// Static mode: only attends Wayland events (configure, frame
    /// callbacks...); between events, `blocking_dispatch`-style waiting
    /// sleeps without spinning the CPU.
    pub fn run(&mut self) -> Result<(), PlatformError> {
        // Static loop: indefinite wait (the SIGHUP channel wakes the poll
        // for config reload).
        while !self.state.closed {
            self.wait_and_dispatch(None)?;
        }
        Ok(())
    }

    /// Like [`Self::run`], but also driving the animation with the given
    /// runtime (Phase 3) on ALL outputs (Phase 5).
    ///
    /// The runtime dictates the pace: on each iteration it is asked
    /// whether it is time to draw ([`WallpaperRuntime::begin_frame`]); if
    /// not, the loop sleeps in `poll` until the runtime's deadline or
    /// until a compositor event arrives — whichever comes first. No
    /// spinning.
    pub fn run_with_runtime(
        &mut self,
        runtime: &mut dyn WallpaperRuntime,
    ) -> Result<(), PlatformError> {
        while !self.state.closed {
            // Global pause (D12): drain lock/battery signals and, if any,
            // NO output is repainted (the wallpaper is covered by the
            // lock screen, or we save battery). Like fullscreen: the
            // runtime is NOT paused — global time goes on and on unpause
            // the animation resumes without a jump.
            self.state.pause.poll();
            let global_pause = self.state.pause.paused();
            if global_pause != self.state.last_global_pause {
                log::info!(
                    "global pause {}: engine {} repainting (lock/battery)",
                    if global_pause { "ON" } else { "OFF" },
                    if global_pause { "stops" } else { "resumes" }
                );
                self.state.last_global_pause = global_pause;
            }

            // Wake deadline: ALWAYS bounded. After drawing, the second
            // begin_frame returns Skip with the next tick's deadline (it
            // does not touch time: now-last < interval). Without this,
            // the poll after the Draw would be indefinite and the
            // animation would only advance on compositor events — latent
            // bug since Phase 3, masked by the constant traffic of a
            // desktop in use.
            let deadline = match runtime.begin_frame(Instant::now()) {
                FrameDecision::Draw => {
                    // The real resolution is imposed by each surface (its
                    // last configure); the runtime contributes time, mouse
                    // and parameters. One runtime frame paints all outputs
                    // with the SAME instant: the animation stays
                    // synchronized across monitors.
                    //
                    // Per-output pause (D12): if a fullscreen window sits
                    // on the output, its wallpaper freezes (not repainted)
                    // — the last buffer stays on screen, in the
                    // compositor's hands. Other outputs keep animating.
                    // The runtime is NOT paused: global time keeps running
                    // and on leaving fullscreen the animation resumes
                    // where it was (no jump).
                    let mut st = runtime.state();
                    // Global pause active: NO output is repainted (the
                    // last buffer stays on screen, in the compositor's
                    // hands, like per-output fullscreen).
                    for idx in 0..self.state.outputs.len() {
                        if global_pause {
                            break;
                        }
                        let (w, h) = buffer_pixels(
                            (
                                self.state.outputs[idx].width,
                                self.state.outputs[idx].height,
                            ),
                            self.state.outputs[idx].scale,
                        );
                        if w == 0 || h == 0 {
                            continue;
                        }
                        st.width = w;
                        st.height = h;
                        // Real-time clock for shaders (day/night tints,
                        // clock wallpapers). Fresh on every animated
                        // frame; one libc call, negligible next to the
                        // render.
                        st.clock = clock::local_hms();
                        // Pointer position (Phase 6 `mouse` permission):
                        // BUFFER pixel coordinates (logical position ×
                        // output scale — the same space u_res speaks),
                        // while the cursor hovers one of our surfaces;
                        // (-1, -1) = unknown otherwise.
                        match self.state.mouse {
                            Some((mx, my)) => {
                                st.mouse_x = (mx * self.state.outputs[idx].scale as f64) as f32;
                                st.mouse_y = (my * self.state.outputs[idx].scale as f64) as f32;
                            }
                            None => {
                                st.mouse_x = -1.0;
                                st.mouse_y = -1.0;
                            }
                        }
                        let fullscreened = self.state.outputs[idx]
                            .output
                            .as_ref()
                            .is_some_and(|o| self.state.toplevel.is_fullscreen_on(o));
                        // Diagnostics: log only on transition (not per
                        // frame).
                        if self
                            .state
                            .last_pause
                            .get(idx)
                            .is_none_or(|prev| *prev != fullscreened)
                        {
                            log::info!(
                                "output {:?}: fullscreen pause = {}",
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
                        // No fps cap (exotic runtime): yield the thread
                        // with timeout 0 instead of spinning dry.
                        FrameDecision::Draw => Some(Instant::now()),
                    }
                }
                FrameDecision::Skip { deadline } => Some(deadline),
            };
            self.wait_and_dispatch(deadline)?;
        }
        Ok(())
    }

    /// Dispatches what is pending and, if there was nothing, waits on the
    /// Wayland socket until `timeout` or until events arrive (whichever
    /// comes first). It mirrors `EventQueue::blocking_dispatch`'s inner
    /// loop, with a timeout added:
    ///
    /// 1. `dispatch_pending`: dispatch what was already read.
    /// 2. `flush`: send pending requests (e.g. frame callbacks).
    /// 3. `prepare_read` + `poll` + `read`: arm the read and sleep.
    ///    `prepare_read` is mandatory to not lose events arriving between
    ///    `dispatch_pending` and `poll`.
    /// 4. Final `dispatch_pending`: hand out what was just read.
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

        // SIGHUP pending during this poll (drained below, inside the
        // guard block).
        let mut hup = false;
        if let Some(guard) = self.event_queue.prepare_read() {
            // The Backend handle is a cheap Arc; BorrowedFd borrows from
            // it, so the handle lives in this block. The SIGHUP channel's
            // eventfd is added if it exists: its poll_fd is valid while
            // the channel lives (a field of the state itself).
            let backend = self.conn.backend();
            let fd = backend.poll_fd();
            let mut fds: Vec<PollFd<'_>> = vec![PollFd::new(&fd, PollFlags::IN)];
            let hup_borrow = self.state.hup.as_ref().map(|c| c.poll_fd());
            if let Some(h) = &hup_borrow {
                fds.push(PollFd::new(h, PollFlags::IN));
            }
            let mut wayland_ready = false;
            match poll(&mut fds, wait.as_ref()) {
                // ready == 0: the timeout expired (animation tick).
                // ready > 0: data on one of the two fds.
                Ok(_) => {
                    wayland_ready = fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP);
                }
                // EINTR (signals like SIGINT): not an error for the loop.
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(PlatformError::Poll(e.to_string())),
            }
            hup = self.state.hup.as_ref().is_some_and(|c| c.drain());
            // The read slot is ONLY consumed with data: `read` with an
            // empty socket BLOCKS until the first event (and after an
            // animation timeout that is the common case). With a pending
            // SIGHUP it is dropped without reading: the reload rebuilds
            // wgpu renderers and the driver does Wayland roundtrips that
            // need the slot free (otherwise it waits on itself: deadlock,
            // seen in a core dump inside wl_display_read_events).
            if wayland_ready && !hup {
                guard
                    .read()
                    .map_err(|e| PlatformError::Poll(e.to_string()))?;
            }
        }
        // Reload OUTSIDE the read guard (see above: driver roundtrips
        // while building renderers). Events left on the socket are read
        // on the next loop turn.
        if hup && self.state.on_hup.as_mut().is_some_and(|f| f()) {
            self.reload_renderers();
        }

        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}

impl BackgroundState {
    /// Full registration cycle for an output: layer-shell surface,
    /// renderer via factory (with the real pointers already created) and
    /// registration. The renderer is built BEFORE the first configure so
    /// it paints the first frame itself (phases 2-4 pattern).
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
            Some(output), // << THE concrete output (the key to Phase 5)
        );
        // Anchored to all four edges => covers the whole screen; the
        // compositor imposes the size on configure (0,0 is requested).
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_size(0, 0);
        // A wallpaper must not steal the keyboard or the pointer.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // Initial scale (to be confirmed by preferred_buffer_scale via
        // `scale_factor_changed`): the buffer is drawn in physical
        // pixels, sharp on HiDPI. Must be set BEFORE the first commit.
        let scale = self
            .output_state
            .info(output)
            .map(|i| i.scale_factor.max(1) as u32)
            .unwrap_or(1);
        layer.wl_surface().set_buffer_scale(scale as i32);
        layer.commit();

        // Per-output renderer, with the raw pointers of THE freshly
        // created surface. The factory may yield a renderer, its own
        // fallback color, both, or fail (global color): a desktop of N
        // screens does not die for one.
        let (renderer, fallback_color) = self.build_renderer(conn, &layer, output);

        log::info!("background surface created on output {:?}", entry_info.name);
        self.outputs.push(OutputEntry {
            output: Some(output.clone()),
            info: entry_info,
            layer,
            width: 0,
            height: 0,
            scale,
            configured: false,
            renderer,
            fallback_color,
        });
    }

    /// Builds an output's renderer via the factory (if any).
    fn build_renderer(
        &mut self,
        conn: &Connection,
        layer: &LayerSurface,
        output: &wl_output::WlOutput,
    ) -> (Option<Box<dyn FrameRenderer>>, Option<Color>) {
        let Some(factory) = self.factory.as_mut() else {
            return (None, None);
        };
        let display_ptr = {
            let ptr = conn.backend().display_id().as_ptr();
            NonNull::new(ptr).expect("live wl_display").cast()
        };
        let surface_ptr = {
            let ptr = layer.wl_surface().id().as_ptr();
            NonNull::new(ptr).expect("live wl_surface").cast()
        };
        let info = self.output_state.info(output);
        let handles = OutputSurfaceHandles {
            display_ptr,
            surface_ptr,
            output_name: info.as_ref().and_then(|i| i.name.clone()),
        };
        match factory(&handles) {
            Ok(fr) => (fr.renderer, fr.color),
            Err(e) => {
                log::warn!(
                    "output {:?} without GPU renderer (color fallback): {e}",
                    handles.output_name
                );
                (None, None)
            }
        }
    }

    /// Applies an entry's integer buffer scale: calls `set_buffer_scale`
    /// and records it on the entry (`draw_entry` computes the buffer size
    /// with [`buffer_pixels`]). Only acts on a real transition.
    fn apply_scale(&mut self, idx: usize, scale: u32) {
        let scale = scale.max(1);
        if self.outputs[idx].scale == scale {
            return;
        }
        log::info!(
            "output {:?}: buffer scale {}→{}",
            self.outputs[idx].info.name,
            self.outputs[idx].scale,
            scale
        );
        self.outputs[idx].scale = scale;
        self.outputs[idx]
            .layer
            .wl_surface()
            .set_buffer_scale(scale as i32);
    }

    /// Finds the entry owning a layer-shell surface.
    fn entry_by_surface(&mut self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.outputs
            .iter()
            .position(|o| o.layer.wl_surface().id() == surface.id())
    }

    /// Phase 1 fallback: fills an ARGB8888 buffer with the given color,
    /// damages it and commits it, for the given entry.
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
            log::error!("could not create buffer {width}x{height}");
            return;
        }; // ARGB8888 in native little-endian memory => bytes B,G,R,A.
        let px = [color.b, color.g, color.r, color.a];
        canvas.as_chunks_mut::<4>().0.iter_mut().for_each(|chunk| {
            *chunk = px;
        });

        let surface = entry.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(surface).expect("attach buffer");
        entry.layer.commit();
    }

    /// Draws an entry (renderer if any; color otherwise), in buffer
    /// pixels (logical size × scale).
    fn draw_entry(&mut self, idx: usize) {
        let (w, h) = buffer_pixels(
            (self.outputs[idx].width, self.outputs[idx].height),
            self.outputs[idx].scale,
        );
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

    /// Color fallback for entry idx (use when there is NO renderer).
    fn draw_entry_solid(&mut self, idx: usize) {
        let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
        let color = self.outputs[idx].fallback_color.unwrap_or(self.color);
        let entry = &mut self.outputs[idx];
        Self::draw_solid_entry(entry, &mut self.pool, w, h, &color);
    }
}

impl CompositorHandler for BackgroundState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        // Per-output DPI: the buffer switches to physical pixels
        // (logical × scale). Repaint right away: a new configure is not
        // guaranteed (the logical size did not change).
        if let Some(idx) = self.entry_by_surface(surface) {
            self.apply_scale(idx, new_factor.max(1) as u32);
            self.draw_entry(idx);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Phase 5 (future): output rotation.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Static color does not animate; the wgpu renderer presents on
        // its own (it does not request compositor frame callbacks).
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // With one surface per output, `enter` is trivially its own.
        // Output logging lives in `create_surface_for_output`.
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
        // HOTPLUG (Phase 5): new output => new background surface.
        self.create_surface_for_output(conn, qh, &output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // The output's info (name/size) arrived or changed.
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
                    "output renamed: {:?} → {:?}",
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
        // HOTPLUG: the output is gone => its surface dies with it.
        let before = self.outputs.len();
        self.outputs
            .retain(|o| o.output.as_ref().is_none_or(|x| x.id() != output.id()));
        if self.outputs.len() != before {
            log::info!("output removed; remaining surfaces: {}", self.outputs.len());
        }
        if self.outputs.is_empty() {
            log::info!("no outputs: the background ends");
            self.closed = true;
        }
    }
}

impl LayerShellHandler for BackgroundState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        // The compositor closed ONE surface (e.g. its output went away).
        if let Some(idx) = self.entry_by_surface(layer.wl_surface()) {
            let name = self.outputs[idx].info.name.clone();
            self.outputs.remove(idx);
            log::info!(
                "surface closed by the compositor ({name:?}); remaining: {}",
                self.outputs.len()
            );
        }
        if self.outputs.is_empty() {
            log::info!("the compositor closed the whole background");
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
        // With the 4-edge anchor the compositor imposes the size; if 0
        // ever arrived (it shouldn't), we use a dignified minimum rather
        // than dying.
        let entry = &mut self.outputs[idx];
        entry.width = w.max(1);
        entry.height = h.max(1);
        entry.configured = true;
        // Repaint on EVERY configure: this is how we survive niri config
        // reloads and output resolution changes.
        self.draw_entry(idx);
    }
}

impl ShmHandler for BackgroundState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

// Dispatch of the foreign-toplevel protocol (fullscreen pause, D12).
// wayland-client requires the impls on the queue-owning state.
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
                    "foreign-toplevel withdrawn by the compositor; fullscreen pause disabled"
                );
            }
            _ => {}
        }
    }

    // The `toplevel` event (opcode 0) CREATES a child object (the
    // window's handle): wayland-client requires declaring its user-data
    // here, or the dispatcher panics at runtime. Handles use () as
    // user-data, like the manager.
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
                // The protocol's array is a list of raw u32s (`state`
                // enum); wayland-client delivers it as a Vec<u8>.
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
    registry_handlers![OutputState, SeatState];
}

// Pointer input (Phase 6 `mouse` permission): seat capability tracking
// (grab a wl_pointer per seat with the pointer capability) and motion
// events over our background surfaces. The layer-shell surface does NOT
// receive the pointer while windows cover it, so the shader sees real
// coordinates only on empty desktop — the best Wayland can offer a
// background surface.
impl SeatHandler for BackgroundState {
    fn seat_state(&mut self) -> &mut SeatState {
        // SeatState is stored in the BackgroundState struct (see below).
        &mut self.seat_state
    }

    fn new_seat(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wayland_client::protocol::wl_seat::WlSeat,
    ) {
    }

    fn remove_seat(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wayland_client::protocol::wl_seat::WlSeat,
    ) {
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wayland_client::protocol::wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Ok(_pointer) = self.seat_state().get_pointer(qh, &seat)
        {
            // The WlPointer object stays alive as long as the SeatState
            // hands it out; sctk keeps its own refcount. Motion events
            // flow into `pointer_frame` from now on.
            log::info!("pointer capability bound (mouse position available)");
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wayland_client::protocol::wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.mouse = None;
            log::info!("pointer capability removed");
        }
    }
}

impl PointerHandler for BackgroundState {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.mouse = Some(event.position);
                }
                PointerEventKind::Leave { .. } => {
                    self.mouse = None;
                }
                _ => {}
            }
        }
    }
}

// Dispatch of wl_surface/wl_callback/wl_buffer/layer-shell: sctk's
// user-data implements Dispatch2 against our handler traits; this macro
// generates the Dispatch impls wayland-client requires.
delegate_dispatch2!(BackgroundState);

#[cfg(test)]
mod tests {
    use super::buffer_pixels;

    #[test]
    fn scale_1_keeps_logical_size() {
        assert_eq!(buffer_pixels((1920, 1200), 1), (1920, 1200));
    }

    #[test]
    fn scale_2_doubles_pixels() {
        assert_eq!(buffer_pixels((1920, 1200), 2), (3840, 2400));
    }

    #[test]
    fn zero_size_stays_zero() {
        assert_eq!(buffer_pixels((0, 1200), 2), (0, 2400));
    }
}
