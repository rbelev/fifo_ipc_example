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
use anyhow::Result;
use nix::sys::stat::Mode;
use nix::unistd;
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::os::unix::fs::FileTypeExt;

pub const REQUEST_FIFO: &str = "/tmp/pw_bridge_requests";

const ACTION_GET_PASSWORD: &str = "get_password";
#[derive(Serialize, Deserialize, Debug)]
pub enum Action {
    GetPassword,
}

impl From<&'static str> for Action {
    fn from(s: &'static str) -> Self {
        match s {
            ACTION_GET_PASSWORD => Action::GetPassword,
            _ => panic!("unknown action"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    pub action: Action,
    pub account: Option<String>,
    pub resp_fifo: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "ok")]
pub enum Response {
    #[serde(rename = "true")]
    GetPasswordResponse { password: String },
    #[serde(rename = "false")]
    GetPasswordError { message: String },
}

pub fn ensure_request_fifo(path: &str) -> Result<()> {
    // create a new fifo and give read, write and execute rights to the owner
    match unistd::mkfifo(path, Mode::S_IRWXU) {
        Ok(_) => Ok(()),
        Err(nix::Error::EEXIST) => {
            if is_fifo(path)? {
                return Ok(());
            }
            anyhow::bail!("{REQUEST_FIFO} *exists but is not a FIFO")
        }
        Err(err) => anyhow::bail!("mkfifo failed: {}", err),
    }
}

pub fn is_fifo(path: &str) -> Result<bool> {
    let meta = fs::symlink_metadata(path)?;
    Ok(meta.file_type().is_fifo())
}
