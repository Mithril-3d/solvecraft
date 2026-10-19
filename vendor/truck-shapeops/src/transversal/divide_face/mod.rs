#![allow(clippy::many_single_char_names)]

use super::faces_classification::FacesClassification;
use super::history::Lineage;
use super::loops_store::*;
use rustc_hash::FxHashMap as HashMap;
use std::ops::Deref;
use truck_meshalgo::prelude::*;
use truck_topology::*;

/// SolidCraft: `p` moved into the surface's parameter range (a point found on the boundary can lie a
/// hair outside it, and a search started from there fails).
fn clamped<P, S: ParametricSurface<Point = P>>(surface: &S, p: Point2) -> Point2 {
    match surface.try_range_tuple() {
        (Some((u0, u1)), Some((v0, v1))) => Point2::new(p.x.clamp(u0, u1), p.y.clamp(v0, v1)),
        _ => p,
    }
}

fn create_parameter_boundary<P, C, S>(
    face: &Face<P, C, S>,
    wire: &Wire<P, C>,
    polys: &mut HashMap<EdgeID<C>, PolylineCurve<P>>,
    tol: f64,
    lenient: bool,
) -> Option<PolylineCurve<Point2>>
where
    P: Copy,
    C: BoundedCurve<Point = P> + ParameterDivision1D<Point = P>,
    S: Clone + ParametricSurface<Point = P> + SearchParameter<D2, Point = P> + SearchNearestParameter<D2, Point = P>,
{
    let surface = face.surface();
    let pt = wire.front_vertex().unwrap().point();
    // SolidCraft: intersection-curve points lie on the surface only within the tolerance; fall
    // back to the nearest parameter when the exact search fails.
    let p: Point2 = surface
        .search_parameter(pt, None, 100)
        .or_else(|| surface.search_nearest_parameter(pt, None, 100))?
        .into();
    let vec = wire.edge_iter().try_fold(vec![p], |mut vec, edge| {
        let poly = polys.entry(edge.id()).or_insert_with(|| {
            let curve = edge.curve();
            let div = curve.parameter_division(curve.range_tuple(), tol).1;
            PolylineCurve(div)
        });
        let mut p = *vec.last().unwrap();
        let closure = |q: &P| -> Option<Point2> {
            // SolidCraft: the points of an intersection curve on a lofted or swept surface lie on
            // the seam between its halves, a hair outside the parameter range, and Newton's
            // method from the previous point's parameter then fails (or the surface bends away
            // from it). `lenient`, the second try after the first gave up: start from the hint
            // moved into the range, and as a last resort search the whole surface.
            let hint = if lenient { clamped(&surface, p) } else { p };
            let found = surface
                .search_parameter(*q, Some(hint.into()), 100)
                .or_else(|| surface.search_nearest_parameter(*q, Some(hint.into()), 100));
            let found = if lenient {
                found
                    .or_else(|| surface.search_parameter(*q, None, 100))
                    .or_else(|| surface.search_nearest_parameter(*q, None, 100))
            } else {
                found
            };
            p = found?.into();
            Some(p)
        };
        let add: Option<Vec<Point2>> = match edge.orientation() {
            true => poly.iter().skip(1).map(closure).collect(),
            false => poly.iter().rev().skip(1).map(closure).collect(),
        };
        vec.append(&mut add?);
        Some(vec)
    })?;
    // SolidCraft: a lenient boundary whose points jump half the parameter range or more has been
    // found across a periodic surface's seam by the whole-surface search (on a cylinder the
    // points of one edge landed at both ends of its angle): it would divide the face wrongly, so
    // the face is left undivided as before.
    if lenient {
        let (u, v) = surface.try_range_tuple();
        let half = |r: Option<(f64, f64)>| r.map_or(f64::INFINITY, |(a, b)| (b - a).abs() / 2.0);
        let (hu, hv) = (half(u), half(v));
        if vec.windows(2).any(|w| (w[1].x - w[0].x).abs() >= hu || (w[1].y - w[0].y).abs() >= hv) {
            return None;
        }
    }
    Some(PolylineCurve(vec))
}

#[derive(Clone, Debug)]
struct WireChunk<'a, C> {
    poly: PolylineCurve<Point2>,
    wire: &'a BoundaryWire<Point3, C>,
}

