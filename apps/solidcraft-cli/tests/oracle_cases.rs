//! The reference tests (tests/oracle, docs/oracle.md) as a `cargo test`, so tools that drive
//! `cargo test` (cargo-mutants) can use them: every case with expected values and no
//! `known_failure` mark must pass, as `cargo xtask oracle` requires.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
#[ignore = "run by cargo-mutants (.cargo/mutants.toml); CI replays the oracle through cargo xtask oracle"]
fn oracle_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("tests/oracle");
    let mut cases: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("recipe.json").is_file())
        .map(|p| format!("tests/oracle/{}", p.file_name().unwrap().to_string_lossy()))
        .collect();
    cases.sort();
    assert!(cases.len() >= 50, "oracle cases in {}", dir.display());
    // One process per core, cases dealt round-robin so the slow ones spread out.
    let n = std::thread::available_parallelism().map_or(4, |n| n.get()).min(cases.len());
    let children: Vec<_> = (0..n)
        .map(|i| {
            let chunk: Vec<&String> = cases.iter().skip(i).step_by(n).collect();
            Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
                .current_dir(&root)
                .arg("oracle")
                .arg("--json")
                .args(chunk)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut results: Vec<Value> = Vec::new();
    for child in children {
        let out = child.wait_with_output().unwrap();
        let part: Vec<Value> =
            serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("oracle JSON: {e}\n{}", String::from_utf8_lossy(&out.stderr)));
        results.extend(part);
    }
    assert_eq!(results.len(), cases.len());
    let mut failed = Vec::new();
    for r in &results {
        let case = r["case"].as_str().unwrap_or("");
        let want: Value =
            std::fs::read_to_string(dir.join(case).join("expected.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if want.is_null() || want["known_failure"].is_string() || r["pass"].as_bool().unwrap_or(false) {
            continue;
        }
        let checks: Vec<String> = r["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| !c["ok"].as_bool().unwrap_or(false))
            .map(|c| format!("{} got {} want {}", c["check"].as_str().unwrap_or(""), c["got"], c["want"]))
            .collect();
        failed.push(format!("{case}: {} {}", r["error"].as_str().unwrap_or(""), checks.join("; ")));
    }
    assert!(failed.is_empty(), "oracle cases failed:\n{}", failed.join("\n"));
}
