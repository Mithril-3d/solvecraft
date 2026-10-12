//! Constraint tools accept a selected curve pair as well as tool-first clicks.

use serde_json::json;
use solvecraft_engine::{Sel, sketch::ConstraintKind};
use solvecraft_ui_egui::scenario::Harness;

fn lines() -> Harness {
    let mut h = Harness::new();
    h.run(&json!([
        {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
        {"call":"engine.execute","params":{"command":"sketch.line","params":{"points":[[10,10],[40,10]],"infer":false}}},
        {"call":"engine.execute","params":{"command":"sketch.line","params":{"points":[[10,30],[40,35]],"infer":false}}},
        {"call":"ui.view","params":{"view":"top"}}
    ]))
    .unwrap();
    h
}

fn select_lines(h: &mut Harness) {
    h.run(&json!([
        {"click":{"sketch":[25,10]}},
        {"click":{"sketch":[25,32.5]},"shift":true}
    ]))
    .unwrap();
    assert_eq!(h.app.session.selection, vec![Sel::SketchCurve { id: "l1".into() }, Sel::SketchCurve { id: "l2".into() }]);
}

fn assert_line_geometry(h: &Harness, command: &str) {
    let sk = &h.app.session.model.state().sketches[0].sketch;
    let point = |name| sk.point(sk.resolve_point(name).unwrap()).unwrap();
    let a = point("l1.start");
    let b = point("l2.start");
    let u = point("l1.end") - a;
    let v = point("l2.end") - b;
    // Normalization also rejects a collapsed line; zero-length vectors must not
    // accidentally satisfy the parallel/perpendicular checks.
    let du = u.normalized().unwrap();
    let dv = v.normalized().unwrap();
    match command {
        "parallel" => assert!(du.cross(dv).abs() < 1e-6),
        "perpendicular" => assert!(du.dot(dv).abs() < 1e-6),
        "equal" => assert!((u.len() - v.len()).abs() < 1e-6),
        "collinear" => {
            assert!(du.cross(dv).abs() < 1e-6);
            assert!((b - a).cross(du).abs() < 1e-6);
        }
        _ => panic!("unexpected line constraint: {command}"),
    }
}

#[test]
fn selected_lines_apply_constraints_with_one_undo_step() {
    for command in ["parallel", "perpendicular", "equal", "collinear"] {
        let mut h = lines();
        select_lines(&mut h);
        let before = h.app.session.doc.clone();
        let undo = h.app.session.undo.len();
        h.run(&json!([{"start":format!("sketch.constraint.{command}")}])).unwrap();
        assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error), "{command}");
        let sk = &h.app.session.model.state().sketches[0].sketch;
        assert!(
            sk.constraints.iter().any(|c| match command {
                "parallel" => matches!(c.kind, ConstraintKind::Parallel { .. }),
                "perpendicular" => matches!(c.kind, ConstraintKind::Perpendicular { .. }),
                "equal" => matches!(c.kind, ConstraintKind::Equal { .. }),
                _ => matches!(c.kind, ConstraintKind::Collinear { .. }),
            }),
            "{command}"
        );
        assert_line_geometry(&h, command);
        let after = h.app.session.doc.clone();
        assert_eq!(h.app.session.undo.len(), undo + 1);
        assert!(h.app.session.selection.is_empty());
        assert!(h.app.tool.is_none());
        h.app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), serde_json::to_value(before).unwrap());
        h.app.session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), serde_json::to_value(after).unwrap());
        assert_line_geometry(&h, command);
    }
}

#[test]
fn selected_circles_apply_constraints() {
    for command in ["tangent", "concentric", "equal"] {
        let mut h = Harness::new();
        h.run(&json!([
            {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
            {"call":"engine.execute","params":{"command":"sketch.circle.center","params":{"center":[10,10],"radius":5}}},
            {"call":"engine.execute","params":{"command":"sketch.circle.center","params":{"center":[25,12],"radius":7}}},
            {"call":"ui.view","params":{"view":"top"}},
            {"click":{"sketch":[10,15]}},
            {"click":{"sketch":[25,19]},"shift":true}
        ]))
        .unwrap();
        assert_eq!(h.app.session.selection.len(), 2);
        let undo = h.app.session.undo.len();
        h.run(&json!([{"start":format!("sketch.constraint.{command}")}])).unwrap();
        assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error), "{command}");
        assert_eq!(h.app.session.undo.len(), undo + 1, "{command}");
        let sk = &h.app.session.model.state().sketches[0].sketch;
        assert!(sk.constraints.iter().any(|c| match command {
            "tangent" => matches!(c.kind, ConstraintKind::Tangent { .. }),
            "concentric" => matches!(c.kind, ConstraintKind::Concentric { .. }),
            _ => matches!(c.kind, ConstraintKind::Equal { .. }),
        }));
        assert!(h.app.session.selection.is_empty());
    }
}

#[test]
fn ambiguous_selection_preserves_tool_first_workflow() {
    for selection in [
        vec![],
        vec![Sel::SketchCurve { id: "l1".into() }],
        vec![Sel::SketchCurve { id: "l1".into() }, Sel::SketchCurve { id: "l2".into() }, Sel::SketchPoint { id: "origin".into() }],
    ] {
        let mut h = lines();
        h.app.session.selection = selection;
        let undo = h.app.session.undo.len();
        h.run(&json!([{"start":"sketch.constraint.parallel"}])).unwrap();
        assert_eq!(h.app.session.undo.len(), undo);
        assert!(h.app.tool.as_ref().unwrap().picks.is_empty());
        h.run(&json!([{"click":{"sketch":[25,10]}}, {"click":{"sketch":[25,32.5]}}])).unwrap();
        assert_eq!(h.app.session.undo.len(), undo + 1);
        assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    }
}

#[test]
fn rejected_preselection_preserves_document_and_error() {
    let mut h = lines();
    h.app.session.execute("sketch.constraint.fix", &json!({"entities":["l1","l2"]})).unwrap();
    select_lines(&mut h);
    let before = serde_json::to_value(&h.app.session.doc).unwrap();
    let undo = h.app.session.undo.len();
    let selection = h.app.session.selection.clone();
    h.run(&json!([{"start":"sketch.constraint.parallel"}])).unwrap();
    assert!(h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), before);
    assert_eq!(h.app.session.undo.len(), undo);
    assert_eq!(h.app.session.selection, selection);
    // Correct the conflict and retry without reselecting; the old error must disappear.
    h.app.session.execute("sketch.constraint.fix", &json!({"entities":["l1","l2"]})).unwrap();
    h.run(&json!([{"start":"sketch.constraint.parallel"}])).unwrap();
    assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    assert!(h.app.session.selection.is_empty());
}
