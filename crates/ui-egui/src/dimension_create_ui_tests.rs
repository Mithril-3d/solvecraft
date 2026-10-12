//! Dimension creation is an editable draft, accepted by the real focused value field.
use crate::scenario::Harness;
use serde_json::json;

fn line(end: [f64; 2]) -> Harness {
    crate::dim_view::cancel();
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.line", json!({"points":[[0,0],end]})).unwrap();
    h.call("ui.view", json!({"view":"top"}));
    // Leave room around the measured shape while testing the draft itself. Separate Fit
    // tests exercise measured dock occlusion without changing the picking projection.
    h.app.cam.distance = 80.0;
    h.app.cam.target = solvecraft_engine::geom::Vec3::new(end[0] * 0.5, end[1] * 0.5, 0.0);
    h.settle();
    h
}
fn place(h: &mut Harness, picks: &[[f64; 2]], at: [f64; 2]) {
    h.step(&json!({"start":"sketch.dimension"})).unwrap();
    for p in picks {
        h.step(&json!({"click":{"world":[p[0],p[1],0]}})).unwrap();
    }
    h.step(&json!({"click":{"world":[at[0],at[1],0]}})).unwrap();
    assert!(crate::dim_view::editing(), "placement opens an editor");
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(egui::Id::new("sc_dim_edit")));
}
fn type_value(h: &mut Harness, text: &str) {
    h.step(&json!({"key":"A","cmd":true})).unwrap();
    h.step(&json!({"text":text})).unwrap();
}
#[test]
fn edge_and_endpoint_creation_commit_once_and_undo_restores_exact_design() {
    for (end, picks, at) in [
        ([20., 0.], vec![[10., 0.]], [10., 5.]),
        ([20., 0.], vec![[0., 0.], [20., 0.]], [10., 5.]),
        ([0., 20.], vec![[0., 0.], [0., 20.]], [-5., 10.]),
        ([12., 16.], vec![[0., 0.], [12., 16.]], [-5., 12.]),
    ] {
        let mut h = line(end);
        let original = h.app.session.doc.to_json();
        let undo = h.app.session.undo.len();
        place(&mut h, &picks, at);
        assert_eq!(h.app.session.doc.to_json(), original);
        assert_eq!(h.app.session.undo.len(), undo);
        type_value(&mut h, "30 mm");
        h.step(&json!({"key":"Enter"})).unwrap();
        assert!(!crate::dim_view::editing());
        assert_eq!(h.app.session.undo.len(), undo + 1);
        let state = h.app.session.world_state();
        let sk = &state.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        assert!(
            (sk.point(sk.resolve_point("l1.start").unwrap()).unwrap().dist(sk.point(sk.resolve_point("l1.end").unwrap()).unwrap()) - 30.).abs()
                < 1e-6
        );
        assert!(sk.constraints.iter().any(|c| c.param.is_some()));
        let accepted = h.app.session.doc.to_json();
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), original);
        h.app.run("edit.redo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), accepted);
    }
}
#[test]
fn escape_invalid_correction_and_seed_enter_do_not_create_partial_dimensions() {
    let mut h = line([20., 0.]);
    let original = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    place(&mut h, &[[10., 0.]], [10., 5.]);
    h.step(&json!({"key":"Escape"})).unwrap();
    assert!(!crate::dim_view::editing());
    assert_eq!(h.app.session.doc.to_json(), original);
    assert_eq!(h.app.session.undo.len(), undo);
    place(&mut h, &[[0., 0.], [20., 0.]], [10., 5.]);
    type_value(&mut h, "unknown_length");
    h.step(&json!({"key":"Enter"})).unwrap();
    assert!(crate::dim_view::editing());
    assert_eq!(h.app.session.doc.to_json(), original);
    assert_eq!(h.app.session.undo.len(), undo);
    type_value(&mut h, "25 mm");
    h.step(&json!({"key":"Enter"})).unwrap();
    assert!(!crate::dim_view::editing());
    assert_eq!(h.app.session.undo.len(), undo + 1);
    h.app.run("edit.undo", json!({})).unwrap();
    h.settle();
    place(&mut h, &[[10., 0.]], [10., 5.]);
    h.step(&json!({"key":"Enter"})).unwrap();
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert!(!crate::dim_view::editing());
}
#[test]
fn switching_documents_cancels_creation_and_existing_parameter_edit_is_separate() {
    let mut h = line([20., 0.]);
    let original = h.app.session.doc.to_json();
    place(&mut h, &[[10., 0.]], [10., 5.]);
    h.call("ui.documents", json!({"action":"new"}));
    assert!(!crate::dim_view::editing());
    h.call("ui.documents", json!({"action":"switch","index":0}));
    assert_eq!(h.app.session.doc.to_json(), original);
    place(&mut h, &[[10., 0.]], [10., 5.]);
    h.step(&json!({"key":"Enter"})).unwrap();
    let undo = h.app.session.undo.len();
    crate::dim_view::edit_param(&h.app, "d1");
    h.settle();
    type_value(&mut h, "24 mm");
    h.step(&json!({"key":"Enter"})).unwrap();
    assert_eq!(h.app.session.undo.len(), undo + 1);
    h.app.run("edit.undo", json!({})).unwrap();
    assert!(h.app.session.doc.param("d1").unwrap().expr.starts_with("20"));
}

