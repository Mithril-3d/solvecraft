//! Continuity-driven bridging surfaces: one B-spline face that leaves one surface and arrives at
//! another with a requested continuity (G0 up to G3). This is the core the Blend, Boundary and
//! Match surfaces are built on.
//!
//! A bridge is a tensor product. Across it (v) every station's cross-section is a Bézier whose
//! leading control points reproduce the support surface's derivatives — the Hermite rows — so the
//! finished face carries the requested continuity by construction: one control row per condition
//! (position, tangent, curvature, change of curvature). Along it (u) those cross-sections are
//! interpolated by a cubic, so the face passes through every station exactly. Every condition is
//! stated against the support surface's own derivatives measured along the direction the bridge
//! leaves in, which makes G2 the curvature vector and G3 the way that curvature vector changes
//! along that direction (its normal part: what a surface-to-surface match can hold).
//!
//! What the support surface does along the leave direction is read off from samples on its own
//! parameter line (the line through the station that leaves in the leave direction at unit
//! speed) by finite differences: the second derivative of a surface curve with a unit tangent is
//! its curvature vector, and the third is the arclength derivative of that curvature vector.

use mt::{BoundedCurve, ParametricCurve, ParametricSurface, ParametricSurface3D, SearchNearestParameter};
use solidcraft_geom::Vec3;
use solidcraft_geom::bspline::{CurveN, HPoint, NurbsSurface};
use truck_modeling as mt;

use crate::body::{Body, from_p3, p3};
use crate::surfaces::surface_body;
use crate::{KernelError, Result, guard};

/// How much of a support surface a bridge repeats along its boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Continuity {
    /// The boundary curve alone: the surfaces meet, nothing else.
    G0,
    /// Position and tangent plane.
    #[default]
    G1,
    /// Position, tangent plane and curvature vector.
    G2,
    /// Position, tangent plane, curvature vector and its change along the leave direction.
    G3,
}

impl Continuity {
    /// Conditions a side carries: 1 (G0) up to 4 (G3).
    pub fn conditions(self) -> usize {
        match self {
            Continuity::G0 => 1,
            Continuity::G1 => 2,
            Continuity::G2 => 3,
            Continuity::G3 => 4,
        }
    }

    /// The word a command parameter uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Continuity::G0 => "g0",
            Continuity::G1 => "g1",
            Continuity::G2 => "g2",
            Continuity::G3 => "g3",
        }
    }

    /// Parse `g0`…`g3` (upper case accepted), `None` for anything else.
    pub fn parse(s: &str) -> Option<Continuity> {
        match s.to_ascii_lowercase().as_str() {
            "g0" => Some(Continuity::G0),
            "g1" => Some(Continuity::G1),
            "g2" => Some(Continuity::G2),
            "g3" => Some(Continuity::G3),
            _ => None,
        }
    }
}

/// One station of a boundary: where the bridge starts, the direction it leaves in, and what the
/// support surface does there along that direction.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The point on the boundary.
    pub p: Vec3,
    /// Unit direction the bridge leaves *towards the other boundary*, in the tangent plane of
    /// the support surface at `p`.
    pub leave: Vec3,
    /// The support surface's curvature vector along `leave` (zero on a plane).
    pub bend: Vec3,
    /// The arclength derivative of `bend` along `leave` (zero on a plane).
    pub bend_change: Vec3,
}

/// The edges of `body` picked by points on them, chained: each runs on from the last, and the
/// chain may close. Used with [`frames`], whose chain must be a chain of one face's own edges.
pub fn chain(body: &Body, at: &[Vec3]) -> Result<Vec<mt::Edge>> {
    body.require_brep("blend")?;
    if at.is_empty() || at.len() > 10_000 || at.iter().any(|p| !p.is_finite()) {
        return Err(KernelError::Invalid("pick 1 to 10000 points on the edges of a chain".into()));
    }
    let mut pool: Vec<mt::Edge> = Vec::new();
    for p in at {
        let e = crate::surfaces::edge_near(body, *p).ok_or_else(|| KernelError::Invalid("no edge near a picked point".to_string()))?;
        if !pool.iter().any(|x| x.id() == e.id()) {
            pool.push(e);
        }
    }
    let first = pool.remove(0);
    let mut chain = vec![first];
    // The picks need not be given in order: run forward from the first edge, then backwards.
    let mut end = match chain.first() {
        Some(e) => e.back().clone(),
        None => return Err(KernelError::Invalid("pick at least one edge".into())),
    };
    while let Some(k) = pool.iter().position(|e| e.front().id() == end.id() || e.back().id() == end.id()) {
        let e = pool.remove(k);
        let e = if e.front().id() == end.id() { e } else { e.inverse() };
        end = e.back().clone();
        chain.push(e);
    }
    let mut start = match chain.first() {
        Some(e) => e.front().clone(),
        None => return Err(KernelError::Invalid("pick at least one edge".into())),
    };
    while let Some(k) = pool.iter().position(|e| e.front().id() == start.id() || e.back().id() == start.id()) {
        let e = pool.remove(k);
        let e = if e.back().id() == start.id() { e } else { e.inverse() };
        start = e.front().clone();
        chain.insert(0, e);
    }
    if !pool.is_empty() {
        return Err(KernelError::Invalid("the picked edges do not run on from one another".into()));
    }
    Ok(chain)
}

