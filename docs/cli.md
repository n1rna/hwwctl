---
title: CLI reference
description: Every hwwctl subcommand, its flags, and the JSON shape it returns.
---

`hwwctl` is the single binary; client and daemon share the executable.
Short-lived invocations (anything except `daemon`) talk to a running
daemon over a Unix socket, auto-spawning one if the socket isn't
live.

## Global flags

| Flag | Source | Default | Purpose |
|---|---|---|---|
| `--socket <PATH>` | `--socket`, then `$HWWCTL_SOCKET` | `$XDG_RUNTIME_DIR/hwwctl.sock` → `/tmp/hwwctl.sock` | Daemon socket. Per-test override for parallel workers. |
| `--json` | flag | off | Emit machine-readable JSON instead of human text. |

Exit codes: `0` (success), `1` (daemon-side error with structured
`code` on stdout), `2` (transport / argument problem; message on
stderr).

## `daemon`

```bash
hwwctl daemon [--log-file PATH]
```

Run the daemon in the foreground. Usually you don't invoke this
directly — any other subcommand auto-spawns a detached daemon if the
socket isn't accepting. Useful for inspecting daemon logs in real
time during development.

| Flag | Default | Purpose |
|---|---|---|
| `--log-file <PATH>` | `$HWWCTL_LOG` or `/tmp/hwwctl.log` | Where the daemon's `tracing` output goes. |

## `ping`

Liveness check. Forces the daemon to spawn if it isn't running.

```json
{
  "kind": "pong",
  "daemon_version": "0.1.0",
  "protocol_version": 1,
  "pid": 12345
}
```

## `start <wallet>`

Spawn a new emulator instance.

```bash
hwwctl start bitbox02 [--no-wait] [--timeout 30]
```

| Wallet | Aliases | Daemon support |
|---|---|---|
| `bitbox02` | `bb02` | **Wired** |
| `coldcard` | `cc` | Not yet — returns `WALLET_UNSUPPORTED` |
| `trezor` | — | Not yet |
| `specter` | — | Not yet |
| `ledger` | — | Not yet |
| `jade` | — | Not yet |

| Flag | Purpose |
|---|---|
| `--no-wait` | Skip the post-spawn readiness check; return as soon as the child process is alive. |
| `--timeout <SECS>` | Override the per-instance startup timeout. |

On success:

```json
{
  "kind": "started",
  "instance": "bitbox02-a3f912",
  "wallet": "bitbox02",
  "state": "running",
  "vid": "0x03eb",
  "pid": "0x2403",
  "serial": "hwwctl-bb02-a3f912",
  "hidraw": "/dev/hidraw0",
  "transport": "tcp 127.0.0.1:43219"
}
```

`serial` is the HID `uniq` field — tests should filter `hidapi`
enumeration by `serial_number` to disambiguate two BitBox02s.

## `stop <instance>`

Idempotent teardown. Unknown ids return success.

```bash
hwwctl stop bitbox02-a3f912
```

```json
{ "kind": "stopped" }
```

## `status [instance]`

Snapshot of one instance, or all instances if no id is given.

```json
{
  "kind": "status",
  "instances": [
    { "instance": "bitbox02-a3f912", "state": "running", "...": "..." }
  ]
}
```

## `logs <instance>`

Per-instance log buffer.

```bash
hwwctl logs bitbox02-a3f912 [--tail 20] [--source emulator|bridge|all]
```

| Flag | Default | Purpose |
|---|---|---|
| `--tail <N>` | unlimited (server-side cap of 500/source) | Return only the last N entries after merge + sort. |
| `--source emulator\|bridge\|all` | `all` | Filter by source. `emulator` is the child's stdout/stderr; `bridge` is HID reports observed crossing the UHID bridge. |

Response shape:

```json
{
  "kind": "logs",
  "entries": [
    {
      "ts_ms": 1234,
      "source": "bridge",
      "direction": ">>",
      "raw_hex": "3f 23 23 00 ..."
    },
    {
      "ts_ms": 1567,
      "source": "emulator",
      "message": "simulator ready on port 43219"
    }
  ]
}
```

`ts_ms` is milliseconds since the instance started — monotonic, no
wall-clock skew.

## `bridge-stats <instance>`

Atomic byte / packet counters per direction. Monotonic since the
bridge started; subtract two snapshots to measure activity over an
interval.

```json
{
  "kind": "bridge_stats",
  "instance": "bitbox02-a3f912",
  "host_to_device_reports": 42,
  "host_to_device_bytes": 2688,
  "device_to_host_reports": 41,
  "device_to_host_bytes": 2624
}
```

## `shutdown`

Drain every instance and exit the daemon cleanly.

```json
{ "kind": "shutting_down" }
```

The socket file is removed shortly after. Auto-spawn picks up from
zero on the next `hwwctl <cmd>`.

## Error envelope

Daemon-side errors come back as:

```json
{
  "kind": "error",
  "code": "BUNDLE_MISSING",
  "message": "BitBox02 simulator bundle is not installed. Run `just bundle-install bitbox02` ..."
}
```

with exit code `1`. Stable codes:

| Code | Meaning |
|---|---|
| `BAD_REQUEST` | Request couldn't be parsed. |
| `WALLET_UNSUPPORTED` | Wallet not wired into the daemon yet (or you're not on Linux). |
| `BUNDLE_MISSING` | No bundle installed under `~/.hwwctl/bundles/{wallet}/`. |
| `SPAWN_FAILED` | Emulator child process failed to spawn. |
| `STARTUP_TIMEOUT` | Emulator started but its transport never became reachable. |
| `BRIDGE_FAILED` | UHID bridge couldn't be created (commonly `/dev/uhid` permissions). |
| `INSTANCE_NOT_FOUND` | `stop` / `status` / `logs` / `bridge-stats` referenced an unknown id. |
| `RESOURCE_EXHAUSTED` | Out of free TCP ports or socket paths to assign. |
| `INTERNAL` | Catch-all daemon bug; message has detail. |

Use `--json` and switch on `.code` rather than parsing message text
— messages may change between versions, codes are stable.

## Client-side errors

`hwwctl` itself can fail before talking to the daemon (bad
arguments, broken socket path, etc.). Those carry the
`CLIENT_ERROR` code and exit `2`. The text goes to stderr in normal
mode; in `--json` mode you also get the same JSON envelope on
stdout.
