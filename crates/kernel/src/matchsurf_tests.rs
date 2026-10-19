//! Tests of surface matching: a sheet is rebuilt to meet a cylinder's wall up to G1, G2 and G3,
//! its seam, its far boundary and its interior are measured, and what cannot be matched is
//! refused.

use super::*;
use crate::bridge::{Continuity, Frame};
use crate::build::SEAM_ANGLE;
use crate::{cylinder, stitch};
use solidcraft_geom::Vec3;
use truck_modeling as mt;

/// The radius of the cylinder a sheet is matched to, the radius the sheet starts at, its height,
/// and how far round the cylinder the sheet runs.
const R_TARGET: f64 = 10.0;
const R_SHEET: f64 = 11.0;
const HEIGHT: f64 = 30.0;
const SPAN: f64 = 0.6;

/// A point on the vertical line at radius `r`, angle `a`, height `z` (Z up).
fn at(a: f64, r: f64, z: f64) -> Vec3 {
    Vec3::new(r * a.cos(), r * a.sin(), z)
}

/// The outward radial and the circumferential unit directions at the angle `a`.
fn radial(a: f64) -> Vec3 {
    Vec3::new(a.cos(), a.sin(), 0.0)
}

fn circ(a: f64) -> Vec3 {
    Vec3::new(-a.sin(), a.cos(), 0.0)
}

/// A cylindrical sheet of radius `r` between the angles `a0` and `a1`, `stations` high: the body
/// a match rebuilds. Its near boundary is the vertical line at `a0`, its far boundary at `a1`.
fn cylinder_sheet(r: f64, a0: f64, a1: f64, stations: usize) -> Body {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..stations {
        let z = HEIGHT * i as f64 / (stations - 1) as f64;
        a.push(Frame { p: at(a0, r, z), leave: circ(a0), bend: radial(a0) * (-1.0 / r), bend_change: Vec3::ZERO });
        b.push(Frame { p: at(a1, r, z), leave: circ(a1) * -1.0, bend: radial(a1) * (-1.0 / r), bend_change: Vec3::ZERO });
    }
    bridge::bridge(&a, &b, [Continuity::G2, Continuity::G2], [0.0, 0.0]).unwrap()
}

/// The stock cylinder, and two picks on the vertical seam line of its wall.
fn cylinder_and_picks() -> (Body, [Vec3; 2]) {
    let c = cylinder(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), R_TARGET, HEIGHT).unwrap();
    (c, [at(SEAM_ANGLE, R_TARGET, 6.0), at(SEAM_ANGLE, R_TARGET, 24.0)])
}

/// The sheet of the usual shape, and a pick on its near boundary.
fn sheet_and_pick() -> (Body, [Vec3; 1]) {
    (cylinder_sheet(R_SHEET, SEAM_ANGLE, SEAM_ANGLE + SPAN, 9), [at(SEAM_ANGLE, R_SHEET, HEIGHT * 0.5)])
}

/// The middle of the edge of `body`'s first face nearest `p`.
fn edge_at(body: &Body, p: Vec3) -> mt::Edge {
    let face = body.solid.face_iter().next().unwrap();
    let mut best: Option<(f64, mt::Edge)> = None;
    for e in face.edge_iter() {
        let c = e.curve();
        let (t0, t1) = c.range_tuple();
        let m = from_p3(c.subs(0.5 * (t0 + t1)));
        let d = m.dist(p);
        if best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, e.clone()));
        }
    }
    best.unwrap().1
}

/// How far the rebuilt face's far boundary lies from the original one, in mm: the far row of the
/// rebuilt surface sampled at its own stations, against the original boundary edge's own points.
fn far_drift(made: &Body, sheet: &Body, far_mid: Vec3) -> f64 {
    let face = made.solid.face_iter().next().unwrap();
    let surf = face.surface();
    let (Some((u0, u1)), Some((_, v1))) = surf.try_range_tuple() else { panic!("unbounded") };
    let edge = edge_at(sheet, far_mid);
    let poly = chain_polyline(&[edge]).unwrap();
    let mut worst = 0.0f64;
    for k in 0..=8 {
        let p = from_p3(surf.subs(u0 + (u1 - u0) * k as f64 / 8.0, v1));
        worst = worst.max(chain_distance(&poly, p));
    }
    worst
}

/// What the rebuilt sheet does at the normalized station `t` of its matching boundary: the
/// point, the unit tangent, the curvature vector and the change of that curvature, read off the
/// finished surface (never from the construction).
fn seam_at(body: &Body, t: f64) -> (Vec3, Vec3, Vec3, Vec3) {
    let face = body.solid.face_iter().next().unwrap();
    let ranges = super::ranges(face).unwrap();
    super::seam_shape(&face.surface(), &ranges, Side::V0, t).unwrap()
}