/// Whether a chain of edges closes back on itself.
pub fn chain_closes(chain: &[mt::Edge]) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else { return false };
    first.front().id() == last.back().id()
}

/// The stations of a chain of edges, `count` of them evenly along its length, and the length
/// itself. A chain that closes is sampled with its last station back on its first.
fn chain_samples(edges: &[mt::Edge], count: usize) -> Result<(Vec<Vec3>, f64)> {
    /// Samples per edge used for the length table.
    const PER_EDGE: usize = 64;
    let mut poly: Vec<Vec3> = Vec::with_capacity(edges.len() * (PER_EDGE + 1));
    for e in edges {
        let c = e.curve();
        let (t0, t1) = c.range_tuple();
        if !(t1 > t0) || ![t0, t1].into_iter().all(f64::is_finite) {
            return Err(KernelError::Invalid("an edge has no parameter range".into()));
        }
        for k in 0..=PER_EDGE {
            poly.push(from_p3(c.subs(t0 + (t1 - t0) * k as f64 / PER_EDGE as f64)));
        }
    }
    if chain_closes(edges)
        && let Some(first) = poly.first().copied()
    {
        poly.push(first);
    }
    let mut cum = Vec::with_capacity(poly.len());
    cum.push(0.0);
    for w in poly.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        let Some(last) = cum.last().copied() else { break };
        cum.push(last + a.dist(*b));
    }
    let total = cum.last().copied().unwrap_or(0.0);
    if !(total > 1e-9) || !total.is_finite() {
        return Err(KernelError::Invalid("the edge chain has no length".into()));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let s = total * i as f64 / (count.max(2) - 1) as f64;
        let hi = cum.partition_point(|x| *x < s).clamp(1, poly.len().saturating_sub(1));
        let (Some(a), Some(b)) = (poly.get(hi - 1), poly.get(hi)) else { return Err(KernelError::Failed("bridge: sample".into())) };
        let (Some(&lo), Some(&hi_s)) = (cum.get(hi - 1), cum.get(hi)) else { return Err(KernelError::Failed("bridge: sample".into())) };
        let span = hi_s - lo;
        let t = if span > 1e-12 { (s - lo) / span } else { 0.0 };
        out.push(*a + (*b - *a) * t);
    }
    Ok((out, total))
}

/// The support surface's frames along `edges` (a chain of its own edges), for a bridge that
/// leaves in direction `leave` (projected into the tangent plane at each station).
pub fn frames(face: &mt::Face, edges: &[mt::Edge], leave: Vec3, count: usize) -> Result<Vec<Frame>> {
    if !(2..=4097).contains(&count) {
        return Err(KernelError::Invalid("a bridge needs 2 to 4097 stations".into()));
    }
    let leave =
        leave.normalized().filter(|l| l.is_finite()).ok_or_else(|| KernelError::Invalid("the leave direction must be a finite vector".into()))?;
    let (pts, length) = chain_samples(edges, count)?;
    let support = Support::of(face, length)?;
    let mut walk = Walk::new(length);
    let along = |n: Vec3| -> Result<Vec3> {
        (leave - n * leave.dot(n))
            .normalized()
            .ok_or_else(|| KernelError::Invalid("the leave direction is normal to the support surface: it has no tangent plane there".to_string()))
    };
    let mut out = Vec::with_capacity(pts.len());
    for p in &pts {
        out.push(support.station(*p, &along, &mut walk, true)?.0);
    }
    Ok(out)
}

