//! Real-time clock for the `u_clock` shader uniform.
//!
//! The GPU cannot read the wall clock: the engine supplies it. Uses
//! `localtime_r` (libc, already in the tree) so shaders see the LOCAL
//! day — the day/night tint and clock wallpapers need local time, not
//! UTC. Pure function of "now": no state, no locks.

// The only unsafe here is the libc call below: no safe std API exposes
// localtime_r; the block is justified inline and nothing else uses it.
#![allow(unsafe_code)]

/// Current local time as `[hours, minutes, seconds]` (f32 for the
/// uniform block). Degrades to `[0, 0, 0]` if the clock is unreadable.
pub fn local_hms() -> [f32; 3] {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let t: libc::time_t = secs;
    // SAFETY: zeroed libc::tm is a valid (empty) record for localtime_r
    // to fill; plain C struct, no invariants to uphold.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `t` is a valid epoch pointer and `tm` a writable record;
    // the _r variant is thread-safe and touches no libc globals.
    let failed = unsafe { libc::localtime_r(&t, &mut tm).is_null() };
    if failed {
        return [0.0; 3];
    }
    [tm.tm_hour as f32, tm.tm_min as f32, tm.tm_sec as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hms_is_in_range() {
        let [h, m, s] = local_hms();
        assert!((0.0..=23.0).contains(&h), "hour {h}");
        assert!((0.0..=59.0).contains(&m), "minute {m}");
        assert!((0.0..=59.0).contains(&s), "second {s}");
    }
}
