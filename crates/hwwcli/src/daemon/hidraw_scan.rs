//! Find the `/dev/hidrawN` node for a freshly-created UHID device by
//! its serial (`HID_UNIQ`).
//!
//! After `bridge.start().await` returns, the UHID `CREATE2` ioctl has
//! succeeded but the kernel still has to walk through the HID
//! subsystem, create a `hidraw` device, and emit a udev event. That
//! happens off the calling thread, so we poll `/sys/class/hidraw` for
//! up to a short deadline and match on the `HID_UNIQ` line in each
//! candidate's `device/uevent`.
//!
//! Best-effort. If nothing matches by the deadline we return `None`
//! — the caller decides whether to fail or continue without a path
//! (the desktop's `hidapi` enumerate doesn't depend on this).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(50);

pub async fn find_hidraw_by_serial(serial: &str, timeout: Duration) -> Option<PathBuf> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(p) = scan_once(serial) {
            return Some(p);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(POLL).await;
    }
}

fn scan_once(serial: &str) -> Option<PathBuf> {
    let class_dir = Path::new("/sys/class/hidraw");
    let entries = std::fs::read_dir(class_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("hidraw") {
            continue;
        }
        let uevent_path = entry.path().join("device").join("uevent");
        let Ok(content) = std::fs::read_to_string(&uevent_path) else {
            continue;
        };
        if uevent_has_uniq(&content, serial) {
            return Some(PathBuf::from(format!("/dev/{name}")));
        }
    }
    None
}

fn uevent_has_uniq(uevent: &str, serial: &str) -> bool {
    for line in uevent.lines() {
        // Two possible forms depending on kernel version:
        //   HID_UNIQ=hwwctl-bb02-a3f912
        //   HID_NAME=...HID_UNIQ=hwwctl-bb02-a3f912
        // Check explicitly for the prefix to avoid substring false
        // positives (one device's UNIQ as another's NAME substring).
        if let Some(v) = line.strip_prefix("HID_UNIQ=") {
            if v.trim() == serial {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniq_match_basic() {
        let u = "HID_ID=0003:000003EB:00002403\nHID_NAME=BitBox02\nHID_UNIQ=hwwctl-bb02-a3f912\n";
        assert!(uevent_has_uniq(u, "hwwctl-bb02-a3f912"));
        assert!(!uevent_has_uniq(u, "hwwctl-bb02-aaaaaa"));
    }

    #[test]
    fn uniq_match_ignores_substring_in_name() {
        // The serial appears as a substring of HID_NAME but not as
        // HID_UNIQ — must not match.
        let u = "HID_NAME=hwwctl-bb02-a3f912 BitBox02\nHID_UNIQ=\n";
        assert!(!uevent_has_uniq(u, "hwwctl-bb02-a3f912"));
    }

    #[test]
    fn empty_uevent_is_no_match() {
        assert!(!uevent_has_uniq("", "anything"));
    }
}
