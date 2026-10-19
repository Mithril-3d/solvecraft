//! Two bores of one radius crossing each other (#788): the surfaces touch where the two ellipses
//! of the crossing meet, which a mesh intersection cannot trace and a boolean cannot take in the
//! middle of a face.
use crate::tangent_cylinders;
use crate::*;
use solidcraft_geom::Vec3;
use std::f64::consts::PI;

const SIDE: f64 = 45.0;

fn volume(b: &Body) -> f64 {
    measure(b).unwrap().volume
}

fn block() -> Body {
    let h = SIDE / 2.0;
    box_solid(Vec3::new(-h, -h, -h), Vec3::new(h, h, h)).unwrap()
}

/// A cylinder of radius `r` along `axis` through `centre`, longer than the block.
fn bore(centre: Vec3, axis: Vec3, r: f64, length: f64) -> Body {
    cylinder(centre - axis * (length / 2.0), axis, r, length).unwrap()
}

/// The block with a bore along Z, then one of the same radius along `axis` through `centre`.
fn crossed(r: f64, centre: Vec3, axis: Vec3) -> Result<Option<Body>> {
    let one = boolean(&block(), &bore(Vec3::ZERO, Vec3::Z, r, 60.0), BoolOp::Cut)?.expect("first bore");
    boolean(&one, &bore(centre, axis, r, 60.0), BoolOp::Cut)
}

fn within(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 2e-3 * want, "{what}: {got} vs {want}");
}

/// Block less two perpendicular bores through its middle: each takes `π r² L`, and the Steinmetz
/// solid they share, 16r³/3, is counted twice.
fn expected_perpendicular(r: f64) -> f64 {
    SIDE.powi(3) - 2.0 * PI * r * r * SIDE + 16.0 * r.powi(3) / 3.0
}

#[test]
fn equal_bores_crossing_at_right_angles() {
    for r in [10.0, 17.5, 4.0] {
        let body = crossed(r, Vec3::ZERO, Vec3::Y).unwrap_or_else(|e| panic!("r {r}: {e}")).unwrap();
        assert_eq!(body.shell_count(), 1, "r {r}");
        within(volume(&body), expected_perpendicular(r), &format!("r {r}"));
    }
}

#[test]
fn equal_bores_crossing_along_every_axis_pair() {
    let r = 10.0;
    let want = expected_perpendicular(r);
    for (first, second) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::X), (Vec3::X, Vec3::Z), (Vec3::Z, Vec3::X), (Vec3::Y, Vec3::Z)] {
        let one = boolean(&block(), &bore(Vec3::ZERO, first, r, 60.0), BoolOp::Cut).unwrap().unwrap();
        let body = boolean(&one, &bore(Vec3::ZERO, second, r, 60.0), BoolOp::Cut).unwrap().unwrap();
        within(volume(&body), want, &format!("{first:?} then {second:?}"));
    }
}

#[test]
fn equal_bores_crossing_at_an_angle() {
    // Two infinite cylinders of radius r at angle t share 16r³ / (3 sin t).
    let (r, t) = (5.0f64, PI / 3.0);
    let axis = Vec3::new(0.0, t.sin(), t.cos());
    let body = crossed(r, Vec3::ZERO, axis).unwrap().unwrap();
    let first = PI * r * r * SIDE;
    let second = PI * r * r * SIDE / t.sin();
    let shared = 16.0 * r.powi(3) / (3.0 * t.sin());
    within(volume(&body), SIDE.powi(3) - first - second + shared, "60 degrees");
}

#[test]
fn equal_bores_crossing_off_the_middle_of_the_block() {
    // Both axes meet at (3, -2, 5): the touching points are not at the block's centre.
    let r = 8.0;
    let c = Vec3::new(3.0, -2.0, 5.0);
    let one = boolean(&block(), &bore(c, Vec3::Z, r, 60.0), BoolOp::Cut).unwrap().unwrap();
    let body = boolean(&one, &bore(c, Vec3::Y, r, 80.0), BoolOp::Cut).unwrap().unwrap();
    // The bores leave the block through its faces; the shared part lies inside it.
    let removed = PI * r * r * SIDE * 2.0 - 16.0 * r.powi(3) / 3.0;
    within(volume(&body), SIDE.powi(3) - removed, "off centre");
}