#[test]
fn rectangle_side_and_fixed_origin_line_dimensions_preserve_other_constraints() {
    for edge in [false, true] {
        let mut h = line([20., 0.]);
        h.app.run("sketch.constraint.fix", json!({"entity":"l1.start"})).unwrap();
        h.settle();
        let original = h.app.session.doc.to_json();
        let undo = h.app.session.undo.len();
        let picks = if edge { vec![[10., 0.]] } else { vec![[0., 0.], [20., 0.]] };
        place(&mut h, &picks, [10., 5.]);
        type_value(&mut h, "30 mm");
        h.step(&json!({"key":"Enter"})).unwrap();
        let state = h.app.session.world_state();
        let sk = &state.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        assert!(sk.point(sk.resolve_point("l1.start").unwrap()).unwrap().len() < 1e-8, "fixed origin remains fixed");
        assert!((sk.point(sk.resolve_point("l1.end").unwrap()).unwrap().x - 30.).abs() < 1e-6);
        assert_eq!(h.app.session.undo.len(), undo + 1);
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), original);
    }
    for (picks, at) in
        [(vec![[0., 0.], [0., 10.]], [-5., 5.]), (vec![[0., 5.]], [-5., 5.]), (vec![[20., 0.], [20., 10.]], [25., 5.]), (vec![[20., 5.]], [25., 5.])]
    {
        let mut h = Harness::new();
        crate::dim_view::cancel();
        h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
        h.app.run("sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[20,10]})).unwrap();
        h.call("ui.view", json!({"view":"top"}));
        h.app.cam.distance = 80.;
        h.app.cam.target = solvecraft_engine::geom::Vec3::new(10., 5., 0.);
        h.settle();
        let baseline = h.app.session.doc.to_json();
        let undo = h.app.session.undo.len();
        place(&mut h, &picks, at);
        type_value(&mut h, "12 mm");
        h.step(&json!({"key":"Enter"})).unwrap();
        let state = h.app.session.world_state();
        let solved = state.sketch(h.app.session.active_sketch.unwrap()).unwrap();
        assert!((solved.profiles[0].area - 240.).abs() < 1e-5, "side dimensions keep rectangle width: {}", solved.profiles[0].area);
        assert_eq!(h.app.session.undo.len(), undo + 1);
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc.to_json(), baseline);
    }
}

