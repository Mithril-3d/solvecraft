//! geom's B-spline curves and surfaces as truck's NURBS geometry, weights kept (homogeneous
//! control points carried over as they are).

use solidcraft_geom::bspline::{HPoint, NurbsSurface};
use truck_modeling as mt;

use crate::body::p3;
use crate::{KernelError, Result};

fn fail(m: impl std::fmt::Display) -> KernelError {
    KernelError::Failed(format!("spline geometry: {m}"))
}

/// A homogeneous control point as truck's.
pub(crate) fn h4(h: HPoint) -> mt::Vector4 {
    mt::Vector4::new(h.x, h.y, h.z, h.w)
}

/// A NURBS curve of knots and homogeneous control points.
pub(crate) fn nurbs_curve(knots: mt::KnotVec, ctrl: &[HPoint]) -> Result<mt::Curve> {
    let c = mt::BSplineCurve::try_new(knots, ctrl.iter().copied().map(h4).collect()).map_err(fail)?;
    Ok(mt::Curve::NurbsCurve(mt::NurbsCurve::new(c)))
}

/// A NURBS surface of knots and homogeneous control points (`rows[i][j]`: i along u).
pub(crate) fn nurbs_surface(knots: (mt::KnotVec, mt::KnotVec), rows: Vec<Vec<mt::Vector4>>) -> Result<mt::Surface> {
    let s = mt::BSplineSurface::try_new(knots, rows).map_err(fail)?;
    Ok(mt::Surface::NurbsSurface(mt::NurbsSurface::new(s)))
}

/// geom's tensor surface as truck's.
pub(crate) fn surface(s: &NurbsSurface) -> Result<mt::Surface> {
    let [nu, nv] = s.count();
    let [ku, kv] = s.knots();
    let rows: Vec<Vec<mt::Vector4>> = (0..nu).map(|i| (0..nv).map(|j| h4(s.point(i, j))).collect()).collect();
    nurbs_surface((mt::KnotVec::from(ku.clone()), mt::KnotVec::from(kv.clone())), rows)
}

/// One face of a whole tensor surface: its four boundary curves as edges, sharing the corners.
/// This is what a patch built as a single control net (`bridge`, `boundary`, the T-spline sheet)
/// hands to the B-rep.
pub(crate) fn tensor_face(s: &NurbsSurface) -> Result<mt::Face> {
    let [nu, nv] = s.count();
    if nu < 2 || nv < 2 {
        return Err(fail("a face needs at least 2 x 2 control points"));
    }
    let [ku, kv] = s.knots();
    let corners = [s.point(0, 0), s.point(nu - 1, 0), s.point(nu - 1, nv - 1), s.point(0, nv - 1)];
    let mut v: Vec<mt::Vertex> = Vec::with_capacity(4);
    for c in corners {
        let p = c.project().ok_or_else(|| fail("a corner has no position"))?;
        v.push(mt::builder::vertex(p3(p)));
    }
    let row = |j: usize| -> Vec<HPoint> { (0..nu).map(|i| s.point(i, j)).collect() };
    let column = |i: usize| -> Vec<HPoint> { (0..nv).map(|j| s.point(i, j)).collect() };
    let (Some(v0), Some(v1), Some(v2), Some(v3)) = (v.first(), v.get(1), v.get(2), v.get(3)) else { return Err(fail("corners")) };
    let edge = |a: &mt::Vertex, b: &mt::Vertex, c: mt::Curve| mt::Edge::try_new(a, b, c).map_err(fail);
    let bottom = edge(v0, v1, nurbs_curve(mt::KnotVec::from(ku.clone()), &row(0))?)?;
    let right = edge(v1, v2, nurbs_curve(mt::KnotVec::from(kv.clone()), &column(nu - 1))?)?;
    let top = edge(v3, v2, nurbs_curve(mt::KnotVec::from(ku.clone()), &row(nv - 1))?)?;
    let left = edge(v0, v3, nurbs_curve(mt::KnotVec::from(kv.clone()), &column(0))?)?;
    let wire: mt::Wire = vec![bottom, right, top.inverse(), left.inverse()].into();
    mt::Face::try_new(vec![wire], surface(s)?).map_err(fail)
}
