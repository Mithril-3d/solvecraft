//! Hovering a dialog's Select chip doesn't change its size or the panel's (#1335).
use serde_json::json;
use solidcraft_ui_egui::scenario::{Harness, handle_at};

#[test]
fn hovering_the_select_chip_keeps_its_size() {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.app.start("sketch.create");
    h.frames(3);
    let away = handle_at("dialog:Plane").unwrap();
    h.step(&json!({"move":[away.x, away.y]})).unwrap();
    h.frames(3);
    let hovered = handle_at("dialog:Plane").unwrap();
    assert!(away.distance(hovered) < 0.01, "the chip moved on hover: {away:?} -> {hovered:?}");
    h.step(&json!({"move":[5.0, 600.0]})).unwrap();
    h.frames(3);
    assert!(away.distance(handle_at("dialog:Plane").unwrap()) < 0.01);
}