/// The frames of `face` at stations on its own boundary, each leaving the face straight across
/// that boundary: `stations` are points on the boundary with the boundary's direction there *as
/// the face's own boundary runs* (its wires as [`mt::Face::boundaries`] gives them), and the leave
/// direction is that direction × the face's outward normal — in the face's tangent plane, at
/// right angles to the boundary, away from the face. `length` is the size of the boundary the
/// stations are on (it sets how closely a station must lie on the surface and the step the shape
/// is read with). Each frame comes with the face's unit normal there (as the face is oriented).
/// With `shape` false only the leave direction and the normal are read (`bend` and
/// `bend_change` are zero), which needs no room on the surface beside the station.
pub fn frames_across(face: &mt::Face, stations: &[(Vec3, Vec3)], length: f64, shape: bool) -> Result<Vec<(Frame, Vec3)>> {
    if stations.len() > 1 << 16 || !(length.is_finite() && length > 0.0) {
        return Err(KernelError::Invalid("frames across a boundary need at most 65536 stations on a boundary of finite size".into()));
    }
    let support = Support::of(face, length)?;
    let outward = if face.orientation() { 1.0 } else { -1.0 };
    let mut walk = Walk::new(length);
    let mut out = Vec::with_capacity(stations.len());
    for (p, tangent) in stations {
        if !p.is_finite() || !tangent.is_finite() {
            return Err(KernelError::Invalid("a boundary station must be finite".into()));
        }
        let away = |n: Vec3| -> Result<Vec3> {
            tangent.cross(n * outward).normalized().ok_or_else(|| KernelError::Invalid("the boundary runs along the face's normal".to_string()))
        };
        let (frame, normal) = support.station(*p, &away, &mut walk, shape)?;
        out.push((frame, normal * outward));
    }
    Ok(out)
}

/// A support surface read at stations: the surface, its parameter box, and how close a station
/// must lie to it.
struct Support {
    surf: mt::Surface,
    range_u: (f64, f64),
    range_v: (f64, f64),
    tol: f64,
}

/// What reading one station hands on to the next: the station's parameters (the hint for the
/// next projection, which keeps the search on the same side of a periodic or split surface) and
/// the finite-difference step that worked.
struct Walk {
    hint: Option<(f64, f64)>,
    step: f64,
}

impl Walk {
    fn new(length: f64) -> Walk {
        // Finite-difference step: a thousandth of the chain, so curvature over the chain is read
        // locally but round-off stays far below the geometry.
        Walk { hint: None, step: (length * 1e-3).clamp(1e-6, 0.5) }
    }
}

impl Support {
    fn of(face: &mt::Face, length: f64) -> Result<Support> {
        let surf = face.surface();
        let (Some(range_u), Some(range_v)) = surf.try_range_tuple() else {
            return Err(KernelError::Failed("bridge: the support surface is unbounded".into()));
        };
        // A face's own edges may sit a fit tolerance off its surface (a cylinder's wall as a NURBS
        // next to its exact ring), so the projection is only asked to be close, not exact.
        let tol = (length * 1e-4).max(1e-6);
        Ok(Support { surf, range_u, range_v, tol })
    }