/// The stations a seam is measured at, as fractions of its length.
fn stations() -> Vec<f64> {
    (0..9).map(|k| k as f64 / 8.0).collect()
}

#[test]
fn a_g2_match_meets_a_cylinder_with_its_own_curvature() {
    let (target, picks) = cylinder_and_picks();
    let (sheet, pick) = sheet_and_pick();
    let made = match_surface(&sheet, &pick, &target, &picks, Continuity::G2).unwrap();
    // The rebuilt boundary has moved onto the picked line: the seam is on the cylinder's wall.
    assert!(seam_at(&made, 0.5).0.dist(at(SEAM_ANGLE, R_TARGET, HEIGHT * 0.5)) < 1e-6, "the seam is on the picked line");
    let (gap, deg, order) = seam_continuity(&made, &target, &picks).unwrap();
    println!("g2: gap {gap:e} mm, normal {deg:e} deg, order {order:?}");
    assert!(gap < 1e-6, "position gap {gap:e}");
    assert!(deg < 0.01, "normal angle {deg}");
    assert!(order.conditions() >= Continuity::G2.conditions(), "{order:?}");
    // The curvature the cylinder bends with along the seam: matched to a thousandth of itself.
    let want = radial(SEAM_ANGLE) * (-1.0 / R_TARGET);
    let mut worst = 0.0f64;
    for t in stations() {
        worst = worst.max(seam_at(&made, t).2.dist(want) * R_TARGET);
    }
    println!("g2: worst curvature jump {worst:e} of the cylinder's own");
    assert!(worst < 1e-3, "curvature jump {worst:e}");
    // The far boundary did not move: it is built through the original boundary's own points and
    // stays on the original edge between them to round-off.
    let far = at(SEAM_ANGLE + SPAN, R_SHEET, HEIGHT * 0.5);
    let drift = far_drift(&made, &sheet, far);
    println!("g2: far boundary drift {drift:e} mm");
    assert!(drift < 1e-9, "the far boundary drifted {drift:e} mm");
    // The interior stays near the original: the crowning that leaves it closest.
    let (face, original) = (made.solid.face_iter().next().unwrap().clone(), sheet.solid.face_iter().next().unwrap().clone());
    let stray = deviation(&face, &original).unwrap();
    println!("g2: interior stray {stray:.6} mm rms");
    // The seam moved 1 mm, so the face cannot be closer than that at the seam itself: over the
    // whole face the root mean square distance is well under half of it.
    assert!(stray < 0.6, "the interior strays {stray} mm");
}

#[test]
fn a_g1_match_holds_the_tangent_and_leaves_the_curvature_free() {
    let (target, picks) = cylinder_and_picks();
    let (sheet, pick) = sheet_and_pick();
    let made = match_surface(&sheet, &pick, &target, &picks, Continuity::G1).unwrap();
    let (gap, deg, order) = seam_continuity(&made, &target, &picks).unwrap();
    let want = radial(SEAM_ANGLE) * (-1.0 / R_TARGET);
    let worst = stations().into_iter().map(|t| seam_at(&made, t).2.dist(want) * R_TARGET).fold(0.0f64, f64::max);
    println!("g1: gap {gap:e} mm, normal {deg:e} deg, order {order:?}, curvature jump {worst:e}");
    assert!(gap < 1e-6 && deg < 0.01, "position {gap:e}, normal {deg}");
    assert_eq!(order, Continuity::G1, "a tangent match leaves the curvature free");
    assert!(worst > 1e-3, "a G1 match need not hold the curvature: {worst:e}");
    // The far boundary is kept whatever the order.
    let far = at(SEAM_ANGLE + SPAN, R_SHEET, HEIGHT * 0.5);
    assert!(far_drift(&made, &sheet, far) < 1e-9);
}

