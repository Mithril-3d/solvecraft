//! #1488: a loft joined into a slab, flush with the slab's outer face and its bottom, takes the
//! boolean's last retry (the tool nudged a fraction of a micron). That leaves sliver faces
//! along the flush faces, one of which reads as a face whose boundary crosses itself; every
//! later round on the body was then refused for it ("the rounds would leave openings"), even
//! one 36 mm away. A round is now refused only for crossed faces it makes.

use serde_json::json;

use super::run;
use crate::Session;

/// The issue's slab `width` thick with a window through it and the pleat loft joined into it.
fn slab_with_pleat(width: f64) -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"corner": [-60, 0, 0], "length": 120, "width": width, "height": 95, "body_name": "Wall"}));
    run(
        &mut s,
        "solid.box",
        json!({"corner": [-24, -0.5, -1], "length": 48, "width": width + 1.0, "height": 91, "operation": "cut", "targets": ["Wall"]}),
    );
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Bottom"}));
    run(
        &mut s,
        "sketch.line",
        json!({"closed": true, "infer": false, "connect": false, "points": [
            [-26, 3], [-24.76384, 3], [-16.0568, 14.97217], [-8, 4.90117], [0, 14.90117], [8, 4.90117], [16.0568, 14.97217], [24.76384, 3], [26, 3],
            [26, 0], [23.23616, 0], [15.9432, 10.02783], [8, 0.09883], [0, 10.09883], [-8, 0.09883], [-15.9432, 10.02783], [-23.23616, 0], [-26, 0]]}),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "offset": 90.5, "name": "Top"}));
    run(
        &mut s,
        "sketch.line",
        json!({"closed": true, "infer": false, "connect": false, "points": [
            [-26, 3], [-24, 3], [-16, 3], [-8, 3], [0, 3], [8, 3], [16, 3], [24, 3], [26, 3],
            [26, 0], [24, 0], [16, 0], [8, 0], [0, 0], [-8, 0], [-16, 0], [-24, 0], [-26, 0]]}),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.loft",
        json!({"sections": [{"sketch": "Bottom"}, {"sketch": "Top"}], "operation": "join", "targets": ["Wall"], "name": "Pleat"}),
    );
    s
}

fn wall(s: &Session) -> solidcraft_kernel::Body {
    s.model.state().body("Wall").map(|b| b.body.clone()).unwrap()
}

/// The R1 round on the slab's end edge at x = −60 works after the join, as it does on the 3 mm
/// slab (and on the 3.5 mm one before the loft), and adds no crossed face of its own.
#[test]
fn a_round_far_from_the_joined_loft_works() {
    for (width, pick) in [(3.5, [-60.0, 1.75, 95.0]), (3.0, [-60.0, 1.5, 95.0])] {
        let mut s = slab_with_pleat(width);
        let before = wall(&s);
        let r = s.execute("solid.fillet", &json!({"body": "Wall", "edges": [pick], "radius": 1}));
        assert!(r.is_ok(), "{width} mm: {r:?}");
        let after = wall(&s);
        assert_eq!(after.face_count(), before.face_count() + 1, "{width} mm: one round face");
        assert_eq!(after.crossed_faces().unwrap(), before.crossed_faces().unwrap(), "{width} mm");
        // The round lies along the picked edge: the end face's top corner is gone.
        let m = solidcraft_kernel::measure(&after).unwrap();
        assert!((m.bbox.min.x + 60.0).abs() < 1e-6 && (m.bbox.max.z - 95.0).abs() < 1e-6, "{width} mm: {:?}", m.bbox);
    }
}

/// A round that does cross faces is still refused, joined loft or not (#871's check stands).
#[test]
fn rounds_that_overlap_are_still_refused() {
    let mut s = slab_with_pleat(3.5);
    // Both long top edges of the 3.5 mm slab with a radius over half its thickness.
    let r = s.execute("solid.fillet", &json!({"body": "Wall", "edges": [[-40, 0, 95], [-40, 3.5, 95]], "radius": 2}));
    assert!(r.is_err(), "{r:?}");
}
