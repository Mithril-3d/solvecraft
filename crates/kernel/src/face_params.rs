//! A face's boundary in its surface's parameters, and exact parameter lines on a surface:
//! shared by the operations that rework a face on the surface it lies on (Untrim, Extend).
//!
//! A face's loops are sampled into parameters with periodic directions unwrapped along each
//! loop; which loop is the outer one is worked out from those parameters (the loop enclosing
//! the most area), never from the order the loops are stored in: an imported face can list
//! its holes first.

use mt::{BoundedCurve, Cut, InnerSpace, Invertible, MetricSpace, ParametricCurve, ParametricSurface, SearchNearestParameter, Transformed};
use solidcraft_geom::Vec3;
use solidcraft_geom::tess::EvalSurface;
use truck_modeling as mt;

use crate::body::from_p3;
use crate::tess::uv::{self, Domain};
use crate::{KernelError, Result};

/// Samples along each boundary edge when working out the face's parameter loops.
pub(crate) const EDGE_SAMPLES: usize = 32;
/// Golden-section steps that place an extreme of the face's span on an edge.
const EXTREME_STEPS: usize = 60;

/// A boundary loop of a face sampled in 3D and in the surface's parameters: for each of its
/// edges (as the loop runs) the curve parameters, points and parameters of its samples, ends
/// included. Periodic parameters are unwrapped along the loop.
pub(crate) struct LoopSamples {
    pub(crate) edges: Vec<(mt::Curve, Vec<(f64, Vec3, [f64; 2])>)>,
}

impl LoopSamples {
    /// The loop as a polygon in parameters (each edge's samples but its last, which the next
    /// edge starts on).
    pub(crate) fn polygon(&self) -> Vec<[f64; 2]> {
        self.edges.iter().flat_map(|(_, s)| s.iter().take(s.len().saturating_sub(1)).map(|x| x.2)).collect()
    }

    /// Signed area of the loop in parameters (positive: counter-clockwise).
    pub(crate) fn area(&self) -> f64 {
        let poly = self.polygon();
        let n = poly.len();
        (0..n)
            .filter_map(|i| {
                let (a, b) = (poly.get(i)?, poly.get((i + 1) % n)?);
                Some(0.5 * (a[0] * b[1] - b[0] * a[1]))
            })
            .sum()
    }

    /// How far `p` is from the loop (from its samples).
    pub(crate) fn dist(&self, p: Vec3) -> f64 {
        self.edges.iter().flat_map(|(_, s)| s.iter().map(|x| x.1.dist(p))).fold(f64::INFINITY, f64::min)
    }
}

/// A face's boundary loops (absolute: as they run on its surface) in the surface's parameters,
/// and which of them is the outer one.
pub(crate) struct FaceLoops {
    pub(crate) eval: EvalSurface,
    pub(crate) dom: Domain,
    pub(crate) wires: Vec<mt::Wire>,
    pub(crate) loops: Vec<LoopSamples>,
    pub(crate) outer: usize,
}

impl FaceLoops {
    /// The loops of `face`, its boundary found on its surface within `accept`; `what` names the
    /// operation for a surface no evaluator takes.
    pub(crate) fn of(face: &mt::Face, accept: f64, what: &str) -> Result<FaceLoops> {
        let eval = crate::tess::surface::eval_surface(&face.surface())
            .ok_or_else(|| KernelError::TooLarge(format!("{what} a face on this kind of surface")))?;
        let dom = uv::domain(&eval);
        let wires: Vec<mt::Wire> = face.absolute_boundaries().clone();
        let loops: Vec<LoopSamples> = wires.iter().map(|w| sample_loop(&eval, &dom, w, accept)).collect::<Result<_>>()?;
        // The outer loop encloses the others: the most area in parameters.
        let outer = loops
            .iter()
            .map(LoopSamples::area)
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .ok_or_else(|| KernelError::Invalid("the face has no boundary".into()))?;
        Ok(FaceLoops { eval, dom, wires, loops, outer })
    }
}

/// Sample a boundary loop (absolute: as it runs on the surface) in the surface's parameters.
pub(crate) fn sample_loop(s: &EvalSurface, dom: &Domain, wire: &mt::Wire, accept: f64) -> Result<LoopSamples> {
    let mut edges = Vec::new();
    let mut prev: Option<[f64; 2]> = None;
    for e in wire.edge_iter() {
        let c = e.oriented_curve();
        let (t0, t1) = c.range_tuple();
        let mut samples = Vec::with_capacity(EDGE_SAMPLES + 1);
        for k in 0..=EDGE_SAMPLES {
            let t = t0 + (t1 - t0) * k as f64 / EDGE_SAMPLES as f64;
            let p = from_p3(c.subs(t));
            let q = parameters(s, dom, p, prev, accept)?;
            samples.push((t, p, q));
            prev = Some(q);
        }
        edges.push((c, samples));
    }
    Ok(LoopSamples { edges })
}

