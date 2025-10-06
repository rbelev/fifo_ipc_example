use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

struct DaemonGuard {
    child: Child,
}

impl DaemonGuard {
    fn new() -> Self {
        let child = Command::new("cargo")
            .args(&["run", "--bin", "pw-daemon", "--", "daemon"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start daemon");

        // Give daemon time to start and create FIFO
        thread::sleep(Duration::from_millis(1000));

        DaemonGuard { child }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn test_daemon_client_communication() {
    let _daemon = DaemonGuard::new();

    // Run the client to request a password
    let output = Command::new("cargo")
        .args(&[
            "run",
            "--bin",
            "pw-cli",
            "--",
            "get",
            "--account",
            "github.com",
        ])
        .output()
        .expect("failed to run client");

    // Verify client output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "client command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Got password:"),
        "expected password in output, got: {}",
        stdout
    );
    assert!(
        stdout.contains("github.com"),
        "expected account name in password"
    );
}

#[test]
fn test_client_missing_account_flag() {
    let _daemon = DaemonGuard::new();

    // Run client without --account flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "pw-cli", "--", "get"])
        .output()
        .expect("failed to run client");

    // Should fail with appropriate error
    assert!(!output.status.success(), "expected client to fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--account is required"),
        "expected account required error"
    );
}

#[test]
fn test_client_missing_account_value() {
    let _daemon = DaemonGuard::new();

    // Run client with --account but no value
    let output = Command::new("cargo")
        .args(&["run", "--bin", "pw-cli", "--", "get", "--account"])
        .output()
        .expect("failed to run client");

    // Should fail with appropriate error
    assert!(!output.status.success(), "expected client to fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--account requires a value"),
        "expected value required error"
    );
}
