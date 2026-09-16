//! Notificaciones de escritorio best-effort vía D-Bus
//! (`org.freedesktop.Notifications`: mako, dunst, quickshell/DMS...).
//!
//! El contrato es deliberadamente tímido: un wallpaper nunca debe fallar
//! (ni colgarse, ni morir) por su canal de notificaciones. Todo error se
//! deglute: sin bus, sin daemon o con daemon sordo, simplemente devuelve
//! `false` y el motor sigue.
//!
//! Se usa para avisar al creador cuando el hot-reload rechaza un shader
//! roto (y cuando se recupera): ver [`DesktopNotifier`].

use std::sync::Mutex;
use std::time::Duration;

use dbus::Message;
use dbus::arg::messageitem::MessageItem;
use dbus::blocking::{BlockingSender, SyncConnection};

/// Se espera esto como máximo al daemon de notificaciones. El envío
/// ocurre en la ruta crítica del bucle de frames: si el daemon no
/// responde, se pierde la notificación, no el frame.
const NOTIFY_TIMEOUT: Duration = Duration::from_millis(300);

/// Urgencia "normal" (1) del spec de notificaciones: un aviso de shader
/// no es crítico y no debe robar atención como una alarma.
const URGENCY_NORMAL: u8 = 1;

/// Hint `urgency` empaquetado como dict `a{sv}`. `MessageItem::from_dict`
/// envuelve cada valor en `Variant` por nosotros; el error del generador
/// es `Infallible` (la construcción no puede fallar con una entrada fija).
fn hint_urgency(level: u8) -> MessageItem {
    let item: Result<MessageItem, std::convert::Infallible> =
        MessageItem::from_dict([Ok(("urgency".to_owned(), MessageItem::Byte(level)))].into_iter());
    match item {
        Ok(h) => h,
        Err(e) => match e {},
    }
}

/// Canal de notificaciones de escritorio.
///
/// - `Some(conn)`: conexión al bus de sesión ya establecida.
/// - `None`: no hay bus (o no se pudo conectar); todas las llamadas son
///   no-ops. Así el resto del motor no necesita comprobar nada.
///
/// La conexión es `Send + Sync` (dbus 0.9: `SyncConnection`) y el envío
/// está protegido por el mutex interno del crate.
pub struct DesktopNotifier {
    conn: Option<SyncConnection>,
    /// Serializa los envíos (la conexión lo permite, pero así el timeout
    /// de un envío no encadena con el del siguiente desde varios hilos).
    lock: Mutex<()>,
}

impl DesktopNotifier {
    /// Intenta conectar al bus de sesión. Nunca falla: sin bus devuelve
    /// un notifier que no hace nada.
    pub fn new() -> Self {
        let conn = SyncConnection::new_session().ok();
        if conn.is_none() {
            log::debug!("sin bus de sesión: notificaciones de escritorio desactivadas");
        }
        DesktopNotifier {
            conn,
            lock: Mutex::new(()),
        }
    }

    /// Envía la notificación y espera la respuesta del daemon (con
    /// [`NOTIFY_TIMEOUT`] de techo). Devuelve `true` si fue aceptada.
    fn send(&self, summary: &str, body: &str) -> bool {
        let Some(conn) = &self.conn else {
            return false;
        };
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut msg = match Message::new_method_call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
        ) {
            Ok(m) => m,
            Err(e) => {
                log::debug!("no se pudo crear el mensaje Notify: {e}");
                return false;
            }
        };

        // Firma requerida por el spec: usssiasb. La última 'a' es el dict
        // de hints a{sv}; actions va vacío (array con firma explícita).
        let actions: [String; 0] = [];
        msg.append_items(&[
            MessageItem::Str("bruma".to_owned()), // app_name
            MessageItem::UInt32(0),               // replaces_id (0 = nueva)
            MessageItem::Str(String::new()),      // app_icon
            MessageItem::Str(summary.to_owned()), // summary
            MessageItem::Str(body.to_owned()),    // body
            MessageItem::from(&actions[..]),      // actions (vacío)
            hint_urgency(URGENCY_NORMAL),         // hints a{sv}
            MessageItem::Int32(5000),             // expire_timeout ms
        ]);

        match conn.send_with_reply_and_block(msg, NOTIFY_TIMEOUT) {
            Ok(_) => true,
            Err(e) => {
                log::debug!("el daemon de notificaciones rechazó el aviso: {e}");
                false
            }
        }
    }

    /// Aviso de shader rechazado en el hot-reload. `err` es el mensaje de
    /// compilación de naga, ya recortado por el caller.
    ///
    /// Devuelve `true` si el daemon mostró la burbuja.
    pub fn shader_rejected(&self, err: &str) -> bool {
        self.send("bruma: shader rechazado", err)
    }

    /// Aviso de recuperación: el shader volvió a compilar tras un rechazo.
    pub fn shader_recovered(&self) -> bool {
        self.send(
            "bruma: shader recuperado",
            "El shader volvió a compilar y se aplicó al fondo.",
        )
    }
}

// FUTURO (Fase 6): reemplazo de burbujas por replaces_id — `Notify`
// devuelve el id asignado en la respuesta; se retendrá cuando el flujo
// de edición de creadores lo pida.

impl Default for DesktopNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DesktopNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopNotifier")
            .field("bus", &self.conn.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sin bus en los tests de CI: el notifier debe degradarse a no-op
    /// sin colgarse ni tumbar nada (contrato best-effort).
    #[test]
    fn sin_bus_degrada_a_noop() {
        let n = DesktopNotifier::new();
        // Puede que haya bus en una sesión de escritorio; en cualquier
        // caso, ninguna de las dos llamadas puede paniquear.
        let _ = n.shader_rejected("error de prueba");
        let _ = n.shader_recovered();
    }

    /// El hint de urgencia genera la firma a{sv} correcta.
    #[test]
    fn hint_urgencia_firma_dict_sv() {
        assert_eq!(hint_urgency(1).signature().to_string(), "a{sv}");
    }
}