    /// The frame at `p`, leaving in `leave(n)` (`n` the surface's unit normal there), and that
    /// normal. Without `read_shape`, the frame's shape is left at zero.
    fn station(&self, p: Vec3, leave: &dyn Fn(Vec3) -> Result<Vec3>, walk: &mut Walk, read_shape: bool) -> Result<(Frame, Vec3)> {
        let (surf, range_u, range_v, tol) = (&self.surf, self.range_u, self.range_v, self.tol);
        let project = |hint: Option<(f64, f64)>| surf.search_nearest_parameter(p3(p), hint, 100);
        let close = |uv: (f64, f64)| from_p3(surf.subs(uv.0, uv.1)).dist(p) <= tol;
        // The station before this one is the hint where it lands close; a chain that wraps the
        // surface (or a periodic one) falls back to a free search.
        let (u, v) = match project(walk.hint).filter(|uv| close(*uv)).or_else(|| project(None).filter(|uv| close(*uv))) {
            Some(uv) => uv,
            None => return Err(KernelError::Invalid("a picked edge is not on the support surface".into())),
        };
        walk.hint = Some((u, v));
        let at = from_p3(surf.subs(u, v));
        if at.dist(p) > tol {
            return Err(KernelError::Invalid("a picked edge is not on the support surface".into()));
        }
        let normal = surf.normal(u, v);
        let n = Vec3::new(normal.x, normal.y, normal.z)
            .normalized()
            .ok_or_else(|| KernelError::Failed("bridge: the support surface has no normal".into()))?;
        let w = leave(n)?;
        if !read_shape {
            return Ok((Frame { p, leave: w, bend: Vec3::ZERO, bend_change: Vec3::ZERO }, n));
        }
        // Parameter-space direction that leaves in `w` at unit 3D speed: S_u·du + S_v·dv = w.
        let su = tangent(surf, u, v, range_u, true);
        let sv = tangent(surf, u, v, range_v, false);
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let det = a * c - b * b;
        let scale = a.max(c).max(1e-30);
        if !(det.abs() > 1e-12 * scale * scale) {
            return Err(KernelError::Failed("bridge: the support surface is singular at the boundary".into()));
        }
        let du = (c * w.dot(su) - b * w.dot(sv)) / det;
        let dv = (a * w.dot(sv) - b * w.dot(su)) / det;
        // The samples of the parameter line. A station sits on the support surface's own
        // boundary, so the line usually leaves the surface: the shape is then read from the
        // samples of the one side that is still on it, which is the surface's own extension past
        // that boundary (exact for a plane and a cylinder, what a blend continues from).
        let slack = 1e-9;
        let on = |t: f64, h: f64| {
            let (x, y) = (u + t * h * du, v + t * h * dv);
            x >= range_u.0 - slack && x <= range_u.1 + slack && y >= range_v.0 - slack && y <= range_v.1 + slack
        };
        let central = [-2.0, -1.0, 0.0, 1.0, 2.0];
        let forward = [0.0, 1.0, 2.0, 3.0, 4.0];
        let backward = [0.0, -1.0, -2.0, -3.0, -4.0];
        let pick = |h: f64| -> Option<[f64; 5]> {
            if central.iter().all(|t| on(*t, h)) {
                Some(central)
            } else if forward.iter().all(|t| on(*t, h)) {
                Some(forward)
            } else if backward.iter().all(|t| on(*t, h)) {
                Some(backward)
            } else {
                None
            }
        };
        let mut h = walk.step;
        let mut chosen = pick(h);
        for _ in 0..8 {
            if chosen.is_some() {
                break;
            }
            h *= 0.25;
            chosen = pick(h);
        }
        let Some(offsets) = chosen else {
            return Err(KernelError::Invalid("the support surface is too small around a picked edge to read its shape".into()));
        };
        let q: Vec<Vec3> = offsets.iter().map(|t| from_p3(surf.subs(u + t * h * du, v + t * h * dv))).collect();
        let (Some(&f0), Some(&f1), Some(&f2), Some(&f3), Some(&f4)) = (q.first(), q.get(1), q.get(2), q.get(3), q.get(4)) else {
            return Err(KernelError::Failed("bridge: samples".into()));
        };
        // Central samples read the line both ways; one-sided samples read it one way only from the
        // station, backwards down it in the second case (the odd derivatives then change sign).
        let (d1, d2, d3) = if offsets[0] == 0.0 && offsets[1] < 0.0 {
            let d1 = (f0 * -25.0 + f1 * 48.0 + f2 * -36.0 + f3 * 16.0 + f4 * -3.0) * (-1.0 / (12.0 * h));
            let d2 = (f0 * 35.0 + f1 * -104.0 + f2 * 114.0 + f3 * -56.0 + f4 * 11.0) * (1.0 / (12.0 * h * h));
            let d3 = (f0 * -5.0 + f1 * 18.0 + f2 * -24.0 + f3 * 14.0 + f4 * -3.0) * (-1.0 / (2.0 * h * h * h));
            (d1, d2, d3)
        } else if offsets[0] == 0.0 {
            let d1 = (f0 * -25.0 + f1 * 48.0 + f2 * -36.0 + f3 * 16.0 + f4 * -3.0) * (1.0 / (12.0 * h));
            let d2 = (f0 * 35.0 + f1 * -104.0 + f2 * 114.0 + f3 * -56.0 + f4 * 11.0) * (1.0 / (12.0 * h * h));
            let d3 = (f0 * -5.0 + f1 * 18.0 + f2 * -24.0 + f3 * 14.0 + f4 * -3.0) * (1.0 / (2.0 * h * h * h));
            (d1, d2, d3)
        } else {
            let d1 = (f0 * -1.0 + f1 * 8.0 + f3 * -8.0 + f4) * (1.0 / (12.0 * h));
            let d2 = (f0 + f1 * -16.0 + f2 * 30.0 + f3 * -16.0 + f4) * (1.0 / (12.0 * h * h));
            let d3 = (f0 * -1.0 + f1 * 2.0 + f3 * -2.0 + f4) * (1.0 / (2.0 * h * h * h));
            (d1, d2, d3)
        };
        let (t, bend, bend_change) = shape(d1, d2, d3)?;
        // Read the same curve the other way when the surface's own parameter direction runs
        // against the leave direction: the curvature vector is the same either way, the change of
        // curvature turns round with the direction of travel.
        let bend_change = if t.dot(w) < 0.0 { -bend_change } else { bend_change };
        walk.step = h;
        Ok((Frame { p, leave: w, bend, bend_change }, n))
    }
}

