//! Untrim (Surface › Untrim): a face restored towards the surface it lies on. Picked inner
//! loops (holes), or all of them, are taken away; optionally the outer boundary goes back to
//! the surface's natural parameter bounds: a free-form surface's own parameter box, and for an
//! analytic surface (a plane, cylinder, cone, sphere or torus, which has no natural bounds of
//! its own, or bounds that go all the way round) the box of the face's own span. The restored
//! boundary runs along the surface's parameter lines, each edge exact: a line, an arc about a
//! revolved surface's axis, a rotated profile, or a B-spline iso-curve.
//!
//! Which loop is the outer one is worked out from the face's parameters ([`FaceLoops`]).

use mt::{BoundedCurve, MetricSpace, ParametricCurve, ParametricSurface};
use solidcraft_geom::Vec3;
use truck_modeling as mt;

use crate::body::Body;
use crate::face_params::{FaceLoops, iso_curve, natural_span, passes};
use crate::record::{Rebuild, rebuilt_body};
use crate::surfaces::surface_body;
use crate::{KernelError, Result, guard};

/// The inner loops Untrim takes away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UntrimHoles<'a> {
    /// Every inner loop of the face.
    All,
    /// The inner loops nearest these points (on or near their edges); none keeps every hole.
    At(&'a [Vec3]),
}

/// Face `face` of `body` (in its face order) untrimmed: the inner loops `holes` names taken
/// away and, with `outer`, the outer boundary restored to the surface's natural bounds (an
/// analytic surface's: the face's own span). The result is a surface body: the untrimmed face on
/// its own, or, with `in_place` and a surface `body`, the body with that face untrimmed (a
/// solid's face is always copied out). Its history is over `[body]`: the untrimmed face is the
/// face it came from, trimmed differently, so it keeps that face's name.
pub fn untrim(body: &Body, face: usize, holes: UntrimHoles, outer: bool, in_place: bool) -> Result<Body> {
    body.require_brep("untrim")?;
    if let UntrimHoles::At(points) = holes {
        if points.len() > 10_000 {
            return Err(KernelError::Invalid("too many holes picked".into()));
        }
        if points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite())) {
            return Err(KernelError::Invalid("a hole point is not finite".into()));
        }
    }
    let faces: Vec<&mt::Face> = body.solid.face_iter().collect();
    let original = *faces.get(face).ok_or_else(|| KernelError::Invalid(format!("the body has no face {face}")))?;
    let size = body.size().max(1e-6);
    let untrimmed = guard("untrim", || untrimmed_face(original, holes, outer, size))?;
    let mut rec = Rebuild::default();
    rec.face(&untrimmed, original);
    let result = if in_place && body.is_surface() {
        surface_body(faces.iter().map(|f| if f.id() == original.id() { untrimmed.clone() } else { (*f).clone() }).collect())
    } else {
        surface_body(vec![untrimmed])
    };
    Ok(rebuilt_body(result, &[&body.solid], &rec))
}

