//! Horizontal/Vertical accepts either one line or two distinct sketch points.

use serde_json::json;
use solvecraft_engine::sketch::ConstraintKind;
use solvecraft_ui_egui::scenario::Harness;

#[test]
fn horizontal_vertical_uses_preselected_line_and_preserves_rejection() {
    for fixed in [false, true] {
        let mut h = Harness::new();
        h.run(&json!([
            {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
            {"call":"engine.execute","params":{"command":"sketch.line","params":{"points":[[10,10],[40,13]],"infer":false}}},
            {"call":"ui.view","params":{"view":"top"}}
        ]))
        .unwrap();
        if fixed {
            h.app.session.execute("sketch.constraint.fix", &json!({"entity":"l1"})).unwrap();
        }
        h.run(&json!([{"click":{"sketch":[25,11.5]}}])).unwrap();
        assert_eq!(h.app.session.selection.len(), 1);
        let selection = h.app.session.selection.clone();
        let before = serde_json::to_value(&h.app.session.doc).unwrap();
        let undo = h.app.session.undo.len();
        h.run(&json!([{"start":"sketch.constraint.horizontal_vertical"}])).unwrap();
        assert_eq!(h.app.status.as_ref().is_some_and(|(_, _, error)| *error), fixed);
        if fixed {
            assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), before);
            assert_eq!(h.app.session.selection, selection);
            assert_eq!(h.app.session.undo.len(), undo);
        } else {
            let sk = &h.app.session.model.state().sketches[0].sketch;
            let a = sk.point(sk.resolve_point("l1.start").unwrap()).unwrap();
            let b = sk.point(sk.resolve_point("l1.end").unwrap()).unwrap();
            assert!((a.y - b.y).abs() < 1e-6);
            assert_eq!(h.app.session.undo.len(), undo + 1);
            assert!(h.app.session.selection.is_empty());
        }
        assert!(h.app.tool.is_none());
    }
}

#[test]
fn horizontal_vertical_leaves_ambiguous_preselection_to_pick_tool() {
    use solvecraft_engine::Sel;
    for selection in [
        vec![Sel::SketchPoint { id: "origin".into() }],
        vec![Sel::SketchPoint { id: "origin".into() }, Sel::SketchCurve { id: "c1".into() }],
        vec![Sel::SketchPoint { id: "origin".into() }, Sel::SketchPoint { id: "c1.center".into() }, Sel::SketchCurve { id: "c1".into() }],
    ] {
        let mut h = Harness::new();
        h.app.session.execute("sketch.create", &json!({"plane":"XY"})).unwrap();
        h.app.session.execute("sketch.circle.center", &json!({"center":[20,3],"radius":5})).unwrap();
        h.app.session.selection = selection.clone();
        let undo = h.app.session.undo.len();
        h.run(&json!([{"start":"sketch.constraint.horizontal_vertical"}])).unwrap();
        assert!(h.app.tool.as_ref().unwrap().picks.is_empty());
        assert_eq!(h.app.session.undo.len(), undo);
        assert_eq!(h.app.session.selection, selection);
        assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    }
}

#[test]
fn horizontal_vertical_uses_preselected_points() {
    for center in [[20.0, 3.0], [3.0, 20.0]] {
        for reverse in [false, true] {
            let mut h = Harness::new();
            let (first, second) = if reverse { (center, [0.0, 0.0]) } else { ([0.0, 0.0], center) };
            h.run(&json!([
                {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
                {"call":"engine.execute","params":{"command":"sketch.circle.center","params":{"center":center,"radius":5}}},
                {"call":"ui.view","params":{"view":"top"}},
                {"click":{"sketch":first}},
                {"click":{"sketch":second},"shift":true}
            ]))
            .unwrap();
            assert_eq!(h.app.session.selection.len(), 2);
            let before = serde_json::to_value(&h.app.session.doc).unwrap();
            let undo = h.app.session.undo.len();
            h.run(&json!([{"start":"sketch.constraint.horizontal_vertical"}])).unwrap();
            assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
            let sk = &h.app.session.model.state().sketches[0].sketch;
            let p = sk.point(sk.resolve_point("c1.center").unwrap()).unwrap();
            assert!((if center[0] > center[1] { p.y } else { p.x }).abs() < 1e-6);
            assert!(h.app.tool.is_none());
            assert!(h.app.session.selection.is_empty());
            assert_eq!(h.app.session.undo.len(), undo + 1);
            let after = serde_json::to_value(&h.app.session.doc).unwrap();
            h.app.session.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), before);
            h.app.session.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(serde_json::to_value(&h.app.session.doc).unwrap(), after);
        }
    }
}

