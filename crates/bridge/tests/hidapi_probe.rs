//! Verify the kernel exposes a UHID-backed device the same way
//! `sigvault-desktop` will see it: through `hidapi`'s device
//! enumeration filtered by `serial_number`.
//!
//! This is the canonical "did the daemon's UHID bridge actually
//! produce a device the host's HID stack sees" check. CI runs it
//! after `hwwctl start bitbox02` to close the loop without needing
//! the desktop app installed.
//!
//! Required env vars:
//!
//! - `HWWCTL_EXPECT_SERIAL` — the serial returned by `hwwctl start`
//!   (e.g. `hwwctl-bb02-a3f912`).
//!
//! Optional:
//!
//! - `HWWCTL_EXPECT_VID` / `HWWCTL_EXPECT_PID` — hex (e.g. `0x03eb`).
//!   When set, the matched device must also have these IDs. Defaults
//!   skip the VID/PID check.
//! - `HWWCTL_PROBE_TIMEOUT_SECS` — total time to keep retrying
//!   enumeration. Defaults to `5`. Useful because UHID device
//!   creation is synchronous but `hidapi`'s next enumerate may race
//!   the kernel's hotplug.
//!
//! Marked `#[ignore]` because it relies on out-of-process setup
//! (`hwwctl daemon` must already have started an instance with the
//! given serial). Invoke from CI:
//!
//! ```text
//! HWWCTL_EXPECT_SERIAL=hwwctl-bb02-... \
//!   cargo test -p bridge --test hidapi_probe -- --ignored
//! ```

use std::thread::sleep;
use std::time::{Duration, Instant};

#[test]
#[ignore]
fn enumerates_uhid_device_by_serial() {
    let expected_serial = std::env::var("HWWCTL_EXPECT_SERIAL").expect(
        "HWWCTL_EXPECT_SERIAL must be set — pass the serial from `hwwctl start`'s JSON output",
    );
    let expected_vid = parse_hex_u16("HWWCTL_EXPECT_VID");
    let expected_pid = parse_hex_u16("HWWCTL_EXPECT_PID");
    let timeout = Duration::from_secs(
        std::env::var("HWWCTL_PROBE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5),
    );

    let mut api =
        hidapi::HidApi::new().expect("hidapi init failed — is libudev / hidraw available?");

    let deadline = Instant::now() + timeout;
    let mut last_listing: Vec<String> = Vec::new();
    loop {
        // Some platforms cache enumeration results; `refresh_devices`
        // forces a re-scan so a device that appeared between
        // construction and the call is visible.
        let _ = api.refresh_devices();

        last_listing.clear();
        for d in api.device_list() {
            let serial = d.serial_number().unwrap_or("");
            last_listing.push(format!(
                "vid={:#06x} pid={:#06x} serial={:?} path={:?}",
                d.vendor_id(),
                d.product_id(),
                serial,
                d.path()
            ));
            if serial != expected_serial {
                continue;
            }
            if let Some(vid) = expected_vid {
                if d.vendor_id() != vid {
                    continue;
                }
            }
            if let Some(pid) = expected_pid {
                if d.product_id() != pid {
                    continue;
                }
            }
            // Match. Print details so CI logs show what we found.
            println!(
                "MATCH vid={:#06x} pid={:#06x} serial={serial} path={:?}",
                d.vendor_id(),
                d.product_id(),
                d.path()
            );
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(100));
    }

    panic!(
        "no HID device with serial={expected_serial}{} appeared within {:?}.\n\
         Devices seen on last refresh:\n  {}",
        match (expected_vid, expected_pid) {
            (Some(v), Some(p)) => format!(" vid={v:#06x} pid={p:#06x}"),
            (Some(v), None) => format!(" vid={v:#06x}"),
            (None, Some(p)) => format!(" pid={p:#06x}"),
            (None, None) => String::new(),
        },
        timeout,
        if last_listing.is_empty() {
            "(none)".to_string()
        } else {
            last_listing.join("\n  ")
        }
    );
}

fn parse_hex_u16(env: &str) -> Option<u16> {
    let raw = std::env::var(env).ok()?;
    let body = raw.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(body, 16).ok()
}
