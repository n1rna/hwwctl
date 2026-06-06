# hwwctl

[![CI](https://github.com/n1rna/hwwctl/actions/workflows/ci.yaml/badge.svg)](https://github.com/n1rna/hwwctl/actions/workflows/ci.yaml)
[![Rust](https://1tt.dev/badge/rust-1.80+-orange.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Wallets](https://1tt.dev/badge/wallets-6-blue.svg?style=flat)](https://github.com/n1rna/hwwctl#supported-wallets)
[![License](https://1tt.dev/badge/license-MIT-green.svg)](https://github.com/n1rna/hwwctl/blob/main/LICENSE)
[![Linux](https://1tt.dev/badge/platform-linux-lightgrey.svg?logo=linux&logoColor=white)](https://github.com/n1rna/hwwctl)

A control plane for hardware-wallet emulators, designed for end-to-end
testing of desktop wallet applications. Spawns six wallet emulators
(Trezor, BitBox02, Coldcard, Specter DIY, Ledger, Jade), exposes each
as a real `/dev/hidraw*` device through Linux UHID, and gives tests a
stable JSON CLI to drive them.

The CLI auto-spawns a long-lived **daemon** that owns every running
emulator and its UHID bridge. Tests speak to it through `hwwctl` —
short-lived invocations against a Unix socket.

```
test code  ──▶  hwwctl <cmd>  ──Unix socket──▶  hwwctl daemon
                                                    │
                                                    ├── BitBox02 simulator (TCP) ──▶ UHID ──▶ /dev/hidrawN
                                                    ├── Coldcard simulator (DGRAM) ─▶ UHID ──▶ /dev/hidrawN
                                                    └── Trezor / Ledger / Jade / Specter (direct TCP/UDP)
```

Desktop apps discover the emulators through `hidapi` exactly as they
would discover real plugged-in hardware.

## Quick start

```bash
# One-time host setup (loads uhid + sets perms)
sudo modprobe uhid
just setup-udev

# Build
just build-release

# Auto-spawning daemon — first call starts it
./target/release/hwwctl ping
./target/release/hwwctl start bitbox02
# → {"kind":"started", "serial":"hwwctl-bb02-…", "hidraw":"/dev/hidrawN", ...}

# When done
./target/release/hwwctl shutdown
```

## Commands

| Command | Use |
|---|---|
| `hwwctl daemon` | Run the daemon explicitly (otherwise auto-spawned). |
| `hwwctl ping` | Liveness check; returns daemon + protocol versions. |
| `hwwctl start <wallet> [--no-wait] [--timeout N]` | Spawn an emulator instance. Returns serial + hidraw path + VID/PID. |
| `hwwctl stop <id>` | Idempotent teardown. |
| `hwwctl status [id]` | Snapshot of all instances (or one). |
| `hwwctl logs <id> [--tail N] [--source emulator\|bridge\|all]` | Unified timeline of emulator stdout and bridge HID traffic. |
| `hwwctl bridge-stats <id>` | Packet + byte counters per direction. |
| `hwwctl shutdown` | Drop all instances and exit the daemon. |

`--json` on any command emits machine-readable output for test
harnesses. Errors carry stable codes — `BUNDLE_MISSING`,
`BRIDGE_FAILED`, `INSTANCE_NOT_FOUND`, … — so tests pattern-match on
`code` rather than parsing English.

### Supported wallets

Phase 2a currently has BitBox02 wired into the daemon. The other five
return `WALLET_UNSUPPORTED` until wired in subsequent phases.

| Wallet | Transport | Bridge | Discovery |
|--------|-----------|--------|-----------|
| **BitBox02** | TCP | UHID (VID 0x03EB / PID 0x2403) | hidapi |
| **Coldcard** | Unix DGRAM | UHID (VID 0xD13E / PID 0xCC10) | hidapi |
| **Trezor** | UDP | direct (no bridge) | trezor-client / UDP |
| **Specter DIY** | TCP | direct (no bridge) | TCP |
| **Ledger** | TCP (Docker/Speculos) | direct (no bridge) | TCP |
| **Jade** | TCP (Docker/QEMU) | direct (no bridge) | TCP |

### Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `HWWCTL_SOCKET` | `$XDG_RUNTIME_DIR/hwwctl.sock` (fallback `/tmp/hwwctl.sock`) | Daemon socket path. Per-test override for parallel workers. |
| `HWWCTL_LOG` | `/tmp/hwwctl.log` | Daemon log file (when running via auto-spawn). |
| `HWWCTL_LOG_LEVEL` | `info,hwwctl=debug,bridge=debug,emulators=debug` | `tracing` env-filter for the daemon. |
| `HWWCTL_GITHUB_REPO` | `n1rna/hwwctl` | Repo for bundle downloads. |
| `TREZOR_FIRMWARE_PATH` | (none) | Path to a local `trezor-firmware/core/` checkout (bypasses bundle). |

## UHID bridge

For wallets that speak USB HID natively (BitBox02, Coldcard), `hwwctl`
creates a virtual HID device via `/dev/uhid`. The kernel exposes it
as `/dev/hidrawN`, and `hidapi`'s `device_list()` returns it with the
configured VID/PID and a per-instance `serial_number`. Two BitBox02s
can run concurrently because each gets a unique serial.

```
hidapi enumerate ─▶ /dev/hidrawN ─▶ (kernel HID) ─▶ /dev/uhid ─▶ GenericBridge ─▶ emulator
```

Linux only; UHID has no macOS / Windows equivalent. The CLI and
protocol crates compile on macOS for development convenience, but
the daemon refuses to start a UHID-backed wallet there.

### Permissions

Run `just setup-udev` once to install the udev rules at
`udev/99-hwwctl.rules`. This grants access to `/dev/uhid` and to the
hidraw nodes for the wallet VID/PIDs.

## Bundles

Each wallet's emulator is packaged as a downloadable bundle of binary
+ runtime data under `~/.hwwctl/bundles/{wallet}/`. The `hwwctl`
daemon resolves binaries through this layout.

Build a bundle locally:

```bash
just bundle-test bitbox02     # builds the simulator in Docker
just bundle-install bitbox02  # extracts into ~/.hwwctl/bundles/bitbox02
```

Ledger and Jade build on the host since their bundles wrap Docker
runtimes (Speculos / QEMU).

## Releases

Linux x86_64 binaries are published as
[GitHub release assets](https://github.com/n1rna/hwwctl/releases).
Each release attaches `hwwctl-linux-x86_64.tar.gz` and a `.sha256`
sidecar; the version string in `--version` matches the release tag.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — workspace structure, data
  flow, key types
- [Wallet Reference](docs/WALLETS.md) — per-wallet details, transport,
  build deps, known issues
- [Development Guide](docs/DEVELOPMENT.md) — building, testing, CI,
  adding new wallets

## License

See repository root for license details.
