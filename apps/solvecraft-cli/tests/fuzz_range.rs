//! Seed ranges must not crash or wrap, including at the last representable seed.

use std::process::Command;

#[test]
fn overflowing_seed_range_is_an_error_not_a_panic() {
    let output =
        Command::new(env!("CARGO_BIN_EXE_solvecraft-cli")).args(["fuzz", "--from", "18446744073709551615", "--count", "2"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("seed range exceeds u64::MAX"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn empty_seed_range_at_maximum_is_valid() {
    let output =
        Command::new(env!("CARGO_BIN_EXE_solvecraft-cli")).args(["fuzz", "--from", "18446744073709551615", "--count", "0"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("0 seeds, 0 design steps,"));
}

#[test]
fn maximum_seed_runs_once_without_wrapping() {
    let output = Command::new(env!("CARGO_BIN_EXE_solvecraft-cli"))
        .args(["fuzz", "--from", "18446744073709551615", "--count", "1", "--steps", "0", "--timeout", "0"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("1 seeds,"));
}
