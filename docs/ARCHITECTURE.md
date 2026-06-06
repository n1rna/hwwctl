# Architecture

## Workspace Structure

`hwwctl` is a Rust workspace with six crates:

```
hwwctl/
├── Cargo.toml                    # Workspace root + workspace.exclude for vendored firmware
├── crates/
│   ├── hwwctl/                   # Binary crate: CLI + daemon
│   │   └── src/
│   │       ├── main.rs           # clap entry point, client vs. daemon dispatch
│   │       ├── auto_spawn.rs     # Detached daemon spawn (setsid) when socket is missing
│   │       ├── client.rs         # Short-lived IPC client
│   │       ├── output.rs         # Human + JSON formatters
│   │       └── daemon/
│   │           ├── mod.rs        # Accept loop, request dispatch
│   │           ├── registry.rs   # Single-task actor over HashMap<InstanceId, Instance>
│   │           ├── instance.rs   # Per-instance state + log buffers + stats counters
│   │           ├── spawn.rs      # Per-wallet spawn dispatch (BitBox02 wired so far)
│   │           └── hidraw_scan.rs # /sys/class/hidraw matching by HID_UNIQ
│   ├── control/                  # IPC protocol types (shared by client + daemon)
│   │   └── src/lib.rs            # Request/Response/ErrorCode, length-prefixed JSON framing
│   ├── emulators/                # Emulator process management
│   │   └── src/
│   │       ├── lib.rs            # Emulator trait, EmulatorStatus, WalletType, TransportConfig
│   │       ├── trezor.rs         # TrezorEmulator: spawn micropython, UDP health check
│   │       ├── generic.rs        # GenericEmulator: TCP/Unix-socket children (BB02, CC, etc.)
│   │       └── wallet_config.rs  # Single source of truth: VID/PID, descriptor, ports
│   ├── bridge/                   # UHID virtual HID device layer
│   │   └── src/
│   │       ├── lib.rs            # Bridge trait, HidReport, InterceptedMessage, Direction
│   │       ├── generic.rs        # GenericBridge: TCP / Unix STREAM / Unix DGRAM ↔ UHID
│   │       ├── trezor.rs         # TrezorBridge: UDP ↔ UHID (unused in daemon, kept for parity)
│   │       └── uhid.rs           # VirtualHidDevice: /dev/uhid wrapper with per-instance serial
│   ├── protocol/                 # Wire protocol decoders
│   │   └── src/
│   │       ├── lib.rs            # DecodedMessage type
│   │       ├── trezor.rs         # Trezor HID framing decoder (header-only, no protobuf)
│   │       └── trezor_debug.rs   # Debug link client + wire client + screen layout parser
│   └── bundler/                  # Firmware bundle download & storage
│       └── src/
│           ├── lib.rs            # BundleManager facade, BundleStatus, RemoteBundle
│           ├── download.rs       # GithubDownloader, tarball extraction, asset name parsing
│           ├── storage.rs        # BundleStorage: ~/.hwwctl/bundles/ layout, manifest I/O
│           └── manifest.rs       # BundleManifest struct (JSON serializable)
├── scripts/
│   ├── build/                    # Per-wallet build scripts (trezor.sh, bitbox02.sh, etc.)
│   └── docker/                   # Dockerfiles for isolated builds
└── justfile                      # Task runner recipes
```

## Crate Dependency Graph

```
hwwctl (binary, CLI + daemon)
  ├── control      (IPC protocol types)
  ├── emulators    (Emulator trait, process spawning)
  ├── bridge       (UHID bridge)
  │   ├── emulators
  │   └── protocol
  ├── protocol     (wire decoders)
  └── bundler      (download/storage)
      └── emulators
```

## Process Model

Two roles share the `hwwctl` binary:

- **Client** — short-lived. `hwwctl <subcommand>` opens the Unix
  socket, sends one length-prefixed JSON `Request`, reads one
  `Response`, exits with code 0 (success), 1 (daemon-side error), or
  2 (transport / argument problem).
- **Daemon** — long-lived. Owns every running emulator + UHID
  bridge. Listens on `$HWWCTL_SOCKET` (resolved via
  `$XDG_RUNTIME_DIR/hwwctl.sock`, fallback `/tmp/hwwctl.sock`). If
  the client finds the socket missing, it auto-spawns a detached
  daemon via `setsid` and polls until the socket comes up.

