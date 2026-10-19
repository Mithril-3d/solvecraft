//! Copy and Paste from the keyboard and the context menu (#1480).

use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::geom::Vec2;

use crate::scenario::Harness;

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// A sketch on XY with a 20 × 10 rectangle (dimensioned, its corner on the origin), looked at
/// from the top, and the rectangle selected.
fn rectangle() -> (Harness, Vec<String>) {
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    let rect = strs(&h.app.run("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [20, 10]})).unwrap()["curves"]);
    h.app.run("sketch.constraint.coincident", json!({"a": format!("{}.start", rect[0]), "b": "origin"})).unwrap();
    h.app.run("sketch.dimension", json!({"entities": [rect[0]], "value": 20})).unwrap();
    h.app.run("sketch.dimension", json!({"entities": [rect[1]], "value": 10})).unwrap();
    h.call("ui.view", json!({"view": "top"}));
    h.call("ui.camera", json!({"target": [20, 10, 0], "distance": 150}));
    let items: Vec<Value> = rect.iter().map(|id| json!({"type": "sketch_curve", "id": id})).collect();
    h.app.run("select.set", json!({ "items": items })).unwrap();
    h.frames(4);
    (h, rect)
}

/// Send the window system's clipboard event (what Ctrl+C / Ctrl+V become), with the pointer at
/// `at` (sketch coordinates) or off the viewport.
fn clipboard_event(h: &mut Harness, event: egui::Event, at: Option<[f64; 2]>) {
    match at {
        Some(p) => h.step(&json!({"move": {"sketch": p}})).unwrap(),
        None => drop(h.call("ui.move", json!({"x": 100, "y": 990}))),
    }
    h.app.synthetic.push(event);
    h.settle();
}

fn curves(h: &Harness) -> Vec<String> {
    h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap().curves.iter().map(|c| c.id.clone()).collect()
}

fn start(h: &Harness, id: &str) -> Vec2 {
    let sk = h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    let c = &sk.curves[sk.curve_index(id).unwrap()];
    sk.point(c.kind.point_ids()[0]).unwrap()
}

#[test]
fn ctrl_c_ctrl_v_pastes_the_sketch_selection_at_the_cursor_selected_in_one_undo_step() {
    let (mut h, rect) = rectangle();
    let before = curves(&h);
    let undo = h.app.session.undo.len();
    clipboard_event(&mut h, egui::Event::Copy, Some([10.0, 5.0]));
    assert!(h.app.session.sketch_clipboard.is_some(), "{:?}", h.app.status);
    // The window system only sends Paste while its clipboard holds text: a copy leaves some.
    h.call("ui.inspect", json!({}));
    clipboard_event(&mut h, egui::Event::Paste("x".into()), Some([60.0, 40.0]));
    let after = curves(&h);
    assert_eq!(after.len(), before.len() + 4, "{:?}", h.app.status);
    let pasted: Vec<String> = after.iter().filter(|c| !before.contains(c)).cloned().collect();
    // Centred at the cursor: the rectangle (centre 10, 5) goes to (60, 40), give or take the snap.
    let corner = start(&h, &pasted[0]);
    assert!(corner.dist(Vec2::new(50.0, 35.0)) < 1.0, "{corner:?}");
    // The originals stay put, with their constraints.
    assert!(start(&h, &rect[0]).dist(Vec2::ZERO) < 1e-9);
    // Selected, ready to move.
    let selected: Vec<String> =
        h.app.session.selection.iter().filter_map(|s| if let Sel::SketchCurve { id } = s { Some(id.clone()) } else { None }).collect();
    assert_eq!(selected, pasted);
    // One undo step.
    assert_eq!(h.app.session.undo.len(), undo + 1);
    h.step(&json!({"key": "Z", "cmd": true})).unwrap();
    assert_eq!(curves(&h), before);
}

#[test]
fn copy_leaves_text_on_the_system_clipboard_so_paste_can_follow() {
    let (mut h, _) = rectangle();
    h.step(&json!({"move": {"sketch": [10, 5]}})).unwrap();
    h.app.synthetic.push(egui::Event::Copy);
    let mut copied = Vec::new();
    for _ in 0..6 {
        h.frame();
        copied.extend(h.output.commands.iter().filter_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        }));
    }
    assert!(copied.iter().any(|t| t.starts_with("SolidCraft sketch geometry")), "{copied:?}");
}

