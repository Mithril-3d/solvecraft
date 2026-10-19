//! Commands that take a plane accept construction planes by name or id, like the origin planes
//! (user report: `solid.mirror` refused a construction plane although its docs list plane
//! names).

use serde_json::json;

use super::run;
use crate::*;

fn min_x(s: &Session, body: &str) -> f64 {
    let b = s.model.state().body(body).map(|b| b.body.clone()).unwrap();
    solidcraft_kernel::measure(&b).unwrap().bbox.min.x
}

#[test]
fn mirror_across_a_construction_plane_follows_it() {
    let mut s = Session::default();
    run(&mut s, "parameters.change", json!({"name": "w", "expression": "100 mm"}));
    let pl = run(&mut s, "construct.plane.offset", json!({"base": "YZ", "offset": "w / 2", "name": "Mid"}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Ledger"}));
    let before = s.model.state().bodies.len();
    // By name: the copy lands at x = 90…100.
    run(&mut s, "solid.mirror", json!({"bodies": ["Ledger"], "plane": "Mid"}));
    let st = s.model.state();
    assert_eq!(st.bodies.len(), before + 1);
    let copy = st.bodies.last().map(|b| b.name.clone()).unwrap();
    assert!((min_x(&s, &copy) - 90.0).abs() < 1e-6, "{}", min_x(&s, &copy));
    // Parametric: moving the plane moves the copy.
    run(&mut s, "parameters.change", json!({"name": "w", "expression": "60 mm"}));
    assert!((min_x(&s, &copy) - 50.0).abs() < 1e-6, "{}", min_x(&s, &copy));
    // By id, and case-insensitively by name.
    let id = pl["feature"].as_u64().unwrap();
    run(&mut s, "solid.mirror", json!({"bodies": ["Ledger"], "plane": id.to_string()}));
    run(&mut s, "solid.mirror", json!({"bodies": ["Ledger"], "plane": "mid"}));
    let st = s.model.state();
    assert_eq!(st.bodies.len(), before + 3);
    for b in st.bodies.iter().skip(before) {
        assert!((min_x(&s, &b.name) - 50.0).abs() < 1e-6, "{}", b.name);
    }
}

/// A plane `{origin, normal}` takes length expressions in its origin, as a primitive's corner
/// does, and the mirror follows them through parameter changes, renames and a save (#1492).
#[test]
fn mirror_plane_origin_follows_its_parameters() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "half", "expression": "50 mm"}));
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 20}));
    run(&mut s, "solid.mirror", json!({"features": ["Box1"], "plane": {"origin": ["half", 0, 0], "normal": [1, 0, 0]}, "name": "Twin"}));
    let at = |s: &Session| min_x(s, &last_body(s));
    // The box spans x = 0…20; mirrored about x = 50 it spans 80…100.
    assert!((at(&s) - 80.0).abs() < 1e-6, "{}", at(&s));
    run(&mut s, "parameters.change", json!({"name": "half", "expression": "30 mm"}));
    assert!((at(&s) - 40.0).abs() < 1e-6, "{}", at(&s));
    run(&mut s, "parameters.rename", json!({"name": "half", "new_name": "mid"}));
    run(&mut s, "parameters.change", json!({"name": "mid", "expression": "25 mm"}));
    assert!((at(&s) - 30.0).abs() < 1e-6, "{}", at(&s));
    // Saved and opened again, the plane still names the parameter.
    let mut s2 = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    assert!(s2.doc.to_json().contains("\"mid\""));
    run(&mut s2, "parameters.change", json!({"name": "mid", "expression": "60 mm"}));
    assert!((at(&s2) - 100.0).abs() < 1e-6, "{}", at(&s2));
    // A parameter the plane uses can't be deleted from under it.
    assert!(s2.execute("parameters.delete", &json!({"name": "mid"})).is_err());
    // Plain numbers still make a fixed plane.
    let r = run(&mut s2, "solid.mirror", json!({"features": ["Box1"], "plane": {"origin": [-10, 0, 0], "normal": [1, 0, 0]}}));
    let id = r["feature"].as_u64().unwrap();
    let f = s2.doc.feature(id).unwrap();
    assert!(matches!(f.kind, solidcraft_doc::FeatureKind::Mirror { plane: solidcraft_doc::PlaneRef::Custom { .. }, .. }), "{:?}", f.kind);
    assert!((at(&s2) + 40.0).abs() < 1e-6, "{}", at(&s2));
}

fn last_body(s: &Session) -> String {
    s.model.state().bodies.last().map(|b| b.name.clone()).unwrap()
}

#[test]
fn unknown_mirror_planes_are_refused_with_the_choices() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "B"}));
    let e = s.execute("solid.mirror", &json!({"bodies": ["B"], "plane": "Nope"})).unwrap_err().to_string();
    assert!(e.contains("Nope"), "{e}");
    let e = s.execute("solid.mirror", &json!({"bodies": ["B"], "plane": 5})).unwrap_err().to_string();
    assert!(e.contains("construction plane"), "{e}");
    // A plane object with an unreadable origin is refused, not put at the world origin.
    assert!(s.execute("solid.mirror", &json!({"bodies": ["B"], "plane": {"origin": ["x", 0, 0], "normal": [1, 0, 0]}})).is_err());
    // A feature that is not a plane is no plane.
    assert!(s.execute("solid.mirror", &json!({"bodies": ["B"], "plane": "Box1"})).is_err());
    assert_eq!(s.model.state().bodies.len(), 1);
}

/// Section Analysis cuts along a construction plane too.
#[test]
fn section_by_a_construction_plane() {
    let mut s = Session::default();
    run(&mut s, "construct.plane.offset", json!({"base": "XY", "offset": 7, "name": "Cut"}));
    let r = run(&mut s, "inspect.section", json!({"plane": "Cut"}));
    assert_eq!(r["section"]["origin"][2].as_f64(), Some(7.0), "{r}");
    assert!(s.execute("inspect.section", &json!({"plane": "Nope"})).is_err());
}
