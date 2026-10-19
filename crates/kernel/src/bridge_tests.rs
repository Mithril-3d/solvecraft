//! Tests of the bridge core: the support surface's shape is read off a real body, and a bridge's
//! continuity is measured back off the surface it built.

use super::*;
use crate::{box_solid, cylinder};

/// The shape of a bridge's cross-section at the parameter `u`, measured off the finished face (not
/// from the construction): position, unit tangent, curvature vector and the arclength derivative
/// of that curvature vector at the start of the cross-section. The samples run inwards from the
/// boundary, so nothing is asked of the surface outside its own domain.
fn section_at(body: &Body, u: f64) -> (Vec3, Vec3, Vec3, Vec3) {
    let face = body.solid.face_iter().next().unwrap();
    let surf = face.surface();
    let (Some(_), Some((v0, _))) = surf.try_range_tuple() else { panic!("unbounded") };
    let h = 1e-3;
    let q: Vec<Vec3> = (0..5).map(|k| from_p3(surf.subs(u, v0 + k as f64 * h))).collect();
    let (f0, f1, f2, f3, f4) = (q[0], q[1], q[2], q[3], q[4]);
    let d1 = (f0 * -25.0 + f1 * 48.0 + f2 * -36.0 + f3 * 16.0 + f4 * -3.0) * (1.0 / (12.0 * h));
    let d2 = (f0 * 35.0 + f1 * -104.0 + f2 * 114.0 + f3 * -56.0 + f4 * 11.0) * (1.0 / (12.0 * h * h));
    let d3 = (f0 * -5.0 + f1 * 18.0 + f2 * -24.0 + f3 * 14.0 + f4 * -3.0) * (1.0 / (2.0 * h * h * h));
    let (t, bend, bend_change) = shape(d1, d2, d3).unwrap();
    (f0, t, bend, bend_change)
}

/// The same at the end of the cross-section (`v = v1`), with the samples running back inwards and
/// the odd derivatives turned round.
fn section_end_at(body: &Body, u: f64) -> (Vec3, Vec3, Vec3, Vec3) {
    let face = body.solid.face_iter().next().unwrap();
    let surf = face.surface();
    let (Some(_), Some((_, v1))) = surf.try_range_tuple() else { panic!("unbounded") };
    let h = 1e-3;
    let q: Vec<Vec3> = (0..5).map(|k| from_p3(surf.subs(u, v1 - k as f64 * h))).collect();
    let (f0, f1, f2, f3, f4) = (q[0], q[1], q[2], q[3], q[4]);
    let d1 = (f0 * -25.0 + f1 * 48.0 + f2 * -36.0 + f3 * 16.0 + f4 * -3.0) * (-1.0 / (12.0 * h));
    let d2 = (f0 * 35.0 + f1 * -104.0 + f2 * 114.0 + f3 * -56.0 + f4 * 11.0) * (1.0 / (12.0 * h * h));
    let d3 = (f0 * -5.0 + f1 * 18.0 + f2 * -24.0 + f3 * 14.0 + f4 * -3.0) * (-1.0 / (2.0 * h * h * h));
    let (t, bend, bend_change) = shape(d1, d2, d3).unwrap();
    (f0, t, bend, bend_change)
}