/// The parameters of `p` on the surface, unwrapped to lie near `near` along periodic directions.
pub(crate) fn parameters(s: &EvalSurface, dom: &Domain, p: Vec3, near: Option<[f64; 2]>, accept: f64) -> Result<[f64; 2]> {
    let mut q = uv::project(s, dom, p, near, accept).ok_or_else(|| KernelError::Failed("the face's boundary does not lie on its surface".into()))?;
    if let Some(n) = near {
        for d in 0..2 {
            q[d] = uv::unwrap(q[d], n[d], dom.period[d]);
        }
    }
    Ok(q)
}

/// The parameter box the outer boundary is restored to: along each direction the surface's own
/// range where it has one of its own (a free-form surface's; not a periodic or unbounded
/// direction, and not an analytic surface's), else the face's own span.
pub(crate) fn natural_span(s: &EvalSurface, dom: &Domain, outer: &LoopSamples, analytic: bool, accept: f64) -> Result<[(f64, f64); 2]> {
    let mut span = [(0.0, 0.0); 2];
    for (d, out) in span.iter_mut().enumerate() {
        *out = match (dom.range.get(d).copied().flatten(), dom.period.get(d).copied().flatten()) {
            (Some(range), None) if !analytic => range,
            (_, period) => {
                let lo = extreme(s, dom, outer, d, false, accept);
                let hi = extreme(s, dom, outer, d, true, accept);
                if let Some(p) = period
                    && hi - lo >= p * (1.0 - 1e-9)
                {
                    return Err(KernelError::TooLarge("a face that goes all the way round its surface".into()));
                }
                (lo, hi)
            }
        };
        if !(out.1 - out.0 > 1e-12 * (1.0 + out.0.abs() + out.1.abs())) || !out.0.is_finite() || !out.1.is_finite() {
            return Err(KernelError::Invalid("the face has no extent along its surface".into()));
        }
    }
    Ok(span)
}