```
┌──────────────────────────────────────────────────────────────┐
│  hwwctl daemon                                                │
│                                                               │
│   Accept loop ─▶ handle_connection(stream)                    │
│                       │                                        │
│                       ▼                                        │
│                  read_frame ─▶ dispatch(Request)               │
│                                       │                        │
│                                       ▼                        │
│                                Registry (one task)             │
│                                       │                        │
│                                       ▼                        │
│                                instances: HashMap              │
│                                       │                        │
│            ┌──────────────────────────┼──────────────────────┐ │
│            ▼                          ▼                      ▼ │
│  Instance(bitbox02-a3f912)  Instance(bitbox02-7c01b3)  ...     │
│   ├── emulator (Box<dyn>)                                       │
│   ├── bridge   (Box<dyn>)                                       │
│   ├── emu_output  (Arc<Mutex<VecDeque<String>>>)                │
│   ├── bridge_log  (Arc<Mutex<VecDeque<BridgeLog>>>)             │
│   └── bridge_stats (Arc<atomic counters>)                       │
└──────────────────────────────────────────────────────────────┘
```

The registry actor serializes every state change through one tokio
task; the `HashMap` has no lock. Per-instance log buffers and
counters are shared `Arc`s with the background drain tasks so the
actor can read them without contending with bridge ingest.

## Data Flow per Instance

```
            ┌────────────────────────────────────────────┐
            │             hwwctl daemon                  │
            │                                            │
            │   spawn::start_bitbox02:                   │
            │     1. resolve bundle binary               │
            │     2. allocate free TCP port + HID serial │
            │     3. GenericEmulator.start (skip probe)  │
            │     4. GenericBridge.start (UHID create)   │
            │     5. wait for /dev/hidrawN by HID_UNIQ   │
            │     6. spawn drain task → log + counters   │
            │     7. insert into Registry                │
            └─────┬──────────────────────┬───────────────┘
                  │                      │
        ┌─────────┴──────────┐   ┌───────┴──────────┐
        │  Emulator process  │   │  UHID device     │
        │  (bitbox02-sim)    │   │  /dev/uhid       │
        │                    │   │       │          │
        │  TCP listen :N     │   │       ▼          │
        │       │            │   │  /dev/hidrawN    │
        └───────┼────────────┘   └───────┼──────────┘
                │ TCP                    │ HID reports
                └──────── bridge ────────┘
```

## Key Types

### `Request` / `Response` (`crates/control/src/lib.rs`)

Internally-tagged serde enums. Variants: `Ping`, `Start`, `Stop`,
`Status`, `Logs`, `BridgeStats`, `Shutdown`. Length-prefixed JSON
framing — 4-byte big-endian length then payload, max 16 MiB. Stable
`ErrorCode` values (SCREAMING_SNAKE) — `BUNDLE_MISSING`,
`BRIDGE_FAILED`, `INSTANCE_NOT_FOUND`, `WALLET_UNSUPPORTED`, … —
so callers pattern-match on `code` not message text.

### `Registry` (`crates/hwwctl/src/daemon/registry.rs`)

Owns `HashMap<InstanceId, Instance>`. Accepts `Command`s via
`mpsc::Sender`, replies via `oneshot::Sender`. The accept loop sends
each incoming `Request` as a `Command` and awaits the reply before
writing the IPC `Response`. Shutdown drops every instance with a
`stop_instance` call.

### `Instance` (`crates/hwwctl/src/daemon/instance.rs`)

```rust
pub(crate) struct Instance {
    pub id: InstanceId,
    pub wallet: Wallet,
    pub state: InstanceState,
    pub vid: u16, pub pid: u16,
    pub serial: String,           // HID_UNIQ, unique per instance
    pub hidraw: Option<PathBuf>,  // best-effort path from /sys/class/hidraw
    pub transport: String,        // human-readable, e.g. "tcp 127.0.0.1:43219"
    pub emulator: Box<dyn Emulator>,
    pub bridge: Option<Box<dyn Bridge>>,
    pub log_drain: Option<JoinHandle<()>>,
    pub started_at: Instant,
    pub emu_output: Arc<Mutex<VecDeque<String>>>,    // shared with emulator's stdio readers
    pub bridge_log: Arc<Mutex<VecDeque<BridgeLog>>>, // filled by log_drain
    pub bridge_stats: Arc<BridgeStatsCounters>,      // atomics
}
```

### `Emulator` trait (`crates/emulators/src/lib.rs`)

```rust
#[async_trait]
pub trait Emulator: Send + Sync {
    fn wallet_type(&self) -> WalletType;
    fn status(&self) -> EmulatorStatus;
    fn transport(&self) -> TransportConfig;
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
    async fn health_check(&self) -> bool;
    fn drain_output(&mut self) -> Vec<String>;
}
```

