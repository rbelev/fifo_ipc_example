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

use anyhow::{Context, Result};
use fifo_ipc::{Action, REQUEST_FIFO, Request, Response, ensure_request_fifo};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

fn daemon_loop() -> Result<()> {
    ensure_request_fifo(REQUEST_FIFO)?;
    println!("Daemon listening on {}", REQUEST_FIFO);

    // We'll open the FIFO for reading continuously. Opening a FIFO for read-only will block until
    // a writer opens it; so we open with read-only and then iterate lines.
    loop {
        // Open requests FIFO for reading (blocking until a client writes)
        let fifo = OpenOptions::new()
            .read(true)
            .open(REQUEST_FIFO)
            .with_context(|| format!("opening request fifo {}", REQUEST_FIFO))?;
        let reader = BufReader::new(fifo);

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            // parse request as JSON
            match serde_json::from_str::<Request>(&line) {
                Ok(req) => {
                    println!("Daemon received request: {:?}", req);
                    // For security, verify the resp_fifo path is inside /tmp and looks sane.
                    if !req.resp_fifo.starts_with("/tmp/") {
                        eprintln!(
                            "refusing to write to resp fifo outside /tmp: {}",
                            req.resp_fifo
                        );
                        continue;
                    }
                    // handle action
                    let resp = handle_request(&req);
                    // write response to resp_fifo
                    if let Err(e) = write_response_to_fifo(&req.resp_fifo, &resp) {
                        eprintln!("failed to write response: {:#}", e);
                    }
                }
                Err(e) => eprintln!("failed to parse request JSON: {} -- raw: {}", e, line),
            }
        }
        // When writer closes, the inner loop ends and we reopen FIFO to block for the next writer.
    }
}

fn handle_request(req: &Request) -> Response {
    match &req.action {
        Action::GetPassword => {
            // In a real implementation: check caller credentials, ACLs, and decrypt from secure storage.
            // Here we'll return a dummy secret for demonstration.
            if let Some(account) = &req.account {
                // mock password derivation
                let pw = format!("pw-for-{}-{}", account, mock_timestamp());
                Response::GetPasswordResponse { password: pw }
            } else {
                Response::GetPasswordError {
                    message: "missing account".into(),
                }
            }
        }
        _ => Response::GetPasswordError {
            message: "unknown action".into(),
        },
    }
}

fn write_response_to_fifo(resp_fifo: &str, resp: &Response) -> Result<()> {
    // Open the response FIFO for writing (this will block if reader is not present). In our protocol,
    // the client creates the FIFO and opens it for read before sending request, so this should succeed.
    let s = serde_json::to_string(resp)? + "\n";
    println!("Opening resp fifo {} to send resp {:#}", resp_fifo, s);

    let mut f = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK) // let us try to write without indefinite block; handle EWOULDBLOCK
        .open(resp_fifo)
        .with_context(|| format!("opening resp fifo {}", resp_fifo))?;
    println!("Writing response to {}", resp_fifo);

    f.write_all(s.as_bytes())
        .with_context(|| format!("writing resp to fifo {}", resp_fifo))?;
    // After writing, close and remove the temporary FIFO
    println!("Wrote response to {}", resp_fifo);
    drop(f);
    let _ = fs::remove_file(resp_fifo);
    println!("Removed resp fifo {}", resp_fifo);
    Ok(())
}

fn mock_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn print_usage_bin(name: &str) {
    eprintln!("Usage: {} <daemon|get> [--account NAME]", name);
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage_bin(&args[0]);
        anyhow::bail!("missing command");
    }
    let cmd = args[1].as_str();
    match cmd {
        "daemon" => {
            daemon_loop()?;
        }
        _ => {
            print_usage_bin(&args[0]);
        }
    }
    Ok(())
}
