//! Enter submits only the focused Parameters editor row.
use egui::{Event, Key, Modifiers};
use serde_json::json;
use solidcraft_ui_egui::scenario::Harness;

fn open() -> Harness {
    let mut h = Harness::new();
    h.step(&json!({"widget":"Parameters"})).unwrap();
    assert!(h.app.params.is_some());
    h
}
fn fill(h: &mut Harness, field: &str, text: &str) {
    h.run(&json!([
        {"click":{"handle":format!("param:new:{field}")}},
        {"key":"A","cmd":true}, {"text":text}
    ]))
    .unwrap();
}
fn enter(h: &mut Harness, repeat: bool) {
    h.app.synthetic.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat, modifiers: Modifiers::NONE });
    h.frame();
    h.app.synthetic.push(Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.frames(3);
}
#[test]
fn parameter_enter_adds_once_and_focuses_next_name() {
    let mut h = open();
    fill(&mut h, "name", "width");
    fill(&mut h, "expression", "25 mm");
    let undo = h.app.session.undo.len();
    enter(&mut h, false);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert!(h.app.params.as_ref().unwrap().new_name.is_empty());
    assert!(h.app.params.as_ref().unwrap().error.is_none());
    let saved = h.app.session.doc.to_json();
    enter(&mut h, true);
    assert_eq!(h.app.session.doc.to_json(), saved);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    // Name has focus after successful creation; no mouse needed for the next name.
    h.run(&json!([{"text":"height"}])).unwrap();
    assert_eq!(h.app.params.as_ref().unwrap().new_name, "height");
    enter(&mut h, false);
    assert_eq!(h.app.session.undo.len(), undo + 2);
    let result = h.app.session.execute("parameters.list", &json!({})).unwrap();
    assert!(result["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "height"));
}
#[test]
fn parameter_enter_invalid_preserves_draft_and_other_rows() {
    let mut h = open();
    h.app.session.execute("parameters.add", &json!({"name":"existing","expression":"10 mm"})).unwrap();
    h.frames(3);
    fill(&mut h, "name", "bad");
    fill(&mut h, "expression", "missing + 1 mm");
    let saved = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    enter(&mut h, false);
    assert_eq!(h.app.session.doc.to_json(), saved);
    assert_eq!(h.app.session.undo.len(), undo);
    let d = h.app.params.as_ref().unwrap();
    assert_eq!(d.new_name, "bad");
    assert_eq!(d.new_expr, "missing + 1 mm");
    assert!(d.error.is_some());
    fill(&mut h, "expression", "15 mm");
    fill(&mut h, "name", "existing");
    enter(&mut h, false);
    assert_eq!(h.app.session.doc.to_json(), saved);
    assert!(h.app.params.as_ref().unwrap().error.is_some());
}
#[test]
fn parameter_enter_edits_expression_without_adding_draft() {
    let mut h = open();
    h.app.session.execute("parameters.add", &json!({"name":"width","expression":"10 mm"})).unwrap();
    h.frames(3);
    fill(&mut h, "name", "unsubmitted");
    let undo = h.app.session.undo.len();
    h.run(&json!([{"click":{"handle":"param:width"}}, {"key":"A","cmd":true}, {"text":"20 mm"}])).unwrap();
    enter(&mut h, false);
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert_eq!(h.app.params.as_ref().unwrap().new_name, "unsubmitted");
    let result = h.app.session.execute("parameters.list", &json!({})).unwrap();
    let ps = result["parameters"].as_array().unwrap();
    assert!(ps.iter().any(|p| p["name"] == "width" && p["expression"] == "20 mm"));
    assert!(!ps.iter().any(|p| p["name"] == "unsubmitted"));
    let saved = h.app.session.doc.to_json();
    h.run(&json!([{"click":{"handle":"param:width"}}, {"key":"A","cmd":true}, {"text":"missing + 2 mm"}])).unwrap();
    enter(&mut h, false);
    assert_eq!(h.app.session.doc.to_json(), saved);
    assert!(h.app.params.as_ref().unwrap().error.is_some());
    assert_eq!(h.app.params.as_ref().unwrap().editing.get("width").map(String::as_str), Some("missing + 2 mm"));
}

#[test]
fn parameter_enter_comment_uses_selected_unit_and_unrelated_focus_does_not_submit() {
    let mut h = open();
    fill(&mut h, "name", "length");
    h.run(&json!([{"click":{"handle":"param:new:unit"}}, {"widget":"cm"}])).unwrap();
    fill(&mut h, "expression", "2 cm");
    fill(&mut h, "comment", "Overall length");
    let undo = h.app.session.undo.len();
    h.run(&json!([{"click":{"handle":"params:title"}}])).unwrap();
    enter(&mut h, false);
    assert_eq!(h.app.session.undo.len(), undo);
    h.run(&json!([{"click":{"handle":"param:new:comment"}}])).unwrap();
    enter(&mut h, false);
    let result = h.app.session.execute("parameters.list", &json!({})).unwrap();
    assert!(result["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "length" && p["unit"] == "cm" && p["comment"] == "Overall length"));
    assert_eq!(h.app.session.undo.len(), undo + 1);
}
#[test]
fn parameter_rename_arrow_keys_move_cursor_and_keep_editing() {
    let mut h = open();
    h.app.session.execute("parameters.add", &json!({"name":"width","expression":"10 mm"})).unwrap();
    // Let the click that opened the dialog age, or egui counts the double-click as a triple.
    h.frames(30);
    h.run(&json!([{"click":{"handle":"param:name:width"},"double":true}])).unwrap();
    h.frames(2);
    let renaming = |h: &Harness| h.app.params.as_ref().unwrap().renaming.clone();
    assert_eq!(renaming(&h), Some(("width".to_string(), "width".to_string())));
    // Move the cursor to the start and type there; every arrow key keeps the rename open.
    h.run(&json!([{"key":"ArrowLeft"}, {"key":"ArrowUp"}, {"key":"ArrowDown"}, {"key":"ArrowRight"}, {"key":"Home"}, {"text":"plate_"}])).unwrap();
    assert_eq!(renaming(&h), Some(("width".to_string(), "plate_width".to_string())));
    enter(&mut h, false);
    assert_eq!(renaming(&h), None);
    let result = h.app.session.execute("parameters.list", &json!({})).unwrap();
    assert!(result["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "plate_width"));
}
#[test]
fn parameter_add_resets_expression_for_next_row() {
    let mut h = open();
    let initial = h.app.params.as_ref().unwrap().new_expr.clone();
    fill(&mut h, "name", "width");
    fill(&mut h, "expression", "25 mm");
    enter(&mut h, false);
    let result = h.app.session.execute("parameters.list", &json!({})).unwrap();
    assert!(result["parameters"].as_array().unwrap().iter().any(|p| p["name"] == "width" && p["expression"] == "25 mm"));
    let d = h.app.params.as_ref().unwrap();
    assert!(d.new_name.is_empty());
    assert_eq!(d.new_expr, initial);
}
