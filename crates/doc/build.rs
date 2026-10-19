//! Lists the design-format changes in `src/format/`: one file per change, in `v<N>/` once a
//! release fixed its number, else in `pending/` (see `src/format/pending/README.md`). The list is
//! generated so that a change adds a file and never edits a line another change also edits.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The last format whose upgrade steps are written out in `format.rs` (`JSON_STEPS`).
const HAND_WRITTEN: u32 = 22;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/format");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut folded: Vec<(u32, PathBuf)> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let version = path.file_name()?.to_str()?.strip_prefix('v')?.parse().ok()?;
            path.is_dir().then_some((version, path))
        })
        .collect();
    folded.sort();
    for (i, (version, _)) in folded.iter().enumerate() {
        let expected = HAND_WRITTEN + 1 + i as u32;
        assert_eq!(*version, expected, "src/format/: expected v{expected}/ next, found v{version}/ (versions are contiguous)");
    }
    let mut out = String::new();
    let mut mods = String::new();
    out.push_str("/// The changes of each format after the hand-written ones, oldest first.\nconst FOLDED: &[(u32, &[Part])] = &[\n");
    for (version, dir) in &folded {
        let _ = writeln!(out, "    ({version}, &[");
        parts(dir, &format!("v{version}"), &mut out, &mut mods);
        out.push_str("    ]),\n");
    }
    out.push_str("];\n\n/// The changes no release has a number for yet (`src/format/pending/`).\nconst PENDING: &[Part] = &[\n");
    parts(&root.join("pending"), "pending", &mut out, &mut mods);
    out.push_str("];\n");
    out.push_str(&mods);
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("format_parts.rs");
    std::fs::write(dest, out).unwrap();
}

/// The `Part`s of the change files in `dir`, in file-name order, and a module for each.
fn parts(dir: &Path, prefix: &str, out: &mut String, mods: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
    files.sort();
    for path in files {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        assert!(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{}: name a format change file with lowercase letters, digits and dashes",
            path.display()
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.trim_start().starts_with("//!"), "{}: start a format change with a `//!` note", path.display());
        let module = format!("{prefix}_{}", name.replace('-', "_"));
        let step = if text.contains("fn step(") { format!("Some({module}::step)") } else { "None".into() };
        let _ = writeln!(out, "        Part {{ name: {name:?}, step: {step} }},");
        let _ = writeln!(mods, "#[path = {:?}]\nmod {module};", path.display().to_string());
    }
}
