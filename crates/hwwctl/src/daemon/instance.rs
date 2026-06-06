//! Per-instance state held by the daemon registry.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bridge::Bridge;
use control::{
    BridgeStatsSnapshot, InstanceId, InstanceState, InstanceSummary, LogEntry, LogSource, Wallet,
};
use emulators::Emulator;
use tokio::task::JoinHandle;

/// How many entries to keep per log source. Sized so a single run
/// can capture a full add-device flow (a few hundred reports each
/// direction) without dragging in unbounded memory.
#[allow(dead_code)] // used only on Linux — spawn.rs is Linux-gated
pub const LOG_BUFFER_CAP: usize = 500;

/// One running emulator + its UHID bridge + the buffers that record
/// what they say.
///
/// Owned exclusively by the [`Registry`](super::registry::Registry)
/// task — never aliased.
pub(crate) struct Instance {
    pub id: InstanceId,
    pub wallet: Wallet,
    pub state: InstanceState,
    pub vid: u16,
    pub pid: u16,
    pub serial: String,
    pub hidraw: Option<PathBuf>,
    pub transport: String,
    pub error: Option<String>,

    pub emulator: Box<dyn Emulator>,
    pub bridge: Option<Box<dyn Bridge>>,

    /// Drain task for the bridge's intercept channel. Kept so we can
    /// abort it when stopping the instance.
    pub log_drain: Option<JoinHandle<()>>,

    /// Started instant; log timestamps are millis since this. Using
    /// monotonic time avoids wall-clock skew when tests compare
    /// entries across log sources.
    pub started_at: Instant,

    /// Shared with the emulator's stdout/stderr reader tasks. Lines
    /// land here without our intervention; we slice on demand for
    /// `Logs` queries.
    pub emu_output: Arc<Mutex<VecDeque<String>>>,

    /// Decoded bridge messages — direction + hex. Filled by the
    /// `log_drain` task, drained on demand.
    pub bridge_log: Arc<Mutex<VecDeque<BridgeLog>>>,

    /// Lock-free counters incremented by the same `log_drain` task.
    /// Atomics so the daemon's IPC handler can read them without
    /// touching the buffer's mutex.
    pub bridge_stats: Arc<BridgeStatsCounters>,
}

/// Internal representation of one bridge log entry. Kept here (not
/// in `control`) so we can store an `Instant` instead of a u64; the
/// public `LogEntry` form converts on the way out.
#[derive(Debug, Clone)]
pub(crate) struct BridgeLog {
    pub at: Instant,
    pub direction: bridge::Direction,
    pub raw_hex: String,
}

#[derive(Debug, Default)]
pub(crate) struct BridgeStatsCounters {
    pub host_to_device_reports: AtomicU64,
    pub host_to_device_bytes: AtomicU64,
    pub device_to_host_reports: AtomicU64,
    pub device_to_host_bytes: AtomicU64,
}

impl BridgeStatsCounters {
    pub fn snapshot(&self, instance: &InstanceId) -> BridgeStatsSnapshot {
        use std::sync::atomic::Ordering::Relaxed;
        BridgeStatsSnapshot {
            instance: instance.clone(),
            host_to_device_reports: self.host_to_device_reports.load(Relaxed),
            host_to_device_bytes: self.host_to_device_bytes.load(Relaxed),
            device_to_host_reports: self.device_to_host_reports.load(Relaxed),
            device_to_host_bytes: self.device_to_host_bytes.load(Relaxed),
        }
    }
}

impl Instance {
    pub fn summary(&self) -> InstanceSummary {
        InstanceSummary {
            instance: self.id.clone(),
            wallet: self.wallet,
            state: self.state,
            vid: self.vid,
            pid: self.pid,
            serial: self.serial.clone(),
            hidraw: self.hidraw.clone(),
            transport: self.transport.clone(),
            error: self.error.clone(),
        }
    }

    /// Build a unified `Vec<LogEntry>` for an IPC response. Pulls
    /// from one or both sources depending on `source`, applies
    /// `tail`, sorts chronologically by `ts_ms`.
    pub fn collect_logs(&self, source: LogSource, tail: Option<usize>) -> Vec<LogEntry> {
        let mut out: Vec<LogEntry> = Vec::new();

        if matches!(source, LogSource::Emulator | LogSource::All) {
            let buf = self.emu_output.lock().unwrap();
            // The emulator's reader doesn't timestamp lines — we
            // approximate by spreading them evenly across the time
            // since `started_at`. Good enough for ordering against
            // bridge entries; for fidelity-critical use the bridge
            // direction stamp anyway.
            let now_ms = self.started_at.elapsed().as_millis() as u64;
            for line in buf.iter() {
                out.push(LogEntry {
                    ts_ms: now_ms,
                    source: LogSource::Emulator,
                    message: line.clone(),
                    direction: String::new(),
                    raw_hex: String::new(),
                });
            }
        }

        if matches!(source, LogSource::Bridge | LogSource::All) {
            let buf = self.bridge_log.lock().unwrap();
            for entry in buf.iter() {
                let ts_ms = entry
                    .at
                    .saturating_duration_since(self.started_at)
                    .as_millis() as u64;
                out.push(LogEntry {
                    ts_ms,
                    source: LogSource::Bridge,
                    message: String::new(),
                    direction: entry.direction.to_string(),
                    raw_hex: entry.raw_hex.clone(),
                });
            }
        }

        out.sort_by_key(|e| e.ts_ms);

        if let Some(n) = tail {
            let len = out.len();
            if len > n {
                out.drain(0..(len - n));
            }
        }

        out
    }
}
