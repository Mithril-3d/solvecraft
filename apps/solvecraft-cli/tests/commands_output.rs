//! The registry remains valid JSON and a closed pipeline reader must not cause a panic.

use std::process::{Command, Stdio};

#[test]
fn registry_output_is_complete_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_solvecraft-cli")).arg("commands").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let registry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let commands = registry.as_array().unwrap();
    assert!(commands.iter().any(|command| command["id"] == "solid.box"));
    assert!(commands.iter().any(|command| command["id"] == "engine.commands"));
    assert!(output.stdout.ends_with(b"\n"));
    assert!(output.stderr.is_empty());
}

#[test]
fn closed_registry_pipe_exits_without_panicking() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_solvecraft-cli")).arg("commands").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // Close our read end before waiting. No sleeps or timing threshold are needed:
    // the registry is larger than the pipe buffer, so it cannot all be written first.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "status: {}; stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
}