/// The angle between two vectors, in degrees.
fn angle_deg(a: Vec3, b: Vec3) -> f64 {
    (a.normalized().unwrap().dot(b.normalized().unwrap())).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Two planes 10 mm apart: the lower one leaves along +y, the upper one has no curvature, and the
/// direction between them is +z — an S-shaped bridge.
fn two_planes(count: usize) -> (Vec<Frame>, Vec<Frame>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..count {
        let y = 10.0 * i as f64 / (count - 1) as f64;
        a.push(Frame { p: Vec3::new(0.0, y, 0.0), leave: Vec3::new(0.0, 1.0, 0.0), bend: Vec3::ZERO, bend_change: Vec3::ZERO });
        b.push(Frame { p: Vec3::new(0.0, y, 10.0), leave: Vec3::new(0.0, -1.0, 0.0), bend: Vec3::ZERO, bend_change: Vec3::ZERO });
    }
    (a, b)
}

/// Two parallel circular cylinders of radius `r`, axes along x at z = r, 20 mm apart in y: their
/// lowest lines are the two boundaries. Each leaves towards the other along ∓y, bends up towards
/// its own axis (curvature 1/r), and its curvature does not change as it goes round — what does
/// change is purely tangential.
fn valley_between_cylinders(count: usize, r: f64) -> (Vec<Frame>, Vec<Frame>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..count {
        let y = 10.0 * i as f64 / (count - 1) as f64;
        a.push(Frame {
            p: Vec3::new(0.0, y, 0.0),
            leave: Vec3::new(0.0, 1.0, 0.0),
            bend: Vec3::new(0.0, 0.0, 1.0 / r),
            bend_change: Vec3::new(0.0, -1.0 / (r * r), 0.0),
        });
        b.push(Frame {
            p: Vec3::new(0.0, y + 20.0, 0.0),
            leave: Vec3::new(0.0, -1.0, 0.0),
            bend: Vec3::new(0.0, 0.0, 1.0 / r),
            bend_change: Vec3::new(0.0, 1.0 / (r * r), 0.0),
        });
    }
    (a, b)
}

/// The edges of `face` chained into one loop.
fn loop_of(face: &mt::Face) -> Vec<mt::Edge> {
    let mut pool: Vec<mt::Edge> = face.edge_iter().collect();
    let mut out = vec![pool.remove(0)];
    let mut end = out[0].back().clone();
    while !pool.is_empty() {
        let k = pool.iter().position(|e| e.front().id() == end.id() || e.back().id() == end.id()).unwrap();
        let e = pool.remove(k);
        let e = if e.front().id() == end.id() { e } else { e.inverse() };
        end = e.back().clone();
        out.push(e);
    }
    out
}

/// The face of `body` that contains `at`.
fn face_at(body: &Body, at: Vec3) -> mt::Face {
    body.solid
        .face_iter()
        .find(|f| f.surface().search_nearest_parameter(p3(at), None, 100).is_some_and(|(u, v)| from_p3(f.surface().subs(u, v)).dist(at) < 1e-6))
        .unwrap()
        .clone()
}

#[test]
fn frames_read_a_plane_and_a_cylinder_wall() {
    // A 40 x 30 x 20 box: the top face's rim leaves along +x, a plane with no curvature.
    let b = box_solid(Vec3::ZERO, Vec3::new(40.0, 30.0, 20.0)).unwrap();
    let top = face_at(&b, Vec3::new(20.0, 15.0, 20.0));
    let rim = loop_of(&top);
    assert!(chain_closes(&rim));
    let f = frames(&top, &rim, Vec3::new(1.0, 0.0, 0.0), 9).unwrap();
    assert_eq!(f.len(), 9);
    for s in &f {
        assert!((s.p.z - 20.0).abs() < 1e-9, "{s:?}");
        assert!(angle_deg(s.leave, Vec3::new(1.0, 0.0, 0.0)) < 1e-6, "{s:?}");
        assert!(s.bend.len() < 1e-6 && s.bend_change.len() < 1e-6, "{s:?}");
    }
    // The chain's picks work the same way: the rim's own points pick the rim's own edges.
    let picked = chain(&b, &[Vec3::new(20.0, 0.0, 20.0), Vec3::new(40.0, 15.0, 20.0)]).unwrap();
    assert_eq!(picked.len(), 2);

    // A cylinder's wall: along the axis it is straight, and a leave direction that is normal to it
    // has no tangent plane to work in.
    let c = cylinder(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 10.0, 30.0).unwrap();
    let wall = face_at(&c, Vec3::new(10.0, 0.0, 15.0));
    let ring = loop_of(&wall);
    let f = frames(&wall, &ring, Vec3::new(0.0, 0.0, 1.0), 5).unwrap();
    for s in &f {
        assert!(s.bend.len() < 1e-6, "the axis direction is straight: {s:?}");
    }
    // A leave direction that is normal to the support surface has no tangent plane to work in.
    assert!(frames(&top, &rim, Vec3::new(0.0, 0.0, 1.0), 9).is_err(), "a face's own normal is no direction to leave in");
    // On the cylinder's wall only a direction with a tangential part is usable.
    assert!(frames(&wall, &ring, Vec3::new(1.0, 0.0, 0.0), 5).is_ok());
}

#[test]
fn a_g1_bridge_meets_both_boundaries_tangentially() {
    let (a, b) = two_planes(9);
    let body = bridge(&a, &b, [Continuity::G1, Continuity::G1], [0.0, 0.0]).unwrap();
    for u in [0.0, 0.5, 1.0] {
        let (_, t, _, _) = section_at(&body, u);
        assert!(angle_deg(t, Vec3::new(0.0, 1.0, 0.0)) < 0.01, "the bridge leaves along the tangent: {u}");
    }
    // The boundary itself is on each support: the first and last rows of the grid.
    let grid = sample_grid(&body, 9, 7).unwrap();
    for (row, frame) in grid.iter().zip(&a) {
        assert!(row[0].dist(frame.p) < 1e-9, "{:?} vs {:?}", row[0], frame.p);
    }
    for (row, frame) in grid.iter().zip(&b) {
        assert!(row[6].dist(frame.p) < 1e-9, "{:?} vs {:?}", row[6], frame.p);
    }
}

#[test]
fn g2_and_g3_bridges_repeat_the_support_shape() {
    let (a, b) = valley_between_cylinders(9, 10.0);
    // A G2 bridge to two cylinders: each end holds the cylinder's own curvature (1/10 towards its
    // axis) and leaves along its tangent.
    let g2 = bridge(&a, &b, [Continuity::G2, Continuity::G2], [0.0, 0.0]).unwrap();
    for (i, u) in [(0usize, 0.0), (4, 0.5), (8, 1.0)] {
        let (p, t, bend, _) = section_at(&g2, u);
        assert!(p.dist(a[i].p) < 1e-9, "the boundary is on the cylinder: {p:?}");
        assert!(angle_deg(t, Vec3::new(0.0, 1.0, 0.0)) < 0.01, "leaves along the tangent: {u}");
        assert!((bend.dist(a[i].bend) / a[i].bend.len()) < 1e-3, "curvature {bend:?} vs {:?}", a[i].bend);
    }
    // The far end, read back to front.
    let (p, t, bend, _) = section_end_at(&g2, 1.0);
    assert!(p.dist(b[8].p) < 1e-9, "the far boundary is on its cylinder: {p:?}");
    assert!(angle_deg(t, Vec3::new(0.0, 1.0, 0.0)) < 0.01, "arrives along the far tangent");
    assert!((bend.dist(b[8].bend) / b[8].bend.len()) < 1e-3, "curvature {bend:?} vs {:?}", b[8].bend);

    // A G2 bridge leaves the change of curvature free; asking for G3 pins its normal part to the
    // cylinder's own (zero: a cylinder's curvature does not change as you go round it).
    let (_, _, _, free) = section_at(&g2, 0.0);
    let g3 = bridge(&a, &b, [Continuity::G3, Continuity::G3], [0.0, 0.0]).unwrap();
    let (_, _, bend3, held) = section_at(&g3, 0.0);
    assert!((bend3.dist(a[0].bend) / a[0].bend.len()) < 1e-3, "G3 still holds the curvature: {bend3:?}");
    let along_normal = |v: Vec3| v.dot(Vec3::new(0.0, 0.0, 1.0));
    assert!(along_normal(held).abs() < 1e-3, "a G3 bridge holds the change of curvature: {held:?}");
    assert!(along_normal(free).abs() > 1e-3, "a G2 bridge is free to change it: {free:?}");
}

#[test]
fn crown_shapes_the_middle_without_touching_the_ends() {
    let (a, b) = valley_between_cylinders(11, 10.0);
    let base = bridge(&a, &b, [Continuity::G2, Continuity::G2], [0.0, 0.0]).unwrap();
    let (_, t0, bend0, _) = section_at(&base, 0.0);
    let mut heights = Vec::new();
    for crown in [-0.5, 0.0, 1.0, 2.0] {
        let body = bridge(&a, &b, [Continuity::G2, Continuity::G2], [crown, crown]).unwrap();
        let grid = sample_grid(&body, 11, 9).unwrap();
        heights.push(grid[5][4].z);
        // The ends hold in every case: the same boundary points, the same tangent, and the same
        // seam curvature as the uncrowned bridge.
        let (_, t, bend, _) = section_at(&body, 0.0);
        assert!(angle_deg(t, t0) < 0.01, "crown {crown} left the tangent alone");
        assert!(bend.dist(bend0) < 1e-9, "crown {crown} left the curvature alone: {bend:?} vs {bend0:?}");
        assert!(grid[5][0].dist(a[5].p) < 1e-9 && grid[5][8].dist(b[5].p) < 1e-9, "crown {crown} left the boundary alone");
    }
    let steps: Vec<f64> = heights.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(steps.iter().all(|d| d.abs() > 1e-6 && d.signum() == steps[0].signum()), "a fuller crown moves the middle one way: {heights:?}");
}

#[test]
fn the_bridge_refuses_what_it_cannot_hold() {
    let (a, b) = two_planes(9);
    // Different station counts.
    assert!(bridge(&a[..6], &b, [Continuity::G1, Continuity::G1], [0.0, 0.0]).is_err());
    // A crown outside its range, and a crown that is not a number.
    for crown in [f64::NAN, 5.0, -1.0, f64::INFINITY] {
        assert!(bridge(&a, &b, [Continuity::G1, Continuity::G1], [crown, crown]).is_err(), "{crown}");
    }
    // Boundaries that meet: no room for a bridge.
    let same: Vec<Frame> = a.iter().map(|f| Frame { p: f.p, ..*f }).collect();
    assert!(bridge(&a, &same, [Continuity::G1, Continuity::G1], [0.0, 0.0]).is_err());
    // A station that is not a number.
    let mut bad = a.clone();
    bad[3].leave = Vec3::new(f64::NAN, 0.0, 0.0);
    assert!(bridge(&bad, &b, [Continuity::G1, Continuity::G1], [0.0, 0.0]).is_err());
    // Fewer than two stations.
    assert!(bridge(&a[..1], &b[..1], [Continuity::G1, Continuity::G1], [0.0, 0.0]).is_err());
    // Every order from G0 to G3 builds.
    for (x, y) in [(Continuity::G0, Continuity::G0), (Continuity::G1, Continuity::G0), (Continuity::G3, Continuity::G1)] {
        let body = bridge(&a, &b, [x, y], [0.0, 0.0]).unwrap();
        let (_, t, _, _) = section_at(&body, 0.0);
        if x != Continuity::G0 {
            assert!(angle_deg(t, Vec3::new(0.0, 1.0, 0.0)) < 0.01, "{x:?}");
        }
    }
}

#[test]
fn continuity_parses_its_words() {
    assert_eq!(Continuity::parse("g0"), Some(Continuity::G0));
    assert_eq!(Continuity::parse("G3"), Some(Continuity::G3));
    assert_eq!(Continuity::parse("g9"), None);
    assert_eq!(Continuity::parse(""), None);
    assert_eq!([Continuity::G0, Continuity::G1, Continuity::G2, Continuity::G3].map(Continuity::conditions), [1, 2, 3, 4]);
}