#[test]
fn paste_away_from_the_viewport_offsets_from_the_original_and_cmd_keys_work_too() {
    let (mut h, rect) = rectangle();
    let before = curves(&h);
    // Ctrl+C / Ctrl+V as key presses (as the control channel and some platforms send them).
    h.step(&json!({"move": {"sketch": [10, 5]}})).unwrap();
    h.step(&json!({"key": "C", "cmd": true})).unwrap();
    h.call("ui.move", json!({"x": 100, "y": 990}));
    h.step(&json!({"key": "V", "cmd": true})).unwrap();
    let pasted: Vec<String> = curves(&h).into_iter().filter(|c| !before.contains(c)).collect();
    assert_eq!(pasted.len(), 4, "{:?}", h.app.status);
    // 20 mm wide: a 2 mm step down and to the right.
    let d = start(&h, &pasted[0]) - start(&h, &rect[0]);
    assert!(d.dist(Vec2::new(2.0, -2.0)) < 1e-6, "{d:?}");
}

#[test]
fn the_context_menu_copies_and_pastes_sketch_geometry_at_the_click() {
    let (mut h, rect) = rectangle();
    let before = curves(&h);
    h.step(&json!({"click": {"sketch": [10, 0]}, "button": "right"})).unwrap();
    let menu = h.call("ui.menu", json!({}));
    let labels: Vec<&str> = menu["result"]["items"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.contains(&"Copy") && labels.contains(&"Paste"), "{labels:?}");
    h.call("ui.menuPick", json!({"item": "Copy"}));
    assert!(h.app.session.sketch_clipboard.is_some());
    h.step(&json!({"click": {"sketch": [70, 30]}, "button": "right"})).unwrap();
    h.call("ui.menuPick", json!({"item": "Paste"}));
    h.frames(3);
    let pasted: Vec<String> = curves(&h).into_iter().filter(|c| !before.contains(c)).collect();
    assert!(!pasted.is_empty(), "{:?}", h.app.status);
    // Whatever the right-click selected was copied, and its copy is centred on the click.
    let sk = h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    let pts: Vec<Vec2> = pasted.iter().flat_map(|c| sk.curves[sk.curve_index(c).unwrap()].kind.point_ids()).map(|i| sk.point(i).unwrap()).collect();
    let (lo, hi) = pts.iter().fold((Vec2::new(1e9, 1e9), Vec2::new(-1e9, -1e9)), |(lo, hi), p| {
        (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y)))
    });
    assert!(((lo + hi) * 0.5).dist(Vec2::new(70.0, 30.0)) < 1.0, "{lo:?} {hi:?}");
    assert!(start(&h, &rect[0]).dist(Vec2::ZERO) < 1e-9);
}

#[test]
fn ctrl_c_ctrl_v_outside_a_sketch_copies_and_pastes_timeline_features() {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
    h.frames(3);
    let id = h.app.session.doc.features[0].id;
    h.app.run("select.set", json!({"items": [{"type": "feature", "id": id}]})).unwrap();
    h.frames(2);
    clipboard_event(&mut h, egui::Event::Copy, None);
    clipboard_event(&mut h, egui::Event::Paste("x".into()), None);
    assert_eq!(h.app.session.doc.features.len(), 2, "{:?}", h.app.status);
}

#[test]
fn ctrl_x_cuts_the_sketch_selection_and_ctrl_v_brings_it_back() {
    let (mut h, rect) = rectangle();
    let before = curves(&h);
    clipboard_event(&mut h, egui::Event::Cut, Some([10.0, 5.0]));
    let left = curves(&h);
    assert!(rect.iter().all(|c| !left.contains(c)), "{:?}", h.app.status);
    assert!(h.app.session.sketch_clipboard.is_some());
    clipboard_event(&mut h, egui::Event::Paste("x".into()), Some([10.0, 5.0]));
    assert_eq!(curves(&h).len(), before.len(), "{:?}", h.app.status);
    // The context menu offers Cut too.
    h.step(&json!({"click": {"sketch": [10, 0]}, "button": "right"})).unwrap();
    let menu = h.call("ui.menu", json!({}));
    let labels: Vec<&str> = menu["result"]["items"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.contains(&"Cut"), "{labels:?}");
}
