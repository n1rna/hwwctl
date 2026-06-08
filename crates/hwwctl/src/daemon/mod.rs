//! Daemon mode: accept Unix-socket connections, dispatch each request
//! through a single-task registry, write back the response.

#[cfg(target_os = "linux")]
mod hidraw_scan;
mod instance;
mod registry;
mod spawn;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use control::{read_frame, write_frame, Request, Response};
#[cfg(target_os = "linux")]
use control::{Error as CtlError, ErrorCode};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use self::registry::Registry;

/// Daemon entry point. Returns an exit code.
pub async fn run(socket: PathBuf, log_file: Option<PathBuf>) -> i32 {
    if let Err(e) = init_logging(log_file.as_deref()) {
        eprintln!("hwwctl daemon: failed to init logging: {e:#}");
        return 2;
    }

    info!(socket = %socket.display(), "starting hwwctl daemon");

    if !cfg!(target_os = "linux") {
        warn!(
            "hwwctl daemon is running on a non-Linux host — UHID-backed wallets \
             (BitBox02, Coldcard) will fail to spawn"
        );
    }

    // Replace stale socket file before binding. The auto-spawn path
    // already unlinks before re-spawning, but a daemon launched
    // directly may still inherit a stale node.
    if socket.exists() {
        if let Err(e) = std::fs::remove_file(&socket) {
            error!(error = %e, "could not remove stale socket file");
            return 2;
        }
    }

    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, "failed to bind {}", socket.display());
            return 2;
        }
    };
    // Permissions: 0600 so other UIDs can't drive our emulators. Tests
    // running as a different user (e.g. inside a container under root)
    // would need to set HWWCTL_SOCKET to their own path.
    if let Err(e) = set_socket_mode(&socket, 0o600) {
        warn!(error = %e, "could not chmod socket — continuing");
    }

    let registry = Arc::new(Registry::spawn());
    let shutdown = Arc::new(tokio::sync::Notify::new());

    // Signal handler: SIGTERM/SIGINT triggers a graceful drain. Tests
    // running the daemon under WDIO send SIGTERM at teardown.
    {
        let registry = Arc::clone(&registry);
        let shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            wait_for_termination_signal().await;
            info!("termination signal received — draining instances");
            registry.shutdown().await;
            shutdown.notify_waiters();
        });
    }

    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let registry = Arc::clone(&registry);
                        let shutdown = Arc::clone(&shutdown);
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, registry, shutdown).await {
                                warn!(error = %format!("{e:#}"), "client connection error");
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "accept failed");
                        break;
                    }
                }
            }
        }
    }

    // Best-effort socket cleanup.
    let _ = std::fs::remove_file(&socket);
    info!("hwwctl daemon exiting");
    0
}

async fn handle_connection(
    mut stream: UnixStream,
    registry: Arc<Registry>,
    shutdown: Arc<tokio::sync::Notify>,
) -> anyhow::Result<()> {
    let request: Option<Request> = read_frame(&mut stream).await?;
    let Some(request) = request else {
        // Client closed before sending anything. Quiet — happens when
        // auto-spawn pokes the socket to check liveness.
        return Ok(());
    };

    // Notice if this is a shutdown request BEFORE dispatch consumes
    // it, so we can sequence the shutdown notification AFTER the
    // response has hit the wire.
    let is_shutdown = matches!(request, Request::Shutdown);
    let response = dispatch(request, &registry).await;
    write_frame(&mut stream, &response).await?;

    // Now that the client has the response in its kernel buffer, it
    // is safe to fire the daemon-wide shutdown signal. Doing this
    // earlier (inside `dispatch`) raced the main loop's
    // `process::exit` against our `write_frame` — see the v0.1.1
    // smoke regression in run 27140773410.
    if is_shutdown {
        shutdown.notify_waiters();
    }
    Ok(())
}

async fn dispatch(request: Request, registry: &Registry) -> Response {
    match request {
        Request::Ping => Response::Pong(control::PongInfo {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: control::PROTOCOL_VERSION,
            pid: std::process::id(),
        }),
        Request::Start(req) => match registry.start(req).await {
            Ok(summary) => Response::Started(summary),
            Err(e) => Response::Error(e),
        },
        Request::Stop(req) => match registry.stop(req.instance).await {
            Ok(()) => Response::Stopped,
            Err(e) => Response::Error(e),
        },
        Request::Status(req) => match registry.status(req.instance).await {
            Ok(list) => Response::Status { instances: list },
            Err(e) => Response::Error(e),
        },
        Request::Logs(req) => match registry.logs(req).await {
            Ok(entries) => Response::Logs { entries },
            Err(e) => Response::Error(e),
        },
        Request::BridgeStats(req) => match registry.bridge_stats(req.instance).await {
            Ok(snap) => Response::BridgeStats(snap),
            Err(e) => Response::Error(e),
        },
        Request::Shutdown => {
            // Drain all instances; the actual daemon-loop exit is
            // signaled by `handle_connection` after the response has
            // been written. See the note there.
            registry.shutdown().await;
            Response::ShuttingDown
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_logging(log_file: Option<&Path>) -> anyhow::Result<()> {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_env("HWWCTL_LOG_LEVEL")
        .unwrap_or_else(|_| EnvFilter::new("info,hwwctl=debug,bridge=debug,emulators=debug"));

    let builder = fmt().with_env_filter(filter).with_target(true);

    if let Some(path) = log_file {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        builder.with_writer(file).with_ansi(false).init();
    } else {
        builder.init();
    }
    Ok(())
}

fn set_socket_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms)
}

async fn wait_for_termination_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to install SIGTERM handler");
            return;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to install SIGINT handler");
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
}

// ── Re-exports used by sibling modules ────────────────────────────────────────

#[cfg(target_os = "linux")]
pub(crate) fn internal_err<E: std::fmt::Display>(e: E) -> CtlError {
    CtlError::new(ErrorCode::Internal, e.to_string())
}