Implementations:
- **`TrezorEmulator`** — spawns `trezor-emu-core`, UDP probe.
- **`GenericEmulator`** — TCP/Unix children. Supports
  `with_skip_probe_delay(d)` for single-client simulators
  (BitBox02) where any readiness probe causes SIGPIPE.

`GenericEmulator::output_buffer()` returns an `Arc<Mutex<VecDeque<String>>>`
shared with the spawned stdio reader tasks. The daemon's `Instance`
holds a clone and can slice it for `Logs` queries without owning the
emulator exclusively.

### `Bridge` trait (`crates/bridge/src/lib.rs`)

```rust
#[async_trait]
pub trait Bridge: Send + Sync {
    async fn start(&mut self) -> anyhow::Result<mpsc::UnboundedReceiver<InterceptedMessage>>;
    async fn stop(&mut self) -> anyhow::Result<()>;
    fn is_running(&self) -> bool;
}
```

`GenericBridge` covers all UHID-backed wallets. Built with
`GenericBridgeConfig::new(...).with_serial(s)` so each instance pins a
distinct HID `uniq` value — desktop apps using `hidapi` then
disambiguate by `serial_number`.

### `BundleManager` (`crates/bundler/src/lib.rs`)

Facade over `BundleStorage` (`~/.hwwctl/bundles/`) and
`GithubDownloader` (GitHub Releases API). Resolves the emulator
binary path the daemon needs to spawn.

### `TransportConfig` (`crates/emulators/src/lib.rs`)

```rust
pub enum TransportConfig {
    Udp { host: String, port: u16 },   // Trezor
    Tcp { host: String, port: u16 },   // BitBox02, Specter, Ledger, Jade
    UnixSocket { path: PathBuf },       // Coldcard (DGRAM probe)
}
```

## How a BitBox02 Start Works

1. Client sends `Request::Start { wallet: BitBox02, wait_ready: true, timeout_secs: None }`.
2. Registry actor calls `spawn::start_bitbox02`:
   - Resolves the simulator binary via `BundleManager::emulator_binary_path`. Missing → `BUNDLE_MISSING`.
   - Picks an unused TCP port (bind ephemeral, drop).
   - Generates `InstanceId = "bitbox02-<6 hex>"` and `serial = "hwwctl-bb02-<6 hex>"`.
   - Builds `GenericEmulator` with `--port <N>` and `with_skip_probe_delay(1500ms)`. `start()` spawns the child, sleeps, marks `Running` (no TCP probe — the simulator is single-client).
   - Builds `GenericBridge` with the unique `serial` and starts it. Creates `/dev/uhid` device → kernel exposes as `/dev/hidrawN`.
   - Spawns a drain task that consumes the bridge's `InterceptedMessage` channel into `bridge_log` (ring, capped at 500) and increments `bridge_stats` atomic counters.
   - If `wait_ready`, polls `/sys/class/hidraw/*/device/uevent` for a `HID_UNIQ=<serial>` match (up to 3 s).
   - Inserts the `Instance` into the registry.
3. Registry replies `Response::Started(InstanceSummary)` with vid/pid/serial/hidraw/transport.

`Stop` is symmetric and idempotent: unknown id → ok. `Shutdown`
iterates all instances and calls `stop_instance` on each.

## Bundle System

```
~/.hwwctl/bundles/
├── trezor/
│   ├── manifest.json         # wallet_type, version, emulator_binary, ...
│   ├── trezor-emu-core
│   ├── lib/                  # bundled shared libraries
│   ├── src/
│   └── run.sh                # wrapper script (sets LD_LIBRARY_PATH)
├── bitbox02/
│   ├── manifest.json
│   └── bitbox02-simulator
└── ...
```

GitHub Releases assets follow the convention
`hwwctl-{wallet}-{platform}.tar.gz`.

## UHID Bridge Architecture

The bridge creates a virtual USB HID device via `/dev/uhid` that
appears as `/dev/hidrawN` to host applications. A dedicated blocking
thread owns the `UHIDDevice` and handles both directions:

- **Emulator → Host**: TCP / Unix recv → channel → UHID input report write
- **Host → Emulator**: UHID output report poll → channel → TCP / Unix send

The UHID `CREATE2` ioctl is synchronous, but the kernel still walks
through the HID subsystem to create `hidraw` asynchronously. `spawn`
polls `/sys/class/hidraw` for the matching `HID_UNIQ` and returns
the discovered `/dev/hidrawN` path in `InstanceSummary::hidraw`.

Linux only. UHID has no macOS / Windows equivalent.

Requires `/dev/uhid` access — `just setup-udev` installs the rules.
