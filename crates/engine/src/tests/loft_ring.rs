//! A loft section that is a ring (a profile with a hole) is refused (#1184): the loft builds each
//! section from its outline, so it used to fill the hole and report success.

use super::*;

/// A sketch `name` at height `z` with circles of radius 20 and 12 about the origin.
fn ring(s: &mut Session, name: &str, z: f64) {
    run(s, "sketch.create", json!({"plane": "XY", "offset": z, "name": name}));
    run(s, "sketch.circle.center", json!({"center": [0, 0], "radius": 20}));
    run(s, "sketch.circle.center", json!({"center": [0, 0], "radius": 12}));
    run(s, "sketch.finish", json!({}));
}

fn refused(s: &mut Session, sections: Value) {
    let saved = s.doc.to_json();
    let err = s.execute("solid.loft", &json!({"sections": sections, "operation": "new"})).err().map(|e| e.to_string());
    assert!(err.as_deref().is_some_and(|e| e.contains("not supported yet: a loft section with a hole")), "{err:?}");
    assert_eq!(s.doc.to_json(), saved);
}

#[test]
fn a_ring_section_is_refused_not_filled() {
    let mut s = Session::default();
    ring(&mut s, "A", 0.0);
    ring(&mut s, "B", 20.0);
    let ring_at = |n: &str| json!({"sketch": n, "profiles": [{"point": [16, 0]}]});
    refused(&mut s, json!([ring_at("A"), ring_at("B")]));
    // A ring and a plain disc, in either order.
    let disc_b = json!({"sketch": "B", "profiles": [{"point": [0, 0]}]});
    refused(&mut s, json!([ring_at("A"), disc_b]));
    refused(&mut s, json!([disc_b, ring_at("A")]));
    // The discs inside the holes still loft into a cylinder of radius 12.
    run(&mut s, "solid.loft", json!({"sections": [{"sketch": "A", "profiles": [{"point": [0, 0]}]}, disc_b], "operation": "new"}));
    let want = std::f64::consts::PI * 12.0 * 12.0 * 20.0;
    let got = volume(&mut s);
    assert!(rel(got, want) < 1e-3, "{got} vs {want}");
}

#[test]
fn a_ring_lofted_to_a_point_is_refused() {
    let mut s = Session::default();
    // Square, since lofting curved sections to a point is not supported at all yet.
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "A"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [-20, -20], "p1": [20, 20]}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [-12, -12], "p1": [12, 12]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "offset": 20, "name": "Top"}));
    run(&mut s, "sketch.point", json!({"point": [0, 0], "id": "apex"}));
    run(&mut s, "sketch.finish", json!({}));
    refused(&mut s, json!([{"sketch": "A", "profiles": [{"point": [16, 0]}]}, {"sketch": "Top", "point": "apex"}]));
}