/// The least (`max == false`) or greatest parameter along direction `d` the loop reaches: the
/// extreme sample, then placed on its edge between the neighbouring samples. Samples where the
/// surface does not move along `d` (a pole) say nothing about `d` and are passed over.
pub(crate) fn extreme(s: &EvalSurface, dom: &Domain, lp: &LoopSamples, d: usize, max: bool, accept: f64) -> f64 {
    let sign = if max { 1.0 } else { -1.0 };
    let mut best: Option<(f64, usize, usize)> = None;
    for (ei, (_, samples)) in lp.edges.iter().enumerate() {
        for (k, (_, _, q)) in samples.iter().enumerate() {
            if uv::degenerate(s, *q, d) {
                continue;
            }
            let x = sign * q[d];
            if best.is_none_or(|(b, _, _)| x > b) {
                best = Some((x, ei, k));
            }
        }
    }
    let Some((x, ei, k)) = best else { return f64::NAN };
    let Some((curve, samples)) = lp.edges.get(ei) else { return sign * x };
    let (Some(at), Some(lo), Some(hi)) = (samples.get(k), samples.get(k.saturating_sub(1)), samples.get(k + 1).or(samples.get(k))) else {
        return sign * x;
    };
    // Golden-section search for the extreme between the neighbouring samples.
    let value = |t: f64| -> Option<f64> {
        let p = from_p3(curve.subs(t));
        let q = uv::project(s, dom, p, Some(at.2), accept)?;
        Some(sign * uv::unwrap(q[d], at.2[d], dom.period[d]))
    };
    let g = (5f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (lo.0, hi.0);
    let mut best = x;
    for _ in 0..EXTREME_STEPS {
        let (c, e) = (b - g * (b - a), a + g * (b - a));
        let (Some(fc), Some(fe)) = (value(c), value(e)) else { break };
        best = best.max(fc).max(fe);
        if fc > fe {
            b = e;
        } else {
            a = c;
        }
    }
    sign * best
}

/// Whether the curve passes within `tol` of `p`.
pub(crate) fn passes(curve: &mt::Curve, p: mt::Point3, tol: f64) -> bool {
    let (t0, t1) = curve.range_tuple();
    let start = (0..=64)
        .map(|k| t0 + (t1 - t0) * k as f64 / 64.0)
        .min_by(|a, b| curve.subs(*a).distance(p).total_cmp(&curve.subs(*b).distance(p)))
        .unwrap_or(t0);
    let t = curve.search_nearest_parameter(p, Some(start), 100).unwrap_or(start).clamp(t0.min(t1), t0.max(t1));
    curve.subs(t).distance(p) <= tol
}

/// The parameter line of `surface` at `w` (v = w when `along_u`, else u = w) from parameter
/// `from` to `to` along it, exact: a line on a plane, a B-spline surface's iso-curve, and on a
/// revolved surface the profile turned to its angle or an arc about the axis.
pub(crate) fn iso_curve(surface: &mt::Surface, along_u: bool, w: f64, from: f64, to: f64) -> Result<mt::Curve> {
    let at = |t: f64| if along_u { surface.subs(t, w) } else { surface.subs(w, t) };
    match surface {
        mt::Surface::Plane(_) => Ok(mt::Curve::Line(mt::Line(at(from), at(to)))),
        mt::Surface::BSplineSurface(_) | mt::Surface::NurbsSurface(_) => {
            let hom: mt::BSplineSurface<mt::Vector4> = match surface {
                mt::Surface::BSplineSurface(b) => mt::BSplineSurface::new(
                    b.knot_vecs().clone(),
                    b.control_points().iter().map(|row| row.iter().map(|p| p.to_homogeneous()).collect()).collect(),
                ),
                mt::Surface::NurbsSurface(n) => n.non_rationalized().clone(),
                _ => return Err(KernelError::Failed("not a B-spline surface".into())),
            };
            let iso = crate::step_in::split::iso_bsp(&hom, w, along_u).map_err(KernelError::Failed)?;
            Ok(piece(mt::Curve::NurbsCurve(iso), from, to))
        }
        mt::Surface::RevolutedCurve(proc) => {
            let rev = proc.entity();
            // Truck's revolved surface is (profile parameter, angle), swapped when the processor
            // is turned over; the processor's matrix places it.
            let along_profile = along_u == proc.orientation();
            let transform = *proc.transform();
            let (origin, axis) = (rev.origin(), rev.axis().normalize());
            if along_profile {
                let profile = rev.entity_curve();
                if let mt::Curve::Line(_) = profile {
                    return Ok(mt::Curve::Line(mt::Line(at(from), at(to))));
                }
                if let mt::Curve::IntersectionCurve(_) = profile {
                    return Err(KernelError::TooLarge("untrimming a surface revolved from an intersection curve".into()));
                }
                let turn = mt::Matrix4::from_translation(origin - mt::Point3::new(0.0, 0.0, 0.0))
                    * mt::Matrix4::from_axis_angle(axis, mt::Rad(w))
                    * mt::Matrix4::from_translation(mt::Point3::new(0.0, 0.0, 0.0) - origin);
                Ok(piece(profile.transformed(transform * turn), from, to))
            } else {
                // The profile point at `w` turns about the axis: an arc from angle `from` to `to`.
                let p = rev.entity_curve().subs(w);
                let centre = origin + axis * (p - origin).dot(axis);
                let r = (p - centre).magnitude();
                if !(r > 1e-12) {
                    return Err(KernelError::Failed("an arc on the axis of a revolved surface".into()));
                }
                let x = (p - centre) / r;
                let frame = crate::step_in::geom::Frame { o: centre, x, y: axis.cross(x), z: axis };
                let arc = if to > from {
                    crate::step_in::geom::conic_arc(&frame, r, r, from, to - from)
                } else {
                    crate::step_in::geom::conic_arc(&frame, r, r, to, from - to).inverse()
                };
                Ok(mt::Curve::NurbsCurve(arc).transformed(transform))
            }
        }
    }
}

/// The piece of `curve` between its parameters `from` and `to`, running from `from`.
pub(crate) fn piece(mut curve: mt::Curve, from: f64, to: f64) -> mt::Curve {
    let (lo, hi) = if from < to { (from, to) } else { (to, from) };
    let (t0, t1) = curve.range_tuple();
    let eps = (t1 - t0).abs() * 1e-12;
    if lo > t0 + eps {
        curve = curve.cut(lo);
    }
    if hi < t1 - eps {
        let _ = curve.cut(hi);
    }
    if from > to {
        curve.invert();
    }
    curve
}
