//! Points on a surface–surface intersection edge, on our own surface evaluators.
//!
//! truck stores an intersection edge as a leader curve (a polyline or B-spline near the
//! intersection) and its two surfaces, and finds each point by projecting the leader point onto
//! each surface in turn from no starting guess: a nearest-point search per surface per point.
//! Here the intersection is solved exactly at a set of leader parameters, and a cubic B-spline
//! is fitted through those points together with both surfaces' parameters there (a smooth
//! leader with its curves on both surfaces). The B-spline runs over the chord length of the
//! solved points, never over the curve's own parameter: truck's parameter can change speed
//! abruptly (a polyline leader with short and long spans), and a cubic interpolated over it
//! rings there, stepping back along the curve (Lee, "Choosing nodes in parametric curve
//! interpolation", CAD 21, 1989). The curve's parameter reaches the chord length through the
//! piecewise-linear map between the seeds, which is monotone. Each point is then refined from
//! the B-spline by Newton's method on the four unknowns (u1, v1, u2, v2): S1(u1, v1) = S2(u2, v2), held on the
//! plane through the B-spline point normal to the B-spline (Barnhill & Kersey, "A marching
//! method for parametric surface/surface intersection", CAGD 7, 1990; Patrikalakis & Maekawa,
//! "Shape Interrogation for CAD/CAM", 2002, ch. 5). Every point is independent once seeded. Where the surfaces are close to
//! tangent along the edge (the system becomes singular) or Newton does not converge, the caller
//! uses truck's evaluation.

use solidcraft_geom::Vec3;
use solidcraft_geom::bspline::CurveN;
use solidcraft_geom::tess::EvalSurface;
use truck_meshalgo::prelude::*;
use truck_modeling as mt;

use super::uv::{self, Domain};
use crate::body::from_p3;

/// Leader parameters at which both surfaces' parameters are found up front.
const SEEDS: usize = 32;
/// Newton iterations per point (two or three suffice from a seed).
const MAX_ITER: usize = 8;
/// Below this sine of the angle between the surface normals the intersection is treated as
/// near-tangent: points there are left to truck.
const MIN_SIN: f64 = 0.05;

/// An intersection edge ready for point evaluation.
pub(crate) struct Intersection {
    surfaces: [EvalSurface; 2],
    domains: [Domain; 2],
    /// The smooth leader over the chord length of the seeds: the intersection point (x, y, z)
    /// and both surfaces' parameters (u1, v1, u2, v2), periodic directions unwrapped along it.
    leader: CurveN<7>,
    /// The curve's parameter at each seed and the chord length there, both increasing.
    seeds: Vec<(f64, f64)>,
    /// Convergence tolerance (mm).
    eps: f64,
}

