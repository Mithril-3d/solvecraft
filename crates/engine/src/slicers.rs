//! Slicers a mesh can be sent to: find the installed ones on Linux, macOS and Windows, start one
//! with a file, and read its application icon (never shipped: it comes from the installed
//! application).
//!
//! Detection reads desktop entries and Flatpak exports (Linux), `.app` bundles (macOS) and the
//! program folders (Windows), plus `PATH` and the usual AppImage folders. Nothing here runs a
//! program except [`Slicer::open`].

use std::path::{Path, PathBuf};

use crate::slicer_icon;

/// Most directory entries looked at in one folder.
const MAX_ENTRIES: usize = 5_000;
/// Largest desktop entry read.
const MAX_DESKTOP_BYTES: u64 = 64 * 1024;

/// A slicer the user can send a mesh to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slicer {
    /// `prusa`, `bambu`, … for the known ones; `custom` for a program the user picked.
    pub id: String,
    pub name: String,
    /// The program and its leading arguments; the file to open is appended.
    pub command: Vec<String>,
    pub icon: Option<IconSource>,
}

/// Where an application's icon is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconSource {
    /// A PNG file (Linux icon themes).
    Image(PathBuf),
    /// A macOS `.icns` file.
    Icns(PathBuf),
    /// A Windows program with icon resources.
    Exe(PathBuf),
}

impl IconSource {
    /// The icon as PNG or ICO bytes, ready for an image decoder; `None` when it can't be read.
    pub fn load(&self) -> Option<Vec<u8>> {
        match self {
            IconSource::Image(p) => slicer_icon::read_capped(p, 4 << 20),
            IconSource::Icns(p) => slicer_icon::icns_png(&slicer_icon::read_capped(p, 32 << 20)?),
            IconSource::Exe(p) => slicer_icon::exe_icon(p),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            IconSource::Image(p) | IconSource::Icns(p) | IconSource::Exe(p) => p,
        }
    }
}

struct Known {
    id: &'static str,
    name: &'static str,
    /// Names the application goes by, letters and digits only, lower case; an entry matches when
    /// its name starts with one of them.
    keys: &'static [&'static str],
    /// Program file names.
    exes: &'static [&'static str],
}

const KNOWN: &[Known] = &[
    Known { id: "prusa", name: "PrusaSlicer", keys: &["prusaslicer"], exes: &["prusa-slicer", "PrusaSlicer", "prusa-slicer.exe"] },
    Known { id: "bambu", name: "Bambu Studio", keys: &["bambustudio"], exes: &["bambu-studio", "BambuStudio", "bambu-studio.exe"] },
    Known { id: "orca", name: "OrcaSlicer", keys: &["orcaslicer"], exes: &["orca-slicer", "OrcaSlicer", "orca-slicer.exe"] },
    Known {
        id: "cura",
        name: "UltiMaker Cura",
        keys: &["ultimakercura", "curaultimaker", "cura"],
        exes: &["cura", "UltiMaker-Cura", "UltiMaker-Cura.exe", "Cura.exe"],
    },
    Known { id: "superslicer", name: "SuperSlicer", keys: &["superslicer"], exes: &["superslicer", "SuperSlicer", "superslicer.exe"] },
    Known { id: "creality", name: "Creality Print", keys: &["crealityprint"], exes: &["CrealityPrint", "creality-print", "CrealityPrint.exe"] },
    Known { id: "elegoo", name: "ElegooSlicer", keys: &["elegooslicer"], exes: &["elegoo-slicer", "ElegooSlicer", "ElegooSlicer.exe"] },
    Known { id: "anycubic", name: "AnycubicSlicerNext", keys: &["anycubicslicer"], exes: &["AnycubicSlicerNext", "AnycubicSlicerNext.exe"] },
    Known { id: "ideamaker", name: "ideaMaker", keys: &["ideamaker"], exes: &["ideamaker", "ideaMaker.exe"] },
    Known { id: "lychee", name: "Lychee Slicer", keys: &["lycheeslicer"], exes: &["lychee-slicer", "Lychee Slicer.exe"] },
    Known { id: "slic3r", name: "Slic3r", keys: &["slic3r"], exes: &["slic3r", "Slic3r", "slic3r.exe"] },
];

