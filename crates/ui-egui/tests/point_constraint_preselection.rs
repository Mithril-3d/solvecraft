use serde_json::json;
use solvecraft_engine::{Sel, sketch::ConstraintKind};
use solvecraft_ui_egui::scenario::Harness;

fn setup() -> Harness {
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.line", json!({"points":[[10,10],[30,10]],"infer":false,"connect":false})).unwrap();
    h.app.run("sketch.point", json!({"point":[20,20],"id":"a"})).unwrap();
    h.app.run("sketch.point", json!({"point":[25,30],"id":"b"})).unwrap();
    h.call("ui.view", json!({"view":"top"}));
    h
}

#[test]
fn clicked_point_constraint_preselection_applies_and_undoes_once() {
    for (command, picks) in [
        ("coincident", vec![[20, 20], [25, 30]]),
        ("coincident", vec![[20, 20], [20, 10]]),
        ("coincident", vec![[20, 10], [20, 20]]),
        ("midpoint", vec![[20, 20], [20, 10]]),
        ("midpoint", vec![[20, 10], [20, 20]]),
        ("symmetry", vec![[20, 10], [20, 20], [25, 30]]),
        ("symmetry", vec![[20, 20], [20, 10], [25, 30]]),
    ] {
        let mut h = setup();
        for (index, pick) in picks.iter().enumerate() {
            h.step(&json!({"click":{"sketch":pick},"shift":index>0})).unwrap();
        }
        assert_eq!(h.app.session.selection.len(), picks.len(), "{command}");
        let before = h.app.session.doc.to_json();
        let undo = h.app.session.undo.len();
        h.step(&json!({"start":format!("sketch.constraint.{command}")})).unwrap();
        assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error), "{command}: {:?}", h.app.status);
        assert!(h.app.tool.is_none());
        assert!(h.app.session.selection.is_empty());
        assert_eq!(h.app.session.undo.len(), undo + 1);
        let state = h.app.session.world_state();
        let sk = &state.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        assert!(sk.constraints.iter().any(|c| match command {
            "coincident" => matches!(c.kind, ConstraintKind::Coincident { .. } | ConstraintKind::PointOnCurve { .. }),
            "midpoint" => matches!(c.kind, ConstraintKind::Midpoint { .. }),
            _ => matches!(c.kind, ConstraintKind::Symmetric { .. }),
        }));
        let point = |id| sk.point(sk.resolve_point(id).unwrap()).unwrap();
        let a = point("a");
        let b = point("b");
        let start = point("l1.start");
        let end = point("l1.end");
        let axis = (end - start).normalized().unwrap();
        match command {
            "midpoint" => assert!(a.dist((start + end) * 0.5) < 1e-6),
            "symmetry" => {
                assert!(((a + b) * 0.5 - start).cross(axis).abs() < 1e-6);
                assert!((b - a).dot(axis).abs() < 1e-6);
            }
            _ if picks.contains(&[20, 10]) => assert!((a - start).cross(axis).abs() < 1e-6),
            _ => assert!(a.dist(b) < 1e-6),
        }
        let after = h.app.session.doc.to_json();
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), before);
        h.app.run("edit.redo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), after);
    }
}

#[test]
fn rejected_and_ambiguous_preselection_preserve_document_and_selection() {
    let mut h = setup();
    h.app.run("sketch.constraint.fix", json!({"entities":["a","b"]})).unwrap();
    h.app.session.selection = vec![Sel::SketchPoint { id: "a".into() }, Sel::SketchPoint { id: "b".into() }];
    let selection = h.app.session.selection.clone();
    let before = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    h.step(&json!({"start":"sketch.constraint.coincident"})).unwrap();
    assert!(h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    assert_eq!(h.app.session.selection, selection);
    assert_eq!(h.app.session.doc.to_json(), before);
    assert_eq!(h.app.session.undo.len(), undo);
    for command in ["coincident", "midpoint", "symmetry"] {
        h.app.session.selection = vec![Sel::SketchPoint { id: "a".into() }];
        h.step(&json!({"start":format!("sketch.constraint.{command}")})).unwrap();
        assert!(h.app.tool.as_ref().unwrap().picks.is_empty());
        assert_eq!(h.app.session.selection.len(), 1);
        assert_eq!(h.app.session.doc.to_json(), before);
    }
}
