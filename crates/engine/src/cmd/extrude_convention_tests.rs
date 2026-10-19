#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::Session;
use serde_json::{Value, json};
use solidcraft_doc::{Document, ExtrudeConvention, FeatureKind};

fn run(s: &mut Session, cmd: &str, params: Value) {
    s.execute(cmd, &params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
}

fn fixture(settings: Value) -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane":"XY","name":"Square"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[10,10]}));
    run(&mut s, "sketch.finish", json!({}));
    let mut p = json!({"sketch":"Square","distance":"10 mm","direction":"symmetric","operation":"new","name":"Depth","body_name":"Block"});
    p.as_object_mut().unwrap().extend(settings.as_object().unwrap().clone());
    run(&mut s, "solid.extrude", p);
    s
}

fn check(s: &Session, lo: f64, hi: f64) {
    assert!(s.model.results.iter().all(|r| r.error.is_none()));
    let st = s.model.state();
    let body = st.body("Block").unwrap();
    let bounds = body.mesh().bounds();
    assert!((bounds.min.z - lo).abs() < 1e-7 && (bounds.max.z - hi).abs() < 1e-7, "{bounds:?}");
    let m = solidcraft_kernel::measure(&body.body).unwrap();
    assert!((m.volume - 100.0 * (hi - lo)).abs() < 1e-7, "{m:?}");
    assert!((m.centroid.z - (lo + hi) * 0.5).abs() < 1e-7);
}

#[test]
fn symmetric_extrude_total_distance_is_centered_and_follows_parameters() {
    for distance in ["10 mm", "-10 mm"] {
        let mut s = fixture(json!({"distance":distance,"from":[0,0,2],"start_offset":"3 mm"}));
        check(&s, 0.0, 10.0);
        let FeatureKind::Extrude { extent, .. } = &s.doc.find_feature("Depth").unwrap().kind else { panic!("extrude") };
        assert_eq!(extent.convention, ExtrudeConvention::TotalDistance);
        let name = s.doc.find_feature("Depth").unwrap().param_names[0].clone();
        run(&mut s, "parameters.change", json!({"name":name,"expression":"20 mm"}));
        check(&s, -5.0, 15.0);
        run(&mut s, "edit.undo", json!({}));
        check(&s, 0.0, 10.0);
        check(&Session::new(Document::from_json(&s.doc.to_json()).unwrap()), 0.0, 10.0);
    }
}

#[test]
fn symmetric_extrude_switch_from_one_side_keeps_total_thickness() {
    let mut s = fixture(json!({"direction":"positive","convention":"legacy_per_side"}));
    check(&s, 0.0, 10.0);
    run(&mut s, "timeline.edit", json!({"feature":"Depth","set":{"extent":{"direction":"symmetric"}}}));
    check(&s, -5.0, 5.0);
    run(&mut s, "edit.undo", json!({}));
    check(&s, 0.0, 10.0);
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature":"Depth","command":"solid.extrude","params":{"sketch":"Square","distance":"10 mm","direction":"symmetric","body_name":"Block"}}),
    );
    check(&s, -5.0, 5.0);
}

#[test]
fn symmetric_extrude_legacy_missing_marker_preserves_geometry_and_edits() {
    let s = fixture(json!({"convention":"legacy_per_side"}));
    let mut saved: Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    saved["format"] = json!("solidcraft/21");
    for f in saved["features"].as_array_mut().unwrap() {
        if f["type"] == "extrude" {
            f["extent"].as_object_mut().unwrap().remove("convention");
        }
    }
    let mut s = Session::new(Document::from_json(&saved.to_string()).unwrap());
    check(&s, -10.0, 10.0);
    run(&mut s, "timeline.edit", json!({"feature":"Depth","set":{"extent":{"distance":"12 mm"}}}));
    check(&s, -12.0, 12.0);
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature":"Depth","command":"solid.extrude","params":{"sketch":"Square","distance":"10 mm","direction":"symmetric","body_name":"Block"}}),
    );
    check(&s, -10.0, 10.0);
    check(&Session::new(Document::from_json(&s.doc.to_json()).unwrap()), -10.0, 10.0);
}

#[test]
fn symmetric_extrude_through_all_keeps_reach_and_two_sides_keep_independent_distances() {
    let mut s = fixture(json!({"distance2":"3 mm"}));
    check(&s, -3.0, 10.0);
    run(&mut s, "solid.box", json!({"length":10,"width":10,"height":10,"corner":[0,0,100],"body_name":"FarAbove"}));
    run(&mut s, "solid.box", json!({"length":10,"width":10,"height":10,"corner":[0,0,-110],"body_name":"FarBelow"}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch":"Square","direction":"symmetric","through_all":true,"operation":"cut","targets":["FarAbove","FarBelow"]}),
    );
    assert!(s.model.state().body("FarAbove").is_none());
    assert!(s.model.state().body("FarBelow").is_none());
}

#[test]
fn symmetric_extrude_invalid_convention_is_atomic() {
    let mut s = fixture(json!({}));
    let before = s.doc.to_json();
    for convention in [json!("unknown"), json!(7), Value::Null] {
        assert!(s.execute("solid.extrude", &json!({"sketch":"Square","distance":10,"direction":"symmetric","convention":convention})).is_err());
        assert_eq!(s.doc.to_json(), before);
    }
}

#[test]
fn symmetric_extrude_thin_wall_uses_total_distance_and_to_object_remains_one_sided() {
    let s = fixture(json!({"thin":true,"thickness":"2 mm"}));
    let st = s.model.state();
    let body = st.body("Block").unwrap();
    let b = body.mesh().bounds();
    assert!((b.min.z + 5.0).abs() < 1e-7 && (b.max.z - 5.0).abs() < 1e-7);
    // A centred 2 mm wall around a 10 mm square: (12² - 8²) × 10.
    assert!((solidcraft_kernel::measure(&body.body).unwrap().volume - 800.0).abs() < 1e-7);
    check(&fixture(json!({"to":[0,0,10]})), 0.0, 10.0);
}

#[test]
fn surface_extrude_shared_extent_retains_its_existing_per_side_semantics() {
    let mut s = fixture(json!({}));
    run(&mut s, "surface.extrude", json!({"sketch":"Square","distance":"10 mm","direction":"symmetric","name":"Sheet","body_name":"Sheet"}));
    let check_sheet = |s: &Session| {
        let st = s.model.state();
        let bounds = st.body("Sheet").unwrap().mesh().bounds();
        assert!((bounds.min.z + 10.0).abs() < 1e-7 && (bounds.max.z - 10.0).abs() < 1e-7);
    };
    check_sheet(&s);
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature":"Sheet","command":"surface.extrude","params":{"sketch":"Square","distance":"10 mm","direction":"symmetric","body_name":"Sheet"}}),
    );
    check_sheet(&s);
    check_sheet(&Session::new(Document::from_json(&s.doc.to_json()).unwrap()));
}
