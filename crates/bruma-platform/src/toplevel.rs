//! Fullscreen window tracking via `wlr-foreign-toplevel-management`
//! (Phase 5: pausing the animation where the wallpaper is not visible).
//!
//! Contract: the compositor announces each window as a "toplevel" with its
//! states (maximized/fullscreen/activated...) and the outputs it sits on
//! (`output_enter`/`output_leave`). With that, this unit answers ONE
//! question: **is there a fullscreen window on this output?**
//!
//! Policy (D12): only `fullscreen` pauses — fullscreen covers the
//! wallpaper by definition. `maximized` does NOT count: on several
//! compositors maximizing does not cover the whole wallpaper buffer and
//! the background would still peek through; pausing there would pause
//! something visible.
//!
//! The pause is **per output**: a fullscreen window on eDP-1 freezes
//! eDP-1's wallpaper; HDMI-A-1 keeps animating.
//!
//! The `Dispatch` impls live in `BackgroundState` (wayland-client requires
//! the queue-owning state to implement them); this module only provides
//! the data model and semantics.

use std::collections::HashMap;

use crate::wayland_client::Proxy;
use crate::wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;

/// Value of the protocol's `state` enum for "fullscreen".
pub(crate) const STATE_FULLSCREEN: u32 = 3;

/// State of a tracked window.
#[derive(Default)]
struct ToplevelData {
    /// Title (`title` event), diagnostics only.
    title: String,
    /// Is it fullscreen (per the last `state` event)?
    fullscreen: bool,
    /// Outputs it sits on (output_enter/output_leave), by id.
    outputs: Vec<crate::wayland_client::backend::ObjectId>,
}

/// Toplevel tracking: pure data model. With no protocol bound,
/// `disabled()` offers the same type with an empty map — the pause is
/// simply "never" and the rest of the engine checks nothing.
pub struct ToplevelTracker {
    toplevels: HashMap<ZwlrForeignToplevelHandleV1, ToplevelData>,
}

impl ToplevelTracker {
    /// Tracker without the protocol: never fullscreen (D12 degradation).
    pub fn disabled() -> Self {
        Self {
            toplevels: HashMap::new(),
        }
    }

    /// Registers a new window (the manager's `toplevel` event).
    pub fn toplevel_created(&mut self, handle: ZwlrForeignToplevelHandleV1) {
        self.toplevels.insert(handle, ToplevelData::default());
    }

    /// The compositor withdrew protocol support (`finished`).
    pub fn reset(&mut self) {
        self.toplevels.clear();
    }

    /// Updates a window's states (`state` event). The protocol's array is
    /// a list of raw u32s (`state` enum).
    pub fn toplevel_state(&mut self, handle: &ZwlrForeignToplevelHandleV1, states: &[u8]) {
        let Some(data) = self.toplevels.get_mut(handle) else {
            return;
        };
        let mut fullscreen = false;
        for chunk in states.as_chunks::<4>().0 {
            let v = u32::from_ne_bytes(*chunk);
            if v == STATE_FULLSCREEN {
                fullscreen = true;
                break;
            }
        }
        if fullscreen != data.fullscreen {
            log::info!(
                "toplevel '{}' fullscreen={} (states: {:?})",
                data.title,
                fullscreen,
                states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_ne_bytes(*c))
                    .collect::<Vec<_>>()
            );
        }
        data.fullscreen = fullscreen;
    }

    /// The window enters an output (`output_enter` event).
    pub fn output_enter(&mut self, handle: &ZwlrForeignToplevelHandleV1, output: &WlOutput) {
        let Some(data) = self.toplevels.get_mut(handle) else {
            return;
        };
        let id = output.id();
        if !data.outputs.contains(&id) {
            log::debug!(
                "toplevel '{}' output_enter {:?} (total: {})",
                data.title,
                id,
                data.outputs.len() + 1
            );
            data.outputs.push(id);
        }
    }

    /// The window leaves an output (`output_leave` event).
    pub fn output_leave(&mut self, handle: &ZwlrForeignToplevelHandleV1, output: &WlOutput) {
        let Some(data) = self.toplevels.get_mut(handle) else {
            return;
        };
        let id = output.id();
        data.outputs.retain(|x| *x != id);
    }

    /// The window closed (`closed` event).
    pub fn toplevel_closed(&mut self, handle: &ZwlrForeignToplevelHandleV1) {
        self.toplevels.remove(handle);
    }

    /// Window title (`title` event), for diagnostics.
    pub fn toplevel_title(&mut self, handle: &ZwlrForeignToplevelHandleV1, title: String) {
        if let Some(data) = self.toplevels.get_mut(handle) {
            data.title = title;
        }
    }

    /// Is there any fullscreen window on the given output?
    pub fn is_fullscreen_on(&self, output: &WlOutput) -> bool {
        let id = output.id();
        self.toplevels
            .values()
            .any(|d| d.fullscreen && d.outputs.contains(&id))
    }
}
