//! Result reporting must not skip requested files or execute a script twice.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("solvecraft-run-results-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn script(&self) -> PathBuf {
        let path = self.0.join("box.json");
        std::fs::write(&path, json!({"commands": [{"command": "solid.box", "params": {"length": 2, "width": 3, "height": 4}}]}).to_string()).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_solvecraft-cli"))
}

fn json_output(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn body_count(path: &Path) -> Value {
    json_output(cli().arg("eval").arg(path).output().unwrap())["body_count"].clone()
}

#[test]
fn results_still_save_and_export_the_script_once() {
    let dir = Scratch::new();
    let saved = dir.0.join("box.solvecraft");
    let stl = dir.0.join("box.stl");
    let obj = dir.0.join("box.obj");
    let result = json_output(
        cli().arg("run").arg(dir.script()).arg("--results").arg("--save").arg(&saved).arg("--out").arg(&stl).arg("--out").arg(&obj).output().unwrap(),
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(body_count(&saved), 1);
    for exported in [&stl, &obj] {
        assert!(std::fs::metadata(exported).unwrap().len() > 0);
        assert_eq!(body_count(exported), 1);
    }
}

#[test]
fn results_with_base_design_and_quiet_still_report_and_save() {
    let dir = Scratch::new();
    let script = dir.script();
    let base = dir.0.join("base.solvecraft");
    let saved = dir.0.join("combined.solvecraft");
    json_output(cli().arg("run").arg(&script).arg("--save").arg(&base).output().unwrap());
    let result = json_output(cli().args(["run", "--results", "--quiet", "--in"]).arg(&base).arg(&script).arg("--save").arg(&saved).output().unwrap());
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(body_count(&saved), 2);
    assert_eq!(body_count(&base), 1);
}

#[test]
fn results_propagate_output_errors() {
    let dir = Scratch::new();
    for flag in ["--save", "--out"] {
        let path = dir.0.join("missing").join(if flag == "--save" { "box.solvecraft" } else { "box.stl" });
        let output = cli().arg("run").arg(dir.script()).arg("--results").arg(flag).arg(&path).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stdout.is_empty());
        assert!(!path.exists());
    }
}

#[test]
fn results_without_script_still_fail() {
    let dir = Scratch::new();
    let base = dir.0.join("base.solvecraft");
    json_output(cli().arg("run").arg(dir.script()).arg("--save").arg(&base).output().unwrap());
    let output = cli().args(["run", "--results", "--in"]).arg(&base).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--results needs a script"));
}
