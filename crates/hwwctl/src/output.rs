//! Human/JSON formatting for client output.
//!
//! Two output modes — pretty (default, for humans) and `--json` (for
//! tests). Both go to stdout on success; errors go to stderr, plus
//! stdout in JSON mode so test code only has to parse one stream.

use control::{BridgeStatsSnapshot, InstanceSummary, LogEntry, LogSource, Response};
use serde::Serialize;

/// Print a client-side (pre-daemon) error: connection problems,
/// argument errors, etc. These never came from the daemon so they
/// have no structured ErrorCode.
pub fn print_client_error(msg: &str, json: bool) {
    if json {
        let env = ClientErrorEnvelope {
            error: ClientError {
                code: "CLIENT_ERROR",
                message: msg,
            },
        };
        // stderr for shells, stdout for tests that parse JSON.
        eprintln!("hwwctl: {msg}");
        if let Ok(s) = serde_json::to_string(&env) {
            println!("{s}");
        }
    } else {
        eprintln!("hwwctl: {msg}");
    }
}

#[derive(Serialize)]
struct ClientErrorEnvelope<'a> {
    error: ClientError<'a>,
}
#[derive(Serialize)]
struct ClientError<'a> {
    code: &'a str,
    message: &'a str,
}

pub fn print_response(resp: &Response, json: bool) {
    if json {
        // The Response enum already has serde tags. Tests decode it
        // directly.
        match serde_json::to_string(resp) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("hwwctl: failed to serialize response: {e}"),
        }
        return;
    }

    match resp {
        Response::Pong(info) => {
            println!(
                "daemon ok  version={}  proto={}  pid={}",
                info.daemon_version, info.protocol_version, info.pid
            );
        }
        Response::Started(s) => {
            println!("started {}", s.instance);
            print_summary(s);
        }
        Response::Stopped => {
            println!("stopped");
        }
        Response::Status { instances } => {
            if instances.is_empty() {
                println!("(no instances)");
            } else {
                for s in instances {
                    print_summary(s);
                    println!();
                }
            }
        }
        Response::Logs { entries } => {
            if entries.is_empty() {
                println!("(no log entries)");
            } else {
                for e in entries {
                    print_log_entry(e);
                }
            }
        }
        Response::BridgeStats(s) => print_bridge_stats(s),
        Response::ShuttingDown => {
            println!("daemon is shutting down");
        }
        Response::Error(e) => {
            eprintln!("error [{:?}]: {}", e.code, e.message);
        }
    }
}

fn print_log_entry(e: &LogEntry) {
    let src = match e.source {
        LogSource::Emulator => "emu",
        LogSource::Bridge => "br ",
        LogSource::All => "*  ",
    };
    if e.source == LogSource::Bridge {
        println!("  {:>8}ms  {src}  {} {}", e.ts_ms, e.direction, e.raw_hex);
    } else {
        println!("  {:>8}ms  {src}  {}", e.ts_ms, e.message);
    }
}

fn print_bridge_stats(s: &BridgeStatsSnapshot) {
    println!("  instance         {}", s.instance);
    println!(
        "  host → device    {:>6} reports  {:>8} bytes",
        s.host_to_device_reports, s.host_to_device_bytes
    );
    println!(
        "  device → host    {:>6} reports  {:>8} bytes",
        s.device_to_host_reports, s.device_to_host_bytes
    );
}

fn print_summary(s: &InstanceSummary) {
    println!("  instance  {}", s.instance);
    println!("  wallet    {}", s.wallet);
    println!("  state     {:?}", s.state);
    println!("  vid       {:#06x}", s.vid);
    println!("  pid       {:#06x}", s.pid);
    println!("  serial    {}", s.serial);
    if let Some(p) = &s.hidraw {
        println!("  hidraw    {}", p.display());
    }
    println!("  transport {}", s.transport);
    if let Some(err) = &s.error {
        println!("  error     {err}");
    }
}