#[test]
fn a_g3_match_holds_the_change_of_curvature() {
    let (target, picks) = cylinder_and_picks();
    let (sheet, pick) = sheet_and_pick();
    let made = match_surface(&sheet, &pick, &target, &picks, Continuity::G3).unwrap();
    let (gap, deg, order) = seam_continuity(&made, &target, &picks).unwrap();
    let want = radial(SEAM_ANGLE) * (-1.0 / R_TARGET);
    let (mut bend, mut change) = (0.0f64, 0.0f64);
    for t in stations() {
        let (_, _, b, c) = seam_at(&made, t);
        bend = bend.max(b.dist(want) * R_TARGET);
        // A cylinder's curvature does not change in its own surface: the normal part of the
        // change of curvature across the seam must be zero too.
        change = change.max(c.dot(radial(SEAM_ANGLE)).abs() * R_TARGET * R_TARGET);
    }
    println!("g3: gap {gap:e} mm, normal {deg:e} deg, order {order:?}, curvature jump {bend:e}, change {change:e}");
    assert!(gap < 1e-6 && deg < 0.01, "position {gap:e}, normal {deg}");
    assert_eq!(order, Continuity::G3, "the seam holds the change of curvature");
    assert!(bend < 1e-3, "curvature jump {bend:e}");
    assert!(change < 1e-3, "change of curvature {change:e}");
    let far = at(SEAM_ANGLE + SPAN, R_SHEET, HEIGHT * 0.5);
    assert!(far_drift(&made, &sheet, far) < 1e-9);
}

#[test]
fn a_match_refuses_what_it_cannot_hold() {
    let (target, picks) = cylinder_and_picks();
    let (sheet, pick) = sheet_and_pick();
    // No pick on the body's face, no pick on the target's edges, and picks that are not numbers.
    assert!(match_surface(&sheet, &[], &target, &picks, Continuity::G1).is_err());
    assert!(match_surface(&sheet, &[Vec3::new(0.0, 0.0, 100.0)], &target, &picks, Continuity::G1).is_err());
    assert!(match_surface(&sheet, &[Vec3::new(f64::NAN, 0.0, 0.0)], &target, &picks, Continuity::G1).is_err());
    assert!(match_surface(&sheet, &vec![Vec3::ZERO; MAX_PICKS + 1], &target, &picks, Continuity::G1).is_err());
    assert!(match_surface(&sheet, &pick, &target, &[], Continuity::G1).is_err());
    // A pick that is not on the target at all, and one in the middle of its wall: no chain there.
    assert!(match_surface(&sheet, &pick, &target, &[Vec3::new(100.0, 0.0, 15.0)], Continuity::G1).is_err());
    assert!(match_surface(&sheet, &pick, &target, &[Vec3::new(10.0, 0.0, 15.0)], Continuity::G1).is_err());
    // A body that is not a surface: the stock cylinder itself.
    assert!(match_surface(&target, &[at(SEAM_ANGLE, R_TARGET, 15.0)], &target, &picks, Continuity::G1).is_err());
}

#[test]
fn a_target_that_jumps_in_curvature_supplies_only_g1() {
    // Two sheets of the target meet along a line with the same tangent plane but different
    // curvature: a cylinder's wall and the plane tangent to it there. The target itself does not
    // hold the curvature along that line, so no match may be told to.
    let wall = cylinder_sheet(R_TARGET, SEAM_ANGLE, SEAM_ANGLE + 0.5, 5);
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..5 {
        let z = HEIGHT * i as f64 / 4.0;
        let (p, q) = (at(SEAM_ANGLE, R_TARGET, z), at(SEAM_ANGLE, R_TARGET, z) + circ(SEAM_ANGLE) * 8.0);
        a.push(Frame { p, leave: circ(SEAM_ANGLE), bend: Vec3::ZERO, bend_change: Vec3::ZERO });
        b.push(Frame { p: q, leave: circ(SEAM_ANGLE) * -1.0, bend: Vec3::ZERO, bend_change: Vec3::ZERO });
    }
    let plane = bridge::bridge(&a, &b, [Continuity::G2, Continuity::G2], [0.0, 0.0]).unwrap();
    let joined = stitch(&[&wall, &plane], 1e-6).unwrap();
    assert_eq!(joined.solid.face_iter().count(), 2, "the two sheets sewed into one body");
    let picks = [at(SEAM_ANGLE, R_TARGET, 6.0), at(SEAM_ANGLE, R_TARGET, 24.0)];
    let (sheet, pick) = sheet_and_pick();
    // The seam is smooth enough to match, so G1 goes through; the curvature jumps there, so G2
    // and G3 do not.
    assert!(match_surface(&sheet, &pick, &joined, &picks, Continuity::G1).is_ok());
    let err = match_surface(&sheet, &pick, &joined, &picks, Continuity::G2).unwrap_err().to_string();
    println!("refused g2: {err}");
    assert!(err.contains("only g1"), "{err}");
    assert!(match_surface(&sheet, &pick, &joined, &picks, Continuity::G3).is_err());
    // The plain cylinder, whose two wall faces are the same surface, supplies G3.
    let (target, plain_picks) = cylinder_and_picks();
    assert!(match_surface(&sheet, &pick, &target, &plain_picks, Continuity::G3).is_ok());
}
