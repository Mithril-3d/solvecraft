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

/// A triangular face whose first edge is an intersection curve of two planes: the parallel planes
/// `z = 0` and `z = 1` when `support_gap` is `1.0` (the projection onto both never converges), the
/// same plane twice when it is `0.0` (the leader lies on both and always evaluates).
fn triangle_with_intersection_edge(support_gap: f64) -> truck_modeling::Shell {
    use truck_modeling::{Curve, Edge, Face, IntersectionCurve, Leader, Line, Shell, Surface, Vertex, Wire};
    let plane = |z: f64| Surface::Plane(Plane::new(Point3::new(0.0, 0.0, z), Point3::new(1.0, 0.0, z), Point3::new(0.0, 1.0, z)));
    let (v0, v1, v2) = (Vertex::new(Point3::new(0.0, 0.0, 0.5)), Vertex::new(Point3::new(1.0, 0.0, 0.5)), Vertex::new(Point3::new(0.5, 1.0, 0.5)));
    let leader = Leader::Polyline(PolylineCurve(vec![Point3::new(0.0, 0.0, 0.5), Point3::new(1.0, 0.0, 0.5)]));
    let bad = IntersectionCurve::new_unchecked(Box::new(plane(0.5 - support_gap / 2.0)), Box::new(plane(0.5 + support_gap / 2.0)), leader, 0.01);
    let line = |a: &Vertex, b: &Vertex| Edge::new(a, b, Curve::Line(Line(a.point(), b.point())));
    let wire = Wire::from(vec![Edge::new(&v0, &v1, Curve::IntersectionCurve(bad)), line(&v1, &v2), line(&v2, &v0)]);
    Shell::from(vec![Face::new(vec![wire], plane(0.5))])
}

/// Meshing an edge whose intersection curve does not evaluate (#1252): `IntersectionCurve::subs`
/// unwraps a failed projection, and the mesher sampled the edge through it. The two supports here
/// are parallel planes, so the projection never converges. Nothing may panic (wasm aborts on one,
/// and the native guard prints the message first) and the mesh stays finite.
#[test]
fn meshing_an_edge_that_does_not_evaluate_neither_panics_nor_goes_non_finite() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use truck_meshalgo::prelude::RobustMeshableShape;
    let shell = triangle_with_intersection_edge(1.0);

    let me = std::thread::current().id();
    let count = Arc::new(AtomicUsize::new(0));
    let c2 = count.clone();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |_| {
        if std::thread::current().id() == me {
            c2.fetch_add(1, Ordering::SeqCst);
        }
    }));
    let meshed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| shell.robust_triangulation(0.01)));
    std::panic::set_hook(prev);
    assert_eq!(count.load(Ordering::SeqCst), 0, "panics while meshing");
    let meshed = meshed.unwrap();
    for face in meshed.face_iter() {
        if let Some(mesh) = face.surface() {
            assert!(mesh.positions().iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()));
        }
    }
}

/// A boolean's result with an edge that does not evaluate is refused with an error (#1252). The
/// mesher's panic on such an edge used to reject the attempt; now that it no longer panics, the
/// result must be refused explicitly or a plausible-looking mesh would let it through.
#[test]
fn a_result_with_an_edge_that_does_not_evaluate_is_refused() {
    use truck_modeling::Solid;
    let solid = |gap: f64| Solid::new_unchecked(vec![triangle_with_intersection_edge(gap)]);
    assert!(!crate::boolean_record::edges_evaluate(&solid(1.0)));
    assert!(crate::boolean_record::edges_evaluate(&solid(0.0)));
}
