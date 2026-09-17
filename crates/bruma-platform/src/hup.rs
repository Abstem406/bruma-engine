//! Despertar por SIGHUP (recarga de config en caliente, Fase 5).
//!
//! El bucle de frames duerme en `poll` sobre el socket de Wayland; un
//! SIGHUP interrumpe ese `poll` (EINTR) pero Rust ignora la señal por
//! defecto (instalada como handler vacío) y el bucle no tiene forma de
//! saber POR QUÉ despertó. Este módulo resuelve eso con la pieza mínima:
//!
//! 1. `eventfd(2)`: un fd contador. `write` de 1 byte despierta a quien
//!    haga `poll` sobre él; `read` lo reinicia (drenaje).
//! 2. El handler de SIGHUP (instalado con `sigaction(2)` de libc, espacio
//!    de señales del proceso) hace SOLAMENTE `write(eventfd, 1)` — una
//!    syscall async-signal-safe. Nada de asignar memoria, tomar locks
//!    o llamar a Wayland desde el handler.
//! 3. El bucle añade el eventfd a su `poll` y, si está listo, drena y
//!    dispara el callback de recarga en el hilo normal.
//!
//! Sin hilos, sin self-pipe. libc ya estaba en el árbol (wayland-backend,
//! dbus): cero dependencias nuevas. Nota: NO se usa el módulo `runtime`
//! de rustix porque su API de señales es experimental y su nombre rota
//! entre versiones (rompió el build fuera del workspace).

// Este módulo es el ÚNICO del crate con unsafe (señales + fd crudo):
// cada bloque está justificado en línea y ninguna otra pieza lo usa.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use rustix::event::EventfdFlags;
use rustix::event::eventfd;
use rustix::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

/// Handler global: escribe al eventfd. Solo puede ser UNO por proceso
/// (la señal es global); lo guarda [`HupChannel::install`].
static EVENTFD_FD: AtomicU64 = AtomicU64::new(0);

/// Handler de SIGHUP: async-signal-safe al 100% (una syscall de write).
/// Los errores se ignoran a propósito: si el eventfd murió, no hay nada
/// sensato que hacer desde un handler de señal.
extern "C" fn hup_handler(_sig: std::ffi::c_int) {
    let raw = EVENTFD_FD.load(Ordering::Relaxed);
    if raw != 0 {
        // SAFETY: el fd se instaló vivo y el bucle lo drena; el número
        // permanece válido mientras el proceso viva (OwnedFd lo conserva).
        // La syscall de write es async-signal-safe; nada más ocurre aquí.
        let fd = unsafe { BorrowedFd::borrow_raw(raw as RawFd) };
        let one: u64 = 1;
        let _ = rustix::io::write(fd, &one.to_ne_bytes());
    }
}

/// Acta de `sigaction(2)` lista para pasar a libc: handler + SA_RESTART +
/// máscara vacía. `handler` es un `sighandler_t` (usize en Linux): un
/// puntero de función casteado o `libc::SIG_IGN`.
fn new_sigaction(handler: usize) -> libc::sigaction {
    // SAFETY: sigaction es un struct C plano; cero equivale a campos
    // default (máscara vacía, sin flags), que luego se ajustan.
    let mut act: libc::sigaction = unsafe { std::mem::zeroed() };
    act.sa_sigaction = handler;
    act.sa_flags = libc::SA_RESTART;
    // Máscara vacía explícita: durante el handler no se bloquea ninguna
    // señal extra más allá de la propia SIGHUP (que POSIX bloquea sola).
    let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::sigemptyset(&mut mask) };
    debug_assert_eq!(r, 0);
    act.sa_mask = mask;
    act
}

/// Canal SIGHUP → bucle. Cero o uno por proceso.
pub struct HupChannel {
    fd: OwnedFd,
}

impl HupChannel {
    /// Crea el eventfd e instala el handler de SIGHUP. Idempotente en
    /// efecto: un segundo `install` reemplaza al primero (mismo fd de
    /// escritura si es el mismo canal).
    pub fn install() -> rustix::io::Result<Self> {
        let fd = eventfd(0, EventfdFlags::empty())?;
        let raw = fd.as_raw_fd() as u64;
        EVENTFD_FD.store(raw, Ordering::Relaxed);

        let act = new_sigaction(hup_handler as *const () as usize);
        // SAFETY: handler extern "C" estático y struct válido.
        let r = unsafe { libc::sigaction(libc::SIGHUP, &act, std::ptr::null_mut()) };
        if r == -1 {
            return Err(rustix::io::Errno::from_raw_os_error(unsafe {
                *libc::__errno_location()
            }));
        }

        Ok(Self { fd })
    }

    /// Fd para añadir al `poll` del bucle.
    pub fn poll_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// ¿Señal pendiente? Drena el contador y responde. NUNCA bloquea:
    /// `read` directo sobre un eventfd en 0 BLOQUEA hasta la próxima
    /// señal (el bucle llama a `drain` tras cada poll, señal o timeout,
    /// y quedó congelado ahí), así que se consulta con `poll` de timeout
    /// CERO y solo se lee si está listo.
    ///
    /// Un solo drenaje basta: eventfd es un contador, no una cola — si
    /// llegaron 3 SIGHUP seguidos, una recarga cubre N señales.
    pub fn drain(&self) -> bool {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
        let cero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        match poll(&mut fds, Some(&cero)) {
            Ok(n) if n > 0 => {
                let mut buf = [0u8; 8];
                matches!(rustix::io::read(&self.fd, &mut buf), Ok(8))
            }
            _ => false,
        }
    }
}

impl Drop for HupChannel {
    fn drop(&mut self) {
        // Retira el handler: vuelve a SIG_IGN, la disposición con la que
        // Rust arranca el proceso (SIGHUP ignorado). Restaurar SIG_DFL
        // aquí mataría el proceso con el próximo HUP.
        // SAFETY: struct válido (cero + máscara vacía); stdout de errores
        // ignorado: en Drop no hay nada sensato que hacer.
        unsafe {
            let ign = new_sigaction(libc::SIG_IGN);
            let _ = libc::sigaction(libc::SIGHUP, &ign, std::ptr::null_mut());
        }
        EVENTFD_FD.store(0, Ordering::Relaxed);
    }
}