#[test]
fn pending_second_nested_plane_uses_world_picks_but_one_local_dimension() {
    use solvecraft_engine::geom::Vec2;
    crate::dim_view::cancel();
    let mut h = Harness::new();
    let parent = h.app.run("component.create", json!({"name":"Assembly"})).unwrap();
    let parent_component = parent["component"].as_u64().unwrap();
    let first = parent["occurrence"].as_u64().unwrap();
    let child = h.app.run("component.create", json!({"name":"Part"})).unwrap();
    let component = child["component"].as_u64().unwrap();
    let leaf = child["occurrence"].as_u64().unwrap();
    h.app.run("component.activate", json!({"component":0})).unwrap();
    let second = h.app.run("occurrence.copy", json!({"component":parent_component})).unwrap()["occurrence"].as_u64().unwrap();
    h.app.run("occurrence.move", json!({"occurrence":second,"translate":[20,30,40],"axis":[1,0,0],"angle":"35 deg","capture":true})).unwrap();
    h.app.run("component.activate", json!({"component":component,"occurrence":leaf,"path":[second,leaf]})).unwrap();
    h.app.run("browser.visibility", json!({"paths":[[first]],"visible":false})).unwrap();
    let sid = h.app.run("sketch.create", json!({"plane":"XY"})).unwrap()["sketch"].as_u64().unwrap();
    // The 2D producer uses sketch-local coordinates; pointer picks below use placed world coordinates.
    h.app.run("sketch.line", json!({"points":[[0,0],[20,0]]})).unwrap();
    h.app.run("occurrence.move", json!({"occurrence":second,"translate":[25,32,40],"axis":[1,0,0],"angle":"35 deg","capture":false})).unwrap();
    h.call("ui.view", json!({"view":"top"}));
    h.app.cam.distance = 90.;
    let plane = h.app.session.world_state().sketch(sid).unwrap().plane;
    h.app.cam.target = plane.to_world(Vec2::new(10., 0.));
    h.settle();
    let baseline = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    let pending = h.app.session.pending_moves.clone();
    h.step(&json!({"start":"sketch.dimension"})).unwrap();
    for p in [Vec2::ZERO, Vec2::new(20., 0.), Vec2::new(10., 5.)] {
        let w = plane.to_world(p);
        h.step(&json!({"click":{"world":[w.x,w.y,w.z]}})).unwrap();
    }
    assert!(crate::dim_view::editing());
    assert_eq!(h.app.session.doc.to_json(), baseline);
    type_value(&mut h, "30 mm");
    h.step(&json!({"key":"Enter"})).unwrap();
    assert_eq!(h.app.session.undo.len(), undo + 1);
    assert_eq!(h.app.session.active_occurrence_path, Some(vec![second, leaf]));
    assert_eq!(h.app.session.pending_moves, pending);
    let state = h.app.session.model.state();
    let sk = &state.sketch(sid).unwrap().sketch;
    assert!(
        (sk.point(sk.resolve_point("l1.start").unwrap()).unwrap().dist(sk.point(sk.resolve_point("l1.end").unwrap()).unwrap()) - 30.).abs() < 1e-6
    );
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc.to_json(), baseline);
    assert_eq!(h.app.session.pending_moves, pending);
    h.app.run("edit.redo", json!({})).unwrap();
    assert_eq!(h.app.session.pending_moves, pending);
    assert_eq!(h.app.session.active_occurrence_path, Some(vec![second, leaf]));
}

#[test]
fn invalid_zero_negative_failed_open_and_click_away_leave_no_dimension() {
    let mut h = line([20., 0.]);
    let baseline = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    for invalid in ["0 mm", "-2 mm"] {
        place(&mut h, &[[10., 0.]], [10., 5.]);
        type_value(&mut h, invalid);
        h.step(&json!({"key":"Enter"})).unwrap();
        assert!(crate::dim_view::editing());
        assert_eq!(h.app.session.doc.to_json(), baseline);
        assert_eq!(h.app.session.undo.len(), undo);
        h.step(&json!({"key":"Escape"})).unwrap();
    }
    place(&mut h, &[[10., 0.]], [10., 5.]);
    assert!(h.app.run("doc.open", json!({"path":"missing-dimension-draft-design.solvecraft"})).is_err());
    assert!(crate::dim_view::editing(), "failed replacement must preserve the draft");
    assert_eq!(h.app.session.doc.to_json(), baseline);
    h.frames(8);
    h.step(&json!({"click":{"world":[12,-10,0]}})).unwrap();
    assert!(!crate::dim_view::editing());
    assert_eq!(h.app.session.doc.to_json(), baseline);
    assert_eq!(h.app.session.undo.len(), undo);
}

