//! Per-wallet spawn dispatch.
//!
//! `start()` is the single entry point used by the registry actor.
//! Each wallet variant has its own private constructor that builds an
//! `emulators::Emulator` + `bridge::Bridge` pair with per-instance
//! configuration (unique port, profile dir, HID serial).
//!
//! Adding a new wallet means adding one match arm and one builder
//! function. Wallets that aren't supported yet return
//! [`ErrorCode::WalletUnsupported`].

use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use bridge::generic::{BridgeTransport, GenericBridge, GenericBridgeConfig};
#[cfg(target_os = "linux")]
use bridge::{Bridge, InterceptedMessage};
#[cfg(target_os = "linux")]
use bundler::BundleManager;
use control::{Error as CtlError, ErrorCode, InstanceId, InstanceSummary, StartRequest};
#[cfg(target_os = "linux")]
use control::{InstanceState, Wallet};
#[cfg(target_os = "linux")]
use emulators::generic::GenericEmulator;
#[cfg(target_os = "linux")]
use emulators::{wallet_config, Emulator, EmulatorStatus, TransportConfig, WalletType};
#[cfg(target_os = "linux")]
use tokio::sync::mpsc;
#[cfg(target_os = "linux")]
use tracing::{debug, info, warn};

#[cfg(target_os = "linux")]
use super::hidraw_scan;
use super::instance::Instance;

