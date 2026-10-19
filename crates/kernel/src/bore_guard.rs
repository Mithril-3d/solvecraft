//! Necessary boundary safety check for the narrowed-bore fallback.
//! This is not a certificate of general Boolean equivalence. It prevents a
//! cylinder recut from silently removing an additional material boundary that
//! the fallback's coarse material samples do not see.
use crate::body::{Body, from_p3};
use solidcraft_geom::{Plane, Seg2, UnitVec3, Vec3};
use std::collections::HashSet;
use truck_modeling as mt;

pub(super) fn boundaries_clear(
    body: &Body,
    bore_faces: &[usize],
    origin: Vec3,
    axis: UnitVec3,
    radius: f64,
    lo: f64,
    hi: f64,
    tol: f64,
) -> Option<()> {
    let plane = Plane::from_normal(origin, axis)?;
    if !origin.is_finite()
        || ![radius, lo, hi, tol].iter().all(|v| v.is_finite())
        || radius <= tol
        || hi <= lo
        || tol <= 0.
        || bore_faces.len() > 16_384
    {
        return None;
    }
    let mut remaining_faces = 16_384usize;
    let mut remaining_uses = 65_536usize;
    for face in body.solid.face_iter() {
        remaining_faces = remaining_faces.checked_sub(1)?;
        for wire in face.absolute_boundaries() {
            for _ in wire.edge_iter() {
                remaining_uses = remaining_uses.checked_sub(1)?;
            }
        }
    }
    // Exempt only existing coincident bore boundaries, never newly inferred
    // edges or every edge of a planar face adjoining a cylinder.
    let bore_faces: HashSet<usize> = bore_faces.iter().copied().collect();
    let mut bore_edges = HashSet::new();
    for (i, face) in body.solid.face_iter().enumerate() {
        if bore_faces.contains(&i) {
            for edge in face.edge_iter() {
                bore_edges.insert(edge.id());
            }
        }
    }
    let mut seen = HashSet::new();
    let mut work = 2_000_000usize;
    for edge in body.solid.edge_iter() {
        if !seen.insert(edge.id()) {
            continue;
        }
        let common_bore = bore_edges.contains(&edge.id());
        work = work.checked_sub(1)?;
        let curve = edge.curve();
        // Original matched-cylinder trims retain the legacy fallback policy.
        // Boolean-produced rational trims need not be exact analytic circles.
        // This exemption follows existing face/edge identity only; additional
        // trims must pass the whole-boundary proof below. It is deliberately
        // not a general proof that the cylinder contains no material.
        if common_bore && matches!(curve, mt::Curve::IntersectionCurve(_) | mt::Curve::BSplineCurve(_) | mt::Curve::NurbsCurve(_)) {
            continue;
        }
        if let mt::Curve::Line(line) = curve {
            let (a, b) = (from_p3(line.0), from_p3(line.1));
            if !a.is_finite() || !b.is_finite() {
                return None;
            }
            let ha = plane.height(a);
            let hb = plane.height(b);
            let error = tol + 512. * f64::EPSILON * (1. + a.len() + b.len() + origin.len());
            if common_bore {
                let pa = plane.to_local(a);
                let pb = plane.to_local(b);
                if pa.dist(pb) <= error && (pa.len() - radius).abs() + (pb.len() - radius).abs() <= error * 2. {
                    continue;
                }
                return None;
            }
            let (mut t0, mut t1): (f64, f64) = (0., 1.);
            let dh = hb - ha;
            if dh.abs() <= error {
                if ha.min(hb) > hi + error || ha.max(hb) < lo - error {
                    continue;
                }
            } else {
                let x = (lo - error - ha) / dh;
                let y = (hi + error - ha) / dh;
                t0 = t0.max(x.min(y));
                t1 = t1.min(x.max(y));
                if t0 > t1 {
                    continue;
                }
            }
            let pa = plane.to_local(a + (b - a) * t0);
            let pb = plane.to_local(a + (b - a) * t1);
            let d = pb - pa;
            let t = if d.len2() > 0. { (-pa.dot(d) / d.len2()).clamp(0., 1.) } else { 0. };
            if (pa + d * t).len() <= radius + error {
                return None;
            }
            continue;
        }
        // Positive rational controls enclose the entire axial extent. Project
        // the current curve onto the axis plane; the existing whole-span
        // certificate proves its Line/Arc image, without point sampling.
        let (knots, controls): (mt::KnotVec, Vec<mt::Vector4>) = match curve {
            mt::Curve::BSplineCurve(c) => {
                if c.control_points().len() > 4096 || c.knot_vec().len() > 8192 {
                    return None;
                }
                work = work.checked_sub(c.control_points().len().checked_add(c.knot_vec().len())?)?;
                (c.knot_vec().clone(), c.control_points().iter().map(|p| mt::Vector4::new(p.x, p.y, p.z, 1.)).collect())
            }
            mt::Curve::NurbsCurve(c) => {
                if c.control_points().len() > 4096 || c.knot_vec().len() > 8192 {
                    return None;
                }
                work = work.checked_sub(c.control_points().len().checked_add(c.knot_vec().len())?)?;
                (c.knot_vec().clone(), c.control_points().clone())
            }
            _ => return None,
        };
        let mut heights = (f64::INFINITY, f64::NEG_INFINITY);
        let mut projection_error: f64 = 0.;
        let mut projected_origin = None;
        let mut projected_spread: f64 = 0.;
        let mut projected = Vec::with_capacity(controls.len());
        for p in controls {
            if ![p.x, p.y, p.z, p.w].iter().all(|v| v.is_finite()) || p.w <= 0. {
                return None;
            }
            let point = Vec3::new(p.x / p.w, p.y / p.w, p.z / p.w);
            if !point.is_finite() {
                return None;
            }
            let error = tol + 512. * f64::EPSILON * (1. + point.len() + origin.len());
            projection_error = projection_error.max(error);
            let h = plane.height(point);
            heights.0 = heights.0.min(h - error);
            heights.1 = heights.1.max(h + error);
            let q = plane.to_local(point);
            let first = *projected_origin.get_or_insert(q);
            projected_spread = projected_spread.max(q.dist(first));
            projected.push(mt::Vector4::new(q.x * p.w, q.y * p.w, 0., p.w));
        }
        if heights.0 > hi || heights.1 < lo {
            continue;
        }
        // A swept seam may be represented as a rational straight edge. Its
        // positive control hull proves a point-sized radial projection even
        // though a nondegenerate planar Line/Arc recognizer cannot accept it.
        if let Some(q) = projected_origin {
            if common_bore && projected_spread + projection_error + (q.len() - radius).abs() <= tol * 8. {
                continue;
            }
            if !common_bore && q.len() - projected_spread - projection_error > radius + tol * 4. {
                continue;
            }
        }
        use mt::{BoundedCurve, ParametricCurve};
        let projected = mt::Curve::NurbsCurve(mt::NurbsCurve::new(mt::BSplineCurve::try_new(knots, projected).ok()?));
        let (a, b) = projected.range_tuple();
        let projected_edge = mt::Edge::new(&mt::Vertex::new(projected.subs(a)), &mt::Vertex::new(projected.subs(b)), projected);
        let segments = crate::prism_curve::segment(&projected_edge, &Plane::XY, tol, &mut work)?;
        for segment in segments {
            if common_bore {
                if let Seg2::Arc { center, radius: r, .. } = segment
                    && center.len() + (r - radius).abs() + projection_error <= tol * 8.
                {
                    continue;
                }
                return None;
            }
            let clearance = match segment {
                Seg2::Line { a, b } => {
                    let d = b - a;
                    let t = if d.len2() > 0. { (-a.dot(d) / d.len2()).clamp(0., 1.) } else { 0. };
                    (a + d * t).len()
                }
                Seg2::Arc { center, radius, .. } => radius - center.len(),
                _ => return None,
            };
            if clearance <= radius + tol * 4. + projection_error {
                return None;
            }
        }
    }
    Some(())
}
