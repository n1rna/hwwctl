//! Auto-spawning the daemon when a short-lived `hwwctl` invocation
//! finds no listener at the socket.
//!
//! On Unix we re-exec the current binary with `daemon`, ask it to
//! detach via `setsid`, then poll the socket until we get a successful
//! connect or hit the deadline. The parent (client) keeps no file
//! handles to the child — the daemon is fully orphaned to init once
//! the client exits.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::net::UnixStream;

/// How long to wait for a freshly-spawned daemon to bind its socket.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval while waiting for the socket to come up.
const SPAWN_POLL: Duration = Duration::from_millis(50);

/// Try connecting to `socket`. If the connect fails because nothing's
/// listening, spawn a detached daemon and poll until it does. Returns
/// the live `UnixStream` ready for use.
pub async fn connect_or_spawn(socket: &Path) -> Result<UnixStream> {
    match UnixStream::connect(socket).await {
        Ok(s) => return Ok(s),
        Err(e) if would_benefit_from_spawn(&e) => {
            // fall through to spawn
        }
        Err(e) => {
            return Err(
                anyhow::Error::new(e).context(format!("connect to {} failed", socket.display()))
            );
        }
    }

    // Stale socket file? If the path exists but nothing's accepting,
    // the daemon previously crashed without cleanup. Best-effort
    // unlink; the daemon will recreate it.
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    spawn_detached_daemon(socket).with_context(|| {
        format!(
            "auto-spawn of daemon for socket {} failed",
            socket.display()
        )
    })?;

    wait_for_socket(socket, SPAWN_READY_TIMEOUT).await
}

fn would_benefit_from_spawn(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
    )
}

fn spawn_detached_daemon(socket: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("locate current exe for daemon spawn")?;

    let log_path = std::env::var_os("HWWCTL_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/hwwctl.log"));

    let mut cmd = Command::new(exe);
    cmd.arg("--socket")
        .arg(socket)
        .arg("daemon")
        .arg("--log-file")
        .arg(&log_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Detach from the controlling terminal. Without this, Ctrl-C in
    // the parent's shell would propagate to the daemon — bad for a
    // test runner that spawned the daemon transparently and expects
    // it to outlive the run.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let _child = cmd.spawn().context("failed to spawn daemon process")?;
    // Deliberately drop the handle — we don't want a SIGCHLD path back
    // to this short-lived client.
    Ok(())
}

async fn wait_for_socket(socket: &Path, timeout: Duration) -> Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    let mut last_err: Option<std::io::Error> = None;
    while Instant::now() < deadline {
        match UnixStream::connect(socket).await {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(SPAWN_POLL).await;
    }
    let detail = last_err
        .map(|e| format!(" (last error: {e})"))
        .unwrap_or_default();
    anyhow::bail!(
        "daemon did not start listening at {} within {:?}{detail}. \
         Check {} for daemon-side errors.",
        socket.display(),
        timeout,
        std::env::var("HWWCTL_LOG").unwrap_or_else(|_| "/tmp/hwwctl.log".to_string()),
    )
}