#[test]
fn dimension_to_fillet_virtual_sharp_keeps_the_arc_and_fixed_origin() {
    crate::dim_view::cancel();
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.line", json!({"points":[[0,0],[0,20],[20,20]],"ids":["riser","top"]})).unwrap();
    h.app.run("sketch.constraint.fix", json!({"entity":"riser.start"})).unwrap();
    h.app.run("sketch.constraint.horizontal_vertical", json!({"line":"riser","mode":"vertical"})).unwrap();
    h.app.run("sketch.constraint.horizontal_vertical", json!({"line":"top","mode":"horizontal"})).unwrap();
    // An existing width requires the fillet's retained virtual sharp. Add an independent
    // height through the visible corner while preserving that width and the fillet arc.
    h.app.run("sketch.dimension", json!({"entities":["top"],"value":20})).unwrap();
    let fillet = h.app.run("sketch.fillet", json!({"point":"riser.end","radius":5})).unwrap();
    let corner = fillet["virtual_corners"][0].as_str().unwrap().to_string();
    h.call("ui.view", json!({"view":"top"}));
    h.app.cam.distance = 90.;
    h.app.cam.target = solvecraft_engine::geom::Vec3::new(5., 10., 0.);
    h.settle();
    let baseline = h.app.session.doc.to_json();
    let undo = h.app.session.undo.len();
    place(&mut h, &[[0., 0.], [0., 20.]], [-5., 10.]);
    type_value(&mut h, "30 mm");
    h.step(&json!({"key":"Enter"})).unwrap();
    assert!(!crate::dim_view::editing());
    assert_eq!(h.app.session.undo.len(), undo + 1);
    let state = h.app.session.world_state();
    let solved = state.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    assert!(solved.report.ok());
    let sk = &solved.sketch;
    assert!((sk.point(sk.point_index(&corner).unwrap()).unwrap().y - 30.).abs() < 1e-6);
    assert!(sk.point(sk.resolve_point("riser.start").unwrap()).unwrap().len() < 1e-8);
    assert!(sk.curves.iter().any(|c| matches!(c.kind, solvecraft_engine::sketch::CurveKind::Arc { .. })), "the dimension retains the fillet arc");
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc.to_json(), baseline);
}

#[test]
fn draft_annotation_does_not_move_committed_dimension_hitboxes() {
    crate::dim_view::cancel();
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[20,10]})).unwrap();
    let width = h.app.run("sketch.dimension", json!({"entities":["l3"],"value":20})).unwrap()["param"].as_str().unwrap().to_string();
    h.call("ui.view", json!({"view":"top"}));
    h.app.cam.distance = 80.;
    h.app.cam.target = solvecraft_engine::geom::Vec3::new(10., 5., 0.);
    h.settle();
    let at = crate::dim_view::text_at(&width).unwrap();
    let drawn = crate::dim_view::drawn();
    let baseline = h.app.session.doc.to_json();
    place(&mut h, &[[0., 0.], [0., 10.]], [-5., 5.]);
    type_value(&mut h, "12 mm");
    h.settle();
    assert_eq!(crate::dim_view::text_at(&width), Some(at));
    assert_eq!(crate::dim_view::drawn(), drawn, "draft annotations must not become committed hit targets");
    assert_eq!(h.app.session.doc.to_json(), baseline);
    h.step(&json!({"key":"Escape"})).unwrap();
    assert_eq!(crate::dim_view::drawn(), drawn);
}
