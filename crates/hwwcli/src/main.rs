//! `hwwctl` — control plane CLI for the hardware-wallet emulator daemon.
//!
//! Two modes of operation in one binary:
//!
//! - `hwwctl daemon` — long-lived process. Owns every running emulator
//!   instance, its UHID bridge, and its captured logs. Listens on a
//!   Unix socket; all other subcommands are short-lived clients
//!   talking to it.
//! - `hwwctl <anything-else>` — short-lived client. Connects to the
//!   socket (auto-spawning the daemon if none is reachable), sends one
//!   [`control::Request`], prints the [`control::Response`] in a
//!   human-friendly form (or as JSON with `--json`), exits.
//!
//! Designed so tests can call it from any language: stable exit codes,
//! stable JSON shape, structured error codes.

mod auto_spawn;
mod client;
mod daemon;
mod output;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use control::Wallet;

const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "hwwctl",
    version = DAEMON_VERSION,
    about = "Control plane for hardware-wallet emulators.",
    long_about = "Drive Trezor / BitBox02 / Coldcard / Specter / Ledger / Jade emulators \
                  from tests via a shared local daemon. Run `hwwctl daemon` once per host \
                  (or rely on auto-spawn); test code uses the other subcommands."
)]
struct Cli {
    /// Override the daemon socket path. Defaults to `$HWWCTL_SOCKET`,
    /// then `$XDG_RUNTIME_DIR/hwwctl.sock`, then `/tmp/hwwctl.sock`.
    #[arg(long, env = "HWWCTL_SOCKET", global = true)]
    socket: Option<PathBuf>,

    /// Emit results as JSON instead of human-friendly text. Tests
    /// should set this. Errors always go to stderr; on `--json` they
    /// also serialize as a JSON `{"error": {...}}` envelope.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the daemon in the foreground. Auto-spawn callers use this
    /// internally; you only run it explicitly when you want logs on
    /// the terminal.
    Daemon {
        /// Where to write the daemon log. Defaults to /tmp/hwwctl.log.
        #[arg(long, env = "HWWCTL_LOG")]
        log_file: Option<PathBuf>,
    },

    /// Health check the daemon, auto-spawning if needed.
    Ping,

    /// Spawn a new emulator instance.
    Start {
        /// Wallet model — `bitbox02`, `coldcard`, `trezor`, `specter`,
        /// `ledger`, `jade`. Aliases: `bb02`, `cc`.
        wallet: Wallet,
        /// Skip the post-spawn readiness wait. The default is to block
        /// until the bridge is up and the emulator transport is
        /// reachable; with this flag the daemon returns immediately
        /// after the child process is spawned.
        #[arg(long)]
        no_wait: bool,
        /// Override the per-instance startup timeout (seconds).
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Stop a running instance by id. Idempotent.
    Stop {
        /// Instance id, e.g. `bitbox02-a3f912`.
        instance: String,
    },

    /// Print a snapshot of running instances. With an id, only that one.
    Status {
        /// Restrict output to a single instance id.
        instance: Option<String>,
    },

    /// Ask the daemon to terminate cleanly, dropping all instances.
    Shutdown,
}

fn main() {
    let cli = Cli::parse();
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(control::default_socket_path);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let exit_code = runtime.block_on(async {
        match cli.cmd {
            Cmd::Daemon { log_file } => daemon::run(socket, log_file).await,
            other => client::run(other, socket, cli.json).await,
        }
    });

    std::process::exit(exit_code);
}