/// Lower-case letters and digits only: how names are compared.
fn normalize(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// The known slicer an application name (or file name) belongs to.
fn known_for(name: &str) -> Option<&'static Known> {
    let n = normalize(name);
    // Companion tools (G-code viewers, command-line builds) are not slicers to send a mesh to.
    if n.contains("viewer") || n.contains("console") || n.contains("gcode") || n.contains("uninstall") {
        return None;
    }
    KNOWN.iter().find(|k| k.keys.iter().any(|key| n.starts_with(key)))
}

/// Folders and variables detection looks at, so it can be tested without the real machine.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// The user's home folder.
    pub home: Option<PathBuf>,
    /// `PATH`.
    pub path_dirs: Vec<PathBuf>,
    /// XDG data folders (each holds `applications/` and `icons/`).
    pub data_dirs: Vec<PathBuf>,
    /// Folders holding AppImages, loose programs and per-slicer folders (Linux).
    pub program_dirs: Vec<PathBuf>,
    /// Folders holding `.app` bundles (macOS).
    pub app_dirs: Vec<PathBuf>,
    /// Folders holding one folder per installed program (Windows).
    pub install_dirs: Vec<PathBuf>,
}

impl Env {
    /// The running machine.
    pub fn current() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = var("HOME").or_else(|| var("USERPROFILE"));
        let mut env = Env {
            home: home.clone(),
            path_dirs: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
            ..Env::default()
        };
        if cfg!(target_os = "macos") {
            env.app_dirs.push("/Applications".into());
            env.app_dirs.extend(home.iter().map(|h| h.join("Applications")));
        } else if cfg!(target_os = "windows") {
            for k in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                env.install_dirs.extend(var(k));
            }
            if let Some(local) = var("LOCALAPPDATA") {
                env.install_dirs.push(local.join("Programs"));
                env.install_dirs.push(local);
            }
        } else {
            let data_home = var("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share")));
            env.data_dirs.extend(data_home.clone());
            env.data_dirs.extend(home.iter().map(|h| h.join(".local/share/flatpak/exports/share")));
            env.data_dirs.push("/var/lib/flatpak/exports/share".into());
            env.data_dirs.push("/var/lib/snapd/desktop".into());
            match std::env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty()) {
                Some(dirs) => env.data_dirs.extend(std::env::split_paths(&dirs)),
                None => env.data_dirs.extend(["/usr/local/share".into(), "/usr/share".into()]),
            }
            for dir in ["Applications", "AppImages", "Apps", ".local/bin", "bin"] {
                env.program_dirs.extend(home.iter().map(|h| h.join(dir)));
            }
            env.program_dirs.extend(["/opt", "/usr/local/bin"].map(PathBuf::from));
        }
        env
    }
}

/// The slicers installed on this machine, known ones in a fixed order. Empty in the browser.
pub fn detect() -> Vec<Slicer> {
    if cfg!(target_arch = "wasm32") { Vec::new() } else { detect_in(&Env::current()) }
}

/// [`detect`] over the folders in `env`.
pub fn detect_in(env: &Env) -> Vec<Slicer> {
    let mut found: Vec<Slicer> = Vec::new();
    let mut add = |s: Slicer| {
        if !found.iter().any(|f| f.id == s.id) {
            found.push(s);
        }
    };
    desktop_entries(env, &mut add);
    path_programs(env, &mut add);
    program_folders(env, &mut add);
    mac_apps(env, &mut add);
    windows_programs(env, &mut add);
    let order = |s: &Slicer| KNOWN.iter().position(|k| k.id == s.id).unwrap_or(usize::MAX);
    found.sort_by_key(order);
    found
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).take(MAX_ENTRIES).collect()).unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn known_slicer(k: &Known, command: Vec<String>, icon: Option<IconSource>) -> Slicer {
    Slicer { id: k.id.into(), name: k.name.into(), command, icon }
}

// --- Linux ---

/// A desktop entry's `[Desktop Entry]` keys.
fn parse_desktop(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut inside = false;
    for line in text.lines().take(2_000) {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
        } else if inside && let Some((k, v)) = line.split_once('=') {
            // Localised keys (`Name[de]`) never replace the plain one.
            if !k.contains('[') {
                out.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
            }
        }
    }
    out
}

/// An `Exec=` line as a command: quotes honoured, field codes (`%F`, …) dropped.
pub fn parse_exec(exec: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args.retain(|a| !(a.len() == 2 && a.starts_with('%')));
    args
}

