---
title: hwwctl
description: Control plane for hardware-wallet emulators — drive Trezor, BitBox02, Coldcard, Specter, Ledger, and Jade emulators from end-to-end tests.
template: splash
hero:
  tagline: A daemon + CLI that drives hardware-wallet emulators from any language, with each emulator exposed as a real <code>/dev/hidraw*</code> device.
  actions:
    - text: Quick start
      link: /quick-start/
      icon: right-arrow
      variant: primary
    - text: View on GitHub
      link: https://github.com/n1rna/hwwctl
      icon: external
---

## What is hwwctl

`hwwctl` spawns hardware-wallet emulators (Trezor, BitBox02,
Coldcard, Specter DIY, Ledger, Jade) behind a real `/dev/hidraw*`
device via Linux UHID. From a desktop wallet application's
perspective, each emulator looks indistinguishable from physically
plugged-in hardware — `hidapi`'s `device_list()` returns the
configured VID/PID and a per-instance `serial_number`.

The CLI auto-spawns a long-lived **daemon** that owns every running
emulator. Tests talk to it through `hwwctl <subcommand>` — short-lived
invocations against a local Unix socket, JSON in / JSON out.

```text
test code  ──▶  hwwctl <cmd>  ──Unix socket──▶  hwwctl daemon
                                                    │
                                                    ├── BitBox02 simulator (TCP) ──▶ UHID ──▶ /dev/hidrawN
                                                    ├── Coldcard simulator (DGRAM) ─▶ UHID ──▶ /dev/hidrawN
                                                    └── Trezor / Ledger / Jade / Specter (direct TCP/UDP)
```

## Designed for tests

- **Stable error codes** — `BUNDLE_MISSING`, `BRIDGE_FAILED`,
  `INSTANCE_NOT_FOUND`, … Pattern-match on `code`, not message text.
- **Worker-isolated** — per-test sockets so parallel runners don't
  share instance state.
- **Per-instance serials** — two BitBox02s coexist with distinct
  `hidapi` serial numbers.
- **JSON output** — `--json` on every command for machine consumption.
- **Per-instance logs + counters** — query emulator stdout and bridge
  HID traffic per instance, with byte/packet counters.

## Supported wallets

| Wallet | Transport | Bridge | Status |
|---|---|---|---|
| BitBox02 | TCP | UHID (VID 0x03EB) | **Wired into the daemon** |
| Coldcard | Unix DGRAM | UHID (VID 0xD13E) | Bundle ready, daemon wiring pending |
| Trezor | UDP | direct | Bundle ready, daemon wiring pending |
| Specter DIY | TCP | direct | Bundle ready, daemon wiring pending |
| Ledger (Speculos) | TCP via Docker | direct | Bundle ready, daemon wiring pending |
| Jade (QEMU) | TCP via Docker | direct | Bundle ready, daemon wiring pending |

## Where to go next

- **[Quick start](/quick-start/)** — install, spawn your first
  emulator, see it in `hidapi`.
- **[CLI reference](/cli/)** — every subcommand, flag, and error code.
- **[Architecture](/architecture/)** — daemon model, registry actor,
  UHID bridge, process lifecycle.
- **[Wallets](/wallets/)** — per-wallet quirks and known issues.
- **[Development](/development/)** — building, testing, contributing.
