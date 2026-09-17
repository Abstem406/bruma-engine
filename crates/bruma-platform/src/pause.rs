//! Pausa global del motor (criterio D12, parte 2): bloqueo de sesión y
//! batería.
//!
//! Fuentes de señal, ambas por D-Bus de **sistema**:
//!
//! - **logind** (`org.freedesktop.login1.Session`): las señales `Lock`/
//!   `Unlock` que el compositor emite al bloquear (niri las dispara vía
//!   session-lock), más la propiedad `LockedHint` (`PropertiesChanged`)
//!   como segunda fuente por si el lock manager solo fija el hint.
//! - **UPower** (`org.freedesktop.UPower.Device` en `DisplayDevice`):
//!   propiedad `State` — `2` (discharging) significa "en batería".
//!
//! Contrato idéntico al de las notificaciones (D11): **best-effort**.
//! Sin bus, sin sesión o con daemon sordo, el watcher degrada a "nunca
//! pausa por esa fuente" y el motor no cambia en nada. `disabled()`
//! existe para tests: ninguna prueba toca el bus real.
//!
//! El modelo de despacho: el bucle de frames llama a [`poll`] una vez
//! por frame; cada llamada drena (sin bloquear) las señales D-Bus que
//! llegaron, y los callbacks actualizan los flags. Cero hilos extra:
//! el watcher vive en el mismo hilo que Wayland.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbus::arg::{PropMap, Variant, cast};
use dbus::blocking::SyncConnection;
use dbus::message::MatchRule;

/// UPower `Device.State`: descargando (en batería).
const UPOWER_STATE_DISCHARGING: u32 = 2;

/// Timeout para los method calls de estado inicial (uno solo, al arranque).
const INIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Causas de pausa global, combinables. La pausa activa si hay ALGUNA.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PauseFlags {
    /// Sesión bloqueada (logind `Lock`/`Unlock`/`LockedHint`).
    pub session_locked: bool,
    /// En batería (UPower `State == 2`).
    pub on_battery: bool,
}

impl PauseFlags {
    /// ¿Hay que pausar el motor?
    pub fn any(&self) -> bool {
        self.session_locked || self.on_battery
    }
}

/// ¿Está el motor en pausa global según los flags actuales?
pub(crate) fn flags_a_pausa(flags: &PauseFlags) -> bool {
    flags.any()
}

/// ¿Corresponde "en batería" según el `State` de UPower?
fn upower_state_a_bateria(state: u32) -> bool {
    state == UPOWER_STATE_DISCHARGING
}

/// Extrae un bool de un `Variant` D-Bus (lo que llega en
/// `PropertiesChanged`). Función pura: el path de extracción exacto que
/// usan los callbacks, testeado sin bus.
fn bool_de_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<bool> {
    cast::<bool>(&*v.0).copied()
}

/// Extrae un u32 de un `Variant` D-Bus.
fn u32_de_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<u32> {
    cast::<u32>(&*v.0).copied()
}

/// Extrae un String de un `Variant` D-Bus.
fn string_de_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<String> {
    cast::<String>(&*v.0).cloned()
}

/// Watcher de pausa global: bloqueo de sesión y batería, best-effort.
///
/// - `Some(conn)`: conexión al bus de sistema con los matches activos.
/// - `None`: sin bus (o sin sesión); `paused()` es siempre `false` y
///   `poll()` no hace nada. El resto del motor no comprueba nada.
pub struct SessionPauseWatcher {
    /// Flags compartidos con los callbacks D-Bus (el crate los invoca
    /// desde su propio despacho dentro de `process`).
    flags: Arc<Mutex<PauseFlags>>,
    conn: Option<Arc<SyncConnection>>,
}

