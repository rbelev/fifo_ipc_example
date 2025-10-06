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
use nix::sys::stat::Mode;
use nix::unistd;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

const REQUEST_FIFO: &str = "/tmp/pw_bridge_requests";

#[derive(Serialize, Deserialize, Debug)]
struct Request {
    action: String,
    account: Option<String>,
    resp_fifo: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Response {
    ok: bool,
    message: Option<String>,
    password: Option<String>,
}

fn ensure_request_fifo() -> Result<()> {
    let path = Path::new(REQUEST_FIFO);

    // create a new fifo and give read, write and execute rights to the owner
    match unistd::mkfifo(path, Mode::S_IRWXO) {
        Ok(_) => Ok(()),
        Err(nix::Error::EEXIST) => {
            let meta = fs::symlink_metadata(path)?;
            if meta.file_type().is_fifo() {
                Ok(())
            } else {
                anyhow::bail!("{REQUEST_FIFO} exists but is not a FIFO")
            }
        }
        Err(err) => anyhow::bail!("mkfifo failed: {}", err),
    }
}

fn daemon_loop() -> Result<()> {
    ensure_request_fifo()?;
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
    match req.action.as_str() {
        "get_password" => {
            // In a real implementation: check caller credentials, ACLs, and decrypt from secure storage.
            // Here we'll return a dummy secret for demonstration.
            if let Some(account) = &req.account {
                // mock password derivation
                let pw = format!("pw-for-{}-{}", account, mock_timestamp());
                Response {
                    ok: true,
                    message: None,
                    password: Some(pw),
                }
            } else {
                Response {
                    ok: false,
                    message: Some("missing account".into()),
                    password: None,
                }
            }
        }
        _ => Response {
            ok: false,
            message: Some("unknown action".into()),
            password: None,
        },
    }
}

fn write_response_to_fifo(resp_fifo: &str, resp: &Response) -> Result<()> {
    // Open the response FIFO for writing (this will block if reader is not present). In our protocol,
    // the client creates the FIFO and opens it for read before sending request, so this should succeed.
    let mut f = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK) // let us try to write without indefinite block; handle EWOULDBLOCK
        .open(resp_fifo)
        .with_context(|| format!("opening resp fifo {}", resp_fifo))?;

    let s = serde_json::to_string(resp)? + "\n";
    f.write_all(s.as_bytes())?;
    // After writing, close and remove the temporary FIFO
    drop(f);
    let _ = fs::remove_file(resp_fifo);
    Ok(())
}

fn client_get(account: &str) -> Result<()> {
    // Create a unique resp fifo in /tmp
    let pid = std::process::id();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let resp_path = format!("/tmp/pw_resp_{}_{}", pid, now);
    unistd::mkfifo(Path::new(&resp_path), Mode::S_IRWXO)?;

    // Ensure we open the response fifo for read BEFORE sending the request so daemon won't block forever when trying to open write.
    let mut resp_file = OpenOptions::new()
        .read(true)
        .open(&resp_path)
        .with_context(|| format!("opening response fifo {} for read", resp_path))?;

    // Build request JSON
    let req = Request {
        action: "get_password".into(),
        account: Some(account.into()),
        resp_fifo: resp_path.clone(),
    };
    let req_json = serde_json::to_string(&req)?;

    // Open the well-known request fifo for write and send request
    let mut wf = OpenOptions::new()
        .write(true)
        .open(REQUEST_FIFO)
        .with_context(|| format!("opening request fifo {} for write", REQUEST_FIFO))?;

    wf.write_all((req_json + "\n").as_bytes())?;
    // Close writer to signal end-of-write
    drop(wf);

    // Read response (blocking until daemon writes)
    let mut buf = String::new();
    resp_file.read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        anyhow::bail!("empty response from daemon");
    }
    let resp: Response =
        serde_json::from_str(&buf).with_context(|| format!("parsing response JSON: {}", buf))?;

    if resp.ok {
        println!("Got password: {}", resp.password.unwrap_or_default());
    } else {
        eprintln!(
            "Daemon error: {}",
            resp.message.unwrap_or_else(|| "unknown".into())
        );
    }

    // remove resp fifo (daemon also tries to remove it; ignore errors)
    let _ = fs::remove_file(&resp_path);
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
    let mut args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage_bin(&args[0]);
        anyhow::bail!("missing command");
    }
    let cmd = args[1].as_str();
    match cmd {
        "daemon" => {
            daemon_loop()?;
        }
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
