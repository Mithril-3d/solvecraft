//! Flat faces that sit on B-spline surfaces become planes.
//!
//! A wall swept along a curve has a top and a bottom that are flat but are made of the same
//! B-spline surfaces as everything else the sweep makes. The coplanar handling of a boolean
//! (coincident faces pushed apart, faces glued back to back, the exact planar fallback) looks
//! for faces on true planes, so it never sees them: two such walls that share a top plane were
//! left to the general intersection, which fails on faces that coincide.

use solidcraft_geom::Vec3;
use truck_modeling as mt;

use crate::body::{Body, Solid, from_p3, p3};
use crate::{KernelError, guard};

/// Most control points looked at in one surface.
const MAX_CONTROL_POINTS: usize = 1_000_000;

/// The control points of a B-spline or NURBS surface, as points.
fn control_points(s: &mt::Surface) -> Option<Vec<Vec3>> {
    match s {
        mt::Surface::BSplineSurface(b) => {
            let pts: Vec<Vec3> = b.control_points().iter().flatten().take(MAX_CONTROL_POINTS + 1).map(|p| from_p3(*p)).collect();
            (pts.len() <= MAX_CONTROL_POINTS).then_some(pts)
        }
        mt::Surface::NurbsSurface(n) => {
            let mut pts = Vec::new();
            for v in n.control_points().iter().flatten() {
                if !(v.w > 0.0) || pts.len() >= MAX_CONTROL_POINTS {
                    return None;
                }
                pts.push(Vec3::new(v.x / v.w, v.y / v.w, v.z / v.w));
            }
            Some(pts)
        }
        _ => None,
    }
}

/// The least and the greatest `along · p` over the control points of a B-spline or NURBS surface; the surface lies
/// between them (it is in the convex hull of its control net). `None` for any other surface.
pub(crate) fn control_net_span(s: &mt::Surface, along: Vec3) -> Option<(f64, f64)> {
    let pts = control_points(s)?;
    pts.iter().map(|p| along.dot(*p)).fold(None, |acc, h| Some(acc.map_or((h, h), |(lo, hi): (f64, f64)| (lo.min(h), hi.max(h)))))
}

/// A point and the unit normal of the plane every point lies within `tol` of, if there is one.
fn common_plane(pts: &[Vec3], tol: f64) -> Option<(Vec3, Vec3)> {
    let p0 = *pts.first()?;
    let far = |from: Vec3, pick: &dyn Fn(Vec3) -> f64| pts.iter().copied().max_by(|a, b| pick(*a - from).total_cmp(&pick(*b - from)));
    let p1 = far(p0, &|d| d.len())?;
    let dir = (p1 - p0).normalized()?;
    let p2 = far(p0, &|d| (d - dir * d.dot(dir)).len())?;
    let n = (p1 - p0).cross(p2 - p0).normalized()?;
    pts.iter().all(|p| n.dot(*p - p0).abs() <= tol).then_some((p0, n))
}

/// `b` with every face that is flat on a B-spline surface put on a plane; `None` when there is
/// no such face. The faces keep their order, their edges and their orientation.
pub(crate) fn planarized(b: &Body) -> Option<Body> {
    if b.is_mesh() {
        return None;
    }
    let tol = (b.size() * 1e-9).max(1e-9);
    guard("planar faces", || {
        use mt::{ParametricSurface, ParametricSurface3D};
        let mut changed = 0usize;
        let mut shells = Vec::new();
        for sh in b.solid.boundaries() {
            let mut faces: Vec<mt::Face> = Vec::new();
            for f in sh.face_iter() {
                let surface = f.surface();
                let plane = control_points(&surface).and_then(|pts| common_plane(&pts, tol));
                let Some((origin, mut n)) = plane else {
                    faces.push(f.clone());
                    continue;
                };
                // The plane faces the way the surface does; a surface with no normal at any of the
                // places looked at is left as it is.
                let (ur, vr) = surface.try_range_tuple();
                let ((u0, u1), (v0, v1)) = (ur.unwrap_or((0.0, 1.0)), vr.unwrap_or((0.0, 1.0)));
                let old = [(0.5, 0.5), (0.25, 0.25), (0.75, 0.75), (0.25, 0.75), (0.75, 0.25)]
                    .iter()
                    .map(|(s, t)| surface.normal(u0 + s * (u1 - u0), v0 + t * (v1 - v0)))
                    .find(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite() && v.x * v.x + v.y * v.y + v.z * v.z > 1e-18);
                let Some(old) = old else {
                    faces.push(f.clone());
                    continue;
                };
                if n.dot(Vec3::new(old.x, old.y, old.z)) < 0.0 {
                    n = -n;
                }
                let u = n.any_perp().normalized().ok_or_else(|| KernelError::Failed("planar faces: no axis".into()))?;
                let w = n.cross(u);
                let plane = mt::Plane::new(p3(origin), p3(origin + u), p3(origin + w));
                let mut nf =
                    mt::Face::try_new(f.absolute_boundaries().to_vec(), mt::Surface::Plane(plane)).map_err(|e| KernelError::Failed(e.to_string()))?;
                if !f.orientation() {
                    nf.invert();
                }
                faces.push(nf);
                changed += 1;
            }
            shells.push(faces.into());
        }
        if changed == 0 {
            return Ok(None);
        }
        let solid = Solid::try_new(shells).map_err(|e| KernelError::Failed(e.to_string()))?;
        Ok(Some(Body::new(solid)?))
    })
    .ok()
    .flatten()
}