/// The face untrimmed (see [`untrim`]).
fn untrimmed_face(face: &mt::Face, holes: UntrimHoles, outer: bool, size: f64) -> Result<mt::Face> {
    let surface = face.surface();
    let accept = (size * 1e-5).max(1e-6);
    let FaceLoops { eval, dom, wires, loops, outer: outer_index } = FaceLoops::of(face, accept, "untrimming")?;
    let inner: Vec<usize> = (0..wires.len()).filter(|i| *i != outer_index).collect();
    let removed: Vec<usize> = match holes {
        UntrimHoles::All => inner.clone(),
        UntrimHoles::At(points) => {
            let mut picked = Vec::new();
            for p in points {
                let (i, d) = inner
                    .iter()
                    .filter_map(|i| loops.get(*i).map(|l| (*i, l.dist(*p))))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .ok_or_else(|| KernelError::Invalid("the face has no holes".into()))?;
                if d > size * 0.05 + 1e-6 {
                    return Err(KernelError::Invalid(format!("no hole of the face at {p:?}")));
                }
                if !picked.contains(&i) {
                    picked.push(i);
                }
            }
            picked
        }
    };
    if !outer && removed.is_empty() {
        return Err(KernelError::Invalid(if inner.is_empty() {
            "the face has no holes to remove (restore its outer boundary instead)".into()
        } else {
            "nothing to untrim: pick holes to remove or restore the outer boundary".into()
        }));
    }
    let (Some(outer_wire), Some(outer_loop)) = (wires.get(outer_index), loops.get(outer_index)) else {
        return Err(KernelError::Invalid("the face has no boundary".into()));
    };
    let first = if outer {
        let analytic = crate::offset::surf_of(face, (size * 1e-7).max(1e-9) * 100.0).is_some();
        let span = natural_span(&eval, &dom, outer_loop, analytic, accept)?;
        bounds_wire(&surface, span, outer_loop.area() > 0.0, size)?
    } else {
        outer_wire.clone()
    };
    let kept = inner.iter().filter(|i| !removed.contains(i)).filter_map(|i| wires.get(*i).cloned());
    let boundaries: Vec<mt::Wire> = std::iter::once(first).chain(kept).collect();
    let mut f = mt::Face::try_new(boundaries, surface).map_err(|e| KernelError::Failed(format!("untrimmed face: {e}")))?;
    if !face.orientation() {
        f.invert();
    }
    Ok(f)
}

