//! Best-effort desktop notifications via D-Bus
//! (`org.freedesktop.Notifications`: mako, dunst, quickshell/DMS...).
//!
//! The contract is deliberately shy: a wallpaper must never fail (nor
//! hang, nor die) because of its notification channel. Every error is
//! swallowed: no bus, no daemon, or a deaf daemon simply returns `false`
//! and the engine goes on.
//!
//! Used to warn the creator when hot-reload rejects a broken shader (and
//! when it recovers): see [`DesktopNotifier`].

use std::sync::Mutex;
use std::time::Duration;

use dbus::Message;
use dbus::arg::messageitem::MessageItem;
use dbus::blocking::{BlockingSender, SyncConnection};

/// At most this is expected from the notification daemon. The send happens
/// on the frame loop's critical path: if the daemon does not answer, the
/// notification is lost, not the frame.
const NOTIFY_TIMEOUT: Duration = Duration::from_millis(300);

/// "Normal" urgency (1) from the notifications spec: a shader notice is
/// not critical and must not steal attention like an alarm.
const URGENCY_NORMAL: u8 = 1;

/// `urgency` hint packed as an `a{sv}` dict. `MessageItem::from_dict`
/// wraps each value in a `Variant` for us; the generator's error is
/// `Infallible` (construction cannot fail with a fixed input).
fn hint_urgency(level: u8) -> MessageItem {
    let item: Result<MessageItem, std::convert::Infallible> =
        MessageItem::from_dict([Ok(("urgency".to_owned(), MessageItem::Byte(level)))].into_iter());
    match item {
        Ok(h) => h,
        Err(e) => match e {},
    }
}

/// Desktop notification channel.
///
/// - `Some(conn)`: established session bus connection.
/// - `None`: no bus (or connection failed); every call is a no-op. So the
///   rest of the engine needs no checks.
///
/// The connection is `Send + Sync` (dbus 0.9: `SyncConnection`) and sends
/// are serialized by the crate's internal mutex.
pub struct DesktopNotifier {
    conn: Option<SyncConnection>,
    /// Serializes sends (the connection allows concurrent ones, but this
    /// way one send's timeout does not chain with the next from several
    /// threads).
    lock: Mutex<()>,
}

impl DesktopNotifier {
    /// Tries to connect to the session bus. Never fails: without a bus it
    /// returns a notifier that does nothing.
    pub fn new() -> Self {
        let conn = SyncConnection::new_session().ok();
        if conn.is_none() {
            log::debug!("no session bus: desktop notifications disabled");
        }
        DesktopNotifier {
            conn,
            lock: Mutex::new(()),
        }
    }

    /// Manually disabled notifier: no bus, no effects. For testing the
    /// no-op path — there must never be a test calling `new()` on a
    /// machine with a desktop, because it would send real bubbles
    /// (it happened: this unit's test notified the user from
    /// `cargo test`).
    pub fn disabled() -> Self {
        DesktopNotifier {
            conn: None,
            lock: Mutex::new(()),
        }
    }

    /// Sends the notification and waits for the daemon's reply (capped at
    /// [`NOTIFY_TIMEOUT`]). Returns `true` if accepted.
    fn send(&self, summary: &str, body: &str) -> bool {
        let Some(conn) = &self.conn else {
            return false;
        };
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());

        let msg = build_notify_message(summary, body);
        match conn.send_with_reply_and_block(msg, NOTIFY_TIMEOUT) {
            Ok(_) => true,
            Err(e) => {
                log::debug!("notification daemon rejected the notice: {e}");
                false
            }
        }
    }

    /// Shader-rejected notice from hot-reload. `err` is naga's compile
    /// message, already clipped by the caller.
    ///
    /// Returns `true` if the daemon showed the bubble.
    pub fn shader_rejected(&self, err: &str) -> bool {
        self.send("bruma: shader rejected", err)
    }

    /// Recovery notice: the shader compiled again after a rejection.
    pub fn shader_recovered(&self) -> bool {
        self.send(
            "bruma: shader recovered",
            "The shader compiled again and was applied to the wallpaper.",
        )
    }
}

// FUTURE (Phase 6): bubble replacement via replaces_id — `Notify` returns
// the assigned id in the reply; it will be kept when the creator editing
// flow asks for it.

/// Builds the full `Notify` message (spec: signature usssiasb). Pure and
/// bus-free: tests can audit the message byte by byte with no desktop
/// effects. The dict's trailing 'a' is the `a{sv}` hints; actions is empty
/// (array with explicit signature).
fn build_notify_message(summary: &str, body: &str) -> Message {
    let mut msg = Message::new_method_call(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
    )
    .expect("the Notify method signature is constant and valid");

    let actions: [String; 0] = [];
    msg.append_items(&[
        MessageItem::Str("bruma".to_owned()), // app_name
        MessageItem::UInt32(0),               // replaces_id (0 = new)
        MessageItem::Str(String::new()),      // app_icon
        MessageItem::Str(summary.to_owned()), // summary
        MessageItem::Str(body.to_owned()),    // body
        MessageItem::from(&actions[..]),      // actions (empty)
        hint_urgency(URGENCY_NORMAL),         // hints a{sv}
        MessageItem::Int32(5000),             // expire_timeout ms
    ]);
    msg
}

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

    /// No-op path: without a connection, both calls return false and touch
    /// NOTHING. Uses `disabled()`, never `new()`: on a machine with a
    /// graphical session `new()` would really connect and `cargo test`
    /// would send real bubbles to the user's desktop.
    #[test]
    fn disabled_is_fully_noop() {
        let n = DesktopNotifier::disabled();
        assert!(!n.shader_rejected("must not reach the bus"));
        assert!(!n.shader_recovered());
    }

    /// The Notify message follows the spec signature: 8 arguments,
    /// app_name "bruma", hints as an a{sv} dict, 5 s expiry.
    /// Pure: no connection, no send.
    #[test]
    fn notify_message_follows_spec() {
        let msg = build_notify_message("test title", "test body");
        let items = msg.get_items();
        assert_eq!(items.len(), 8, "usssiasb signature: 8 arguments");
        assert_eq!(items[0], MessageItem::Str("bruma".to_owned()));
        assert_eq!(items[1], MessageItem::UInt32(0));
        assert_eq!(items[3], MessageItem::Str("test title".to_owned()));
        assert_eq!(items[4], MessageItem::Str("test body".to_owned()));
        assert_eq!(items[6].signature(), "a{sv}", "hints as dict");
        assert_eq!(items[7], MessageItem::Int32(5000));
    }

    /// The urgency hint yields the correct a{sv} signature.
    #[test]
    fn urgency_hint_has_dict_sv_signature() {
        assert_eq!(hint_urgency(1).signature().to_string(), "a{sv}");
    }
}
