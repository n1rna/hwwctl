//! IPC protocol for the `hwwctl` daemon.
//!
//! One length-prefixed JSON message per request, one per response. Both
//! sides speak the same enums so the daemon and the CLI cannot drift
//! silently — adding a new request variant fails compilation everywhere
//! that pattern-matches.
//!
//! The shape is deliberately minimal: requests carry the data they need,
//! responses carry either a typed payload or a structured [`Error`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bumped whenever the wire format changes in a way that older clients
/// would mis-decode. The daemon stamps every response with its current
/// value so a stale `hwwctl` binary can detect drift and bail loudly
/// instead of acting on garbled data.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default Unix socket path when `HWWCTL_SOCKET` is unset and
/// `$XDG_RUNTIME_DIR` is unavailable. Tests may override either.
pub const FALLBACK_SOCKET_PATH: &str = "/tmp/hwwctl.sock";

// ── IDs and identifiers ────────────────────────────────────────────────────────

/// Identifies which hardware wallet model an emulator instance represents.
///
/// Mirrors `emulators::WalletType` but lives here so the control crate
/// stays free of the emulator dep — clients only need protocol types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wallet {
    Trezor,
    #[serde(rename = "bitbox02")]
    BitBox02,
    Coldcard,
    Specter,
    Ledger,
    Jade,
}

impl Wallet {
    pub fn as_str(&self) -> &'static str {
        match self {
            Wallet::Trezor => "trezor",
            Wallet::BitBox02 => "bitbox02",
            Wallet::Coldcard => "coldcard",
            Wallet::Specter => "specter",
            Wallet::Ledger => "ledger",
            Wallet::Jade => "jade",
        }
    }
}

impl std::str::FromStr for Wallet {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "trezor" => Ok(Wallet::Trezor),
            "bitbox02" | "bb02" => Ok(Wallet::BitBox02),
            "coldcard" | "cc" => Ok(Wallet::Coldcard),
            "specter" => Ok(Wallet::Specter),
            "ledger" => Ok(Wallet::Ledger),
            "jade" => Ok(Wallet::Jade),
            other => Err(format!("unknown wallet '{other}'")),
        }
    }
}

impl std::fmt::Display for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Opaque identifier for a running emulator instance owned by the daemon.
///
/// Format: `"{wallet}-{6 hex chars}"`, e.g. `bitbox02-a3f912`. Clients
/// treat it as opaque — generation lives in the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(pub String);

impl InstanceId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Requests ──────────────────────────────────────────────────────────────────

/// Top-level request envelope. One per IPC message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Liveness check — returns daemon version + protocol version.
    Ping,
    /// Spawn a new emulator instance.
    Start(StartRequest),
    /// Terminate an instance. Idempotent — unknown ids return Ok.
    Stop(StopRequest),
    /// Snapshot of one instance, or all if `instance` is None.
    Status(StatusRequest),
    /// Ask the daemon to exit cleanly, dropping all instances.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRequest {
    pub wallet: Wallet,
    /// Block until the bridge is fully ready (hidraw enumerable). When
    /// `false`, the daemon returns as soon as the child process is
    /// spawned and the transport is reachable; the test layer is then
    /// responsible for its own readiness check.
    #[serde(default = "default_true")]
    pub wait_ready: bool,
    /// Override the default startup timeout for this instance.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopRequest {
    pub instance: InstanceId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusRequest {
    /// `None` ⇒ snapshot all instances.
    #[serde(default)]
    pub instance: Option<InstanceId>,
}

// ── Responses ─────────────────────────────────────────────────────────────────

/// Top-level response envelope. The daemon always replies exactly once
/// per request with one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Pong(PongInfo),
    Started(InstanceSummary),
    Stopped,
    /// Snapshot list. Modeled as a struct variant rather than a
    /// newtype-wrapping-`Vec` because serde's internally-tagged
    /// representation can't flatten a sequence into the variant's
    /// JSON object.
    Status {
        instances: Vec<InstanceSummary>,
    },
    ShuttingDown,
    Error(Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongInfo {
    pub daemon_version: String,
    pub protocol_version: u32,
    pub pid: u32,
}

/// Public snapshot of a running instance. Serializable so the TS test
/// client can decode it directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub instance: InstanceId,
    pub wallet: Wallet,
    pub state: InstanceState,
    /// USB vendor id presented to the host (e.g. `0x03EB`). Hex-encoded
    /// in JSON for readability via [`hex_u16`] serializer.
    #[serde(with = "hex_u16")]
    pub vid: u16,
    #[serde(with = "hex_u16")]
    pub pid: u16,
    /// HID `uniq` value — the serial number the desktop's hidapi will
    /// read out of `/dev/hidrawN`. Unique per instance so two BitBox02s
    /// running concurrently can be told apart.
    pub serial: String,
    /// Best-effort path to the `/dev/hidrawN` node, if discovery
    /// succeeded. Tests should prefer `serial` for filtering.
    pub hidraw: Option<PathBuf>,
    /// Underlying transport between daemon and emulator process, for
    /// debugging.
    pub transport: String,
    /// Last error message, if `state == Error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Starting,
    Running,
    Stopping,
    Error,
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Structured error returned in [`Response::Error`]. Tests pattern-match
/// on [`code`](Self::code) rather than the human message.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Stable error codes. Add new variants instead of repurposing existing
/// ones — the TS client switches on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Request couldn't be parsed at all.
    BadRequest,
    /// Wallet type not yet supported by this daemon build.
    WalletUnsupported,
    /// Bundle for the wallet is not installed locally.
    BundleMissing,
    /// Emulator child process failed to spawn.
    SpawnFailed,
    /// Emulator started but its transport never became reachable.
    StartupTimeout,
    /// UHID bridge couldn't be created (likely permissions on /dev/uhid).
    BridgeFailed,
    /// `stop`/`status` referenced an instance id the daemon doesn't know.
    InstanceNotFound,
    /// Out of free TCP ports / socket paths to assign.
    ResourceExhausted,
    /// Catch-all — daemon-side bug, message has detail.
    Internal,
}

