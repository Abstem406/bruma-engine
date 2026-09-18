//! Global engine pause (D12, part 2): session lock and battery.
//!
//! Signal sources, both over the **system** D-Bus:
//!
//! - **logind** (`org.freedesktop.login1.Session`): the `Lock`/`Unlock`
//!   signals the compositor emits on lock (niri fires them via
//!   session-lock), plus the `LockedHint` property (`PropertiesChanged`)
//!   as a second source in case the lock manager only sets the hint.
//! - **UPower** (`org.freedesktop.UPower.Device` on `DisplayDevice`):
//!   the `State` property — `2` (discharging) means "on battery".
//!
//! Contract identical to notifications (D11): **best-effort**. Without a
//! bus, a session, or with a deaf daemon, the watcher degrades to "never
//! pauses from that source" and the engine changes nothing. `disabled()`
//! exists for tests: no test touches the real bus.
//!
//! The dispatch model: the frame loop calls [`poll`] once per frame; each
//! call drains (without blocking) the D-Bus signals that arrived, and the
//! callbacks update the flags. Zero extra threads: the watcher lives on
//! the same thread as Wayland.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbus::arg::{PropMap, Variant, cast};
use dbus::blocking::SyncConnection;
use dbus::message::MatchRule;

/// UPower `Device.State`: discharging (on battery).
const UPOWER_STATE_DISCHARGING: u32 = 2;

/// Timeout for the initial state method calls (a single one, at startup).
const INIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Global pause causes, combinable. Paused if there is ANY.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PauseFlags {
    /// Session locked (logind `Lock`/`Unlock`/`LockedHint`).
    pub session_locked: bool,
    /// On battery (UPower `State == 2`).
    pub on_battery: bool,
    /// The user disabled the battery pause (`--no-battery-pause`): the
    /// flag is ignored even while discharging.
    pub battery_ignored: bool,
}

impl PauseFlags {
    /// Should the engine pause?
    pub fn any(&self) -> bool {
        self.session_locked || (self.on_battery && !self.battery_ignored)
    }
}

/// Is the engine globally paused according to the current flags?
pub(crate) fn flags_to_pause(flags: &PauseFlags) -> bool {
    flags.any()
}

/// Does UPower's `State` mean "on battery"?
fn upower_state_to_battery(state: u32) -> bool {
    state == UPOWER_STATE_DISCHARGING
}

/// Extracts a bool from a D-Bus `Variant` (what arrives in
/// `PropertiesChanged`). Pure function: the exact extraction path the
/// callbacks use, tested without a bus.
fn bool_from_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<bool> {
    cast::<bool>(&*v.0).copied()
}

/// Extracts a u32 from a D-Bus `Variant`.
fn u32_from_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<u32> {
    cast::<u32>(&*v.0).copied()
}

/// Extracts a String from a D-Bus `Variant`.
fn string_from_variant(v: &Variant<Box<dyn dbus::arg::RefArg>>) -> Option<String> {
    cast::<String>(&*v.0).cloned()
}

/// Global pause watcher: session lock and battery, best-effort.
///
/// - `Some(conn)`: system bus connection with the matches active.
/// - `None`: no bus (or no session); `paused()` is always `false` and
///   `poll()` does nothing. The rest of the engine checks nothing.
pub struct SessionPauseWatcher {
    /// Flags shared with the D-Bus callbacks (the crate invokes them from
    /// its own dispatch inside `process`).
    flags: Arc<Mutex<PauseFlags>>,
    conn: Option<Arc<SyncConnection>>,
}

