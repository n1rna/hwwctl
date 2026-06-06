# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when
working with code in this repository.

## Project

`hwwctl` is a CLI + long-lived daemon for driving hardware-wallet
**emulators** from end-to-end tests. It supports six wallets (Trezor,
BitBox02, Coldcard, Specter DIY, Ledger, Jade), spawns each behind a
real `/dev/hidraw*` device through Linux UHID, and exposes everything
over a JSON Unix-socket control plane that test code calls from any
language.

Built with Rust + tokio. Linux-only at runtime (UHID requirement); the
crates compile on macOS for dev convenience but the daemon refuses to
start a UHID-backed wallet there.

## Commands

```bash
just build            # Debug build
just build-release    # Release build
just daemon           # Run daemon in foreground (normally auto-spawned)
just ctl <args>       # Run an hwwctl subcommand against the daemon
just test             # Run all tests (cargo test --workspace)
just lint             # Clippy (cargo clippy --workspace --all-targets)
just fmt              # Format (cargo fmt)
just ci               # Full CI check: fmt + lint + test
just logs             # Tail /tmp/hwwctl.log
just setup-udev       # Install udev rules for UHID + hidraw (one-time)
```

Single crate test: `cargo test -p <crate-name>` (crates: `hwwctl`,
`emulators`, `bridge`, `protocol`, `bundler`, `control`).

Integration tests (require bundles + /dev/uhid):
- `cargo test -p bridge --test e2e -- --ignored --test-threads=1`
- `cargo test -p bridge --test bb02_repro -- --ignored` — full UHID bridge flow against the BitBox02 simulator
- `cargo test -p bridge --test coldcard_e2e -- --ignored`
- `cargo test -p bridge --test hidapi_probe -- --ignored` — used by CI to confirm a running daemon's UHID device is enumerable by hidapi

System build deps: `libusb-1.0-0-dev libudev-dev pkg-config libclang-dev`.

## Architecture

### Crate dependency graph

```
hwwctl (binary, CLI + daemon)
  ├── control      (IPC protocol types — Request/Response/ErrorCode + framing)
  ├── emulators    (Emulator trait, process spawning)
  ├── bridge       (UHID virtual HID relay)
  │   ├── emulators
  │   └── protocol
  ├── protocol     (wire protocol decoders)
  └── bundler      (GitHub Release download + ~/.hwwctl/bundles/ storage)
      └── emulators
```

### Key types and data flow

- **`Request` / `Response`** (`crates/control/src/lib.rs`) — IPC enums.
  Length-prefixed JSON framing; `Request::Start` returns
  `Response::Started(InstanceSummary)` with the assigned hidraw path,
  HID serial, VID/PID, and transport. Stable `ErrorCode`s
  (`BUNDLE_MISSING`, `BRIDGE_FAILED`, `INSTANCE_NOT_FOUND`, …) so tests
  pattern-match on codes rather than message strings.
- **`Registry`** (`crates/hwwctl/src/daemon/registry.rs`) — Single-task
  actor that owns a `HashMap<InstanceId, Instance>`. All mutations
  serialize through one tokio task; no locks on the map. Commands
  arrive over an mpsc; each carries a oneshot reply.
- **`Instance`** (`crates/hwwctl/src/daemon/instance.rs`) — Per-instance
  state: emulator, bridge, captured emulator stdout (shared Arc with
  the emulator's reader tasks), bridge HID log ring buffer, atomic
  byte/packet counters.
- **`Emulator` trait** (`crates/emulators/src/lib.rs`) — Async lifecycle
  (`start`/`stop`/`health_check`). Two impls: `TrezorEmulator` (UDP) and
  `GenericEmulator` (TCP or Unix-socket children). `GenericEmulator`
  supports a `with_skip_probe_delay` mode for single-client simulators
  (BitBox02) where any readiness probe risks killing the simulator with
  SIGPIPE.
- **`Bridge` trait** (`crates/bridge/src/lib.rs`) — UHID relay
  producing `InterceptedMessage` streams. `GenericBridge` supports
  TCP, Unix STREAM, and Unix DGRAM transports. Each bridge takes an
  optional `serial` (UHID `uniq` field) so multiple emulators of the
  same wallet type coexist with distinct HID serials.
- **`BundleManager`** (`crates/bundler/src/lib.rs`) — Facade over
  `BundleStorage` + `GithubDownloader`. Bundles live at
  `~/.hwwctl/bundles/{wallet}/manifest.json`.

### Daemon lifecycle

`hwwctl <subcommand>` is a short-lived client. If the Unix socket
isn't accepting, the client auto-spawns a detached daemon (via
`setsid`), polls until the socket comes up, then sends one request and
exits. The daemon stays running until `hwwctl shutdown` or SIGTERM /
SIGINT.

Socket path resolution: `$HWWCTL_SOCKET` → `$XDG_RUNTIME_DIR/hwwctl.sock`
→ `/tmp/hwwctl.sock`. Per-worker tests should set `$HWWCTL_SOCKET` so
parallel workers don't share instance state.