// ── Framing ───────────────────────────────────────────────────────────────────

/// Per-connection framing: 4-byte big-endian length, then JSON payload.
///
/// Chosen over newline-delimited JSON so binary payloads can be added
/// later (e.g. screen frames) without re-framing. 16 MiB max per
/// message is plenty for any control-plane payload and small enough
/// that an accidental garbage-length read doesn't try to allocate
/// gigabytes.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Write one frame to an async stream. Cancellation-safe iff the caller
/// drops the stream — partial writes are not retried.
pub async fn write_frame<W: AsyncWriteExt + Unpin, T: Serialize>(
    w: &mut W,
    msg: &T,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    if bytes.len() > MAX_FRAME_BYTES {
        anyhow::bail!("frame size {} exceeds limit {MAX_FRAME_BYTES}", bytes.len());
    }
    let len = (bytes.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Read one frame from an async stream. Returns `Ok(None)` on clean EOF
/// before any bytes arrive, so the daemon's accept loop can distinguish
/// "client hung up" from a real protocol error.
pub async fn read_frame<R: AsyncReadExt + Unpin, T: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> anyhow::Result<Option<T>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        anyhow::bail!("incoming frame length {len} exceeds limit {MAX_FRAME_BYTES}");
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

// ── Socket-path resolution ────────────────────────────────────────────────────

/// Resolve the canonical control-socket path. Order:
///
/// 1. `$HWWCTL_SOCKET` if set (lets tests/CI inject a unique path).
/// 2. `$XDG_RUNTIME_DIR/hwwctl.sock` if `XDG_RUNTIME_DIR` is set and
///    writable — standard systemd-user location, gets cleaned up at
///    logout.
/// 3. [`FALLBACK_SOCKET_PATH`] otherwise. Last-resort `/tmp`-based
///    path; the daemon refuses to overwrite a stale socket from a
///    different user.
pub fn default_socket_path() -> PathBuf {
    if let Ok(v) = std::env::var("HWWCTL_SOCKET") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("hwwctl.sock");
        }
    }
    PathBuf::from(FALLBACK_SOCKET_PATH)
}

// ── Hex serde helper ──────────────────────────────────────────────────────────

mod hex_u16 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:#06x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        let raw = String::deserialize(d)?;
        let body = raw.strip_prefix("0x").unwrap_or(&raw);
        u16::from_str_radix(body, 16).map_err(serde::de::Error::custom)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[test]
    fn wallet_parse_roundtrip() {
        for w in [
            Wallet::Trezor,
            Wallet::BitBox02,
            Wallet::Coldcard,
            Wallet::Specter,
            Wallet::Ledger,
            Wallet::Jade,
        ] {
            let s: Wallet = w.as_str().parse().unwrap();
            assert_eq!(s, w);
        }
        assert!("nope".parse::<Wallet>().is_err());
    }

    #[test]
    fn wallet_aliases() {
        assert_eq!("bb02".parse::<Wallet>().unwrap(), Wallet::BitBox02);
        assert_eq!("cc".parse::<Wallet>().unwrap(), Wallet::Coldcard);
    }

    #[test]
    fn error_serializes_with_screaming_snake_code() {
        let e = Error::new(ErrorCode::InstanceNotFound, "nope");
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"INSTANCE_NOT_FOUND\""), "got {s}");
    }

    #[test]
    fn instance_summary_serializes_hex_vid() {
        let summary = InstanceSummary {
            instance: InstanceId::new("bitbox02-a3f912"),
            wallet: Wallet::BitBox02,
            state: InstanceState::Running,
            vid: 0x03EB,
            pid: 0x2403,
            serial: "hwwctl-bb02-a3f912".into(),
            hidraw: None,
            transport: "tcp 127.0.0.1:15423".into(),
            error: None,
        };
        let s = serde_json::to_string(&summary).unwrap();
        assert!(s.contains("0x03eb"), "vid missing hex form: {s}");
        assert!(s.contains("0x2403"), "pid missing hex form: {s}");
    }

    #[tokio::test]
    async fn framing_roundtrip() {
        let (mut a, mut b) = duplex(4096);
        let req = Request::Ping;
        write_frame(&mut a, &req).await.unwrap();
        let got: Option<Request> = read_frame(&mut b).await.unwrap();
        assert!(matches!(got, Some(Request::Ping)));
    }

    #[tokio::test]
    async fn framing_clean_eof_returns_none() {
        let (a, mut b) = duplex(4096);
        drop(a);
        let got: anyhow::Result<Option<Request>> = read_frame(&mut b).await;
        assert!(matches!(got, Ok(None)));
    }

    #[test]
    fn default_socket_path_respects_env() {
        // Set HWWCTL_SOCKET and verify it wins. Use a path that
        // wouldn't otherwise match anything to avoid colliding with
        // any inherited test env.
        let saved = std::env::var("HWWCTL_SOCKET").ok();
        std::env::set_var("HWWCTL_SOCKET", "/tmp/hwwctl-test-override.sock");
        let p = default_socket_path();
        assert_eq!(p, PathBuf::from("/tmp/hwwctl-test-override.sock"));
        match saved {
            Some(v) => std::env::set_var("HWWCTL_SOCKET", v),
            None => std::env::remove_var("HWWCTL_SOCKET"),
        }
    }
}