impl SessionPauseWatcher {
    /// Connects to the system bus and subscribes to signals. Never fails:
    /// any error degrades to a watcher that never pauses.
    pub fn new() -> Self {
        let conn = match SyncConnection::new_system() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                log::info!("no system bus: lock/battery pause unavailable ({e})");
                return Self::disabled();
            }
        };

        let flags = Arc::new(Mutex::new(PauseFlags::default()));

        // Synchronous initial state (signals only announce CHANGES): so a
        // start with the session already locked, or already on battery,
        // pauses from the first frame.
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

        // Signals: only if the initial state could be queried (same
        // session object; if logind didn't answer, it doesn't insist).
        if let Some(session_path) = locked.and_then(|_| Self::session_path(&conn)) {
            Self::subscribe_session(&conn, &flags, &session_path);
        }
        if battery.is_some() {
            Self::subscribe_battery(&conn, &flags);
        }

        log::info!(
            "global pause: lock source {} (logind), battery source {} (UPower)",
            if locked.is_some() {
                "available"
            } else {
                "unavailable"
            },
            if battery.is_some() {
                "available"
            } else {
                "unavailable"
            }
        );

        Self {
            flags,
            conn: Some(conn),
        }
    }

    /// Watcher without a bus: never pauses (for tests and total
    /// degradation).
    pub fn disabled() -> Self {
        Self {
            flags: Arc::new(Mutex::new(PauseFlags::default())),
            conn: None,
        }
    }

    /// Should the engine pause right now?
    pub fn paused(&self) -> bool {
        match self.flags.lock() {
            Ok(f) => flags_to_pause(&f),
            Err(_) => false,
        }
    }

    /// Ignores the battery flag from now on (`--no-battery-pause`). The
    /// lock pause keeps working.
    pub fn set_battery_ignored(&self, ignored: bool) {
        if let Ok(mut f) = self.flags.lock() {
            f.battery_ignored = ignored;
        }
    }

    /// Drains (without blocking) pending D-Bus signals. One call per
    /// frame from the loop; the callbacks update the flags.
    pub fn poll(&self) {
        let Some(conn) = &self.conn else { return };
        // Drain cap: with normal signals (one now and then) a single pass
        // suffices; 32 absorbs bursts without being able to spin forever
        // if someone floods the bus.
        for _ in 0..32 {
            match conn.process(Duration::ZERO) {
                Ok(true) => continue,
                _ => break,
            }
        }
    }

    /// Object path of the logind Session for the graphical session where
    /// the compositor that launched us runs.
    ///
    /// NOTE: `GetSessionByPID` is useless — Wayland compositors run as
    /// user services (systemd --user), outside logind's session scope: it
    /// answers `NoSessionForPID` for any PID (verified on niri).
    /// Instead: `ListSessions` and the first session with
    /// `Type="wayland"`.
    ///
    /// The returned path comes ESCAPED by logind (session "4" →
    /// `.../session/_34`): it must be used AS IS — the
    /// `Lock`/`Unlock`/`PropertiesChanged` signals are emitted on that
    /// escaped path, not the numeric one (bug caught in the demo: the
    /// match on `/session/4` never received anything).
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
            .and_then(|v| string_from_variant(&v));
            if ty.as_deref() == Some("wayland") {
                log::info!("logind graphical session: {path} (Type=wayland)");
                return Some(path);
            }
        }
        None
    }

    /// Session's current `LockedHint`. `None` = logind didn't answer.
    fn query_session(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>) -> Option<bool> {
        let path = Self::session_path(conn)?;
        let v = Self::get_property(
            conn,
            "org.freedesktop.login1",
            &path,
            "org.freedesktop.login1.Session",
            "LockedHint",
        )?;
        let value = bool_from_variant(&v)?;
        if let Ok(mut f) = flags.lock() {
            f.session_locked = value;
        }
        log::info!(
            "initial state: session {} (LockedHint={value})",
            if value { "locked" } else { "unlocked" }
        );
        Some(value)
    }

    /// Current `State` of UPower's DisplayDevice. `None` = no answer.
    fn query_battery(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>) -> Option<bool> {
        const DEV: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
        let v = Self::get_property(
            conn,
            "org.freedesktop.UPower",
            DEV,
            "org.freedesktop.UPower.Device",
            "State",
        )?;
        let state = u32_from_variant(&v)?;
        let on_battery = upower_state_to_battery(state);
        if let Ok(mut f) = flags.lock() {
            f.on_battery = on_battery;
        }
        log::info!(
            "initial state: {} (UPower State={state})",
            if on_battery {
                "on battery"
            } else {
                "on AC power"
            }
        );
        Some(on_battery)
    }

    /// Generic D-Bus property Get, returned as a raw `Variant`.
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

    /// Subscribes to the session's `Lock`, `Unlock` and
    /// `PropertiesChanged` (LockedHint). Each match is independent: if one
    /// fails, the rest stay alive.
    fn subscribe_session(conn: &SyncConnection, flags: &Arc<Mutex<PauseFlags>>, path: &str) {
        // Lock / Unlock: empty signals; primary source.
        for (member, value) in [("Lock", true), ("Unlock", false)] {
            let rule = MatchRule::new_signal("org.freedesktop.login1.Session", member)
                .with_path(path.to_owned());
            let f = flags.clone();
            if let Err(e) = conn.add_match::<(), _>(rule, move |_: (), _, _| {
                if let Ok(mut f) = f.lock() {
                    f.session_locked = value;
                }
                log::info!(
                    "global pause: session {}",
                    if value { "locked" } else { "unlocked" }
                );
                true
            }) {
                log::info!("no logind {member} signal: that pause source degrades ({e})");
            }
        }

        // PropertiesChanged: second source (LockedHint), in case the lock
        // manager only sets the hint without asking logind for the Lock.
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
                if let Some(b) = props.get("LockedHint").and_then(bool_from_variant) {
                    if let Ok(mut f) = f.lock() {
                        f.session_locked = b;
                    }
                    log::info!(
                        "global pause: session {} (LockedHint)",
                        if b { "locked" } else { "unlocked" }
                    );
                }
                true
            },
        ) {
            log::info!("no logind PropertiesChanged: LockedHint as a source degrades ({e})");
        }
    }

    /// Subscribes to UPower's `PropertiesChanged` (DisplayDevice): `State`.
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
                if let Some(state) = props.get("State").and_then(u32_from_variant) {
                    let on_battery = upower_state_to_battery(state);
                    if let Ok(mut f) = f.lock() {
                        f.on_battery = on_battery;
                    }
                    log::info!(
                        "global pause: {} (UPower State={state})",
                        if on_battery {
                            "on battery"
                        } else {
                            "on AC power"
                        }
                    );
                }
                true
            },
        ) {
            log::info!("no UPower PropertiesChanged: battery pause degrades ({e})");
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

    // Project rule (D11 lesson): tests never touch the real bus. The whole
    // effectful path is tested via `disabled()` and pure functions.

    #[test]
    fn flags_semantics() {
        assert!(!PauseFlags::default().any());
        assert!(
            PauseFlags {
                session_locked: true,
                on_battery: false,
                battery_ignored: false
            }
            .any()
        );
        assert!(
            PauseFlags {
                session_locked: false,
                on_battery: true,
                battery_ignored: false
            }
            .any()
        );
        // The escape hatch: on battery but the user said keep rendering.
        assert!(
            !PauseFlags {
                session_locked: false,
                on_battery: true,
                battery_ignored: true
            }
            .any()
        );
        // The lock pause is NEVER ignorable.
        assert!(
            PauseFlags {
                session_locked: true,
                on_battery: true,
                battery_ignored: true
            }
            .any()
        );
        assert!(flags_to_pause(&PauseFlags {
            session_locked: false,
            on_battery: true,
            battery_ignored: false
        }));
        assert!(!flags_to_pause(&PauseFlags::default()));
    }

    #[test]
    fn upower_state_2_is_battery() {
        assert!(upower_state_to_battery(2)); // discharging
        assert!(!upower_state_to_battery(1)); // charging
        assert!(!upower_state_to_battery(4)); // fully charged
        assert!(!upower_state_to_battery(0)); // unknown
    }

    #[test]
    fn dbus_variant_extraction() {
        // The exact wrapper arriving in PropertiesChanged:
        // Variant(Box<dyn RefArg>) with bool/u32 inside.
        let b: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(true));
        assert_eq!(bool_from_variant(&b), Some(true));
        let b2: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(false));
        assert_eq!(bool_from_variant(&b2), Some(false));

        let u: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new(2u32));
        assert_eq!(u32_from_variant(&u), Some(2));

        // Wrong type → None (degradation, not panic).
        assert_eq!(bool_from_variant(&u), None);
        let s: Variant<Box<dyn dbus::arg::RefArg>> = Variant(Box::new("no".to_owned()));
        assert_eq!(u32_from_variant(&s), None);
        assert_eq!(string_from_variant(&s), Some("no".to_owned()));
        assert_eq!(string_from_variant(&u), None);
    }

    #[test]
    fn disabled_watcher_never_pauses() {
        let w = SessionPauseWatcher::disabled();
        assert!(!w.paused());
        w.poll(); // no-op, no panic
        assert!(!w.paused());
    }
}
