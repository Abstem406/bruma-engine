//! Rastreo de ventanas fullscreen vía `wlr-foreign-toplevel-management`
//! (Fase 5: pausa de la animación donde no se ve el fondo).
//!
//! Contrato: el compositor anuncia cada ventana como un "toplevel" con
//! sus estados (maximized/fullscreen/activated...) y las salidas sobre
//! las que está (`output_enter`/`output_leave`). Con eso, esta unidad
//! responde UNA pregunta: **¿hay una ventana fullscreen en esta salida?**
//!
//! Política (D12): solo `fullscreen` pausa — el fullscreen cubre el
//! fondo por definición. `maximized` NO cuenta: en varios compositors
//! maximizar no cubre todo el buffer del fondo y el wallpaper seguiría
//! asomando; pausarlo ahí sería pausar algo visible.
//!
//! La pausa es **por salida**: una ventana fullscreen en eDP-1 congela
//! el wallpaper de eDP-1; HDMI-A-1 sigue animando.
//!
//! Los impls `Dispatch` viven en `BackgroundState` (wayland-client
//! exige que el estado dueño de la cola los implemente); este módulo
//! aporta solo el modelo de datos y la semántica.

use std::collections::HashMap;

use crate::wayland_client::Proxy;
use crate::wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;

/// Valor del enum `state` del protocolo para "fullscreen".
pub(crate) const STATE_FULLSCREEN: u32 = 3;

/// Estado de una ventana rastreada.
#[derive(Default)]
struct ToplevelData {
    /// Título (evento `title`), solo para diagnóstico.
    title: String,
    /// ¿Está en fullscreen (según el último evento `state`)?
    fullscreen: bool,
    /// Salidas sobre las que está (output_enter/output_leave), por id.
    outputs: Vec<crate::wayland_client::backend::ObjectId>,
}

/// Rastreo de toplevels: modelo de datos puro. Sin protocolo ligado,
/// `disabled()` ofrece el mismo tipo con el mapa vacío — la pausa es
/// simplemente "nunca" y el resto del motor no comprueba nada.
pub struct ToplevelTracker {
    toplevels: HashMap<ZwlrForeignToplevelHandleV1, ToplevelData>,
}

impl ToplevelTracker {
    /// Tracker sin protocolo: nunca hay fullscreen (degradación D12).
    pub fn disabled() -> Self {
        Self {
            toplevels: HashMap::new(),
        }
    }

    /// Registra una ventana nueva (evento `toplevel` del manager).
    pub fn toplevel_created(&mut self, handle: ZwlrForeignToplevelHandleV1) {
        self.toplevels.insert(handle, ToplevelData::default());
    }

    /// El compositor retira el soporte del protocolo (`finished`).
    pub fn reset(&mut self) {
        self.toplevels.clear();
    }

    /// Actualiza los estados de una ventana (evento `state`). El array
    /// del protocolo es una lista de u32 crudos (enum `state`).
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
                "toplevel '{}' fullscreen={} (estados: {:?})",
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

    /// La ventana entra en una salida (evento `output_enter`).
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

    /// La ventana sale de una salida (evento `output_leave`).
    pub fn output_leave(&mut self, handle: &ZwlrForeignToplevelHandleV1, output: &WlOutput) {
        let Some(data) = self.toplevels.get_mut(handle) else {
            return;
        };
        let id = output.id();
        data.outputs.retain(|x| *x != id);
    }

    /// La ventana se cerró (evento `closed`).
    pub fn toplevel_closed(&mut self, handle: &ZwlrForeignToplevelHandleV1) {
        self.toplevels.remove(handle);
    }

    /// Título de la ventana (evento `title`), para diagnóstico.
    pub fn toplevel_title(&mut self, handle: &ZwlrForeignToplevelHandleV1, title: String) {
        if let Some(data) = self.toplevels.get_mut(handle) {
            data.title = title;
        }
    }

    /// ¿Hay alguna ventana fullscreen en la salida indicada?
    pub fn is_fullscreen_on(&self, output: &WlOutput) -> bool {
        let id = output.id();
        self.toplevels
            .values()
            .any(|d| d.fullscreen && d.outputs.contains(&id))
    }
}