impl SessionPauseWatcher {
    /// Conecta al bus de sistema y subscribe las señales. Nunca falla:
    /// cualquier error degrada a un watcher que no pausa.
    pub fn new() -> Self {
        let conn = match SyncConnection::new_system() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                log::info!("sin bus de sistema: pausa por bloqueo/batería no disponible ({e})");
                return Self::disabled();
            }
        };

        let flags = Arc::new(Mutex::new(PauseFlags::default()));

        // Estado inicial sincrónico (las señales solo avisan de CAMBIOS):
        // así un arranque con la sesión ya bloqueada o ya en batería
        // pausa desde el primer frame.
        let locked = Self::query_session(&conn, &flags);
        let battery = Self::query_battery(&conn, &flags);
        if let Ok(mut f) = flags.lock() {
            if let Some(l) = locked {
                f.session_locked = l;
            }
            if let Some(b) = battery {
                f.on_battery = b;
            }
        }

        // Señales: solo si el estado inicial se pudo consultar (mismo
        // objeto de sesión; si logind no respondió, no insiste).
        if let Some(session_path) = locked.and_then(|_| Self::session_path(&conn)) {
            Self::subscribe_session(&conn, &flags, &session_path);
        }
        if battery.is_some() {
            Self::subscribe_battery(&conn, &flags);
        }

        log::info!(
            "pausa global: fuente bloqueo {} (logind), fuente batería {} (UPower)",
            if locked.is_some() {
                "disponible"
            } else {
                "no disponible"
            },
            if battery.is_some() {
                "disponible"
            } else {
                "no disponible"
            }
        );

        Self {
            flags,
            conn: Some(conn),
        }
    }

    /// Watcher sin bus: nunca pausa (para tests y degradación total).
    pub fn disabled() -> Self {
        Self {
            flags: Arc::new(Mutex::new(PauseFlags::default())),
            conn: None,
        }
    }

    /// ¿Hay que pausar el motor ahora mismo?
    pub fn paused(&self) -> bool {
        match self.flags.lock() {
            Ok(f) => flags_a_pausa(&f),
            Err(_) => false,
        }
    }

    /// Drena (sin bloquear) las señales D-Bus pendientes. Una llamada
    /// por frame desde el bucle; los callbacks actualizan los flags.
    pub fn poll(&self) {
        let Some(conn) = &self.conn else { return };
        // Cota de drenaje: con señales normales (una cada tanto) una
        // vuelta basta; 32 absorbe ráfagas sin poder girar infinito si
        // alguien nos inunda el bus.
        for _ in 0..32 {
            match conn.process(Duration::ZERO) {
                Ok(true) => continue,
                _ => break,
            }
        }
    }

    /// Path del objeto Session de logind de la sesión gráfica donde
    /// corre el compositor que nos lanzó.
    ///
    /// NOTA: `GetSessionByPID` NO sirve — los compositors Wayland corren
    /// como servicios de usuario (systemd --user), fuera del alcance de
    /// sesión de logind: responde `NoSessionForPID` para cualquier PID
    /// (verificado en niri). En su lugar: `ListSessions` y la primera
    /// sesión con `Type="wayland"`.
    ///
    /// El path devuelto viene ESCAPADO por logind (sesión "4" →
    /// `.../session/_34`): hay que usarlo TAL CUAL — las señales
    /// `Lock`/`Unlock`/`PropertiesChanged` se emiten por ese path
    /// escapado, no por el numérico (bug cazado en la demo: el match
    /// por `/session/4` nunca recibía nada).
    fn session_path(conn: &SyncConnection) -> Option<dbus::Path<'static>> {
        let proxy = conn.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            INIT_TIMEOUT,
        );
        type SessionRow = (String, u32, String, String, dbus::Path<'static>);
        let (sessions,): (Vec<SessionRow>,) = proxy
            .method_call("org.freedesktop.login1.Manager", "ListSessions", ())
            .ok()?;
        for (_, _, _, _, path) in sessions {
            let ty = Self::get_property(
                conn,
                "org.freedesktop.login1",
                &path.to_string(),
                "org.freedesktop.login1.Session",
                "Type",
            )
            .and_then(|v| string_de_variant(&v));
            if ty.as_deref() == Some("wayland") {
                log::info!("sesión gráfica de logind: {path} (Type=wayland)");
                return Some(path);
            }
        }
        None
    }

    /// `LockedHint` actual de la sesión. `None` = logind no respondió.
    fn query_session(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>) -> Option<bool> {
        let path = Self::session_path(conn)?;
        let v = Self::get_property(
            conn,
            "org.freedesktop.login1",
            &path,
            "org.freedesktop.login1.Session",
            "LockedHint",
        )?;
        let value = bool_de_variant(&v)?;
        if let Ok(mut f) = flags.lock() {
            f.session_locked = value;
        }
        log::info!(
            "estado inicial: sesión {} (LockedHint={value})",
            if value { "bloqueada" } else { "desbloqueada" }
        );
        Some(value)
    }

    /// `State` actual de UPower DisplayDevice. `None` = no respondió.
    fn query_battery(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>) -> Option<bool> {
        const DEV: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
        let v = Self::get_property(
            conn,
            "org.freedesktop.UPower",
            DEV,
            "org.freedesktop.UPower.Device",
            "State",
        )?;
        let state = u32_de_variant(&v)?;
        let on_battery = upower_state_a_bateria(state);
        if let Ok(mut f) = flags.lock() {
            f.on_battery = on_battery;
        }
        log::info!(
            "estado inicial: {} (UPower State={state})",
            if on_battery {
                "en batería"
            } else {
                "con corriente"
            }
        );
        Some(on_battery)
    }

    /// Get genérico de propiedad D-Bus, devuelto como `Variant` crudo.
    fn get_property(
        conn: &SyncConnection,
        dest: &str,
        path: &str,
        iface: &str,
        prop: &str,
    ) -> Option<Variant<Box<dyn dbus::arg::RefArg>>> {
        let proxy = conn.with_proxy(dest, path, INIT_TIMEOUT);
        let (v,): (Variant<Box<dyn dbus::arg::RefArg>>,) = proxy
            .method_call("org.freedesktop.DBus.Properties", "Get", (iface, prop))
            .ok()?;
        Some(v)
    }

    /// Subscribe `Lock`, `Unlock` y `PropertiesChanged` (LockedHint) de
    /// la sesión. Cada match es independiente: si uno falla, el resto
    /// queda vivo.
    fn subscribe_session(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>, path: &str) {
        // Lock / Unlock: señales vacías; fuente primaria.
        for (member, value) in [("Lock", true), ("Unlock", false)] {
            let rule = MatchRule::new_signal("org.freedesktop.login1.Session", member)
                .with_path(path.to_owned());
            let f = flags.clone();
            if let Err(e) = conn.add_match::<(), _>(rule, move |_: (), _, _| {
                if let Ok(mut f) = f.lock() {
                    f.session_locked = value;
                }
                log::info!(
                    "pausa global: sesión {}",
                    if value { "bloqueada" } else { "desbloqueada" }
                );
                true
            }) {
                log::info!("sin señal {member} de logind: esa fuente de pausa degrada ({e})");
            }
        }

        // PropertiesChanged: segunda fuente (LockedHint), por si el lock
        // manager solo fija el hint sin pedir el Lock a logind.
        let rule = MatchRule::new_signal("org.freedesktop.DBus.Properties", "PropertiesChanged")
            .with_path(path.to_owned())
            .with_sender("org.freedesktop.login1");
        let f = flags.clone();
        if let Err(e) = conn.add_match::<(String, PropMap, Vec<String>), _>(
            rule,
            move |(iface, props, _): (String, PropMap, Vec<String>), _, _| {
                if iface != "org.freedesktop.login1.Session" {
                    return true;
                }
                if let Some(b) = props.get("LockedHint").and_then(bool_de_variant) {
                    if let Ok(mut f) = f.lock() {
                        f.session_locked = b;
                    }
                    log::info!(
                        "pausa global: sesión {} (LockedHint)",
                        if b { "bloqueada" } else { "desbloqueada" }
                    );
                }
                true
            },
        ) {
            log::info!("sin PropertiesChanged de logind: LockedHint como fuente degrada ({e})");
        }
    }

    /// Subscribe `PropertiesChanged` de UPower (DisplayDevice): `State`.
    fn subscribe_battery(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>) {
        let rule = MatchRule::new_signal("org.freedesktop.DBus.Properties", "PropertiesChanged")
            .with_path("/org/freedesktop/UPower/devices/DisplayDevice")
            .with_sender("org.freedesktop.UPower");
        let f = flags.clone();
        if let Err(e) = conn.add_match::<(String, PropMap, Vec<String>), _>(
            rule,
            move |(iface, props, _): (String, PropMap, Vec<String>), _, _| {
                if iface != "org.freedesktop.UPower.Device" {
                    return true;
                }
                if let Some(state) = props.get("State").and_then(u32_de_variant) {
                    let on_battery = upower_state_a_bateria(state);
                    if let Ok(mut f) = f.lock() {
                        f.on_battery = on_battery;
                    }
                    log::info!(
                        "pausa global: {} (UPower State={state})",
                        if on_battery {
                            "en batería"
                        } else {
                            "con corriente"
                        }
                    );
                }
                true
            },
        ) {
            log::info!("sin PropertiesChanged de UPower: pausa por batería degrada ({e})");
        }
    }
}

