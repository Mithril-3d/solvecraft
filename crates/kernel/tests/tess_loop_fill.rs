//! The fallback in the vendored tessellator that triangulates the boundary loop of a face that
//! came out with no triangle (a lens thinner than the tolerance). Its ear clipping is checked on
//! the polygons it can meet: convex, concave, either winding, with collinear points, and loops
//! it must refuse.
//!
//! The vendored crates are outside the workspace, so their own tests never run in CI; the
//! function is reached through the crate's `test-hooks` feature (on only for kernel tests).

use truck_meshalgo::tessellation::loop_fill_test_hooks::ear_clipping;

fn area2(points: &[[f64; 2]]) -> f64 {
    points.iter().zip(points.iter().cycle().skip(1)).map(|(p, q)| p[0] * q[1] - q[0] * p[1]).sum()
}

/// The triangles turn the same way as the polygon, there are `n - 2` of them, and they add up to
/// its area: together they cover it once, with no overlap.
fn assert_covers_once(points: &[[f64; 2]]) {
    let triangles = ear_clipping(points).expect("a simple polygon is triangulated");
    assert_eq!(triangles.len(), points.len() - 2);
    let side = area2(points).signum();
    let mut sum = 0.0;
    for t in &triangles {
        let t = area2(&t.map(|i| points[i]));
        assert!(t * side > 0.0, "a triangle turns the other way, or has no area: {t}");
        sum += t;
    }
    assert!((sum - area2(points)).abs() < 1e-9, "triangles cover {sum}, the polygon {}", area2(points));
}

#[test]
fn a_triangle_is_itself() {
    let points = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    assert_eq!(ear_clipping(&points), Some(vec![[0, 1, 2]]));
}

#[test]
fn a_convex_polygon_either_way_round() {
    let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    assert_covers_once(&square);
    let mut reversed = square;
    reversed.reverse();
    assert_covers_once(&reversed);
}

#[test]
fn a_concave_polygon_either_way_round() {
    // An L: the reflex corner is at (1, 1).
    let l = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
    assert_covers_once(&l);
    let mut reversed = l;
    reversed.reverse();
    assert_covers_once(&reversed);
}

#[test]
fn a_thin_lens_made_of_an_arc_and_a_chord() {
    let lens = [[0.0, 0.0], [1.0, -0.15], [2.0, 0.0]];
    assert_covers_once(&lens);
    let more = [[0.0, 0.0], [0.5, -0.1], [1.0, -0.15], [1.5, -0.1], [2.0, 0.0]];
    assert_covers_once(&more);
}

#[test]
fn collinear_points_on_a_side_are_kept_as_corners() {
    let square = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let triangles = ear_clipping(&square).expect("triangulated");
    assert_eq!(triangles.len(), 3);
    let covered: f64 = triangles.iter().map(|t| area2(&t.map(|i| square[i]))).sum();
    assert!((covered - area2(&square)).abs() < 1e-9);
}

#[test]
fn a_loop_without_area_is_refused() {
    assert_eq!(ear_clipping(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]), None, "three collinear points");
    assert_eq!(ear_clipping(&[[0.0, 0.0], [1.0, 1.0], [0.0, 0.0], [1.0, 1.0]]), None, "a line walked twice");
}

#[test]
fn fewer_than_three_points_are_refused() {
    assert_eq!(ear_clipping(&[]), None);
    assert_eq!(ear_clipping(&[[0.0, 0.0]]), None);
    assert_eq!(ear_clipping(&[[0.0, 0.0], [1.0, 0.0]]), None);
}

/// A loop that crosses itself (the #1648 face pinched to a point is one) would get overlapping
/// triangles, and so a wrong mesh that hides the problem. It is refused, symmetric or not.
#[test]
fn a_loop_crossing_itself_is_refused() {
    let bowtie = [[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]];
    assert_eq!(ear_clipping(&bowtie), None);
    let lopsided = [[0.0, 0.0], [3.0, 2.0], [3.0, 0.0], [0.0, 1.0]];
    assert_eq!(ear_clipping(&lopsided), None);
    let pinched = [[0.0, 0.0], [5.0, 0.0], [5.0, 0.3], [3.0, -0.1], [2.0, -0.1], [0.0, 0.3]];
    assert_eq!(ear_clipping(&pinched), None);
}

/// The loop of a face closes on its first point: a point given twice in a row is a corner with no
/// area, and the polygon is still triangulated on the rest.
#[test]
fn a_repeated_point_is_handled_without_a_wrong_mesh() {
    let points = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
    if let Some(triangles) = ear_clipping(&points) {
        let covered: f64 = triangles.iter().map(|t| area2(&t.map(|i| points[i]))).sum();
        assert!((covered - area2(&points)).abs() < 1e-9, "triangles cover {covered}");
    }
}

/// The largest loop the fallback takes, as a circle with a bite out of it: all of it is covered
/// exactly once.
#[test]
fn the_largest_loop_is_triangulated() {
    let n = 256;
    let points: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            let r = if i % 16 == 0 { 0.6 } else { 1.0 };
            [r * t.cos(), r * t.sin()]
        })
        .collect();
    assert_covers_once(&points);
}