#[test]
fn equal_cylinders_joined_and_intersected() {
    let r = 10.0;
    let (z, y) = (bore(Vec3::ZERO, Vec3::Z, r, 60.0), bore(Vec3::ZERO, Vec3::Y, r, 60.0));
    let tee = boolean(&z, &y, BoolOp::Union).unwrap().unwrap();
    within(volume(&tee), 2.0 * PI * r * r * 60.0 - 16.0 * r.powi(3) / 3.0, "join");
    assert_eq!(tee.shell_count(), 1);
    let steinmetz = boolean(&z, &y, BoolOp::Intersect).unwrap().unwrap();
    within(volume(&steinmetz), 16.0 * r.powi(3) / 3.0, "intersect");
}

#[test]
fn the_result_keeps_a_history_over_the_operands_as_given() {
    let r = 10.0;
    let one = boolean(&block(), &bore(Vec3::ZERO, Vec3::Z, r, 60.0), BoolOp::Cut).unwrap().unwrap();
    let tool = bore(Vec3::ZERO, Vec3::Y, r, 60.0);
    let body = boolean(&one, &tool, BoolOp::Cut).unwrap().unwrap();
    let history = body.history().expect("the cut has a history");
    assert_eq!(history.operands, 2);
    assert_eq!(history.faces.len(), body.face_count());
}

#[test]
fn faces_split_for_the_crossing_leave_the_operands_alone() {
    // The split is on the working copies: the target keeps its faces.
    let r = 10.0;
    let one = boolean(&block(), &bore(Vec3::ZERO, Vec3::Z, r, 60.0), BoolOp::Cut).unwrap().unwrap();
    let (faces, volume_before) = (one.face_count(), volume(&one));
    let _ = boolean(&one, &bore(Vec3::ZERO, Vec3::Y, r, 60.0), BoolOp::Cut).unwrap();
    assert_eq!(one.face_count(), faces);
    assert_eq!(volume(&one), volume_before);
}

#[test]
fn crossings_that_are_not_touching_are_left_alone() {
    let r = 10.0;
    let size = 100.0;
    let tol = size * 1e-6;
    let cyl = |face, origin: Vec3, axis: Vec3, radius| tangent_cylinders::Cylinder::new(face, origin, axis, radius).unwrap();
    let z = cyl(0, Vec3::ZERO, Vec3::Z, r);
    // Different radius, parallel axes, skew axes: no touching points to split at.
    assert!(tangent_cylinders::crossing(&z, &cyl(1, Vec3::ZERO, Vec3::Y, r * 0.9), tol).is_none());
    assert!(tangent_cylinders::crossing(&z, &cyl(1, Vec3::new(5.0, 0.0, 0.0), Vec3::Z, r), tol).is_none());
    assert!(tangent_cylinders::crossing(&z, &cyl(1, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1e-6, 0.0, 1.0), r), tol).is_none());
    assert!(tangent_cylinders::crossing(&z, &cyl(1, Vec3::new(1.0, 0.0, 0.0), Vec3::Y, r), tol).is_none());
    // Axes that meet: two touching points, a radius either side of the crossing.
    let found = tangent_cylinders::crossing(&z, &cyl(1, Vec3::ZERO, Vec3::Y, r), tol).unwrap();
    assert!(found.points.iter().any(|p| p.dist(Vec3::new(r, 0.0, 0.0)) < 1e-9));
    assert!(found.points.iter().any(|p| p.dist(Vec3::new(-r, 0.0, 0.0)) < 1e-9));
}

#[test]
fn hostile_cylinders_are_refused() {
    let tol = 1e-4;
    let cyl = |axis: Vec3, radius, origin: Vec3| tangent_cylinders::Cylinder::new(0, origin, axis, radius);
    // An axis with no direction cannot make a cylinder at all.
    assert!(cyl(Vec3::new(f64::NAN, 0.0, 1.0), 10.0, Vec3::ZERO).is_none());
    assert!(cyl(Vec3::ZERO, 10.0, Vec3::ZERO).is_none());
    let good = cyl(Vec3::Z, 10.0, Vec3::ZERO).unwrap();
    for bad in [
        cyl(Vec3::Y, f64::NAN, Vec3::ZERO).unwrap(),
        cyl(Vec3::Y, f64::INFINITY, Vec3::ZERO).unwrap(),
        cyl(Vec3::Y, 10.0, Vec3::new(f64::INFINITY, 0.0, 0.0)).unwrap(),
    ] {
        assert!(tangent_cylinders::crossing(&good, &bad, tol).is_none(), "{bad:?}");
    }
}
