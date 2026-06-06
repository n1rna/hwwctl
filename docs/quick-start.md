---
title: Quick start
description: Install hwwctl, spawn a BitBox02 emulator, and confirm hidapi sees it.
---

This page gets you from zero to a running emulator that your
desktop wallet app can talk to.

## Prerequisites

- Linux (UHID is a Linux kernel interface; macOS and Windows are not
  supported at runtime — the CLI compiles on macOS but the daemon
  refuses to spawn UHID-backed wallets there).
- Rust stable, or download a pre-built release.
- The `uhid` kernel module loaded:

  ```bash
  sudo modprobe uhid
  sudo chmod 666 /dev/uhid    # or run `just setup-udev` for a real udev rule
  ```

## Install

### Option A — pre-built release

```bash
curl -fsSL https://github.com/n1rna/hwwctl/releases/download/hwwctl-v0.1.0/hwwctl-linux-x86_64.tar.gz \
  | sudo tar -xz -C /usr/local/bin
hwwctl --version
```

### Option B — from source

```bash
git clone https://github.com/n1rna/hwwctl
cd hwwctl
just build-release
# Binary at ./target/release/hwwctl
```

## Install a wallet bundle

Emulator binaries (the actual Trezor / BitBox02 / etc. simulators)
ship as bundles under `~/.hwwctl/bundles/{wallet}/`. To build the
BitBox02 simulator locally:

```bash
just bundle-test bitbox02      # builds the simulator in Docker
just bundle-install bitbox02   # extracts into ~/.hwwctl/bundles/bitbox02
```

## First spawn

```bash
# Daemon auto-spawns on first command; no need to start it explicitly.
hwwctl --json ping
# {"kind":"pong","daemon_version":"0.1.0","protocol_version":1,"pid":12345}

hwwctl --json start bitbox02
# {"kind":"started","instance":"bitbox02-a3f912","wallet":"bitbox02",
#  "state":"running","vid":"0x03eb","pid":"0x2403",
#  "serial":"hwwctl-bb02-a3f912","hidraw":"/dev/hidraw0",
#  "transport":"tcp 127.0.0.1:43219"}
```

The `serial` field is the HID `uniq` value — your test code filters
hidapi enumeration by this to disambiguate from real devices or
other instances.

## Confirm hidapi sees it

```bash
# Anywhere with `lsusb` / `hidapi` installed:
ls -la /dev/hidraw*
# Should show the path returned in `start`'s JSON.

# Python with hidapi:
python3 -c "import hid; [print(d) for d in hid.enumerate(0x03eb, 0x2403)]"
```

## Inspect activity

```bash
hwwctl --json logs bitbox02-a3f912 --source emulator
hwwctl --json logs bitbox02-a3f912 --source bridge --tail 10
hwwctl --json bridge-stats bitbox02-a3f912
```

## Stop + shutdown

```bash
hwwctl --json stop bitbox02-a3f912
hwwctl --json shutdown        # drops every instance, exits the daemon
```

## Next

- **[CLI reference](/cli/)** — full subcommand surface.
- **[Architecture](/architecture/)** — what's happening under the
  hood when you call `start`.