/// A first derivative of the surface in one parameter, by central differences (one-sided where
/// the parameter is at the edge of the domain).
fn tangent(surf: &mt::Surface, u: f64, v: f64, range: (f64, f64), along_u: bool) -> Vec3 {
    let span = (range.1 - range.0).max(1e-12);
    let h = (span * 1e-5).max(1e-12);
    let at = |x: f64| if along_u { from_p3(surf.subs(x, v)) } else { from_p3(surf.subs(u, x)) };
    let x = if along_u { u } else { v };
    if x - h >= range.0 && x + h <= range.1 {
        (at(x + h) - at(x - h)) * (0.5 / h)
    } else if x + h <= range.1 {
        (at(x + h) - at(x)) * (1.0 / h)
    } else {
        (at(x.min(range.1)) - at((x - h).max(range.0))) * (1.0 / h)
    }
}

/// The geometric shape of a surface curve at a point from its first three derivatives along the
/// curve: the unit tangent, the curvature vector and the arclength derivative of that curvature
/// vector. All three are what a surface-to-surface continuity condition is stated with.
fn shape(d1: Vec3, d2: Vec3, d3: Vec3) -> Result<(Vec3, Vec3, Vec3)> {
    let sigma = d1.len();
    if !(sigma > 1e-12) || !sigma.is_finite() {
        return Err(KernelError::Failed("bridge: the support surface's parameter line does not move".into()));
    }
    let t = d1 * (1.0 / sigma);
    let tp = d2 * (1.0 / sigma) - d1 * (d1.dot(d2) / (sigma * sigma * sigma));
    let nn = d2 - t * d2.dot(t);
    let np = d3 - t * d3.dot(t) - tp * d2.dot(t) - t * d2.dot(tp);
    let kappa = np * (sigma * sigma) - nn * (2.0 * d1.dot(d2));
    let bend_change = kappa * (1.0 / (sigma * sigma * sigma * sigma * sigma));
    if ![t, nn, bend_change].into_iter().all(Vec3::is_finite) {
        return Err(KernelError::Failed("bridge: the support surface's shape is not finite at the boundary".into()));
    }
    Ok((t, nn * (1.0 / (sigma * sigma)), bend_change))
}

/// The third derivative a curve needs at a boundary so that the arclength derivative of its
/// curvature vector is `target`, from its first two derivatives there. Only the normal-plane
/// part of the third derivative can hold that condition; the rest is a reparameterisation of the
/// cross-section and is left at zero.
fn third_derivative(d1: Vec3, d2: Vec3, target: Vec3) -> Result<Vec3> {
    let sigma = d1.len();
    if !(sigma > 1e-12) || !sigma.is_finite() {
        return Err(KernelError::Failed("bridge: no room for the curvature condition".into()));
    }
    let t = d1 * (1.0 / sigma);
    let tp = d2 * (1.0 / sigma) - d1 * (d1.dot(d2) / (sigma * sigma * sigma));
    let nn = d2 - t * d2.dot(t);
    let r = tp * (-d2.dot(t)) - t * d2.dot(tp);
    let x = (target * (sigma.powi(5)) - r * (sigma * sigma) + nn * (2.0 * d1.dot(d2))) * (1.0 / (sigma * sigma));
    if !x.is_finite() {
        return Err(KernelError::Failed("bridge: the curvature condition is not finite".into()));
    }
    Ok(x - t * x.dot(t))
}