type FaceWithShapesOpStatus<C, S> = (Face<Point3, C, S>, ShapesOpStatus);
fn divide_one_face<C, S>(
    face: &Face<Point3, C, S>,
    loops: &Loops<Point3, C>,
    tol: f64,
) -> Option<Vec<FaceWithShapesOpStatus<C, S>>>
where
    C: BoundedCurve<Point = Point3> + ParameterDivision1D<Point = Point3>,
    S: Clone + ParametricSurface3D + SearchParameter<D2, Point = Point3> + SearchNearestParameter<D2, Point = Point3>,
{
    let (mut pre_faces, mut negative_wires) = (Vec::new(), Vec::new());
    let mut map = HashMap::default();
    loops.iter().try_for_each(|wire| {
        let poly = crate::tr!(
            create_parameter_boundary(face, wire, &mut map, tol, false).or_else(|| create_parameter_boundary(face, wire, &mut map, tol, true)),
            "parameter boundary"
        );
        match poly.area() > 0.0 {
            true => pre_faces.push(vec![WireChunk { poly, wire }]),
            false => negative_wires.push(WireChunk { poly, wire }),
        }
        Some(())
    })?;
    if std::env::var_os("SHAPEOPS_TRACE").is_some() {
        for c in pre_faces.iter().flatten() {
            eprintln!("  positive loop: area {:.4} pts {} first {:?}", c.poly.area(), c.poly.len(), c.poly.front());
        }
        for c in &negative_wires {
            eprintln!("  negative loop: area {:.4} pts {} first {:?} status {:?}", c.poly.area(), c.poly.len(), c.poly.front(), c.wire.status());
        }
    }
    negative_wires.into_iter().try_for_each(|chunk| {
        let pt = chunk.poly.front();
        // SolidCraft: the innermost piece that holds the loop (nested intersection loops, such
        // as both walls of a tube crossing one face), never the loop's own twin.
        let hole = -chunk.poly.area();
        let op = crate::tr!(
            pre_faces
                .iter_mut()
                .filter(|face| face[0].poly.area() > hole * (1.0 + 1e-6) && face[0].poly.include(pt))
                .min_by(|x, y| x[0].poly.area().total_cmp(&y[0].poly.area())),
            "loop not inside any face piece"
        );
        op.push(chunk);
        Some(())
    })?;
    let vec: Vec<_> = pre_faces
        .into_iter()
        .map(|pre_face| -> Option<_> {
            let surface = face.surface();
            let op = pre_face
                .iter()
                .find(|chunk| chunk.wire.status() != ShapesOpStatus::Unknown);
            let status = match op {
                Some(chunk) => chunk.wire.status(),
                None => ShapesOpStatus::Unknown,
            };
            let wires: Vec<Wire<Point3, C>> = pre_face
                .into_iter()
                .map(|chunk| chunk.wire.deref().clone())
                .collect();
            // SolidCraft: no panics (wasm can't catch them): a wire visiting a vertex twice gives up.
            let mut new_face = crate::tr!(Face::try_new(wires, surface).ok(), "face of a wire visiting a vertex twice");
            if !face.orientation() {
                new_face.invert();
            }
            Some((new_face, status))
        })
        .collect::<Option<_>>()?;
    Some(vec)
}

pub fn divide_faces<C, S>(
    shell: &Shell<Point3, C, S>,
    loops_store: &LoopsStore<Point3, C>,
    tol: f64,
    lineage: &mut Lineage<C, S>,
) -> Option<FacesClassification<Point3, C, S>>
where
    C: BoundedCurve<Point = Point3> + ParameterDivision1D<Point = Point3>,
    S: Clone + ParametricSurface3D + SearchParameter<D2, Point = Point3> + SearchNearestParameter<D2, Point = Point3>,
{
    let mut res = FacesClassification::<Point3, C, S>::default();
    shell
        .iter()
        .zip(loops_store)
        .try_for_each(|(face, loops)| {
            if loops
                .iter()
                .all(|wire| wire.status() == ShapesOpStatus::Unknown)
            {
                res.push(face.clone(), ShapesOpStatus::Unknown);
            } else {
                let vec = crate::tr!(divide_one_face(face, loops, tol), "divide one face");
                vec.into_iter().for_each(|(piece, status)| {
                    lineage.face_piece(face.id(), piece.id());
                    res.push(piece, status)
                });
            }
            Some(())
        })?;
    Some(res)
}

#[cfg(test)]
mod tests;
