//! Opening a large design keeps the window alive: frames keep coming, the title bar still
//! moves the window, the design underneath waits, and the result is the synchronous open's.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, ViewportCommand, ViewportId, pos2, vec2};
use serde_json::json;
use solidcraft_engine::Session;

use crate::SolveApp;

/// The whole app in a headless 1600 x 1000 window with the custom title bar.
struct Win {
    ctx: egui::Context,
    app: SolveApp,
    time: f64,
}

impl Win {
    fn new() -> Self {
        let mut app = SolveApp::new(Session::default(), Default::default());
        app.custom_titlebar = true;
        let mut w = Win { ctx: egui::Context::default(), app, time: 0.0 };
        for _ in 0..3 {
            w.frame(vec![]);
        }
        w
    }

    /// One frame: the viewport commands it sent.
    fn frame(&mut self, events: Vec<Event>) -> Vec<ViewportCommand> {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))),
            time: Some(self.time),
            focused: true,
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()
    }

    /// The frames of a press-drag-release along `path`: the viewport commands they sent.
    fn gesture(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
        let button = |p: Pos2, pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true)]];
        events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
        events.push(vec![button(*path.last().unwrap(), false)]);
        events.into_iter().flat_map(|e| self.frame(e)).collect()
    }
}

/// A design whose timeline takes a while to rebuild: a plate with `n` posts joined and `n`
/// holes cut, saved into `dir`.
fn large_design(dir: &std::path::Path, n: usize) -> String {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 400, "width": 400, "height": 10, "corner": [-200, -200, 0]})).unwrap();
    for i in 0..n {
        let (x, y) = ((i % 12) as f64 * 30.0 - 165.0, (i / 12) as f64 * 30.0 - 165.0);
        s.execute("solid.cylinder", &json!({"radius": 6, "height": 20, "base": [x, y, 5], "operation": "join"})).unwrap();
        s.execute("solid.cylinder", &json!({"radius": 2, "height": 40, "base": [x + 9.0, y + 9.0, -5], "operation": "cut"})).unwrap();
    }
    let path = dir.join("large.solidcraft").to_string_lossy().to_string();
    s.execute("file.save", &json!({"path": path})).unwrap();
    path
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("solidcraft-file-load-{name}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// What two sessions must agree on after opening the same file.
fn fingerprint(s: &Session) -> serde_json::Value {
    let world = s.world_state();
    let bodies: Vec<_> = world.bodies.iter().map(|b| (b.name.clone(), b.mesh().triangles.len())).collect();
    let errors = s.model.results.iter().filter(|r| r.error.is_some()).count();
    let b = solidcraft_engine::view::bounds(s);
    json!({
        "name": s.doc.name,
        "path": s.path,
        "features": s.doc.features.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        "bodies": bodies,
        "errors": errors,
        "bounds": [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z],
        "dirty": s.is_dirty(),
    })
}

#[test]
fn the_window_keeps_drawing_and_moving_while_a_large_design_opens() {
    let dir = scratch("drag");
    let path = large_design(&dir, 4);
    let mut sync = Session::default();
    sync.execute("doc.open", &json!({"path": path})).unwrap();

    let mut w = Win::new();
    let blank = w.app.session.doc.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    w.app.file_load.worker_gate = Some((started_tx, release_rx));
    w.app.open_path_async(&path);
    // The deadline only bounds a broken test; responsiveness is proved by ordering, not time.
    let worker = started_rx.recv_timeout(std::time::Duration::from_secs(60)).expect("file worker started");
    assert_ne!(worker, std::thread::current().id(), "opening runs on another thread");
    for _ in 0..3 {
        w.frame(vec![]);
        assert!(w.app.file_load.busy(), "frames finish before the file worker is released");
        assert!(std::sync::Arc::ptr_eq(&w.app.session.doc, &blank), "the old design stays intact");
    }
    let bar = pos2(500.0, 17.0);
    let viewport = w.app.viewport.rect.map_or(pos2(800.0, 500.0), |r| r.center());
    let cmds = w.gesture(&[bar, bar + vec2(12.0, 4.0), bar + vec2(40.0, 10.0)]);
    assert!(cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "the title bar drag was handled during the open");
    let cam = serde_json::to_value(w.app.cam).unwrap();
    w.gesture(&[viewport, viewport + vec2(30.0, 0.0), viewport + vec2(80.0, 20.0)]);
    assert_eq!(serde_json::to_value(w.app.cam).unwrap(), cam, "the design underneath took no camera input");
    assert!(w.app.tool.is_none());
    assert!(crate::scenario::handle_at("file_loading:cancel").is_some(), "the Opening overlay with Cancel was shown");
    assert!(w.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).is_err());
    assert!(w.app.file_load.busy(), "all window interactions completed before releasing the worker");
    assert!(std::sync::Arc::ptr_eq(&w.app.session.doc, &blank));
    release_tx.send(()).expect("release the file worker after window interactions");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while w.app.file_load.busy() && std::time::Instant::now() < deadline {
        w.frame(vec![]);
        std::thread::yield_now();
    }
    assert!(!w.app.file_load.busy(), "the released open finished");
    for _ in 0..3 {
        w.frame(vec![]);
    }
    assert!(!std::sync::Arc::ptr_eq(&w.app.session.doc, &blank));
    assert_eq!(fingerprint(&w.app.session), fingerprint(&sync), "the background open matches a synchronous one");
    assert_eq!(fingerprint(&w.app.session)["errors"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_stops_the_open_and_keeps_the_current_design() {
    let dir = scratch("cancel");
    let path = large_design(&dir, 4);
    let mut w = Win::new();
    w.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
    let current = w.app.session.doc.clone();
    // The worker waits before the third feature until Cancel: the load is certainly still
    // running when Cancel is clicked, on a fast machine or a slow one.
    w.app.file_load.hold_at = Some(2);
    w.app.open_path_async(&path);
    let began = std::time::Instant::now();
    while w.app.file_load.features().is_none_or(|(done, _)| done < 2) {
        assert!(began.elapsed() < std::time::Duration::from_secs(120), "the load reached the held feature");
        w.frame(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // The overlay (and any saved preview the worker sent) is laid out and shown.
    for _ in 0..3 {
        w.frame(vec![]);
    }
    assert!(w.app.file_load.busy());
    let cancel = crate::scenario::handle_at("file_loading:cancel").expect("the overlay's Cancel button");
    w.gesture(&[cancel]);
    assert!(!w.app.file_load.busy(), "Cancel gives the window back at once");
    for _ in 0..100_000 {
        if w.app.file_load.idle() {
            break;
        }
        w.frame(vec![]);
    }
    for _ in 0..3 {
        w.frame(vec![]);
    }
    assert!(std::sync::Arc::ptr_eq(&w.app.session.doc, &current), "the current design stays");
    assert_eq!(w.app.docs.count(), 1, "no tab was opened");
    assert!(w.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).is_ok(), "editing continues");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A control-channel (MCP) open is answered when the file is in, with the synchronous
/// command's reply, and the window draws frames while it waits. Commands sent meanwhile wait.
#[test]
fn a_control_open_answers_after_loading_without_freezing_the_window() {
    let dir = scratch("control");
    let path = large_design(&dir, 4);
    let mut sync = Session::default();
    let expected = sync.execute("doc.open", &json!({"path": path})).unwrap();
    let mut w = Win::new();
    let (tx, rx) = std::sync::mpsc::channel();
    w.app.control_rx = Some(rx);
    let (open, open_rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": "doc.open", "params": {"path": path}}));
    let (count, count_rx) = crate::control::ControlRequest::new("document.inspect", json!({}));
    let (edit, edit_rx) =
        crate::control::ControlRequest::new("engine.execute", json!({"command": "solid.box", "params": {"length": 1, "width": 1, "height": 1}}));
    for r in [open, count, edit] {
        tx.send(r).unwrap();
    }
    let mut frames = 0;
    let reply = loop {
        frames += 1;
        w.frame(vec![]);
        if let Ok(v) = open_rx.try_recv() {
            break v;
        }
        assert!(frames < 100_000);
    };
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"], expected);
    assert!(count_rx.try_recv().is_ok(), "inspection answered while the file loaded");
    for _ in 0..3 {
        w.frame(vec![]);
    }
    let edited = edit_rx.try_recv().expect("the held edit ran after the open");
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(w.app.session.doc.features.len(), sync.doc.features.len() + 1, "the edit went into the opened design");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A control-channel (MCP) import hands its `up` and `names` to the command, as the engine
/// session takes them, and a caller that said which way is up is not offered to stand the part up.
#[test]
fn a_control_import_passes_up_and_names_on() {
    let dir = scratch("import-options");
    let path = dir.join("tall_in_y.step").to_string_lossy().to_string();
    let mut src = Session::default();
    src.execute("solid.box", &json!({"length": 10, "width": 40, "height": 5})).unwrap();
    src.execute("file.export", &json!({"path": path})).unwrap();

    // The reply, the part's height and whether the offer is up, for an import by this window.
    let import = |params: serde_json::Value| {
        let mut w = Win::new();
        let (tx, rx) = std::sync::mpsc::channel();
        w.app.control_rx = Some(rx);
        let (req, reply_rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": "file.insert_step", "params": params.clone()}));
        tx.send(req).unwrap();
        let mut reply = None;
        for _ in 0..100_000 {
            w.frame(vec![]);
            reply = reply_rx.try_recv().ok();
            if reply.is_some() {
                break;
            }
        }
        let reply = reply.expect("the import was answered");
        assert_eq!(reply["ok"], true, "{params}: {reply}");
        let mut sync = Session::default();
        assert_eq!(reply["result"], sync.execute("file.insert_step", &params).unwrap(), "the window's reply is the session's");
        let b = solidcraft_engine::view::bounds(&w.app.session);
        (reply, [b.max.x - b.min.x, b.max.y - b.min.y, b.max.z - b.min.z], w.app.stand_up.is_some())
    };

    // As before: the file's axes, and the offer to stand it up.
    let (_, size, offered) = import(json!({"path": path}));
    assert!(size[1] > 39.9 && offered, "{size:?} {offered}");
    // Y up said by the caller: stood up by the import, nothing left to offer.
    let (reply, size, offered) = import(json!({"path": path, "up": "y", "names": "product"}));
    assert!(size[2] > 39.9 && !offered, "{size:?} {offered}: {reply}");
    assert!(reply["result"]["stand_up"].is_u64(), "{reply}");
    // Z up said by the caller: kept as it is, and not asked about either.
    let (_, size, offered) = import(json!({"path": path, "up": "z"}));
    assert!(size[1] > 39.9 && !offered, "{size:?} {offered}");
    let _ = std::fs::remove_dir_all(&dir);
}