/// One station's cross-section: a Bézier whose leading control points hold the conditions of
/// each side. `speed` is how fast the cross-section leaves each boundary (any positive value; it
/// shapes the middle without touching the conditions).
fn section(a: &Frame, b: &Frame, orders: [Continuity; 2], n: usize, speed: [f64; 2]) -> Result<Vec<Vec3>> {
    let (Some(speed_a), Some(speed_b)) = (speed.first().copied(), speed.get(1).copied()) else {
        return Err(KernelError::Failed("bridge: speed".into()));
    };
    let (ka, kb) = (orders[0].conditions(), orders[1].conditions());
    if ka + kb - 1 != n {
        return Err(KernelError::Failed("bridge: degree".into()));
    }
    let mut c = vec![Vec3::ZERO; n + 1];
    if let Some(slot) = c.get_mut(0) {
        *slot = a.p;
    }
    if let Some(slot) = c.get_mut(n) {
        *slot = b.p;
    }
    let k = n as f64;
    // The first end: one control point per condition.
    if ka >= 2 {
        let p1 = a.p + a.leave * (speed_a / k);
        if let Some(slot) = c.get_mut(1) {
            *slot = p1;
        }
        if ka >= 3 {
            if n < 2 {
                return Err(KernelError::Failed("bridge: degree".into()));
            }
            let p2 = a.bend * (speed_a * speed_a / (k * (k - 1.0))) + p1 * 2.0 - a.p;
            if let Some(slot) = c.get_mut(2) {
                *slot = p2;
            }
            if ka >= 4 {
                if n < 3 {
                    return Err(KernelError::Failed("bridge: degree".into()));
                }
                let d1 = (p1 - a.p) * k;
                let d2 = (a.p - p1 * 2.0 + p2) * (k * (k - 1.0));
                let a3 = third_derivative(d1, d2, a.bend_change)?;
                if let Some(slot) = c.get_mut(3) {
                    *slot = a3 * (1.0 / (k * (k - 1.0) * (k - 2.0))) + p2 * 3.0 - p1 * 3.0 + a.p;
                }
            }
        }
    }
    // The second end, from its own conditions read backwards.
    if kb >= 2 {
        let q1 = b.p + b.leave * (speed_b / k);
        if let Some(slot) = c.get_mut(n - 1) {
            *slot = q1;
        }
        if kb >= 3 {
            if n < 2 {
                return Err(KernelError::Failed("bridge: degree".into()));
            }
            let q2 = b.bend * (speed_b * speed_b / (k * (k - 1.0))) + q1 * 2.0 - b.p;
            if let Some(slot) = c.get_mut(n - 2) {
                *slot = q2;
            }
            if kb >= 4 {
                if n < 3 {
                    return Err(KernelError::Failed("bridge: degree".into()));
                }
                let d1 = (q1 - b.p) * k;
                let d2 = (b.p - q1 * 2.0 + q2) * (k * (k - 1.0));
                let q3 = third_derivative(d1, d2, b.bend_change)?;
                if let Some(slot) = c.get_mut(n - 3) {
                    *slot = q3 * (1.0 / (k * (k - 1.0) * (k - 2.0))) + q2 * 3.0 - q1 * 3.0 + b.p;
                }
            }
        }
    }
    if c.iter().any(|p| !p.is_finite()) {
        return Err(KernelError::Failed("bridge: the cross-section is not finite".into()));
    }
    Ok(c)
}

/// The bridge surface between two boundaries of `stations` frames each: one B-spline face
/// leaving `a` and arriving at `b` with the requested continuity. `crown` scales how fast the
/// cross-sections leave each boundary (0: the plain bridge, up to 3: fuller, down to -0.9:
/// flatter), which shapes the middle without touching the conditions at either end.
pub fn bridge(a: &[Frame], b: &[Frame], orders: [Continuity; 2], crown: [f64; 2]) -> Result<Body> {
    let (Some(&crown_a), Some(&crown_b)) = (crown.first(), crown.get(1)) else { return Err(KernelError::Failed("bridge: crown".into())) };
    if a.len() != b.len() || a.len() < 2 || a.len() > 4097 {
        return Err(KernelError::Invalid("a bridge needs 2 to 4097 stations, the same on both sides".into()));
    }
    for c in [crown_a, crown_b] {
        if !c.is_finite() || !(-0.9..=3.0).contains(&c) {
            return Err(KernelError::Invalid("a bridge's crown must be between -0.9 and 3".into()));
        }
    }
    let stations = a.len();
    let n = orders[0].conditions() + orders[1].conditions() - 1;
    if n == 0 || n > solidcraft_geom::bspline::MAX_SPLINE_DEGREE {
        return Err(KernelError::Failed("bridge: degree".into()));
    }
    let mut layers: Vec<Vec<Vec3>> = vec![Vec::with_capacity(stations); n + 1];
    for (fa, fb) in a.iter().zip(b) {
        if !fa.p.is_finite() || !fb.p.is_finite() || !fa.leave.is_finite() || !fb.leave.is_finite() {
            return Err(KernelError::Invalid("a bridge station must be finite".into()));
        }
        let chord = fa.p.dist(fb.p);
        if !(chord > 1e-9) {
            return Err(KernelError::Invalid("the two boundaries meet: there is no room for a bridge".into()));
        }
        let speed = [chord / 3.0 * (1.0 + crown_a), chord / 3.0 * (1.0 + crown_b)];
        let controls = section(fa, fb, orders, n, speed)?;
        for (layer, c) in layers.iter_mut().zip(controls) {
            layer.push(c);
        }
    }
    // Along the bridge: a cubic through the stations' cross-sections, so the face passes through
    // each of them exactly.
    let params: Vec<f64> = (0..stations).map(|i| i as f64 / (stations - 1) as f64).collect();
    let degree = 3.min(stations - 1);
    let mut knots_u: Option<Vec<f64>> = None;
    let mut ctrl = vec![Vec3::ZERO; (n + 1) * stations];
    for (j, layer) in layers.iter().enumerate() {
        let points: Vec<[f64; 3]> = layer.iter().map(|p| [p.x, p.y, p.z]).collect();
        let curve = CurveN::<3>::interpolate(&params, &points, degree).map_err(|e| KernelError::Failed(format!("bridge: {e}")))?;
        if knots_u.is_none() {
            knots_u = Some(curve.knots.clone());
        }
        for (i, c) in curve.ctrl.iter().enumerate() {
            if let Some(slot) = ctrl.get_mut(j * stations + i) {
                *slot = Vec3::new(c[0], c[1], c[2]);
            }
        }
    }
    let knots_u = knots_u.ok_or_else(|| KernelError::Failed("bridge: knots".into()))?;
    let mut knots_v = vec![0.0; n + 1];
    knots_v.extend(std::iter::repeat_n(1.0, n + 1));
    let points: Vec<HPoint> = ctrl.iter().map(|p| HPoint::weighted(*p, 1.0)).collect();
    let nurbs = NurbsSurface::from_homogeneous([degree, n], [knots_u, knots_v], points).map_err(|e| KernelError::Failed(format!("bridge: {e}")))?;
    guard("bridge surface", || crate::spline_conv::tensor_face(&nurbs).map(|f| surface_body(vec![f])))
}