#[test]
fn horizontal_vertical_aligns_origin_and_circle_center() {
    for (center, horizontal) in [([20.0, 3.0], true), ([3.0, 20.0], false)] {
        for origin_first in [true, false] {
            let mut h = Harness::new();
            h.run(&json!([
                {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
                {"call":"engine.execute","params":{"command":"sketch.circle.center","params":{"center":center,"radius":5}}},
                {"call":"ui.view","params":{"view":"top"}},
                {"start":"sketch.constraint.horizontal_vertical"}
            ]))
            .unwrap();
            let (first, second) = if origin_first { ([0.0, 0.0], center) } else { (center, [0.0, 0.0]) };
            let undo = h.app.session.undo.len();
            h.run(&json!([{"click":{"sketch":first}}, {"expect":{"errors":0}}])).unwrap();
            assert_eq!(h.app.tool.as_ref().unwrap().picks.len(), 1);
            assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
            assert_eq!(h.app.session.undo.len(), undo);
            // Repeating the first point must not complete a self-constraint.
            h.run(&json!([{"click":{"sketch":first}}, {"click":{"sketch":second}}, {"expect":{"errors":0}}])).unwrap();
            let aligned = |h: &Harness| {
                let sk = &h.app.session.model.state().sketches[0].sketch;
                let p = sk.point(sk.resolve_point("c1.center").unwrap()).unwrap();
                if horizontal { p.y } else { p.x }
            };
            assert!(aligned(&h).abs() < 1e-6);
            let sk = &h.app.session.model.state().sketches[0].sketch;
            assert!(sk.constraints.iter().any(|c| if horizontal {
                matches!(c.kind, ConstraintKind::HorizontalPoints { .. })
            } else {
                matches!(c.kind, ConstraintKind::VerticalPoints { .. })
            }));
            assert_eq!(h.app.session.undo.len(), undo + 1);
            assert!(h.app.tool.as_ref().unwrap().picks.is_empty());
            h.run(&json!([{"call":"engine.execute","params":{"command":"edit.undo","params":{}}}])).unwrap();
            assert!((aligned(&h) - 3.0).abs() < 1e-6);
            h.run(&json!([{"call":"engine.execute","params":{"command":"edit.redo","params":{}}}])).unwrap();
            assert!(aligned(&h).abs() < 1e-6);
        }
    }
}

#[test]
fn horizontal_vertical_still_accepts_one_line() {
    let mut h = Harness::new();
    h.run(&json!([
        {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
        {"call":"engine.execute","params":{"command":"sketch.line","params":{"points":[[10,10],[40,13]],"infer":false}}},
        {"call":"ui.view","params":{"view":"top"}},
        {"start":"sketch.constraint.horizontal_vertical"},
        {"click":{"sketch":[25,11.5]}},
        {"expect":{"errors":0}}
    ]))
    .unwrap();
    let sk = &h.app.session.model.state().sketches[0].sketch;
    assert!(sk.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Horizontal { .. })));
    let a = sk.point(sk.resolve_point("l1.start").unwrap()).unwrap();
    let b = sk.point(sk.resolve_point("l1.end").unwrap()).unwrap();
    assert!((a.y - b.y).abs() < 1e-6);
}

#[test]
fn horizontal_vertical_ignores_curves_and_cancels_a_pending_point() {
    let mut h = Harness::new();
    h.run(&json!([
        {"call":"engine.execute","params":{"command":"sketch.create","params":{"plane":"XY"}}},
        {"call":"engine.execute","params":{"command":"sketch.circle.center","params":{"center":[20,3],"radius":5}}},
        {"call":"ui.view","params":{"view":"top"}},
        {"start":"sketch.constraint.horizontal_vertical"},
        {"click":{"sketch":[25,3]}}
    ]))
    .unwrap();
    assert!(h.app.tool.as_ref().unwrap().picks.is_empty());
    assert!(!h.app.status.as_ref().is_some_and(|(_, _, error)| *error));
    let undo = h.app.session.undo.len();
    h.run(&json!([{"click":{"sketch":[0,0]}}, {"click":{"sketch":[25,3]}}])).unwrap();
    assert_eq!(h.app.tool.as_ref().unwrap().picks.len(), 1);
    h.run(&json!([{"key":"Escape"}])).unwrap();
    assert!(h.app.tool.is_none());
    assert_eq!(h.app.session.undo.len(), undo);
    let sk = &h.app.session.model.state().sketches[0].sketch;
    let p = sk.point(sk.resolve_point("c1.center").unwrap()).unwrap();
    assert!((p.y - 3.0).abs() < 1e-6);
}