/// The closed boundary of the parameter box `span` on `surface`, counter-clockwise in its
/// parameters when `ccw`: four parameter-line edges, fewer where a side shrinks to a point (a
/// pole: the corners at its ends are one vertex).
fn bounds_wire(surface: &mt::Surface, span: [(f64, f64); 2], ccw: bool, size: f64) -> Result<mt::Wire> {
    let [(u0, u1), (v0, v1)] = span;
    let tol = size * 1e-6 + 1e-9;
    // The corners counter-clockwise from (u0, v0); each side runs between two of them.
    let mut corners = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
    if !ccw {
        corners.reverse();
    }
    let points: Vec<mt::Point3> = corners.iter().map(|(u, v)| surface.subs(*u, *v)).collect();
    if points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite())) {
        return Err(KernelError::Failed("the surface cannot be evaluated at its bounds".into()));
    }
    // Corners at one point (a pole) are one vertex.
    let mut vertices: Vec<mt::Vertex> = Vec::with_capacity(4);
    for (i, p) in points.iter().enumerate() {
        let same = points.iter().take(i).position(|q| q.distance(*p) <= tol).and_then(|j| vertices.get(j).cloned());
        vertices.push(same.unwrap_or_else(|| mt::Vertex::new(*p)));
    }
    let mut edges = Vec::with_capacity(4);
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (Some(&(ua, va)), Some(&(ub, vb)), Some(a), Some(b)) = (corners.get(i), corners.get(j), vertices.get(i), vertices.get(j)) else {
            continue;
        };
        let along_u = va == vb;
        let (w, from, to) = if along_u { (va, ua, ub) } else { (ua, va, vb) };
        let mid = if along_u { surface.subs(0.5 * (from + to), w) } else { surface.subs(w, 0.5 * (from + to)) };
        if a.id() == b.id() {
            // A side shrunk to a pole is no edge; one that closes on itself can't be bounded.
            if mid.distance(a.point()) > tol {
                return Err(KernelError::TooLarge("a surface that closes on itself within the face's bounds".into()));
            }
            continue;
        }
        let curve = iso_curve(surface, along_u, w, from, to)?;
        let close = |p: mt::Point3, q: mt::Point3| p.distance(q) <= tol * 10.0;
        let (t0, t1) = curve.range_tuple();
        if !close(curve.subs(t0), a.point()) || !close(curve.subs(t1), b.point()) || !passes(&curve, mid, tol * 10.0) {
            return Err(KernelError::Failed("a restored boundary edge does not follow its surface".into()));
        }
        edges.push(mt::Edge::try_new(a, b, curve).map_err(|e| KernelError::Failed(format!("restored boundary edge: {e}")))?);
    }
    if edges.len() < 2 {
        return Err(KernelError::Invalid("the face's bounds shrink to a point".into()));
    }
    Ok(edges.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extrude, measure};
    use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2};
    use std::f64::consts::PI;

    fn rel(a: f64, b: f64) -> f64 {
        (a - b).abs() / b.abs().max(1e-9)
    }

    fn inner_loops(b: &Body) -> usize {
        b.solid.face_iter().map(|f| f.boundaries().len().saturating_sub(1)).sum()
    }

    /// A 40 × 20 × 2 plate with three holes of radius 2 at x = 10, 20, 30 on y = 10.
    fn plate() -> Body {
        let outer = Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), Vec2::new(40.0, 20.0), Vec2::new(0.0, 20.0)]);
        let holes = [10.0, 20.0, 30.0].iter().map(|x| Loop2::circle(Vec2::new(*x, 10.0), 2.0).reversed()).collect();
        extrude(&Plane::XY, &[Region2 { outer, holes }], 0.0, 2.0).unwrap().pop().unwrap()
    }

    fn face_at(b: &Body, p: Vec3) -> usize {
        crate::surfaces::nearest_face(b, p).unwrap()
    }

    /// The plate's top with every hole taken away is the whole 40 × 20 rectangle, one face with
    /// no inner loops; one hole taken away leaves two (area 800 − 2 · 4π); the face's own loops
    /// are kept by identity. Nothing to take away is refused.
    #[test]
    fn plate_with_three_holes_untrimmed_to_its_rectangle() {
        let b = plate();
        let top = face_at(&b, Vec3::new(5.0, 5.0, 2.0));
        let all = untrim(&b, top, UntrimHoles::All, false, false).unwrap();
        assert!(all.is_surface() && all.face_count() == 1 && inner_loops(&all) == 0);
        assert!(rel(measure(&all).unwrap().area, 800.0) < 1e-9, "{}", measure(&all).unwrap().area);
        let one = untrim(&b, top, UntrimHoles::At(&[Vec3::new(22.0, 10.0, 2.0)]), false, false).unwrap();
        assert_eq!(inner_loops(&one), 2);
        let want = 800.0 - 2.0 * 4.0 * PI;
        assert!(rel(measure(&one).unwrap().area, want) < 1e-3, "{} vs {want}", measure(&one).unwrap().area); // (measured on meshes)
        // The plate is untouched (three holes through its top and bottom); the history says the
        // face is the top, trimmed differently.
        assert_eq!(inner_loops(&b), 6);
        let h = all.history().unwrap();
        assert_eq!(h.faces.first().map(|f| f.change), Some(crate::history::Change::Modified));
        // Nothing to do: a face without holes, a point on no hole, an empty pick.
        let side = b.solid.face_iter().position(|f| f.boundaries().len() == 1).unwrap();
        assert!(untrim(&b, side, UntrimHoles::All, false, false).is_err());
        assert!(untrim(&b, top, UntrimHoles::At(&[Vec3::new(5.0, 5.0, 2.0)]), false, false).is_err());
        assert!(untrim(&b, top, UntrimHoles::At(&[]), false, false).is_err());
        assert!(untrim(&b, 99, UntrimHoles::All, false, false).is_err());
    }

    /// A face whose hole is stored before its outer loop (as an imported face may list them):
    /// the outer loop is the one enclosing the most, so the 10 × 10 square comes back whole.
    #[test]
    fn the_outer_loop_is_found_whatever_order_the_loops_are_in() {
        let r = Region2 {
            outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)]),
            holes: vec![Loop2::polygon(&[Vec2::new(4.0, 4.0), Vec2::new(4.0, 6.0), Vec2::new(6.0, 6.0), Vec2::new(6.0, 4.0)])],
        };
        let sheet = crate::patch_region(&Plane::XY, &r).unwrap();
        let f = sheet.solid.face_iter().next().unwrap().clone();
        let mut loops = f.absolute_boundaries().clone();
        loops.reverse();
        let swapped = mt::Face::new_unchecked(loops, f.surface());
        let swapped = if f.orientation() { swapped } else { swapped.inverse() };
        let b = surface_body(vec![swapped]);
        assert!(rel(measure(&b).unwrap().area, 96.0) < 1e-9);
        let back = untrim(&b, 0, UntrimHoles::All, false, true).unwrap();
        assert!(back.face_count() == 1 && inner_loops(&back) == 0);
        assert!(rel(measure(&back).unwrap().area, 100.0) < 1e-9, "{}", measure(&back).unwrap().area);
    }

    /// A half cylinder sheet (radius 10, angles 0…π, height 20) trimmed by the slanted plane
    /// z = 15 − y/2: its top edge runs from 15 down to 10 and back, its area ∫ (15 − 5 sin θ) r dθ
    /// = 150π − 100. Restored, it fills its own span again, angles 0…π and heights 0…15: 150π,
    /// on the same cylinder.
    #[test]
    fn a_trimmed_cylinder_face_is_restored_to_its_full_span() {
        let arc = vec![Seg2::Arc { center: Vec2::new(0.0, 0.0), radius: 10.0, start: 0.0, sweep: PI }];
        let sheet = crate::extrude_sheet(&Plane::XY, &[arc], 0.0, 20.0).unwrap();
        let tool = crate::SplitTool::Plane(Plane::from_normal(Vec3::new(0.0, 0.0, 15.0), Vec3::new(0.0, 0.5, 1.0)).unwrap());
        let cut = crate::trim(&sheet, &tool, Vec3::new(0.0, 10.0, 1.0)).unwrap();
        assert_eq!(cut.face_count(), 1);
        let trimmed = measure(&cut).unwrap().area;
        assert!(rel(trimmed, 150.0 * PI - 100.0) < 1e-3, "{trimmed}"); // (measured on meshes)
        // Nothing to take away and the outer boundary kept: nothing to do.
        assert!(untrim(&cut, 0, UntrimHoles::All, false, false).is_err());
        let restored = untrim(&cut, 0, UntrimHoles::All, true, false).unwrap();
        assert!(restored.face_count() == 1 && inner_loops(&restored) == 0);
        let a = measure(&restored).unwrap().area;
        assert!(rel(a, 150.0 * PI) < 1e-3, "{a} vs {}", 150.0 * PI); // (measured on meshes)
        let bb = measure(&restored).unwrap().bbox;
        assert!(bb.min.z.abs() < 1e-9 && (bb.max.z - 15.0).abs() < 1e-6, "{bb:?}");
        assert!(matches!(
            crate::face_surface(&restored, 0),
            Some(crate::FaceSurface::Cylinder { radius, .. }) if (radius - 10.0).abs() < 1e-6
        ));
    }

    /// A revolved face: the cone frustum swept by the line from radius 5 at z = 0 to radius 10
    /// at z = −10 (slant √125), its first face (angles 0…½), trimmed by the plane z = −8 + y/2.
    /// The cut reaches lowest (profile parameter 0.8) at angle 0, so the face's own span is
    /// angles 0…½ by parameters 0…0.8, restored with arcs about the axis and straight sides:
    /// ½ · √125 · ∫₀^0.8 (5 + 5t) dt = ½ · √125 · 5.6.
    #[test]
    fn a_trimmed_cone_face_is_restored_to_its_span() {
        let line = vec![Seg2::Line { a: Vec2::new(5.0, 0.0), b: Vec2::new(10.0, 10.0) }];
        let sheet = crate::revolve_sheet(&Plane::XZ, &[line], Vec2::ZERO, Vec2::Y, 0.0, 1.0).unwrap();
        let tool = crate::SplitTool::Plane(Plane::from_normal(Vec3::new(0.0, 0.0, -8.0), Vec3::new(0.0, -0.5, 1.0)).unwrap());
        let (c, s) = (0.25f64.cos(), 0.25f64.sin());
        let cut = crate::trim(&sheet, &tool, Vec3::new(5.0 * c, 5.0 * s, 0.0)).unwrap();
        let face = face_at(&cut, Vec3::new(5.5 * c, 5.5 * s, -1.0));
        let restored = untrim(&cut, face, UntrimHoles::All, true, false).unwrap();
        let want = 0.5 * 125f64.sqrt() * 5.6;
        let a = measure(&restored).unwrap().area;
        assert!(rel(a, want) < 1e-3, "{a} vs {want}"); // (measured on meshes)
        let bb = measure(&restored).unwrap().bbox;
        assert!((bb.min.z + 8.0).abs() < 1e-6 && bb.max.z.abs() < 1e-6, "{bb:?}");
        assert!(matches!(crate::face_surface(&restored, 0), Some(crate::FaceSurface::Cone { .. })));
    }

    /// A torus face (a revolved arc) with a piece of its outer equator cut off by the plane
    /// x = 12: the cut touches one side of the face only, so its span is the face's whole span
    /// and the restored face is the face as it was before the cut.
    #[test]
    fn a_torus_face_cut_at_one_side_is_restored_whole() {
        let torus = crate::torus(Vec3::ZERO, 10.0, 3.0).unwrap();
        let f = torus.solid.face_iter().next().unwrap().clone();
        let face = surface_body(vec![f]);
        let before = measure(&face).unwrap().area;
        let tool = crate::SplitTool::Plane(Plane::from_normal(Vec3::new(12.0, 0.0, 0.0), Vec3::X).unwrap());
        let cut = crate::trim(&face, &tool, Vec3::new(0.0, 13.0, 0.0)).unwrap();
        let trimmed = measure(&cut).unwrap().area;
        assert!(trimmed < before * 0.99, "{trimmed} vs {before}");
        let restored = untrim(&cut, 0, UntrimHoles::All, true, false).unwrap();
        let a = measure(&restored).unwrap().area;
        assert!(rel(a, before) < 1e-3, "{a} vs {before}"); // (measured on meshes)
        assert!(matches!(
            crate::face_surface(&restored, 0),
            Some(crate::FaceSurface::Torus { major, minor, .. }) if (major - 10.0).abs() < 1e-6 && (minor - 3.0).abs() < 1e-6
        ));
    }

    /// A free-form face goes back to its surface's own parameter box: half of the saddle
    /// z = x y / 10 over 0…10 × 0…10 (a bilinear patch, its iso-lines straight), x from 0 to 5,
    /// restored to the whole patch: x and y 0…10, z 0…10.
    #[test]
    fn a_free_form_face_is_restored_to_its_surfaces_bounds() {
        let pt = |x: f64, y: f64| mt::Point3::new(x, y, x * y / 10.0);
        let knots = mt::KnotVec::bezier_knot(1);
        let saddle = mt::BSplineSurface::new((knots.clone(), knots), vec![vec![pt(0.0, 0.0), pt(0.0, 10.0)], vec![pt(10.0, 0.0), pt(10.0, 10.0)]]);
        let corners = [pt(0.0, 0.0), pt(5.0, 0.0), pt(5.0, 10.0), pt(0.0, 10.0)];
        let v: Vec<mt::Vertex> = corners.iter().map(|p| mt::Vertex::new(*p)).collect();
        let edges: Vec<mt::Edge> =
            (0..4).map(|i| mt::Edge::new_unchecked(&v[i], &v[(i + 1) % 4], mt::Curve::Line(mt::Line(corners[i], corners[(i + 1) % 4])))).collect();
        let half = surface_body(vec![mt::Face::new_unchecked(vec![edges.into()], mt::Surface::BSplineSurface(saddle))]);
        assert!(crate::face_surface(&half, 0).is_none(), "not analytic");
        assert!((measure(&half).unwrap().bbox.max.x - 5.0).abs() < 1e-9);
        let whole = untrim(&half, 0, UntrimHoles::All, true, false).unwrap();
        let bb = measure(&whole).unwrap().bbox;
        assert!(bb.min.x.abs() < 1e-9 && bb.min.y.abs() < 1e-9 && bb.min.z.abs() < 1e-9, "{bb:?}");
        assert!((bb.max.x - 10.0).abs() < 1e-9 && (bb.max.y - 10.0).abs() < 1e-9 && (bb.max.z - 10.0).abs() < 1e-9, "{bb:?}");
        assert_eq!(whole.solid.face_iter().next().unwrap().boundaries().first().map(|w| w.len()), Some(4));
    }

    /// The plate exported to STEP and read back: its imported top face untrims to the whole
    /// rectangle as the plate's own did.
    #[test]
    fn an_imported_step_face_is_untrimmed() {
        let text = crate::step_export(&[&plate()], "untrim test").unwrap();
        let imported = crate::step_import(&text).unwrap();
        let b = &imported.bodies.first().unwrap().body;
        let top = face_at(b, Vec3::new(5.0, 5.0, 2.0));
        assert_eq!(b.solid.face_iter().nth(top).unwrap().boundaries().len(), 4);
        let all = untrim(b, top, UntrimHoles::All, false, false).unwrap();
        assert!(all.face_count() == 1 && inner_loops(&all) == 0);
        assert!(rel(measure(&all).unwrap().area, 800.0) < 1e-9);
        let restored = untrim(b, top, UntrimHoles::All, true, false).unwrap();
        assert!(rel(measure(&restored).unwrap().area, 800.0) < 1e-9);
    }

    /// In place: a surface body of the plate's top and bottom keeps its bottom (and its three
    /// holes), the top replaced by the whole rectangle: 800 + 800 − 3 · 4π. A solid's face is
    /// copied out whatever `in_place` says.
    #[test]
    fn in_place_untrims_a_surface_and_copies_a_solids_face() {
        let b = plate();
        let top = face_at(&b, Vec3::new(5.0, 5.0, 2.0));
        let copied = untrim(&b, top, UntrimHoles::All, false, true).unwrap();
        assert_eq!(copied.face_count(), 1);
        let sheet = crate::copy_faces(&b, &[Vec3::new(5.0, 5.0, 2.0), Vec3::new(5.0, 5.0, 0.0)]).unwrap();
        assert!(sheet.is_surface() && sheet.face_count() == 2 && inner_loops(&sheet) == 6);
        let top = face_at(&sheet, Vec3::new(5.0, 5.0, 2.0));
        let whole = untrim(&sheet, top, UntrimHoles::All, false, true).unwrap();
        assert!(whole.is_surface());
        assert_eq!((whole.face_count(), inner_loops(&whole)), (2, 3));
        let want = 1600.0 - 3.0 * 4.0 * PI;
        assert!(rel(measure(&whole).unwrap().area, want) < 1e-3, "{} vs {want}", measure(&whole).unwrap().area); // (measured on meshes)
    }

    /// Points that are not finite and too many picks are refused, never panic.
    #[test]
    fn hostile_inputs_are_refused() {
        let b = plate();
        let top = face_at(&b, Vec3::new(5.0, 5.0, 2.0));
        assert!(untrim(&b, top, UntrimHoles::At(&[Vec3::new(f64::NAN, 0.0, 0.0)]), false, false).is_err());
        let many = vec![Vec3::ZERO; 10_001];
        assert!(untrim(&b, top, UntrimHoles::At(&many), false, false).is_err());
    }
}