impl Intersection {
    /// `None` unless `curve` is an intersection curve whose surfaces have evaluators and whose
    /// leader projects onto both.
    pub(crate) fn new(curve: &mt::Curve) -> Option<Self> {
        let mt::Curve::IntersectionCurve(ic) = curve else { return None };
        let surfaces = [super::surface::eval_surface(ic.surface0())?, super::surface::eval_surface(ic.surface1())?];
        let domains = [uv::domain(&surfaces[0]), uv::domain(&surfaces[1])];
        let (t0, t1) = curve.range_tuple();
        if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
            return None;
        }
        let leader = |t: f64| match ic.leader() {
            mt::Leader::Polyline(l) => (from_p3(l.subs(t)), from_v3(l.der(t))),
            mt::Leader::BSpline(l) => (from_p3(l.subs(t)), from_v3(l.der(t))),
        };
        // The leader lies within the curve's tolerance of both surfaces.
        let accept = (ic.tolerance() * 100.0).max(1e-6);
        let scale = (0..=SEEDS).map(|k| leader(t0 + (t1 - t0) * k as f64 / SEEDS as f64).0.len()).fold(1.0, f64::max);
        let mut it = Intersection {
            surfaces,
            domains,
            leader: CurveN { degree: 1, knots: Vec::new(), ctrl: Vec::new() },
            seeds: Vec::new(),
            eps: scale * 1e-10,
        };
        let (mut params, mut points): (Vec<f64>, Vec<[f64; 7]>) = (Vec::with_capacity(SEEDS + 1), Vec::with_capacity(SEEDS + 1));
        let mut prev: Option<[f64; 4]> = None;
        for k in 0..=SEEDS {
            let t = t0 + (t1 - t0) * k as f64 / SEEDS as f64;
            let (p, d) = leader(t);
            let mut q = [0.0; 4];
            for (i, (s, dom)) in it.surfaces.iter().zip(&it.domains).enumerate() {
                let hint = prev.map(|w| [w[2 * i], w[2 * i + 1]]);
                let mut uv = uv::project(s, dom, p, hint, accept)?;
                if let Some(h) = hint {
                    for d in 0..2 {
                        uv[d] = uv::unwrap(uv[d], h[d], dom.period[d]);
                    }
                }
                q[2 * i] = uv[0];
                q[2 * i + 1] = uv[1];
            }
            // The fit takes only points exactly on the intersection: a leader point Newton
            // cannot place (near-tangent, or the solution leaves a surface's domain where a
            // polyline leader overshoots a vertex) is off the curve and would bend the fit back
            // along it. The ends stay either way, so the fit spans the whole curve.
            let solved = d.normalized().and_then(|dir| it.newton(p, dir, q));
            prev = Some(solved.map_or(q, |(_, q)| q));
            let (p, q) = match solved {
                Some(x) => x,
                None if k == 0 || k == SEEDS => (p, q),
                None => continue,
            };
            // A seed that lands on the previous one adds no chord and stays out of the fit
            // (whose parameters must increase).
            let s = match it.seeds.last().zip(points.last()) {
                Some((&(_, s), &[x, y, z, ..])) => {
                    let chord = p.dist(Vec3::new(x, y, z));
                    if !(chord > it.eps) {
                        continue;
                    }
                    s + chord
                }
                None => 0.0,
            };
            it.seeds.push((t, s));
            params.push(s);
            points.push([p.x, p.y, p.z, q[0], q[1], q[2], q[3]]);
        }
        let degree = points.len().checked_sub(1)?.min(3);
        it.leader = CurveN::interpolate(&params, &points, degree).ok()?;
        Some(it)
    }

    /// The point at curve parameter `t` and both surfaces' parameters there, or `None`
    /// (near-tangent, or Newton failed).
    pub(crate) fn eval(&self, t: f64) -> Option<(Vec3, [f64; 4])> {
        let (x, dx) = self.leader.eval(self.chord_at(t)?)?;
        let dir = Vec3::new(dx[0], dx[1], dx[2]).normalized()?;
        self.newton(Vec3::new(x[0], x[1], x[2]), dir, [x[3], x[4], x[5], x[6]])
    }

    /// The chord length at curve parameter `t`: linear between the seeds, clamped to the ends.
    fn chord_at(&self, t: f64) -> Option<f64> {
        let (&(ta, sa), &(tb, sb)) = (self.seeds.first()?, self.seeds.last()?);
        if !(t > ta) {
            return Some(sa);
        }
        if !(t < tb) {
            return Some(sb);
        }
        let k = self.seeds.partition_point(|&(tk, _)| tk <= t);
        let (Some(&(t0, s0)), Some(&(t1, s1))) = (<[_]>::get(&self.seeds, k.checked_sub(1)?), <[_]>::get(&self.seeds, k)) else { return None };
        Some(s0 + (s1 - s0) * (t - t0) / (t1 - t0))
    }

    /// The two surfaces, in the order of the parameters [`Intersection::eval`] gives.
    pub(crate) fn surfaces(&self) -> &[EvalSurface; 2] {
        &self.surfaces
    }

    /// Newton from the parameters `x` on S1 = S2, held on the plane through `l` normal to `dir`:
    /// the point and its parameters, or `None` (near-tangent, singular, or not converged).
    fn newton(&self, l: Vec3, dir: Vec3, mut x: [f64; 4]) -> Option<(Vec3, [f64; 4])> {
        for _ in 0..MAX_ITER {
            let (Some((p1, u1, v1)), Some((p2, u2, v2))) = (self.surfaces[0].derivs(x[0], x[1]), self.surfaces[1].derivs(x[2], x[3])) else {
                return None;
            };
            let gap = p1 - p2;
            let plane = dir.dot(p1 - l);
            let n = u1.cross(v1).normalized()?.cross(u2.cross(v2).normalized()?);
            if n.len() < MIN_SIN {
                return None;
            }
            if gap.len() <= self.eps && plane.abs() <= self.eps {
                return Some(((p1 + p2) * 0.5, x));
            }
            // J · dx = −F, F = (S1 − S2, dir · (S1 − L)).
            let j = [[u1.x, v1.x, -u2.x, -v2.x], [u1.y, v1.y, -u2.y, -v2.y], [u1.z, v1.z, -u2.z, -v2.z], [dir.dot(u1), dir.dot(v1), 0.0, 0.0]];
            let dx = solve4(j, [-gap.x, -gap.y, -gap.z, -plane])?;
            for (xi, d) in x.iter_mut().zip(dx) {
                *xi += d;
            }
            for i in 0..2 {
                let [u, v] = self.domains[i].clamp([x[2 * i], x[2 * i + 1]]);
                x[2 * i] = u;
                x[2 * i + 1] = v;
            }
        }
        None
    }
}