fn desktop_entries(env: &Env, add: &mut impl FnMut(Slicer)) {
    for data in &env.data_dirs {
        for file in entries(&data.join("applications")) {
            if file.extension().is_none_or(|e| e != "desktop") || std::fs::metadata(&file).map(|m| m.len() > MAX_DESKTOP_BYTES).unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let entry = parse_desktop(&text);
            let Some(known) = entry.get("Name").and_then(|n| known_for(n)) else { continue };
            if entry.get("Type").is_some_and(|t| t != "Application") {
                continue;
            }
            let command = entry.get("Exec").map(|e| parse_exec(e)).unwrap_or_default();
            let Some(program) = command.first() else { continue };
            // A stale entry points at a program that is gone.
            if Path::new(program).is_absolute() && !Path::new(program).is_file() {
                continue;
            }
            let icon = entry.get("Icon").and_then(|i| theme_icon(env, i));
            add(known_slicer(known, command, icon));
        }
    }
}

/// An icon by theme name (or absolute PNG path) from the icon themes.
fn theme_icon(env: &Env, name: &str) -> Option<IconSource> {
    const SIZES: [u32; 7] = [256, 128, 192, 64, 48, 512, 32];
    let direct = Path::new(name);
    if direct.is_absolute() {
        return (direct.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) && direct.is_file())
            .then(|| IconSource::Image(direct.to_path_buf()));
    }
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return None;
    }
    for data in &env.data_dirs {
        for size in SIZES {
            let p = data.join(format!("icons/hicolor/{size}x{size}/apps/{name}.png"));
            if p.is_file() {
                return Some(IconSource::Image(p));
            }
        }
        let p = data.join(format!("pixmaps/{name}.png"));
        if p.is_file() {
            return Some(IconSource::Image(p));
        }
    }
    None
}

fn path_programs(env: &Env, add: &mut impl FnMut(Slicer)) {
    for k in KNOWN {
        for dir in &env.path_dirs {
            for exe in k.exes {
                let p = dir.join(exe);
                if is_executable(&p) {
                    let icon = k.exes.iter().chain([&k.id]).find_map(|n| theme_icon(env, n.trim_end_matches(".exe")));
                    add(known_slicer(k, vec![s(&p)], icon));
                }
            }
        }
    }
}

/// AppImages and per-slicer folders in the usual places.
fn program_folders(env: &Env, add: &mut impl FnMut(Slicer)) {
    for dir in &env.program_dirs {
        for p in entries(dir) {
            let name = file_name(&p);
            let Some(known) = known_for(&name) else { continue };
            if p.is_dir() {
                let inside = known.exes.iter().flat_map(|e| [p.join(e), p.join("bin").join(e)]).find(|c| is_executable(c));
                if let Some(exe) = inside {
                    let icon = theme_icon(env, known.id);
                    add(known_slicer(known, vec![s(&exe)], icon));
                }
            } else if name.to_ascii_lowercase().ends_with(".appimage") && is_executable(&p) {
                add(known_slicer(known, vec![s(&p)], None));
            }
        }
    }
}

// --- macOS ---

fn mac_apps(env: &Env, add: &mut impl FnMut(Slicer)) {
    for dir in &env.app_dirs {
        for app in entries(dir) {
            let name = file_name(&app);
            let Some(stem) = name.strip_suffix(".app") else { continue };
            let Some(known) = known_for(stem) else { continue };
            add(known_slicer(known, vec!["open".into(), "-a".into(), s(&app)], mac_icon(&app)));
        }
    }
}

/// The icon of a `.app` bundle: the `.icns` named like the app, else the first one.
fn mac_icon(app: &Path) -> Option<IconSource> {
    let icons: Vec<PathBuf> = entries(&app.join("Contents/Resources")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "icns")).collect();
    let stem = normalize(&file_name(app).replace(".app", ""));
    let pick = icons.iter().find(|p| p.file_stem().is_some_and(|n| normalize(&n.to_string_lossy()) == stem)).or_else(|| icons.iter().min());
    pick.cloned().map(IconSource::Icns)
}

// --- Windows ---

