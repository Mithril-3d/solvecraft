//! Regressions against the actual patched truck constructor, run by workspace CI.
use truck_meshalgo::prelude::PolylineCurve;
use truck_modeling::{BoundedCurve, ParametricCurve, Plane, Point3, Sphere};
use truck_shapeops::IntersectionCurveWithParameters;

fn planes() -> (Plane, Plane) {
    let o = Point3::new(0.0, 0.0, 0.0);
    (Plane::new(o, Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 1.0, 0.0)), Plane::new(o, Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 0.0, 1.0)))
}

#[test]
fn degenerate_intersection_polylines_return_none_without_panicking() {
    let origin = Point3::new(0.0, 0.0, 0.0);
    for points in [
        vec![],
        vec![origin],
        vec![origin, origin],
        vec![origin; 3],
        vec![origin, Point3::new(f64::NAN, 0.0, 0.0)],
        vec![origin, Point3::new(f64::INFINITY, 0.0, 0.0)],
    ] {
        let (a, b) = planes();
        let result = std::panic::catch_unwind(|| IntersectionCurveWithParameters::try_new(a, b, PolylineCurve(points.clone()), 0.01));
        assert!(result.is_ok(), "constructor panicked for {points:?}");
        assert!(result.unwrap().is_none(), "collapsed input must not create a curve: {points:?}");
    }
}

#[test]
fn closed_intersection_retains_the_closure_and_spherical_supports() {
    let a = Sphere::new(Point3::new(0.0, 0.0, 1.0), 2.0_f64.sqrt());
    let b = Sphere::new(Point3::new(0.0, 0.0, -1.0), 2.0_f64.sqrt());
    let poly = PolylineCurve(vec![
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(-1.0, 0.0, 0.0),
        Point3::new(0.0, -1.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
    ]);
    let curve = IntersectionCurveWithParameters::try_new(a, b, poly, 0.01).unwrap();
    let (lo, hi) = curve.range_tuple();
    assert_eq!(curve.subs(lo), curve.subs(hi));
    for i in 0..=40 {
        let p = curve.subs(lo + (hi - lo) * i as f64 / 40.0);
        assert!(p.z.abs() < 1e-8, "{p:?}");
        assert!(((p.x * p.x + p.y * p.y).sqrt() - 1.0).abs() <= 0.01, "{p:?}");
    }
}

#[test]
fn open_intersection_keeps_endpoints_on_both_planes() {
    let (a, b) = planes();
    let start = Point3::new(1.0, 0.0, 0.0);
    let end = Point3::new(3.0, 0.0, 0.0);
    let curve = IntersectionCurveWithParameters::try_new(a, b, PolylineCurve(vec![start, end]), 0.01).unwrap();
    let (lo, hi) = curve.range_tuple();
    assert_eq!(curve.subs(lo), start);
    assert_eq!(curve.subs(hi), end);
    assert_eq!(curve.subs((lo + hi) / 2.0), Point3::new(2.0, 0.0, 0.0));
}
