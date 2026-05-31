//! Per-instance state held by the daemon registry.

use std::path::PathBuf;

use bridge::Bridge;
use control::{InstanceId, InstanceState, InstanceSummary, Wallet};
use emulators::Emulator;
use tokio::task::JoinHandle;

/// One running emulator + its UHID bridge.
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
}