/// How long we wait, post-bridge-start, for `/dev/hidrawN` to be
/// enumerable by the kernel. UHID device creation is synchronous from
/// our side, but the kernel hotplugs the hidraw node asynchronously.
#[cfg(target_os = "linux")]
const HIDRAW_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn start(
    instances: &mut HashMap<InstanceId, Instance>,
    req: StartRequest,
) -> Result<InstanceSummary, CtlError> {
    #[cfg(target_os = "linux")]
    {
        match req.wallet {
            Wallet::BitBox02 => start_bitbox02(instances, req).await,
            _ => Err(CtlError::new(
                ErrorCode::WalletUnsupported,
                format!(
                    "{} is not wired into the daemon yet — currently only `bitbox02` is supported",
                    req.wallet
                ),
            )),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (instances, req);
        Err(CtlError::new(
            ErrorCode::WalletUnsupported,
            format!(
                "hwwctl daemon requires Linux (`/dev/uhid`); current host is {}. \
                 Build and run the daemon inside the Linux test container.",
                std::env::consts::OS,
            ),
        ))
    }
}

// ── BitBox02 ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
async fn start_bitbox02(
    instances: &mut HashMap<InstanceId, Instance>,
    req: StartRequest,
) -> Result<InstanceSummary, CtlError> {
    let cfg = &wallet_config::BITBOX02;
    let hid = cfg
        .hid
        .as_ref()
        .expect("BitBox02 config always has hid set");

    // 1. Find the bundle binary.
    let bundle = bundle_manager()?;
    let bin_path = bundle
        .emulator_binary_path(WalletType::BitBox02)
        .ok_or_else(|| {
            CtlError::new(
                ErrorCode::BundleMissing,
                "BitBox02 simulator bundle is not installed. Run `just bundle-install bitbox02` \
                 or download it via the TUI.",
            )
        })?;
    if !bin_path.exists() {
        return Err(CtlError::new(
            ErrorCode::BundleMissing,
            format!(
                "BitBox02 simulator binary not found at {} — bundle manifest references a \
                 missing file, reinstall the bundle.",
                bin_path.display()
            ),
        ));
    }
    let bundle_dir = bin_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 2. Allocate per-instance resources.
    let id_suffix = random_suffix();
    let id = InstanceId::new(format!("bitbox02-{id_suffix}"));
    let serial = format!("hwwctl-bb02-{id_suffix}");
    let port = pick_free_tcp_port().map_err(|e| {
        CtlError::new(
            ErrorCode::ResourceExhausted,
            format!("could not find a free TCP port for the simulator: {e:#}"),
        )
    })?;
    let profile_dir = PathBuf::from(format!("/tmp/hwwctl-{id}"));
    if let Err(e) = std::fs::create_dir_all(&profile_dir) {
        return Err(CtlError::new(
            ErrorCode::Internal,
            format!(
                "could not create profile dir {}: {e}",
                profile_dir.display()
            ),
        ));
    }

    info!(
        %id,
        port,
        serial = %serial,
        binary = %bin_path.display(),
        "spawning BitBox02 instance"
    );

    // 3. Build and start the emulator process.
    let transport = TransportConfig::Tcp {
        host: "127.0.0.1".into(),
        port,
    };
    let port_str = port.to_string();
    let timeout_secs = req.timeout_secs.unwrap_or(cfg.startup_timeout_secs);
    let mut emu = GenericEmulator::new(
        WalletType::BitBox02,
        bin_path.clone(),
        bundle_dir,
        profile_dir.clone(),
        transport.clone(),
    )
    .with_arg("--port")
    .with_arg(&port_str)
    .with_startup_timeout(Duration::from_secs(timeout_secs))
    // BitBox02 simulator is single-client. Any readiness probe —
    // even SO_LINGER(0)/RST — races the simulator's initial socket
    // write and gives it SIGPIPE before our bridge gets a chance to
    // connect. Skip the probe; the bridge connect is the first
    // session, and a stuck-binding simulator surfaces as a clean
    // ECONNREFUSED a moment later.
    .with_skip_probe_delay(Duration::from_millis(1500));

    if let Err(e) = emu.start().await {
        return Err(CtlError::new(
            ErrorCode::SpawnFailed,
            format!("BitBox02 simulator failed to start: {e:#}"),
        ));
    }
    match emu.status() {
        emulators::EmulatorStatus::Running => {}
        emulators::EmulatorStatus::Error(msg) => {
            // Best-effort tidy-up.
            let _ = emu.stop().await;
            return Err(CtlError::new(
                ErrorCode::StartupTimeout,
                format!("BitBox02 simulator never became reachable: {msg}"),
            ));
        }
        other => {
            let _ = emu.stop().await;
            return Err(CtlError::new(
                ErrorCode::Internal,
                format!("BitBox02 simulator in unexpected state after start: {other}"),
            ));
        }
    }

    // 4. Build and start the UHID bridge.
    let bridge_cfg = GenericBridgeConfig::new(
        hid.vid,
        hid.pid,
        cfg.display_name,
        hid.report_descriptor,
        BridgeTransport::Tcp {
            host: "127.0.0.1".into(),
            port,
        },
    )
    .with_serial(serial.clone());

    let mut bridge = GenericBridge::new(bridge_cfg);
    let bridge_rx: mpsc::UnboundedReceiver<InterceptedMessage> = match bridge.start().await {
        Ok(rx) => rx,
        Err(e) => {
            // Bridge failed; tear the emulator down so we don't leak it.
            let _ = emu.stop().await;
            return Err(CtlError::new(
                ErrorCode::BridgeFailed,
                format!(
                    "UHID bridge for BitBox02 failed to start: {e:#}. Common cause: \
                     /dev/uhid is not writable (run `just setup-udev` once, or check group \
                     membership)."
                ),
            ));
        }
    };

    // 5. Spawn a background drain so the bridge's mpsc doesn't block.
    //    Phase 2b will route this into a per-instance log buffer; for
    //    now we just consume.
    let log_drain = {
        let id = id.clone();
        tokio::spawn(async move {
            let mut rx = bridge_rx;
            while let Some(msg) = rx.recv().await {
                debug!(%id, ?msg.direction, "bridge msg");
            }
        })
    };

    // 6. Wait for /dev/hidrawN to be enumerable (best-effort).
    let hidraw = if req.wait_ready {
        match hidraw_scan::find_hidraw_by_serial(&serial, HIDRAW_SETTLE_TIMEOUT).await {
            Some(p) => Some(p),
            None => {
                // Not fatal — `hidapi` enumeration on the desktop side
                // doesn't strictly require we map the path here. Log
                // and continue.
                warn!(
                    %id,
                    serial = %serial,
                    "could not find hidraw node by serial within timeout — \
                     desktop hidapi may still enumerate it, but `hidraw` field \
                     will be null in the response"
                );
                None
            }
        }
    } else {
        None
    };

    let inst = Instance {
        id: id.clone(),
        wallet: Wallet::BitBox02,
        state: InstanceState::Running,
        vid: hid.vid,
        pid: hid.pid,
        serial,
        hidraw,
        transport: format!("tcp 127.0.0.1:{port}"),
        error: None,
        emulator: Box::new(emu),
        bridge: Some(Box::new(bridge)),
        log_drain: Some(log_drain),
    };

    let summary = inst.summary();
    instances.insert(id, inst);
    Ok(summary)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn bundle_manager() -> Result<BundleManager, CtlError> {
    let repo = std::env::var("HWWTUI_GITHUB_REPO").unwrap_or_else(|_| "n1rna/hwwtui".to_string());
    BundleManager::new(&repo).map_err(|e| super::internal_err(e))
}

#[cfg(target_os = "linux")]
fn random_suffix() -> String {
    // 6 hex chars = 24 bits of entropy. Plenty for the (small) set of
    // concurrently-running instances. Avoids dragging in the `rand`
    // crate by hashing the current time + pid; this is not
    // cryptographic — only collision-avoidance.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;
    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    std::process::id().hash(&mut h);
    // Mix in something instance-counter-ish so two calls in the same
    // nanosecond still differ.
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    n.hash(&mut h);
    format!("{:06x}", (h.finish() as u32) & 0xff_ffff)
}

#[cfg(target_os = "linux")]
fn pick_free_tcp_port() -> anyhow::Result<u16> {
    // Bind to 127.0.0.1:0 — the kernel picks an unused port. Drop the
    // listener immediately; the port stays in TIME_WAIT-free state on
    // Linux because we never accepted. Race window: minimal, and the
    // simulator will fail loudly if we lose it.
    let sock = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = sock.local_addr()?.port();
    drop(sock);
    Ok(port)
}