/// The surface of a bridge face as a grid of points, for callers that measure the result.
pub fn sample_grid(body: &Body, along: usize, across: usize) -> Result<Vec<Vec<Vec3>>> {
    if !(2..=1025).contains(&along) || !(2..=1025).contains(&across) {
        return Err(KernelError::Invalid("a grid needs 2 to 1025 samples each way".into()));
    }
    let face = body.solid.face_iter().next().ok_or_else(|| KernelError::Failed("bridge: no face".into()))?;
    let surf = face.surface();
    let (Some((u0, u1)), Some((v0, v1))) = surf.try_range_tuple() else {
        return Err(KernelError::Failed("bridge: the surface is unbounded".into()));
    };
    let mut out = Vec::with_capacity(along);
    for i in 0..along {
        let u = u0 + (u1 - u0) * i as f64 / (along - 1) as f64;
        let mut row = Vec::with_capacity(across);
        for j in 0..across {
            let v = v0 + (v1 - v0) * j as f64 / (across - 1) as f64;
            row.push(from_p3(surf.subs(u, v)));
        }
        out.push(row);
    }
    Ok(out)
}

/// The most any point of a bridge's boundary row is from the chain it was built on — the row at
/// `v0` for the first boundary, at `v1` for the second: what a stitch would have to close up.
fn boundary_gap(made: &Body, chain: &[mt::Edge], at_end: bool) -> Result<f64> {
    let face = made.solid.face_iter().next().ok_or_else(|| KernelError::Failed("bridge: no face".into()))?;
    let surf = face.surface();
    let (Some((u0, u1)), Some((v0, v1))) = surf.try_range_tuple() else {
        return Err(KernelError::Failed("bridge: the surface is unbounded".into()));
    };
    let v = if at_end { v1 } else { v0 };
    let (poly, _) = chain_samples(chain, 257)?;
    let mut worst: f64 = 0.0;
    for i in 0..=256 {
        let u = u0 + (u1 - u0) * i as f64 / 256.0;
        let p = from_p3(surf.subs(u, v));
        let mut best = f64::INFINITY;
        for w in poly.windows(2) {
            let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
            let t = if b.dist(*a) > 1e-12 { ((p - *a).dot(*b - *a) / b.dist(*a).powi(2)).clamp(0.0, 1.0) } else { 0.0 };
            best = best.min(p.dist(*a + (*b - *a) * t));
        }
        worst = worst.max(best);
    }
    Ok(worst)
}

/// The midpoint of a chain of edges.
fn chain_mid(edges: &[mt::Edge]) -> Result<Vec3> {
    let (pts, _) = chain_samples(edges, 3)?;
    pts.get(1).copied().ok_or_else(|| KernelError::Failed("bridge: no chain".into()))
}