impl Default for SessionPauseWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regla del proyecto (lección D11): los tests jamás tocan el bus
    // real. Todo el camino con efectos se prueba vía `disabled()` y
    // funciones puras.

    #[test]
    fn semantica_de_flags() {
        assert!(!PauseFlags::default().any());
        assert!(
            PauseFlags {
                session_locked: true,
                on_battery: false
            }
            .any()
        );
        assert!(
            PauseFlags {
                session_locked: false,
                on_battery: true
            }
            .any()
        );
        assert!(
            PauseFlags {
                session_locked: true,
                on_battery: true
            }
            .any()
        );
        assert!(flags_a_pausa(&PauseFlags {
            session_locked: false,
            on_battery: true
        }));
        assert!(!flags_a_pausa(&PauseFlags::default()));
    }

    #[test]
    fn upower_state_2_es_bateria() {
        assert!(upower_state_a_bateria(2)); // discharging
        assert!(!upower_state_a_bateria(1)); // charging
        assert!(!upower_state_a_bateria(4)); // fully charged
        assert!(!upower_state_a_bateria(0)); // unknown
    }

    #[test]
    fn extraccion_de_variantes_dbus() {
        // El envoltorio exacto que llega en PropertiesChanged:
        // Variant(Box<dyn RefArg>) con bool/u32 dentro.
        let b: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(true));
        assert_eq!(bool_de_variant(&b), Some(true));
        let b2: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(false));
        assert_eq!(bool_de_variant(&b2), Some(false));

        let u: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(2u32));
        assert_eq!(u32_de_variant(&u), Some(2));

        // Tipo equivocado → None (degradación, no panic).
        assert_eq!(bool_de_variant(&u), None);
        let s: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new("no".to_owned()));
        assert_eq!(u32_de_variant(&s), None);
        assert_eq!(string_de_variant(&s), Some("no".to_owned()));
        assert_eq!(string_de_variant(&u), None);
    }

    #[test]
    fn watcher_disabled_nunca_pausa() {
        let w = SessionPauseWatcher::disabled();
        assert!(!w.paused());
        w.poll(); // no-op, sin panic
        assert!(!w.paused());
    }
}
