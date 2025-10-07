// Single-file example (Rust) demonstrating a simple CLI <-> daemon bridge using FIFOs (named pipes).
//
// Features shown:
// - Daemon creates a well-known request FIFO at /tmp/pw_bridge_requests
// - CLI creates a temporary response FIFO, writes a JSON request pointing at that FIFO
// - Daemon reads request, performs a mock "fetch secret" and writes JSON response into the response FIFO
// - All secrets are passed only through FIFOs and not stored on disk otherwise
//
// Usage:
// 1) Start daemon in one terminal:
//    cargo run --bin onepw_cli_bridge -- daemon
//
// 2) Use the CLI from another terminal to request a secret:
//    cargo run --bin onepw_cli_bridge -- get --account github.com
//
use anyhow::{Context, Result, anyhow};
use fifo_ipc::{Action, REQUEST_FIFO, Request, Response};
use nix::sys::stat::Mode;
use nix::unistd;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn client_get(account: &str) -> Result<()> {
    println!("Requesting password for account {}", account);
    let resp_path = get_resp_filepath()?;
    unistd::mkfifo(Path::new(&resp_path), Mode::S_IRWXU)?;

    // Build request JSON
    let req = Request {
        action: Action::GetPassword,
        account: Some(account.into()),
        resp_fifo: resp_path.clone(),
    };
    send_request(&req)?;

    // Ensure we open the response fifo for read BEFORE sending the request so daemon won't block forever when trying to open write.
    let mut resp_file = OpenOptions::new()
        .read(true)
        // .custom_flags(libc::O_NONBLOCK)
        .open(&resp_path)
        .with_context(|| format!("opening response fifo {} for read", resp_path))?;
    println!("Request sent; waiting for response...");
    // Read response (blocking until daemon writes)
    let mut buf = String::new();
    resp_file.read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        anyhow::bail!("empty response from daemon");
    }
    let resp: Response =
        serde_json::from_str(&buf).with_context(|| format!("parsing response JSON: {}", buf))?;

    match resp {
        Response::GetPasswordResponse { password } => {
            println!("Got password: {}", password);
        }
        Response::GetPasswordError { message } => {
            eprintln!("Daemon error: {message}");
        }
    };

    // remove resp fifo (daemon also tries to remove it; ignore errors)
    let _ = fs::remove_file(&resp_path);
    Ok(())
}

fn send_request(req: &Request) -> Result<()> {
    let req_json = serde_json::to_string(&req)?;
    println!("Sending request: {}", req_json);
    // Open the well-known request fifo for write and send request
    let mut wf = OpenOptions::new()
        .write(true)
        // .custom_flags(libc::O_NONBLOCK)
        .open(REQUEST_FIFO)
        .with_context(|| format!("opening request fifo {} for write", REQUEST_FIFO))?;

    wf.write_all((req_json + "\n").as_bytes())?;
    // Close writer to signal end-of-write
    drop(wf);
    Ok(())
}

fn print_usage_bin(name: &str) {
    eprintln!("Usage: {} <daemon|get> [--account NAME]", name);
}

// Create a unique resp fifo in /tmp
fn get_resp_filepath() -> Result<String> {
    let pid = std::process::id();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(format!("/tmp/pw_resp_{}_{}", pid, now))
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage_bin(&args[0]);
        anyhow::bail!("missing command");
    }
    let cmd = args[1].as_str();
    match cmd {
        "get" => {
            // simple parsing: --account <name>
            let account = args
                .iter()
                .position(|arg| arg == "--account")
                .ok_or_else(|| anyhow!("--account is required for get"))
                .and_then(|idx| {
                    args.get(idx + 1)
                        .ok_or_else(|| anyhow!("--account requires a value"))
                })
                .map(|arg| arg.as_str())?;

            client_get(account)?;
        }
        _ => {
            print_usage_bin(&args[0]);
        }
    }
    Ok(())
}