fn windows_programs(env: &Env, add: &mut impl FnMut(Slicer)) {
    for root in &env.install_dirs {
        for dir in entries(root).into_iter().filter(|p| p.is_dir()) {
            let Some(known) = known_for(&file_name(&dir)) else { continue };
            let exe = known.exes.iter().filter(|e| e.ends_with(".exe")).flat_map(|e| [dir.join(e), dir.join("bin").join(e)]).find(|c| c.is_file());
            if let Some(exe) = exe {
                add(known_slicer(known, vec![s(&exe)], Some(IconSource::Exe(exe.clone()))));
            }
        }
    }
}

// --- custom programs and launching ---

/// `name` with its first letter in capitals.
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A program the user picked: an application bundle, a program file or an AppImage.
pub fn custom(path: &str) -> Option<Slicer> {
    let p = Path::new(path);
    let name = file_name(p);
    let lower = name.to_ascii_lowercase();
    let mac_app = lower.ends_with(".app") && p.is_dir();
    if !(mac_app || (p.is_file() && is_executable(p))) {
        return None;
    }
    let stem = [".app", ".exe", ".appimage"]
        .iter()
        .find(|ext| lower.ends_with(**ext))
        .and_then(|ext| name.get(..name.len() - ext.len()))
        .map_or_else(|| name.clone(), str::to_string);
    let name = known_for(&stem).map_or_else(|| capitalised(&stem), |k| k.name.to_string());
    let (command, icon) = if mac_app {
        (vec!["open".into(), "-a".into(), path.to_string()], mac_icon(p))
    } else if lower.ends_with(".exe") {
        (vec![path.to_string()], Some(IconSource::Exe(p.to_path_buf())))
    } else {
        let stem = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (vec![path.to_string()], theme_icon(&Env::current(), &stem))
    };
    Some(Slicer { id: "custom".into(), name, command, icon })
}

/// A slicer from a command parameter: the id or name of an installed one, else a program path.
pub fn resolve(spec: &str) -> Option<Slicer> {
    let wanted = normalize(spec);
    if !wanted.is_empty()
        && let Some(found) = detect().into_iter().find(|d| normalize(&d.id) == wanted || normalize(&d.name) == wanted)
    {
        return Some(found);
    }
    custom(spec)
}

/// `args` with the file added: at the end, or inside the `@@ … @@` pair of a Flatpak command, which
/// is how Flatpak lets the application see a file (`@@u` would hand it over as a URI).
#[cfg(not(target_arch = "wasm32"))]
fn with_file(args: &[String], path: &str) -> Vec<String> {
    let mut out: Vec<String> = args.iter().map(|a| if a == "@@u" { "@@".to_string() } else { a.clone() }).collect();
    let marks: Vec<usize> = out.iter().enumerate().filter(|(_, a)| *a == "@@").map(|(i, _)| i).collect();
    match marks.as_slice() {
        [.., _, last] => out.insert(*last, path.to_string()),
        _ => out.push(path.to_string()),
    }
    out
}