### CLI / IPC

- `ping` — daemon liveness + version
- `start <wallet> [--no-wait] [--timeout N]` — spawn an instance,
  return `InstanceSummary`
- `stop <id>` — idempotent (unknown id → ok)
- `status [id]` — snapshot all or one
- `logs <id> [--tail N] [--source emulator|bridge|all]` — unified
  timeline; bridge entries carry direction + raw hex
- `bridge-stats <id>` — packet/byte counters per direction;
  subtract two snapshots to measure activity
- `shutdown` — drain all instances and exit the daemon

### UHID Bridge

```
Desktop App (hidapi) ↔ /dev/hidraw ↔ /dev/uhid ↔ GenericBridge ↔ Emulator
```

- **TCP transport** (BitBox02): eager connect, two tasks (read/write),
  report ID stripping
- **Unix DGRAM** (Coldcard): connect-on-first-write, bound client
  socket (MicroPython requires explicit path for sendto), auto-reconnect
- **UHID thread**: blocking thread polls `/dev/uhid` at 1ms intervals
  for output reports; shutdown via `oneshot::Receiver` — handles both
  `Ok(())` and `Err(Closed)`.

### Wallet-specific notes

- **Trezor**: Direct UDP, debug link on port+1. `TrezorWireClient`
  supports Initialize, GetFeatures, LoadDevice, GetPublicKey. Not yet
  wired into the daemon (returns `WALLET_UNSUPPORTED`).
- **BitBox02**: Wired into the daemon. Single-client TCP simulator —
  any readiness probe causes a SIGPIPE on the simulator before the
  bridge can connect, so the daemon uses `skip_probe_delay` (1500ms)
  and lets the bridge be the first client. The `bitbox-api` crate
  handles the noise-protocol pairing dance for tests that need an
  initialized device.
- **Coldcard**: Headless via bash launcher (opens /dev/null fds for
  display/LED pipes). `MICROPYPATH` needs leading colon to append (not
  replace) frozen module path. Not yet wired into the daemon.
- **Specter**: `SDL_VIDEODRIVER=dummy` to prevent segfault. USB VCP
  disabled by default — patched to enabled. Monkey-patches
  `Specter.setup` to inject test mnemonic. Gentle TCP probe (RST kills
  VCP server). Not yet wired into the daemon.
- **Ledger**: Speculos Docker, model `nanosp`. No UHID bridge — direct
  TCP discovery. Not yet wired into the daemon.
- **Jade**: QEMU ESP32 Docker. No UHID bridge — direct TCP discovery.
  Not yet wired into the daemon.

### Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `HWWCTL_SOCKET` | resolved per `default_socket_path()` | Daemon socket. |
| `HWWCTL_LOG` | `/tmp/hwwctl.log` | Daemon log file (auto-spawn path). |
| `HWWCTL_LOG_LEVEL` | `info,hwwctl=debug,bridge=debug,emulators=debug` | tracing filter. |
| `HWWCTL_GITHUB_REPO` | `n1rna/hwwctl` | Repo for bundle downloads. |
| `TREZOR_FIRMWARE_PATH` | (none) | Local trezor-firmware/core path. |
| `TREZOR_PORT` | `21324` | UDP port for the Trezor emulator. |

### Bundle build system

`scripts/build/{wallet}.sh` — per-wallet build scripts.
`scripts/docker/Dockerfile.{wallet}` — isolated build environments.
`just bundle-test {wallet}` builds in Docker;
`just bundle-install {wallet}` installs into `~/.hwwctl/bundles/`.

Key build notes:
- **Coldcard**: `rsync -L` to follow symlinks, exclude broken symlinks
  (`l-port`, `l-mpy`), patch version before compilation.
- **Ledger / Jade**: Use Docker at runtime (Speculos / QEMU), not just
  build time.
- **Specter**: Patch `hosts/usb.py` to enable USB by default.

### Permissions

`udev/99-hwwctl.rules` — udev rules for `/dev/uhid` and wallet hidraw
devices. Supports both real USB devices (`ATTRS{idVendor}`) and UHID
virtual devices (`KERNELS` matching `0003:VID:PID.*`). Uses
`TAG+="uaccess"` for systemd/logind auto-grant.

`just setup-udev` installs the rules, sets `/dev/uhid` permissions,
adds user to `plugdev` group.

### CI workflows

- `.github/workflows/ci.yaml` — `cargo fmt --check`, `clippy -D warnings`,
  `cargo test --workspace`, release build.
- `.github/workflows/hwwctl.yaml` — daemon smoke. Scaffold job (fast,
  no UHID) covers IPC + error codes; bitbox02-e2e job builds the
  simulator bundle from source, starts a daemon, spawns a BitBox02,
  verifies hidapi enumerates the resulting device.
- `.github/workflows/release-hwwctl.yaml` — builds Linux x86_64
  release binary + SHA256, attaches to a GitHub release on
  `hwwctl-v*` tag push.
- `.github/workflows/build-bundles.yaml` — manually-fired bundle
  build / release.