/// Which face of `body` the blend should continue from a chain, and the direction it leaves in:
/// the face whose tangent plane holds as much of `leave` as possible.
fn support_face(body: &Body, chain: &[mt::Edge], leave: Vec3) -> Result<(mt::Face, Vec3)> {
    let mut best: Option<(f64, mt::Face)> = None;
    for face in body.solid.face_iter() {
        if !chain.iter().all(|e| face.edge_iter().any(|f| f.id() == e.id())) {
            continue;
        }
        let mid = chain_mid(chain)?;
        let surf = face.surface();
        let Some((u, v)) = surf.search_nearest_parameter(p3(mid), None, 100) else { continue };
        if from_p3(surf.subs(u, v)).dist(mid) > body.size() * 1e-6 + 1e-9 {
            continue;
        }
        let normal = surf.normal(u, v);
        let n = Vec3::new(normal.x, normal.y, normal.z).normalized().unwrap_or(Vec3::ZERO);
        let in_plane = leave - n * leave.dot(n);
        let score = in_plane.len();
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, face.clone()));
        }
    }
    let (score, face) = best.ok_or_else(|| KernelError::Invalid("no face of the body holds the picked edges".into()))?;
    if score < 0.2 {
        return Err(KernelError::Invalid(
            "the direction towards the other boundary lies in no face at the picked edges: give `align` points to leave by".into(),
        ));
    }
    Ok((face, leave))
}

/// A blend surface between two picked edge chains of two bodies: the surfaces the blend leaves
/// and arrives at are the faces the chains bound (the best-facing one at each side), and each
/// side repeats that surface up to its requested continuity. `align` overrides the direction
/// each side leaves in (points in world space); `crown` how full each side is; `stitch` sews the
/// blend to its two bodies where their boundaries meet. The reply's `gap`, if any, is how far the
/// blend's own boundary lay from the chains it was built on.
pub fn blend_between(
    a: &Body,
    picks_a: &[Vec3],
    b: &Body,
    picks_b: &[Vec3],
    orders: [Continuity; 2],
    crown: [f64; 2],
    align: Option<[Vec3; 2]>,
    samples: usize,
    stitch: bool,
) -> Result<(Body, f64)> {
    a.require_brep("blend")?;
    b.require_brep("blend")?;
    if !(5..=401).contains(&samples) {
        return Err(KernelError::Invalid("a blend needs 5 to 401 stations".into()));
    }
    let chain_a = chain(a, picks_a)?;
    let chain_b = chain(b, picks_b)?;
    let (mid_a, mid_b) = (chain_mid(&chain_a)?, chain_mid(&chain_b)?);
    let (leave_a, leave_b) = match align {
        Some([p, q]) => (
            (p - mid_a).normalized().ok_or_else(|| KernelError::Invalid("`align` must not sit on the boundary".into()))?,
            (q - mid_b).normalized().ok_or_else(|| KernelError::Invalid("`align` must not sit on the boundary".into()))?,
        ),
        None => (
            (mid_b - mid_a).normalized().ok_or_else(|| KernelError::Invalid("the two boundaries coincide: there is nothing to blend".into()))?,
            (mid_a - mid_b).normalized().ok_or_else(|| KernelError::Invalid("the two boundaries coincide: there is nothing to blend".into()))?,
        ),
    };
    let (face_a, leave_a) = support_face(a, &chain_a, leave_a)?;
    let (face_b, leave_b) = support_face(b, &chain_b, leave_b)?;
    let frames_a = frames(&face_a, &chain_a, leave_a, samples)?;
    let mut frames_b = frames(&face_b, &chain_b, leave_b, samples)?;
    // The two chains are sampled from their own starts: turn one round when that pairs the
    // stations better (the blend is built station to station).
    let direct: f64 = frames_a.iter().zip(&frames_b).map(|(x, y)| x.p.dist(y.p)).sum();
    let reversed: f64 = frames_a.iter().zip(frames_b.iter().rev()).map(|(x, y)| x.p.dist(y.p)).sum();
    if reversed < direct {
        frames_b.reverse();
    }
    let made = bridge(&frames_a, &frames_b, orders, crown)?;
    let gap = boundary_gap(&made, &chain_a, false)?.max(boundary_gap(&made, &chain_b, true)?);
    let faces: Vec<mt::Face> = made.solid.face_iter().cloned().collect();
    let mut rec = crate::record::Rebuild::default();
    for f in &faces {
        rec.made(f, Vec::new());
    }
    let blend = crate::record::rebuilt_body(surface_body(faces), &[&a.solid, &b.solid], &rec);
    if !stitch {
        return Ok((blend, gap));
    }
    // Sewing up a boundary the blend only approximates: the stitch is given what the boundary
    // actually strays, so that a real gap between the bodies is still refused.
    let out = crate::surfaces::stitch(&[a, b, &blend], gap * 1.05 + 1e-9)?;
    Ok((out, gap))
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