impl Slicer {
    /// Start the slicer with `path` loaded; does not wait for it to exit.
    pub fn open(&self, path: &str) -> std::result::Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Err(format!("{} can't be started from the browser", self.name))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::process::{Command, Stdio};
            let (program, args) = self.command.split_first().ok_or_else(|| format!("{} has no program to start", self.name))?;
            Command::new(program)
                .args(with_file(args, path))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(drop)
                .map_err(|e| format!("couldn't start {} ({e}); the file is saved at {path}", self.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("solidcraft-slicers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_match_known_slicers_but_not_their_companions() {
        assert_eq!(known_for("PrusaSlicer").map(|k| k.id), Some("prusa"));
        assert_eq!(known_for("Bambu Studio").map(|k| k.id), Some("bambu"));
        assert_eq!(known_for("UltiMaker Cura 5.9").map(|k| k.id), Some("cura"));
        assert!(known_for("PrusaSlicer G-code Viewer").is_none());
        assert!(known_for("Firefox").is_none());
    }

    #[test]
    fn exec_lines_become_commands() {
        assert_eq!(parse_exec("prusa-slicer %F"), ["prusa-slicer"]);
        assert_eq!(parse_exec(r#""/opt/My Apps/bambu" --x %U"#), ["/opt/My Apps/bambu", "--x"]);
        assert_eq!(
            parse_exec("flatpak run --branch=stable com.prusa3d.PrusaSlicer @@u %U @@"),
            ["flatpak", "run", "--branch=stable", "com.prusa3d.PrusaSlicer", "@@u", "@@"]
        );
        assert!(parse_exec("").is_empty());
    }

    #[test]
    fn the_file_goes_where_the_command_expects_it() {
        let v = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(with_file(&v("run --file-forwarding app --x @@u @@"), "/t/a.3mf"), v("run --file-forwarding app --x @@ /t/a.3mf @@"));
        assert_eq!(with_file(&v("-a /Applications/X.app"), "/t/a.stl"), v("-a /Applications/X.app /t/a.stl"));
        assert_eq!(with_file(&[], "/t/a.stl"), v("/t/a.stl"));
    }

    #[test]
    fn finds_linux_desktop_entries_and_their_icons() {
        let root = scratch("linux");
        let data = root.join("share");
        std::fs::create_dir_all(data.join("applications")).unwrap();
        std::fs::create_dir_all(data.join("icons/hicolor/128x128/apps")).unwrap();
        std::fs::write(data.join("icons/hicolor/128x128/apps/PrusaSlicer.png"), b"png").unwrap();
        std::fs::write(
            data.join("applications/a.desktop"),
            "[Desktop Entry]\nType=Application\nName=PrusaSlicer\nName[de]=Prusa\nExec=flatpak run com.prusa3d.PrusaSlicer %U\nIcon=PrusaSlicer\n",
        )
        .unwrap();
        std::fs::write(data.join("applications/b.desktop"), "[Desktop Entry]\nName=Firefox\nExec=firefox\n").unwrap();
        std::fs::write(data.join("applications/c.desktop"), "[Desktop Entry]\nName=PrusaSlicer G-code Viewer\nExec=prusa-slicer --gcodeviewer\n")
            .unwrap();
        let found = detect_in(&Env { data_dirs: vec![data.clone()], ..Env::default() });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "prusa");
        assert_eq!(found[0].command, ["flatpak", "run", "com.prusa3d.PrusaSlicer"]);
        assert_eq!(found[0].icon, Some(IconSource::Image(data.join("icons/hicolor/128x128/apps/PrusaSlicer.png"))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_mac_bundles_and_windows_folders() {
        let root = scratch("mac-win");
        let apps = root.join("Applications");
        std::fs::create_dir_all(apps.join("BambuStudio.app/Contents/Resources")).unwrap();
        std::fs::write(apps.join("BambuStudio.app/Contents/Resources/BambuStudio.icns"), b"x").unwrap();
        std::fs::create_dir_all(apps.join("Safari.app")).unwrap();
        let prog = root.join("Program Files");
        std::fs::create_dir_all(prog.join("Prusa3D")).unwrap();
        std::fs::create_dir_all(prog.join("OrcaSlicer")).unwrap();
        std::fs::write(prog.join("OrcaSlicer/orca-slicer.exe"), b"x").unwrap();
        let found = detect_in(&Env { app_dirs: vec![apps.clone()], install_dirs: vec![prog.clone()], ..Env::default() });
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["bambu", "orca"]);
        assert_eq!(found[0].command, ["open", "-a", apps.join("BambuStudio.app").to_str().unwrap()]);
        assert!(matches!(found[0].icon, Some(IconSource::Icns(_))));
        assert_eq!(found[1].icon, Some(IconSource::Exe(prog.join("OrcaSlicer/orca-slicer.exe"))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finds_programs_on_the_path_and_appimages() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("path");
        let bin = root.join("bin");
        let apps = root.join("Applications");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&apps).unwrap();
        let exe = bin.join("orca-slicer");
        let image = apps.join("PrusaSlicer-2.8.AppImage");
        for f in [&exe, &image] {
            std::fs::write(f, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(bin.join("prusa-slicer"), b"not executable").unwrap();
        let found = detect_in(&Env { path_dirs: vec![bin.clone()], program_dirs: vec![apps.clone()], ..Env::default() });
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["prusa", "orca"]);
        assert_eq!(found[0].command, [image.to_str().unwrap()]);
        // A picked program becomes a custom entry named after it; a missing path is refused.
        let c = custom(exe.to_str().unwrap()).unwrap();
        assert_eq!((c.id.as_str(), c.name.as_str()), ("custom", "OrcaSlicer"));
        assert_eq!(capitalised("mycura"), "Mycura");
        assert!(custom(root.join("nope").to_str().unwrap()).is_none());
        assert!(custom(root.to_str().unwrap()).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