fn from_v3(v: mt::Vector3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// `a · x = b` for a 4 × 4 system by Gaussian elimination with partial pivoting; `None` when
/// singular.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    let scale = a.iter().flatten().fold(0.0f64, |m, x| m.max(x.abs()));
    if !(scale > 0.0) {
        return None;
    }
    for c in 0..4 {
        let p = (c..4).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if !(a[p][c].abs() > 1e-12 * scale) {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        let pivot = a[c];
        for r in c + 1..4 {
            let f = a[r][c] / pivot[c];
            for (x, p) in a[r].iter_mut().zip(pivot).skip(c) {
                *x -= f * p;
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; 4];
    for c in (0..4).rev() {
        let s: f64 = (c + 1..4).map(|k| a[c][k] * x[k]).sum();
        x[c] = (b[c] - s) / a[c][c];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_four_by_four_system() {
        let a = [[2.0, 1.0, 0.0, 0.0], [1.0, 3.0, 1.0, 0.0], [0.0, 1.0, 4.0, 1.0], [0.0, 0.0, 1.0, 5.0]];
        let x = [1.0, -2.0, 0.5, 3.0];
        let b = [0, 1, 2, 3].map(|r| (0..4).map(|c| a[r][c] * x[c]).sum::<f64>());
        let Some(got) = solve4(a, b) else { panic!("solves") };
        for (g, w) in got.iter().zip(x) {
            assert!((g - w).abs() < 1e-12);
        }
        assert!(solve4([[1.0; 4]; 4], [1.0; 4]).is_none(), "singular");
    }

    /// Points on the intersection edges of a boolean (a cross hole: two cylinders) lie on both
    /// surfaces, on truck's curve (the parameterisation follows our smooth leader, so the same
    /// parameter may sit at a slightly different place along it), and advance along it.
    #[test]
    fn intersection_points_lie_on_both_surfaces_and_on_truck_s_curve() {
        let Ok(pin) = crate::cylinder(Vec3::new(5.0, 5.0, -1.0), Vec3::Z, 2.0, 6.0) else { panic!("pin") };
        let Ok(cross) = crate::cylinder(Vec3::new(5.0, -1.0, 2.0), Vec3::Y, 1.0, 12.0) else { panic!("cross") };
        let Ok(Some(body)) = crate::boolean(&pin, &cross, crate::BoolOp::Cut) else { panic!("cut") };
        let mut checked = 0;
        for e in body.solid.edge_iter() {
            let curve = e.curve();
            let Some(ix) = Intersection::new(&curve) else { continue };
            let (t0, t1) = curve.range_tuple();
            let dense: Vec<Vec3> = (0..=1000).map(|k| from_p3(curve.subs(t0 + (t1 - t0) * f64::from(k) / 1000.0))).collect();
            let near = |p: Vec3| dense.windows(2).map(|w| p.dist_to_segment(w[0], w[1])).fold(f64::MAX, f64::min);
            // Arc length along truck's curve to the point nearest `p`.
            let along = |p: Vec3| {
                let (mut run, mut best) = (0.0, (f64::MAX, 0.0));
                for w in dense.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let len = a.dist(b);
                    let f = if len > 0.0 { ((p - a).dot(b - a) / (len * len)).clamp(0.0, 1.0) } else { 0.0 };
                    let d = p.dist(a + (b - a) * f);
                    if d < best.0 {
                        best = (d, run + f * len);
                    }
                    run += len;
                }
                best.1
            };
            let mut last: Option<Vec3> = None;
            for k in 0..=50 {
                let t = t0 + (t1 - t0) * f64::from(k) / 50.0;
                let Some((p, _)) = ix.eval(t) else { continue };
                for (s, dom) in ix.surfaces.iter().zip(&ix.domains) {
                    assert!(uv::project(s, dom, p, None, 1e-8).is_some(), "t {t}: {p:?} off a surface");
                }
                assert!(near(p) < 1e-4, "t {t}: {p:?} is {} from truck's curve", near(p));
                if let Some(q) = last {
                    assert!(along(p) > along(q), "t {t}: the points advance along the curve");
                }
                last = Some(p);
                checked += 1;
            }
        }
        assert!(checked > 100, "{checked} points checked");
    }
}
