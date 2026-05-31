//! Short-lived `hwwctl` client. One connection, one request, one
//! response, exit.
//!
//! Returns the process exit code: `0` for success, `1` for daemon
//! errors with structured codes, `2` for connection/protocol problems.

use std::path::PathBuf;

use control::{
    read_frame, write_frame, InstanceId, Request, Response, StartRequest, StatusRequest,
    StopRequest,
};

use crate::auto_spawn;
use crate::output;
use crate::Cmd;

pub async fn run(cmd: Cmd, socket: PathBuf, json: bool) -> i32 {
    let request = match build_request(cmd) {
        Ok(r) => r,
        Err(e) => {
            output::print_client_error(&format!("bad arguments: {e}"), json);
            return 2;
        }
    };

    let mut stream = match auto_spawn::connect_or_spawn(&socket).await {
        Ok(s) => s,
        Err(e) => {
            output::print_client_error(&format!("{e:#}"), json);
            return 2;
        }
    };

    if let Err(e) = write_frame(&mut stream, &request).await {
        output::print_client_error(&format!("write request: {e:#}"), json);
        return 2;
    }

    let response: Response = match read_frame(&mut stream).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            output::print_client_error("daemon closed the connection before replying", json);
            return 2;
        }
        Err(e) => {
            output::print_client_error(&format!("read response: {e:#}"), json);
            return 2;
        }
    };

    output::print_response(&response, json);

    match response {
        Response::Error(_) => 1,
        _ => 0,
    }
}

fn build_request(cmd: Cmd) -> anyhow::Result<Request> {
    Ok(match cmd {
        Cmd::Daemon { .. } => anyhow::bail!("`daemon` is not a client subcommand"),
        Cmd::Ping => Request::Ping,
        Cmd::Start {
            wallet,
            no_wait,
            timeout,
        } => Request::Start(StartRequest {
            wallet,
            wait_ready: !no_wait,
            timeout_secs: timeout,
        }),
        Cmd::Stop { instance } => Request::Stop(StopRequest {
            instance: InstanceId::new(instance),
        }),
        Cmd::Status { instance } => Request::Status(StatusRequest {
            instance: instance.map(InstanceId::new),
        }),
        Cmd::Shutdown => Request::Shutdown,
    })
}
