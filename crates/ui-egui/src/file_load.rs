//! Interactive file operations run off the window thread. The old design stays intact until
//! the command and display meshes are ready; failures and cancelled jobs never replace it.

use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
    mpsc::{Receiver, TryRecvError},
};

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;
use solvecraft_engine::{Session, doc::Document};

use crate::SolveApp;

struct Request {
    path: String,
    insert: bool,
}
struct Made {
    session: Session,
    result: Value,
    image: Option<egui::ColorImage>,
}
type Job = Result<Made, String>;
struct Running {
    request: Request,
    base: Arc<Document>,
    before: Vec<String>,
    rx: Receiver<Job>,
    phase: Arc<AtomicU8>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct FileLoad {
    queued: VecDeque<Request>,
    running: Option<Running>,
}

impl FileLoad {
    pub fn busy(&self) -> bool {
        self.running.as_ref().is_some_and(|r| !r.cancelled.load(Ordering::Relaxed)) || !self.queued.is_empty()
    }
}

pub fn begin(app: &mut SolveApp, path: &str, insert: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        if insert {
            app.insert_path(path);
        } else {
            app.open_path(path);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.file_load.queued.len() >= crate::documents::MAX_OPEN {
            app.set_status("too many queued files", true);
            return;
        }
        app.file_load.queued.push_back(Request { path: path.into(), insert });
        start(app);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start(app: &mut SolveApp) {
    if app.file_load.running.is_some() {
        return;
    }
    let Some(request) = app.file_load.queued.pop_front() else { return };
    if !request.insert && app.docs.count() >= crate::documents::MAX_OPEN && !crate::documents::active_is_blank(app) {
        app.set_status("too many open designs; close a tab before opening another", true);
        return;
    }
    let before = if request.insert { app.session.world_state().bodies.iter().map(|b| b.name.clone()).collect() } else { Vec::new() };
    let mut scratch = if request.insert { app.session.scratch() } else { Session::default() };
    if request.insert {
        scratch.selection = app.session.selection.clone();
    }
    let base = app.session.doc.clone();
    let phase = Arc::new(AtomicU8::new(0));
    let worker_phase = phase.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let path = request.path.clone();
    let insert = request.insert;
    let spawned = std::thread::Builder::new().name("file-load".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(scratch, &path, insert, &worker_phase, &worker_cancelled)))
            .unwrap_or_else(|_| Err("the file operation failed; the current design is unchanged".into()));
        let _ = tx.send(result);
    });
    match spawned {
        Ok(_) => app.file_load.running = Some(Running { request, base, before, rx, phase, cancelled }),
        Err(e) => app.set_status(format!("could not start file operation: {e}"), true),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn compute(mut session: Session, path: &str, insert: bool, phase: &AtomicU8, cancelled: &AtomicBool) -> Job {
    let profile = std::env::var_os("SOLVECRAFT_PROFILE_OPEN").is_some();
    let began = std::time::Instant::now();
    if cancelled.load(Ordering::Relaxed) {
        return Err("file operation cancelled".into());
    }
    let command = if !insert {
        "doc.open"
    } else if solvecraft_engine::io::is_mesh_path(path) {
        "file.insert_mesh"
    } else {
        "file.insert_step"
    };
    let result = session.execute(command, &json!({"path": path})).map_err(|e| e.to_string())?;
    let geometry_ms = began.elapsed().as_secs_f64() * 1000.0;
    let view_began = std::time::Instant::now();
    phase.store(1, Ordering::Relaxed);
    let world = session.world_state();
    let placement_ms = view_began.elapsed().as_secs_f64() * 1000.0;
    let mesh_began = std::time::Instant::now();
    // Fit, picking and the first viewport frame must not trigger cold tessellation on the UI.
    for b in &world.bodies {
        if cancelled.load(Ordering::Relaxed) {
            return Err("file operation cancelled".into());
        }
        let _ = b.mesh();
    }
    let tessellation_ms = mesh_began.elapsed().as_secs_f64() * 1000.0;
    phase.store(2, Ordering::Relaxed);
    let view_ms = view_began.elapsed().as_secs_f64() * 1000.0;
    let thumbnail_began = std::time::Instant::now();
    let image = if insert { None } else { crate::home::remember(&session) };
    if profile {
        eprintln!(
            "SolidCraft open phases: geometry_ms={geometry_ms:.3} placement_ms={placement_ms:.3} tessellation_ms={tessellation_ms:.3} display_warmup_ms={view_ms:.3} thumbnail_ms={:.3} total_ms={:.3}",
            thumbnail_began.elapsed().as_secs_f64() * 1000.0,
            began.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(Made { session, result, image })
}

pub fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    let result = app.file_load.running.as_ref().and_then(|r| match r.rx.try_recv() {
        Ok(job) => Some(job),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("the file operation stopped; the current design is unchanged".into())),
    });
    if let Some(job) = result
        && let Some(running) = app.file_load.running.take()
    {
        match job {
            _ if running.cancelled.load(Ordering::Relaxed) => {}
            Ok(made) if !running.request.insert || Arc::ptr_eq(&app.session.doc, &running.base) => {
                if running.request.insert {
                    app.session.accept_computed(made.session);
                    app.big_operation = true;
                } else {
                    crate::documents::adopt(app, made.session);
                    app.home.add_recent(&running.request.path);
                    crate::home::install(&mut app.home, ctx, &running.request.path, made.image);
                }
                app.home.open = false;
                app.fit_view();
                crate::stand_up::offer(app, &running.request.path, &running.before);
                let warnings = made.result.get("warnings").and_then(Value::as_array);
                app.set_status(
                    if let Some(w) = warnings.filter(|w| !w.is_empty()) {
                        format!("imported with {} warning(s): {}", w.len(), w.first().and_then(Value::as_str).unwrap_or(""))
                    } else {
                        format!("{} done", if running.request.insert { "Insert" } else { "Open" })
                    },
                    false,
                );
            }
            Ok(_) => app.set_status("the active design changed while importing; the import was discarded", true),
            Err(e) => app.set_status(e, true),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    start(app);
    if app.file_load.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(r) = &app.file_load.running else { return };
    let cancelled = r.cancelled.load(Ordering::Relaxed);
    if cancelled && app.file_load.queued.is_empty() {
        return;
    }
    let name = std::path::Path::new(&r.request.path).file_name().map(|p| p.to_string_lossy()).unwrap_or_default();
    let phase = r.phase.load(Ordering::Relaxed);
    let mut cancel = false;
    egui::Modal::new(egui::Id::new("file_loading")).show(ctx, |ui| {
        ui.heading(if cancelled {
            "Waiting for file operation"
        } else if r.request.insert {
            "Inserting geometry"
        } else {
            "Opening design"
        });
        ui.label(name.as_ref());
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if cancelled {
                "Waiting for the previous operation to stop..."
            } else {
                match phase {
                    1 => "Preparing the view...",
                    2 => "Preparing the thumbnail...",
                    _ => "Reading and building geometry...",
                }
            });
        });
        cancel = ui.button("Cancel").clicked();
    });
    if cancel {
        // Keep the running job until it exits: cancelling must not allow another large
        // import to overlap it in memory. The user can continue editing the current design.
        if let Some(r) = &app.file_load.running {
            r.cancelled.store(true, Ordering::Relaxed);
        }
        app.file_load.queued.clear();
        app.set_status("file operation cancelled", false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_import_keeps_the_current_document() {
        let mut app = SolveApp::new(Session::default(), Default::default());
        let old = app.session.doc.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Err("bad STEP".into())).unwrap();
        app.file_load.running = Some(Running {
            request: Request { path: "bad.step".into(), insert: true },
            base: old.clone(),
            before: Vec::new(),
            rx,
            phase: Arc::new(AtomicU8::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        tick(&mut app, &egui::Context::default());
        assert!(Arc::ptr_eq(&app.session.doc, &old));
        assert!(!app.file_load.busy());
        assert!(app.status.as_ref().unwrap().0.contains("bad STEP"));
    }
    #[test]
    fn cancelling_discards_a_finished_result_and_keeps_the_worker_slot_until_completion() {
        let mut app = SolveApp::new(Session::default(), Default::default());
        let old = app.session.doc.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(true));
        app.file_load.running = Some(Running {
            request: Request { path: "cancelled.step".into(), insert: true },
            base: old.clone(),
            before: Vec::new(),
            rx,
            phase: Arc::new(AtomicU8::new(0)),
            cancelled,
        });
        assert!(!app.file_load.busy());
        assert!(app.file_load.running.is_some());
        let mut computed = Session::default();
        computed.execute("solid.box", &serde_json::json!({"length":10,"width":10,"height":10})).unwrap();
        tx.send(Ok(Made { session: computed, result: Value::Null, image: None })).unwrap();
        tick(&mut app, &egui::Context::default());
        assert!(app.file_load.running.is_none());
        assert!(Arc::ptr_eq(&old, &app.session.doc));
    }
}
