//! Joint origins on an edge of a face (its middle, or a corner) follow the face as it changes size.

use serde_json::json;
use solidcraft_geom::Vec3;

use super::tests::{place, run};
use crate::Session;

/// A 2 mm panel `W` x 300 (W = 400) in component Panel, and a 20 x 10 x 6 bracket in component
/// Bracket, off to the side.
fn panel() -> Session {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "W", "expression": "400 mm"}));
    run(&mut s, "component.create", json!({"name": "Panel"}));
    run(&mut s, "solid.box", json!({"length": "W", "width": 300, "height": 2, "corner": [0, 0, 0]}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "component.create", json!({"name": "Bracket"}));
    run(&mut s, "solid.box", json!({"length": 20, "width": 10, "height": 6, "corner": [0, 500, 0]}));
    s
}

/// The middle of the bracket's bottom face, where it is now.
fn bracket(s: &Session) -> Vec3 {
    place(s, "Bracket", Vec3::new(10.0, 505.0, 0.0))
}

fn near(a: Vec3, b: Vec3) -> bool {
    a.dist(b) < 1e-6
}

#[test]
fn a_part_set_in_from_a_corner_stays_there_as_the_panel_grows() {
    let mut s = panel();
    // The front right corner of the top face; the bracket 40 mm in along both edges.
    run(
        &mut s,
        "joint.create",
        json!({"type": "planar", "a": {"occurrence": "Panel", "edge": [200, 0, 2], "corner": [400, 0, 2]},
               "b": {"occurrence": "Bracket", "face": [10, 505, 0]}, "values": [40, 40, 0], "name": "Corner"}),
    );
    for w in [400.0, 600.0, 250.0, 1000.0] {
        run(&mut s, "parameters.change", json!({"name": "W", "expression": format!("{w} mm")}));
        assert!(near(bracket(&s), Vec3::new(w - 40.0, 40.0, 2.0)), "W {w}: {:?}", bracket(&s));
        assert_eq!(run(&mut s, "joint.solve", json!({}))["errors"], json!([]));
    }
    let snap = s.doc.assembly.joints.first().map(|j| j.a.snap.clone());
    assert!(
        matches!(&snap, Some(solidcraft_doc::joints::Snap::Edge { edge: Some(_), face: Some(_), corner: Some(_), .. })),
        "the edge, the face and the corner's other edge are kept by name: {snap:?}"
    );
}

#[test]
fn a_part_at_an_edges_middle_follows_it_and_the_face_can_be_chosen() {
    let mut s = panel();
    // The right edge of the top face: its middle is (W, 150, 2), y points into the face (−X).
    run(
        &mut s,
        "joint.create",
        json!({"type": "planar", "a": {"occurrence": "Panel", "edge": [400, 100, 2]},
               "b": {"occurrence": "Bracket", "face": [10, 505, 0]}, "values": [0, 40, 0], "name": "Mid"}),
    );
    for w in [400.0, 700.0, 320.0] {
        run(&mut s, "parameters.change", json!({"name": "W", "expression": format!("{w} mm")}));
        assert!(near(bracket(&s), Vec3::new(w - 40.0, 150.0, 2.0)), "W {w}: {:?}", bracket(&s));
    }
    // The same edge on the end face: z points out of the end (+X), y into it (down).
    run(&mut s, "joint.delete", json!({"joint": "Mid"}));
    run(
        &mut s,
        "joint.create",
        json!({"type": "rigid", "a": {"occurrence": "Panel", "edge": [320, 100, 2], "on": [320, 150, 1]},
               "b": {"occurrence": "Bracket", "face": [10, 505, 0]}, "name": "End"}),
    );
    assert!(near(bracket(&s), Vec3::new(320.0, 150.0, 2.0)), "{:?}", bracket(&s));
    // The bracket's bottom faces the end face: its top (6 mm up its own z) is out along +X.
    assert!(
        near(place(&s, "Bracket", Vec3::new(10.0, 505.0, 6.0)), Vec3::new(326.0, 150.0, 2.0)),
        "{:?}",
        place(&s, "Bracket", Vec3::new(10.0, 505.0, 6.0))
    );
}

#[test]
fn edge_origins_refuse_bad_picks_and_report_a_lost_edge() {
    let mut s = panel();
    let bad = |s: &mut Session, a: serde_json::Value| {
        s.execute("joint.create", &json!({"type": "rigid", "a": a, "b": {"occurrence": "Bracket", "face": [10, 505, 0]}})).unwrap_err().to_string()
    };
    // Not on an edge; a face the edge doesn't bound.
    assert!(bad(&mut s, json!({"occurrence": "Panel", "edge": [200, 150, 2]})).contains("no edge there"));
    assert!(bad(&mut s, json!({"occurrence": "Panel", "edge": [200, 0, 2], "on": [200, 150, 0]})).contains("doesn't bound"));
    // A design whose edge name no longer finds anything: the joint is left out with the reason.
    run(
        &mut s,
        "joint.create",
        json!({"type": "rigid", "a": {"occurrence": "Panel", "edge": [200, 0, 2]}, "b": {"occurrence": "Bracket", "face": [10, 505, 0]}, "name": "J"}),
    );
    let mut doc = (*s.doc).clone();
    if let Some(j) = doc.assembly.joints.first_mut()
        && let solidcraft_doc::joints::Snap::Edge { edge, .. } = &mut j.a.snap
    {
        *edge = Some("F999:nothing|F998:nowhere".into());
    }
    *s.doc_mut() = doc;
    let r = run(&mut s, "joint.solve", json!({}));
    assert!(r["errors"][0]["why"].as_str().is_some_and(|w| w.contains("no longer exists")), "{r}");
}
