//! SIGHUP wake-up (hot config reload, Phase 5).
//!
//! The frame loop sleeps in `poll` on the Wayland socket; a SIGHUP
//! interrupts that `poll` (EINTR) but Rust ignores the signal by default
//! (installed as an empty handler) and the loop has no way of knowing WHY
//! it woke up. This module solves that with the minimal piece:
//!
//! 1. `eventfd(2)`: a counter fd. Writing 1 byte wakes whoever `poll`s on
//!    it; reading resets it (drain).
//! 2. The SIGHUP handler (installed with libc's `sigaction(2)`, process
//!    signal space) does ONLY `write(eventfd, 1)` — one async-signal-safe
//!    syscall. No allocation, no locks, no Wayland from the handler.
//! 3. The loop adds the eventfd to its `poll` and, if ready, drains it and
//!    fires the reload callback on the normal thread.
//!
//! No threads, no self-pipe. libc was already in the tree
//! (wayland-backend, dbus): zero new dependencies. Note: rustix's
//! `runtime` module is NOT used because its signal API is experimental
//! and its name rotates between versions (it broke the build outside the
//! workspace).

// This module is the ONLY one in the crate with unsafe (signals + raw
// fd): every block is justified inline and nothing else uses it.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use rustix::event::EventfdFlags;
use rustix::event::eventfd;
use rustix::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

/// Global handler: writes to the eventfd. Only ONE per process is allowed
/// (the signal is global); [`HupChannel::install`] stores it.
static EVENTFD_FD: AtomicU64 = AtomicU64::new(0);

/// SIGHUP handler: 100% async-signal-safe (one write syscall). Errors are
/// ignored on purpose: if the eventfd died, there is nothing sensible to
/// do from a signal handler.
extern "C" fn hup_handler(_sig: std::ffi::c_int) {
    let raw = EVENTFD_FD.load(Ordering::Relaxed);
    if raw != 0 {
        // SAFETY: the fd was installed alive and the loop drains it; the
        // number stays valid while the process lives (OwnedFd keeps it).
        // The write syscall is async-signal-safe; nothing else happens
        // here.
        let fd = unsafe { BorrowedFd::borrow_raw(raw as RawFd) };
        let one: u64 = 1;
        let _ = rustix::io::write(fd, &one.to_ne_bytes());
    }
}

/// A `sigaction(2)` record ready to hand to libc: handler + SA_RESTART +
/// empty mask. `handler` is a `sighandler_t` (usize on Linux): a cast
/// function pointer or `libc::SIG_IGN`.
fn new_sigaction(handler: usize) -> libc::sigaction {
    // SAFETY: sigaction is a plain C struct; zero equals default fields
    // (empty mask, no flags), adjusted afterwards.
    let mut act: libc::sigaction = unsafe { std::mem::zeroed() };
    act.sa_sigaction = handler;
    act.sa_flags = libc::SA_RESTART;
    // Explicit empty mask: the handler blocks no extra signals beyond
    // SIGHUP itself (which POSIX blocks on its own).
    let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::sigemptyset(&mut mask) };
    debug_assert_eq!(r, 0);
    act.sa_mask = mask;
    act
}

/// SIGHUP → loop channel. Zero or one per process.
pub struct HupChannel {
    fd: OwnedFd,
}

impl HupChannel {
    /// Creates the eventfd and installs the SIGHUP handler. Effectively
    /// idempotent: a second `install` replaces the first (same write fd
    /// if it is the same channel).
    pub fn install() -> rustix::io::Result<Self> {
        let fd = eventfd(0, EventfdFlags::empty())?;
        let raw = fd.as_raw_fd() as u64;
        EVENTFD_FD.store(raw, Ordering::Relaxed);

        let act = new_sigaction(hup_handler as *const () as usize);
        // SAFETY: static extern "C" handler and valid struct.
        let r = unsafe { libc::sigaction(libc::SIGHUP, &act, std::ptr::null_mut()) };
        if r == -1 {
            return Err(rustix::io::Errno::from_raw_os_error(unsafe {
                *libc::__errno_location()
            }));
        }

        Ok(Self { fd })
    }

    /// Fd to add to the loop's `poll`.
    pub fn poll_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Signal pending? Drains the counter and answers. NEVER blocks:
    /// a direct `read` on an eventfd at 0 BLOCKS until the next signal
    /// (the loop calls `drain` after every poll, signal or timeout, and
    /// froze right there), so it consults with a ZERO-timeout `poll` and
    /// only reads if ready.
    ///
    /// A single drain suffices: eventfd is a counter, not a queue — if 3
    /// SIGHUPs arrived back to back, one reload covers N signals.
    pub fn drain(&self) -> bool {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        match poll(&mut fds, Some(&zero)) {
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
        // Removes the handler: back to SIG_IGN, the disposition Rust
        // starts the process with (SIGHUP ignored). Restoring SIG_DFL
        // here would kill the process on the next HUP.
        // SAFETY: valid struct (zero + empty mask); error output ignored:
        // in Drop there is nothing sensible to do.
        unsafe {
            let ign = new_sigaction(libc::SIG_IGN);
            let _ = libc::sigaction(libc::SIGHUP, &ign, std::ptr::null_mut());
        }
        EVENTFD_FD.store(0, Ordering::Relaxed);
    }
}
