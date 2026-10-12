//! Booleans of prisms in 2D: when the target is a prism (two caps square to one direction,
//! walls of planes and cylinders along it) and the tool a prism along the same direction
//! spanning the target's height, the result is the prism over the 2D boolean of their
//! sections. Plates with holes, slots and patterns of them: exact, and immune to the mesh
//! intersection's trouble with walls meeting along lines.
//! Stepped extrusions (case floors and walls) use sections between every cap level, so a
//! circular boss can meet the rim without shifting either operand off its exact plane.

use solvecraft_geom::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use truck_modeling as mt;

use crate::body::{Body, from_p3};
use crate::{BoolOp, KernelError, Result};

const MAX_PLANAR_SEGMENTS: usize = 8192;
const MAX_PLANAR_EVENTS: usize = 131_072;
const MAX_PLANAR_PAIRS: usize = 16_000_000;

/// Boolean of planar regions bounded by straight lines and circular arcs. Boundaries stay
/// analytic; disjoint results and retained islands are returned as separate regions.
pub fn planar_regions_boolean(a: &[Region2], b: &[Region2], op: BoolOp, tolerance: f64) -> Result<Vec<Region2>> {
    crate::guard("planar region boolean", || {
        if !tolerance.is_finite() || !(1e-9..=1.0).contains(&tolerance) {
            return Err(KernelError::Invalid("planar boolean tolerance must be finite and between 1e-9 and 1 mm".into()));
        }
        let loops = || a.iter().chain(b).flat_map(|r| std::iter::once(&r.outer).chain(&r.holes));
        let mut edges = 0usize;
        let mut samples = 0usize;
        for lp in loops() {
            edges = edges.saturating_add(lp.segs.len());
            if edges > MAX_PLANAR_SEGMENTS || lp.segs.is_empty() || !lp.is_closed(tolerance * 10.0) {
                return Err(KernelError::Invalid("planar boolean needs closed loops within its geometry budget".into()));
            }
            for s in &lp.segs {
                let finite = |p: Vec2| p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9;
                let valid = match *s {
                    Seg2::Line { a, b } => finite(a) && finite(b) && a.dist(b) > 1e-12,
                    Seg2::Arc { center, radius, start, sweep } => {
                        finite(center)
                            && radius.is_finite()
                            && radius > 0.0
                            && radius < 1e9
                            && start.is_finite()
                            && sweep.is_finite()
                            && sweep.abs() > 1e-12
                            && sweep.abs() <= std::f64::consts::TAU + 1e-9
                    }
                    _ => false,
                };
                if !valid {
                    return Err(KernelError::Invalid("planar boolean supports finite nondegenerate lines and circular arcs".into()));
                }
                samples = samples.saturating_add(s.polyline(1e-3).len());
                if samples > 65_536 {
                    return Err(KernelError::Invalid("planar boolean sampled geometry exceeds its budget".into()));
                }
            }
        }
        boolean2d(a, b, op, tolerance)
            .ok_or_else(|| KernelError::Failed("planar region arrangement failed or exceeded its bounded processing budget".into()))
    })
}

/// Extrude normalized planar material as one logical compound, retaining every disjoint
/// region and its holes. Planar arrangement certifies separation before shells are joined.
pub fn extrude_planar_regions(plane: &Plane, regions: &[Region2], start: f64, end: f64) -> Result<Body> {
    crate::guard("planar material extrusion", || {
        // Validate aggregate inputs first; an arrangement compares opposite operands,
        // so regions in the same operand must be normalized through balanced unions.
        planar_regions_boolean(regions, &[], BoolOp::Union, 1e-7)?;
        let mut groups: Vec<Vec<Region2>> = regions.iter().cloned().map(|r| vec![r]).collect();
        let mut remaining_pairs = 32_000_000usize;
        while groups.len() > 1 {
            let mut next = Vec::new();
            let mut iter = groups.into_iter();
            while let Some(a) = iter.next() {
                if let Some(b) = iter.next() {
                    let count = |rs: &[Region2]| {
                        rs.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).fold(0usize, |n, l| n.saturating_add(l.segs.len()))
                    };
                    remaining_pairs = remaining_pairs
                        .checked_sub(count(&a).saturating_mul(count(&b)))
                        .ok_or_else(|| KernelError::Invalid("planar extrusion arrangement exceeds its work budget".into()))?;
                    next.push(planar_regions_boolean(&a, &b, BoolOp::Union, 1e-7)?);
                } else {
                    next.push(a);
                }
            }
            groups = next;
        }
        let normalized = groups.pop().unwrap_or_default();
        if normalized.is_empty() {
            return Err(KernelError::Invalid("empty planar material extrusion".into()));
        }
        for (i, a) in normalized.iter().enumerate() {
            for b in normalized.iter().skip(i + 1) {
                for s in std::iter::once(&a.outer).chain(&a.holes).flat_map(|l| &l.segs) {
                    for t in std::iter::once(&b.outer).chain(&b.holes).flat_map(|l| &l.segs) {
                        remaining_pairs = remaining_pairs
                            .checked_sub(1)
                            .ok_or_else(|| KernelError::Invalid("planar extrusion separation exceeds its work budget".into()))?;
                        let endpoints_touch = [s.start(), s.end()].iter().any(|p| t.point_at(t.closest_param(*p)).dist(*p) <= 1e-7)
                            || [t.start(), t.end()].iter().any(|p| s.point_at(s.closest_param(*p)).dist(*p) <= 1e-7);
                        if endpoints_touch || !crossings(s, t, 1e-7).is_empty() {
                            return Err(KernelError::Invalid("planar material pieces touch at a boundary".into()));
                        }
                    }
                }
            }
        }
        let pieces = crate::extrude(plane, &normalized, start, end)?;
        crate::ops::join_pieces(pieces)?.ok_or_else(|| KernelError::Failed("empty planar material extrusion".into()))
    })
}

/// A prism: the plane of its bottom cap (normal along the walls), its height and section.
struct Prism {
    lo: f64,
    hi: f64,
    regions: Vec<Region2>,
}

/// Certify that a curved wall is swept along `d` from its surface representation.
/// Seam vertices alone cannot establish this: a half-cylinder's seam normals can
/// both be perpendicular to a transverse direction, silently filling its hole.
fn wall_along(surface: &mt::Surface, d: Vec3, tol: f64, remaining: &mut usize) -> bool {
    if let mt::Surface::RevolutedCurve(processor) = surface {
        let revolution = processor.entity();
        let mt::Curve::Line(line) = revolution.entity_curve() else { return false };
        let axis = revolution.axis();
        let meridian = line.1 - line.0;
        let local_axis = Vec3::new(axis.x, axis.y, axis.z);
        let local_meridian = Vec3::new(meridian.x, meridian.y, meridian.z);
        let (Some(a), Some(m)) = (local_axis.normalized(), local_meridian.normalized()) else { return false };
        if a.cross(m).len() > 1e-9 {
            return false; // A conical meridian is not a cylindrical wall.
        }
        let placed = *processor.transform() * mt::Vector4::new(axis.x, axis.y, axis.z, 0.0);
        return placed.w.abs() < 1e-12 && Vec3::new(placed.x, placed.y, placed.z).normalized().is_some_and(|a| a.cross(d).len() < 1e-9);
    }
    let dimensions = match surface {
        mt::Surface::BSplineSurface(s) => (s.control_points().len(), s.control_points().first().map_or(0, Vec::len)),
        mt::Surface::NurbsSurface(s) => (s.control_points().len(), s.control_points().first().map_or(0, Vec::len)),
        _ => return false,
    };
    let (rows, cols) = dimensions;
    let count = rows.saturating_mul(cols);
    let rectangular = match surface {
        mt::Surface::BSplineSurface(s) => s.control_points().iter().all(|r| r.len() == cols),
        mt::Surface::NurbsSurface(s) => s.control_points().iter().all(|r| r.len() == cols),
        _ => false,
    };
    if rows < 2 || cols < 2 || count > 16_384 || !rectangular {
        return false;
    }
    let Some(left) = remaining.checked_sub(count.saturating_mul(2)) else { return false };
    *remaining = left;
    let points: Vec<Vec<(Vec3, f64)>> = match surface {
        mt::Surface::BSplineSurface(s) => s.control_points().iter().map(|r| r.iter().map(|p| (from_p3(*p), 1.0)).collect()).collect(),
        mt::Surface::NurbsSurface(s) => {
            s.control_points().iter().map(|r| r.iter().map(|p| (Vec3::new(p.x / p.w, p.y / p.w, p.z / p.w), p.w)).collect()).collect()
        }
        _ => return false,
    };
    if points.iter().any(|r| r.len() != cols)
        || points.iter().flatten().any(|(p, w)| !w.is_finite() || *w <= 0.0 || [p.x, p.y, p.z].iter().any(|x| !x.is_finite()))
    {
        return false;
    }
    let max_weight = points.iter().flatten().map(|p| p.1).fold(0.0, f64::max);
    if points.iter().flatten().any(|p| p.1 / max_weight < 1e-12) {
        return false;
    }
    // Either parameter can be the extrusion direction. Projected positions must
    // be invariant in that parameter; rational weights must also separate into
    // a product of one factor per parameter, otherwise mixed weights bend the wall.
    for transpose in [false, true] {
        let (groups, length) = if transpose { (cols, rows) } else { (rows, cols) };
        let at = |i: usize, j: usize| if transpose { points[j][i] } else { points[i][j] };
        let mut valid = true;
        let mut extended = false;
        for i in 0..groups {
            let (base, base_w) = at(i, 0);
            for j in 0..length {
                let (p, w) = at(i, j);
                let delta = p - base;
                let magnitude = [p.x.abs(), p.y.abs(), p.z.abs(), base.x.abs(), base.y.abs(), base.z.abs()].into_iter().fold(1.0, f64::max);
                if delta.cross(d).len() > tol * 1e-3 + magnitude * f64::EPSILON * 128.0 {
                    valid = false;
                    break;
                }
                extended |= delta.dot(d).abs() > tol * 1e-3;
                let lhs = w / max_weight * (at(0, 0).1 / max_weight);
                let rhs = base_w / max_weight * (at(0, j).1 / max_weight);
                if (lhs - rhs).abs() > 1e-12 * lhs.abs().max(rhs.abs()) {
                    valid = false;
                    break;
                }
            }
            if !valid {
                break;
            }
        }
        if valid && extended {
            return true;
        }
    }
    false
}

/// The body as a prism along `d` (unit), seen in `plane` (normal `d`).
fn prism_of(b: &Body, d: Vec3, plane: &Plane, tol: f64) -> Option<Prism> {
    // A simple extrusion with N section edges has N walls plus its cap faces.
    // Even disconnected two-arc sections need at most 2N faces. Keep that
    // complete planar arrangement envelope, independently of stepped limits.
    let mut remaining_faces = MAX_PLANAR_SEGMENTS * 2;
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut tops: Vec<&mt::Face> = Vec::new();
    let mut top_h = Vec::new();
    let mut bottom_h = Vec::new();
    let mut wall_work = 262_144;
    let mut curve_work = 2_000_000usize;
    for f in b.solid.face_iter() {
        remaining_faces = remaining_faces.checked_sub(1)?;
        curve_work = curve_work.checked_sub(1)?;
        let surf = f.oriented_surface();
        if let mt::Surface::Plane(p) = &surf {
            let n = p.normal();
            let n = Vec3::new(n.x, n.y, n.z).normalized()?;
            let h = plane.height(from_p3(p.origin()));
            if n.dot(d) > 1.0 - 1e-9 {
                tops.push(f);
                top_h.push(h);
                hi = hi.max(h);
                continue;
            }
            if n.dot(d) < -1.0 + 1e-9 {
                bottom_h.push(h);
                lo = lo.min(h);
                continue;
            }
            if n.dot(d).abs() > 1e-9 {
                return None;
            }
            continue;
        }
        if !wall_along(&surf, d, tol, &mut wall_work) {
            return None;
        }
    }
    // One height for every top (and bottom) cap.
    if top_h.iter().any(|h| (h - hi).abs() > tol) || bottom_h.iter().any(|h| (h - lo).abs() > tol) || !(lo.is_finite() && hi > lo + tol) {
        return None;
    }
    let mut regions = Vec::new();
    let mut remaining_segments = MAX_PLANAR_SEGMENTS;
    let mut remaining_loops = MAX_PLANAR_SEGMENTS;
    for f in tops {
        let mut loops = Vec::new();
        for w in f.boundaries() {
            remaining_loops = remaining_loops.checked_sub(1)?;
            let mut segs = Vec::new();
            for e in w.edge_iter() {
                let segments = crate::prism_curve::segment(e, plane, tol, &mut curve_work)?;
                remaining_segments = remaining_segments.checked_sub(segments.len())?;
                segs.extend(segments);
            }
            loops.push(Loop2 { segs });
        }
        let (outer, holes) = split_outer(loops)?;
        regions.push(Region2 { outer, holes });
    }
    Some(Prism { lo, hi, regions })
}

/// The loop with the largest area is the outer one.
fn split_outer(mut loops: Vec<Loop2>) -> Option<(Loop2, Vec<Loop2>)> {
    let i = (0..loops.len()).max_by(|a, b| loops[*a].signed_area().abs().total_cmp(&loops[*b].signed_area().abs()))?;
    let outer = loops.remove(i);
    Some((outer, loops))
}

fn heights_of(b: &Body, plane: &Plane) -> (f64, f64) {
    b.solid.vertex_iter().map(|v| plane.height(from_p3(v.point()))).fold((f64::INFINITY, f64::NEG_INFINITY), |m, h| (m.0.min(h), m.1.max(h)))
}

/// Points where two segments cross (parameters on each), lines and arcs only.
fn crossings(s: &Seg2, t: &Seg2, tol: f64) -> Vec<(f64, f64)> {
    let mut pts: Vec<Vec2> = Vec::new();
    match (*s, *t) {
        (Seg2::Line { a, b }, Seg2::Line { a: c, b: d }) => {
            let (r, q) = (b - a, d - c);
            let den = r.cross(q);
            if den.abs() > 1e-300 {
                let u = (c - a).cross(q) / den;
                pts.push(a + r * u);
            }
        }
        (Seg2::Line { a, b }, Seg2::Arc { center, radius, .. }) | (Seg2::Arc { center, radius, .. }, Seg2::Line { a, b }) => {
            let dir = b - a;
            let f = a - center;
            let (qa, qb, qc) = (dir.dot(dir), 2.0 * f.dot(dir), f.dot(f) - radius * radius);
            let disc = qb * qb - 4.0 * qa * qc;
            if qa > 1e-300 && disc >= -tol * tol * qa {
                let sd = disc.max(0.0).sqrt();
                for u in [(-qb - sd) / (2.0 * qa), (-qb + sd) / (2.0 * qa)] {
                    pts.push(a + dir * u);
                }
            }
        }
        (Seg2::Arc { center: c0, radius: r0, .. }, Seg2::Arc { center: c1, radius: r1, .. }) => {
            let dd = c0.dist(c1);
            if dd > 1e-12 && dd <= r0 + r1 + tol && dd >= (r0 - r1).abs() - tol {
                let a = (r0 * r0 - r1 * r1 + dd * dd) / (2.0 * dd);
                let h = (r0 * r0 - a * a).max(0.0).sqrt();
                let m = c0 + (c1 - c0) * (a / dd);
                let perp = (c1 - c0).perp() * (1.0 / dd);
                pts.push(m + perp * h);
                pts.push(m - perp * h);
            }
        }
        _ => {}
    }
    let mut out = Vec::new();
    for p in pts {
        let (u, v) = (s.closest_param(p), t.closest_param(p));
        if s.point_at(u).dist(p) < tol && t.point_at(v).dist(p) < tol {
            out.push((u, v));
        }
    }
    out
}

/// Ray parity against analytic lines/arcs. Fragment classification must not depend on a
/// display chord tolerance: a tiny tool can sit between a circular boundary and its chords.
fn inside_loop(lp: &Loop2, p: Vec2) -> bool {
    let mut odd = false;
    for seg in &lp.segs {
        match *seg {
            Seg2::Line { a, b } => {
                if (a.y > p.y) != (b.y > p.y) {
                    let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                    if x > p.x {
                        odd = !odd;
                    }
                }
            }
            Seg2::Arc { center, radius, start, sweep } => {
                let y = (p.y - center.y) / radius;
                if !(-1.0..=1.0).contains(&y) {
                    continue;
                }
                let a = y.asin();
                let root = (1.0 - y * y).max(0.0).sqrt();
                let full = (sweep.abs() - std::f64::consts::TAU).abs() < 1e-12;
                for (i, (angle, x)) in [(a, center.x + radius * root), (std::f64::consts::PI - a, center.x - radius * root)].into_iter().enumerate() {
                    // A tangent has one geometric root. Interior tangencies contribute zero
                    // parity; a minimum at an endpoint follows the same half-open rule as lines.
                    if root == 0.0 && i == 1 {
                        continue;
                    }
                    if x <= p.x {
                        continue;
                    }
                    let mut delta = if sweep > 0.0 { angle - start } else { start - angle };
                    delta = delta.rem_euclid(std::f64::consts::TAU);
                    if (delta - std::f64::consts::TAU).abs() < 1e-12 {
                        delta = 0.0;
                    }
                    if delta > sweep.abs() + 1e-12 {
                        continue;
                    }
                    let dy = angle.cos() * sweep;
                    let tangent = dy.abs() < sweep.abs() * 1e-12;
                    let at_start = delta < 1e-12;
                    let at_end = (delta - sweep.abs()).abs() < 1e-12;
                    let crosses = if full {
                        !tangent
                    } else if at_start {
                        dy > 0.0 && !tangent || tangent && y < 0.0
                    } else if at_end {
                        dy < 0.0 && !tangent || tangent && y < 0.0
                    } else {
                        !tangent
                    };
                    if crosses {
                        odd = !odd;
                    }
                }
            }
            _ => {
                // Private prism callers only supply lines/arcs. Keep a controlled fallback
                // for future internal callers; the public helper rejects other segment kinds.
                let points = seg.polyline(1e-7);
                for pair in points.windows(2) {
                    if let [a, b] = pair
                        && (a.y > p.y) != (b.y > p.y)
                        && a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) > p.x
                    {
                        odd = !odd;
                    }
                }
            }
        }
    }
    odd
}

fn inside(regions: &[Region2], p: Vec2) -> bool {
    regions.iter().any(|r| inside_loop(&r.outer, p) && !r.holes.iter().any(|h| inside_loop(h, p)))
}

fn dist_to(regions: &[Region2], p: Vec2) -> f64 {
    regions
        .iter()
        .flat_map(|r| std::iter::once(&r.outer).chain(&r.holes))
        .flat_map(|l| l.segs.iter())
        .map(|s| s.point_at(s.closest_param(p)).dist(p))
        .fold(f64::INFINITY, f64::min)
}

/// Direction of the boundary of `regions` through `p` (on it), as the boundary runs.
fn dir_at(regions: &[Region2], p: Vec2, tol: f64) -> Option<Vec2> {
    for r in regions {
        for l in std::iter::once(&r.outer).chain(&r.holes) {
            for s in &l.segs {
                let t = s.closest_param(p);
                if s.point_at(t).dist(p) < tol {
                    return s.derivative(t).normalized();
                }
            }
        }
    }
    None
}

/// Loops oriented: outers counter-clockwise, holes clockwise.
fn oriented(regions: &[Region2]) -> Vec<Region2> {
    regions
        .iter()
        .map(|r| Region2 {
            outer: if r.outer.signed_area() < 0.0 { r.outer.reversed() } else { r.outer.clone() },
            holes: r.holes.iter().map(|h| if h.signed_area() > 0.0 { h.reversed() } else { h.clone() }).collect(),
        })
        .collect()
}

/// 2D boolean of regions bounded by lines and arcs.
fn boolean2d(a: &[Region2], b: &[Region2], op: BoolOp, tol: f64) -> Option<Vec<Region2>> {
    let count =
        |rs: &[Region2]| rs.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).fold(0usize, |n, lp| n.saturating_add(lp.segs.len()));
    let (na, nb) = (count(a), count(b));
    if na.saturating_add(nb) > MAX_PLANAR_SEGMENTS || na.saturating_mul(nb) > MAX_PLANAR_PAIRS {
        return None;
    }
    let (a, b) = (oriented(a), oriented(b));
    let segs =
        |rs: &[Region2]| -> Vec<Seg2> { rs.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).flat_map(|l| l.segs.clone()).collect() };
    let (sa, sb) = (segs(&a), segs(&b));
    if sa.iter().chain(&sb).any(|s| !matches!(s, Seg2::Line { .. } | Seg2::Arc { .. })) {
        return None;
    }
    // Split every segment where it crosses the other side's boundary.
    let mut cut_a: Vec<Vec<f64>> = vec![Vec::new(); sa.len()];
    let mut cut_b: Vec<Vec<f64>> = vec![Vec::new(); sb.len()];
    let mut events = 0usize;
    for (i, s) in sa.iter().enumerate() {
        for (j, t) in sb.iter().enumerate() {
            for (u, v) in crossings(s, t, tol) {
                events += 2;
                if events > MAX_PLANAR_EVENTS {
                    return None;
                }
                cut_a.get_mut(i)?.push(u);
                cut_b.get_mut(j)?.push(v);
            }
        }
    }
    // Ends of the other side's segments lying on a segment split it too (touching corners).
    for (i, s) in sa.iter().enumerate() {
        for t in &sb {
            for p in [t.start(), t.end()] {
                let u = s.closest_param(p);
                if s.point_at(u).dist(p) < tol {
                    events += 1;
                    if events > MAX_PLANAR_EVENTS {
                        return None;
                    }
                    cut_a.get_mut(i)?.push(u);
                }
            }
        }
    }
    for (j, t) in sb.iter().enumerate() {
        for s in &sa {
            for p in [s.start(), s.end()] {
                let v = t.closest_param(p);
                if t.point_at(v).dist(p) < tol {
                    events += 1;
                    if events > MAX_PLANAR_EVENTS {
                        return None;
                    }
                    cut_b.get_mut(j)?.push(v);
                }
            }
        }
    }
    let pieces = |s: &Seg2, cuts: &mut Vec<f64>| -> Vec<Seg2> {
        cuts.sort_by(f64::total_cmp);
        let mut out = Vec::new();
        let mut rest = *s;
        let mut done = 0.0;
        for &c in cuts.iter() {
            if c <= done + 1e-9 || c >= 1.0 - 1e-9 {
                continue;
            }
            let local = (c - done) / (1.0 - done);
            let (x, y) = rest.split_at(local);
            if x.length() > tol {
                out.push(x);
                rest = y;
                done = c;
            }
        }
        out.push(rest);
        out
    };
    let mut kept: Vec<Seg2> = Vec::new();
    // A's pieces.
    for (i, s) in sa.iter().enumerate() {
        if cut_a.get(i)?.len() > MAX_PLANAR_SEGMENTS {
            return None;
        }
        for p in pieces(s, cut_a.get_mut(i)?) {
            let m = p.point_at(0.5);
            let on = dist_to(&b, m) < tol;
            let keep = if on {
                let same = dir_at(&b, m, tol).is_some_and(|d| d.dot(p.derivative(0.5)) > 0.0);
                match op {
                    BoolOp::Union | BoolOp::Intersect => same,
                    BoolOp::Cut => !same,
                }
            } else {
                let ins = inside(&b, m);
                match op {
                    BoolOp::Union | BoolOp::Cut => !ins,
                    BoolOp::Intersect => ins,
                }
            };
            if keep {
                if kept.len() >= MAX_PLANAR_SEGMENTS {
                    return None;
                }
                kept.push(p);
            }
        }
    }
    // B's pieces (coincident ones were decided with A's).
    for (j, t) in sb.iter().enumerate() {
        if cut_b.get(j)?.len() > MAX_PLANAR_SEGMENTS {
            return None;
        }
        for p in pieces(t, cut_b.get_mut(j)?) {
            let m = p.point_at(0.5);
            if dist_to(&a, m) < tol {
                continue;
            }
            let ins = inside(&a, m);
            if kept.len() >= MAX_PLANAR_SEGMENTS {
                return None;
            }
            match op {
                BoolOp::Union if !ins => kept.push(p),
                BoolOp::Intersect if ins => kept.push(p),
                BoolOp::Cut if ins => kept.push(p.reversed()),
                _ => {}
            }
        }
    }
    // Chain into loops.
    let mut loops: Vec<Loop2> = Vec::new();
    let mut used = vec![false; kept.len()];
    for start in 0..kept.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut segs = vec![*kept.get(start)?];
        if let Some(u) = used.get_mut(start) {
            *u = true;
        }
        let first = kept.get(start)?.start();
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > kept.len() + 2 {
                return None;
            }
            let end = segs.last()?.end();
            let full_circle =
                segs.first().is_some_and(|s| matches!(s, Seg2::Arc { sweep, .. } if (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9));
            if end.dist(first) < tol * 10.0 && (segs.len() > 1 || full_circle) {
                break;
            }
            let next = (0..kept.len())
                .filter(|k| !used.get(*k).copied().unwrap_or(true))
                .min_by(|x, y| kept[*x].start().dist(end).total_cmp(&kept[*y].start().dist(end)))?;
            if kept.get(next)?.start().dist(end) > tol * 10.0 {
                if end.dist(first) < tol * 10.0 {
                    break;
                }
                return None;
            }
            if let Some(u) = used.get_mut(next) {
                *u = true;
            }
            segs.push(*kept.get(next)?);
        }
        let l = Loop2 { segs };
        if l.signed_area().abs() > tol * tol {
            loops.push(l);
        }
    }
    // Outers (counter-clockwise) take the holes they contain, the smallest first.
    let (mut outers, holes): (Vec<Loop2>, Vec<Loop2>) = loops.into_iter().partition(|l| l.signed_area() > 0.0);
    outers.sort_by(|x, y| x.signed_area().abs().total_cmp(&y.signed_area().abs()));
    let mut regions: Vec<Region2> = outers.into_iter().map(|o| Region2 { outer: o, holes: Vec::new() }).collect();
    for h in holes {
        let p = h.segs.first()?.point_at(0.5);
        let r = regions.iter_mut().find(|r| inside_loop(&r.outer, p) && r.outer.signed_area().abs() > h.signed_area().abs())?;
        r.holes.push(h);
    }
    Some(regions)
}

/// The boolean done on prism sections, or `None` when it doesn't apply.
/// The pieces of a segment between the given points (those lying on it, inside it).
fn split_seg(seg: &Seg2, pts: &[Vec2], tol: f64) -> Option<Vec<Seg2>> {
    if !matches!(seg, Seg2::Line { .. } | Seg2::Arc { .. }) {
        return None;
    }
    let mut ts: Vec<f64> = pts
        .iter()
        .filter_map(|p| {
            let t = seg.closest_param(*p);
            (t > 1e-9 && t < 1.0 - 1e-9 && seg.point_at(t).dist(*p) < tol).then_some(t)
        })
        .collect();
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    let mut out = Vec::new();
    let mut t0 = 0.0;
    for t1 in ts.into_iter().chain(std::iter::once(1.0)) {
        out.push(match *seg {
            Seg2::Line { .. } => Seg2::Line { a: seg.point_at(t0), b: seg.point_at(t1) },
            Seg2::Arc { center, radius, start, sweep } => Seg2::Arc { center, radius, start: start + sweep * t0, sweep: sweep * (t1 - t0) },
            _ => return None,
        });
        t0 = t1;
    }
    Some(out)
}

/// The union of two prisms along `d` stacked end to end (or overlapping) with different
/// sections, sewn from its faces: each slab between the prisms' end levels has walls over its
/// section's outline; each level has a face up over the section below less the one above, and
/// a face down over the one above less the one below. Every outline is split where any other
/// meets it, so the faces share their edges. `None` when a slab is empty or a curve isn't a
/// line or an arc.
fn sew_stack(plane: &Plane, d: Vec3, pa: &Prism, pb: &Prism, tol: f64) -> Option<Result<Body>> {
    let mut levels = vec![pa.lo, pa.hi, pb.lo, pb.hi];
    levels.sort_by(f64::total_cmp);
    levels.dedup_by(|x, y| (*x - *y).abs() < tol);
    let covers = |p: &Prism, z0: f64, z1: f64| p.lo <= z0 + tol && p.hi >= z1 - tol;
    let mut slabs: Vec<Vec<Region2>> = Vec::new();
    for w in levels.windows(2) {
        let (z0, z1) = (*w.first()?, *w.get(1)?);
        let s = match (covers(pa, z0, z1), covers(pb, z0, z1)) {
            (true, true) => boolean2d(&pa.regions, &pb.regions, BoolOp::Union, tol)?,
            (true, false) => pa.regions.clone(),
            (false, true) => pb.regions.clone(),
            (false, false) => return None,
        };
        slabs.push(oriented(&s));
    }
    sew_sections(plane, d, &levels, &slabs, tol, false)
}

/// Sew the exact sections between successive levels, including empty gaps.
fn sew_sections(plane: &Plane, d: Vec3, levels: &[f64], slabs: &[Vec<Region2>], tol: f64, coalesce: bool) -> Option<Result<Body>> {
    if levels.len() != slabs.len() + 1
        || slabs.len() > 128
        || plane.normal().dot(d) < 1. - 1e-12
        || levels.iter().any(|z| !z.is_finite())
        || levels.windows(2).any(|z| z[0] >= z[1])
    {
        return None;
    }
    let mut blocks = Vec::new();
    let mut start = None;
    for (i, slab) in slabs.iter().enumerate() {
        if slab.is_empty() {
            if let Some(first) = start.take() {
                blocks.push((first, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(first) = start {
        blocks.push((first, slabs.len()));
    }
    if blocks.is_empty() {
        return None;
    }
    Some((|| {
        let mut pieces = Vec::new();
        let mut remaining_constructed_faces = MAX_PLANAR_SEGMENTS * 2;
        let mut remaining_faces = MAX_PLANAR_SEGMENTS * 2;
        for (first, end) in blocks {
            let section = slabs.get(first).ok_or_else(|| KernelError::Failed("missing prism section".into()))?;
            if slabs[first..end].iter().all(|s| s == section) {
                let edge_count = section.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).flat_map(|l| &l.segs).fold(0usize, |n, s| {
                    n.saturating_add(match s {
                        Seg2::Arc { sweep, .. } if sweep.abs() > 1.5 * std::f64::consts::PI => 2,
                        _ => 1,
                    })
                });
                remaining_constructed_faces = remaining_constructed_faces
                    .checked_sub(edge_count.saturating_add(section.len().saturating_mul(2)))
                    .ok_or_else(|| KernelError::Failed("prism construction face budget".into()))?;
                // This is a NEW exact extrusion of the current Boolean section,
                // not copied lineage from either operand's old footprint.
                for piece in crate::extrude(plane, section, levels[first], levels[end])? {
                    remaining_faces =
                        remaining_faces.checked_sub(piece.face_count()).ok_or_else(|| KernelError::Failed("prism output face budget".into()))?;
                    pieces.push(piece);
                }
            } else {
                let piece = sew_connected_sections(plane, d, &levels[first..=end], &slabs[first..end], tol, coalesce)
                    .ok_or_else(|| KernelError::Failed("stepped prism construction failed".into()))??;
                remaining_faces =
                    remaining_faces.checked_sub(piece.face_count()).ok_or_else(|| KernelError::Failed("prism output face budget".into()))?;
                pieces.push(piece);
            }
        }
        let mut old = std::collections::HashMap::new();
        let mut shells = Vec::new();
        let mut remaining_map_faces = MAX_PLANAR_SEGMENTS * 2;
        for piece in &pieces {
            for (index, face) in piece.solid.face_iter().enumerate() {
                remaining_map_faces = remaining_map_faces.checked_sub(1).ok_or_else(|| KernelError::Failed("prism output face budget".into()))?;
                let source = piece.face_sources().and_then(|s| s.get(index)).and_then(Option::as_ref);
                if old.insert(face.id(), Some((face.orientation(), source))).is_some() {
                    old.insert(face.id(), None);
                }
            }
            shells.extend(piece.solid.boundaries().iter().cloned());
        }
        let body = Body::new(crate::body::Solid::try_new(shells).map_err(|e| KernelError::Failed(format!("prism compound: {e}")))?)?;
        let sources = body
            .solid
            .face_iter()
            .map(|face| match old.get(&face.id()) {
                Some(Some((orientation, source))) if *orientation == face.orientation() => source.cloned(),
                _ => None,
            })
            .collect();
        Ok(body.with_face_sources(Some(std::sync::Arc::new(sources))))
    })())
}

/// Stable exact identity for the bounded analytic segment subset.
fn wall_key(s: &Seg2) -> Option<[u64; 6]> {
    let bits = |v: f64| v.is_finite().then(|| if v == 0. { 0 } else { v.to_bits() });
    match s {
        Seg2::Line { a, b } => Some([0, bits(a.x)?, bits(a.y)?, bits(b.x)?, bits(b.y)?, 0]),
        Seg2::Arc { center, radius, start, sweep } => Some([1, bits(center.x)?, bits(center.y)?, bits(*radius)?, bits(*start)?, bits(*sweep)?]),
        _ => None,
    }
}

/// A genuinely stepped connected axial block has no single extrusion footprint.
fn sew_connected_sections(plane: &Plane, d: Vec3, levels: &[f64], slabs: &[Vec<Region2>], tol: f64, coalesce: bool) -> Option<Result<Body>> {
    use crate::sew::{EdgeSpec, FaceSpec, SurfSpec};
    // Faces at each level: up where the slab below sticks out, down where the one above does.
    let diff = |x: &[Region2], y: &[Region2]| -> Option<Vec<Region2>> {
        match (x.is_empty(), y.is_empty()) {
            (true, _) => Some(Vec::new()),
            (false, true) => Some(x.to_vec()),
            (false, false) => boolean2d(x, y, BoolOp::Cut, tol).map(|r| oriented(&r)),
        }
    };
    let mut caps: Vec<(f64, bool, Vec<Region2>)> = Vec::new();
    for (k, z) in levels.iter().enumerate() {
        let below: &[Region2] = if k == 0 { &[] } else { slabs.get(k - 1)? };
        let above: &[Region2] = slabs.get(k).map(Vec::as_slice).unwrap_or(&[]);
        caps.push((*z, true, diff(below, above)?));
        caps.push((*z, false, diff(above, below)?));
    }
    // Every segment end, everywhere: outlines split there.
    let loops_of =
        |rs: &[Region2]| -> Vec<Loop2> { rs.iter().flat_map(|r| std::iter::once(r.outer.clone()).chain(r.holes.iter().cloned())).collect() };
    let mut pts: Vec<Vec2> = Vec::new();
    for lp in slabs.iter().flat_map(|s| loops_of(s)).chain(caps.iter().flat_map(|c| loops_of(&c.2))) {
        for sg in &lp.segs {
            pts.push(sg.start());
            pts.push(sg.end());
        }
    }
    let at = |u: Vec2, z: f64| plane.to_world(u) + d * z;
    let edge = |sg: &Seg2, z: f64| -> EdgeSpec {
        match sg {
            Seg2::Arc { .. } => EdgeSpec::Arc { a: at(sg.start(), z), b: at(sg.end(), z), mid: at(sg.point_at(0.5), z) },
            _ => EdgeSpec::Line { a: at(sg.start(), z), b: at(sg.end(), z) },
        }
    };
    // Exact oriented generated segments alone define a run. No support fitting,
    // proximity matching, or old face lineage is used here.
    let mut runs: Vec<(usize, usize, Seg2)> = Vec::new();
    let mut last = std::collections::BTreeMap::new();
    let mut remaining = MAX_PLANAR_SEGMENTS * 2;
    for (i, slab) in slabs.iter().enumerate() {
        for lp in loops_of(slab) {
            for sg0 in &lp.segs {
                for sg in split_seg(sg0, &pts, tol)? {
                    remaining = remaining.checked_sub(1)?;
                    let key = wall_key(&sg)?;
                    if coalesce {
                        if let Some(&index) = last.get(&key) {
                            let run: &mut (usize, usize, Seg2) = runs.get_mut(index)?;
                            if run.1 == i {
                                run.1 = i + 1;
                                continue;
                            }
                            // Duplicate oriented boundaries in one slab are not
                            // a valid reason to reconstruct a larger wall.
                            if run.1 == i + 1 {
                                return None;
                            }
                        }
                        last.insert(key, runs.len());
                    }
                    runs.push((i, i + 1, sg));
                }
            }
        }
    }
    let mut faces: Vec<FaceSpec> = Vec::new();
    let mut sources = Vec::new();
    let frame_is_orthonormal = plane.origin.is_finite()
        && plane.x.is_finite()
        && plane.y.is_finite()
        && d.is_finite()
        && (plane.x.len2() - 1.).abs() <= 64. * f64::EPSILON
        && (plane.y.len2() - 1.).abs() <= 64. * f64::EPSILON
        && (d.len2() - 1.).abs() <= 64. * f64::EPSILON
        && plane.x.dot(plane.y).abs() <= 64. * f64::EPSILON
        && plane.x.dot(d).abs() <= 64. * f64::EPSILON
        && plane.y.dot(d).abs() <= 64. * f64::EPSILON;
    for (first, end, sg) in runs {
        let (z0, z1) = (levels[first], levels[end]);
        let (a, b) = (sg.start(), sg.end());
        let mut lp3 = Vec::with_capacity(2 + 2 * (end - first));
        lp3.push(edge(&sg, z0));
        // Keep station vertices at both ends: neighboring stepped walls/caps
        // must share complete edges rather than create T-junctions.
        for k in first..end {
            lp3.push(EdgeSpec::Line { a: at(b, levels[k]), b: at(b, levels[k + 1]) });
        }
        lp3.push(edge(&sg.reversed(), z1));
        for k in (first..end).rev() {
            lp3.push(EdgeSpec::Line { a: at(a, levels[k + 1]), b: at(a, levels[k]) });
        }
        let outward = plane.dir_to_world(sg.derivative(0.5)).cross(d).normalized()?;
        let surface = match sg {
            Seg2::Arc { .. } => SurfSpec::Extruded { a: at(a, z0), b: at(b, z0), mid: at(sg.point_at(0.5), z0), dir: d * (z1 - z0) },
            _ => SurfSpec::Plane,
        };
        faces.push(FaceSpec { loops: vec![lp3], surface, probe: Some((at(sg.point_at(0.5), (z0 + z1) / 2.0), outward)) });
        // Axis-aligned generated lines have a common oriented support frame
        // independent of axial stations or along-line endpoint interpolation.
        let generated_plane = if coalesce
            && frame_is_orthonormal
            && let Seg2::Line { a, b } = sg
        {
            let (origin, x) = if a.y == b.y && a.x != b.x {
                (at(Vec2::new(0., a.y), 0.), plane.x * (b.x - a.x).signum())
            } else if a.x == b.x && a.y != b.y {
                (at(Vec2::new(a.x, 0.), 0.), plane.y * (b.y - a.y).signum())
            } else {
                (Vec3::ZERO, Vec3::ZERO)
            };
            (origin.is_finite() && x.len2() > 0.).then_some(Plane { origin, x, y: d })
        } else {
            None
        };
        sources.push((generated_plane.is_some() || end > first + 1).then_some(crate::FaceSource::ExtrudeSide {
            segment: sg,
            plane: generated_plane,
            coalesced: end > first + 1,
        }));
    }
    for (z, up, rs) in &caps {
        for r in rs {
            let mut loops = Vec::new();
            for lp in std::iter::once(&r.outer).chain(&r.holes) {
                let mut segs: Vec<Seg2> = Vec::new();
                for sg in &lp.segs {
                    segs.extend(split_seg(sg, &pts, tol)?);
                }
                let segs: Vec<Seg2> = if *up { segs } else { segs.iter().rev().map(Seg2::reversed).collect() };
                loops.push(segs.iter().map(|sg| edge(sg, *z)).collect::<Vec<_>>());
            }
            faces.push(FaceSpec { loops, surface: SurfSpec::Plane, probe: None });
            sources.push(None);
        }
    }
    Some(crate::sew::sew_touching_with_sources(&faces, &sources).and_then(
        |body| {
            if coalesce { body.regularize_generated_planar_faces() } else { Ok(body) }
        },
    ))
}

fn simple_prism_boolean(a: &Body, b: &Body, op: BoolOp) -> Option<Result<Option<Body>>> {
    let size = a.size().max(b.size());
    let tol = (size * 1e-7).max(1e-9) * 10.0;
    // Candidate directions: a's planar face normals (each line once).
    let mut dirs: Vec<Vec3> = Vec::new();
    let mut remaining_direction_faces = MAX_PLANAR_SEGMENTS * 2;
    for f in a.solid.face_iter() {
        remaining_direction_faces = remaining_direction_faces.checked_sub(1)?;
        if let mt::Surface::Plane(p) = f.oriented_surface() {
            let n = p.normal();
            if let Some(n) = Vec3::new(n.x, n.y, n.z).normalized()
                && !dirs.iter().any(|m| m.dot(n).abs() > 1.0 - 1e-9)
            {
                dirs.push(if n.dot(Vec3::new(1e-3, 1e-6, 1.0)) < 0.0 { -n } else { n });
                if dirs.len() == 8 {
                    break;
                }
            }
        }
    }
    let (d, plane, pa, pb) = dirs.into_iter().take(8).find_map(|d| {
        let plane = Plane::from_normal(Vec3::ZERO, d)?;
        let pa = prism_of(a, d, &plane, tol)?;
        let pb = prism_of(b, d, &plane, tol)?;
        Some((d, plane, pa, pb))
    })?;
    let _ = d;
    let (blo, bhi) = heights_of(b, &plane);
    let spans = blo <= pa.lo + tol && bhi >= pa.hi - tol;
    let same = (pb.lo - pa.lo).abs() < tol && (pb.hi - pa.hi).abs() < tol;
    let area =
        |rs: &[Region2]| rs.iter().map(|r| r.outer.signed_area().abs() - r.holes.iter().map(|h| h.signed_area().abs()).sum::<f64>()).sum::<f64>();
    // Prisms of one section overlapping or touching end to end: one longer prism.
    let stacked = op == BoolOp::Union && !same && pb.lo <= pa.hi + tol && pa.lo <= pb.hi + tol;
    let ok = match op {
        BoolOp::Cut | BoolOp::Intersect => spans,
        BoolOp::Union => same || stacked,
    };
    if !ok {
        return None;
    }
    let regions = boolean2d(&pa.regions, &pb.regions, op, tol)?;
    if regions.is_empty() {
        return Some(Ok(None));
    }
    let (lo, hi) = if stacked {
        // Different sections (a boss hanging over a plate's edge): sewn slab by slab.
        let (u, x, y) = (area(&regions), area(&pa.regions), area(&pb.regions));
        if (u - x).abs() > tol * (1.0 + u) || (u - y).abs() > tol * (1.0 + u) {
            return sew_stack(&plane, d, &pa, &pb, tol).map(|r| r.map(Some));
        }
        (pa.lo.min(pb.lo), pa.hi.max(pb.hi))
    } else {
        (pa.lo, pa.hi)
    };
    let base = plane.offset(lo);
    let mut bodies = match crate::extrude(&base, &regions, 0.0, hi - lo) {
        Ok(b) => b,
        Err(e) => return Some(Err(e)),
    };
    let first = bodies.pop()?;
    if !bodies.is_empty() {
        // Several pieces: one body of several lumps.
        let mut shells = first.solid.boundaries().clone();
        for b in bodies {
            shells.extend(b.solid.boundaries().iter().cloned());
        }
        return Some(crate::body::Solid::try_new(shells).map_err(|e| crate::KernelError::Failed(e.to_string())).and_then(Body::new).map(Some));
    }
    Some(Ok(Some(first)))
}

/// A stepped extrusion (case floor plus walls), recovered from its cap events.
/// Downward caps add a section above their level; upward caps remove one.
fn stack_of(b: &Body, d: Vec3, plane: &Plane, tol: f64) -> Option<Vec<Prism>> {
    if let Some(p) = prism_of(b, d, plane, tol) {
        return Some(vec![p]);
    }
    if b.face_count() > 2048 {
        return None;
    }
    let mut events = Vec::new();
    let mut wall_work = 262_144;
    let mut curve_work = 2_000_000;
    for f in b.solid.face_iter() {
        let surf = f.oriented_surface();
        if let mt::Surface::Plane(p) = &surf {
            let n = p.normal();
            let n = Vec3::new(n.x, n.y, n.z).normalized()?;
            if n.dot(d).abs() > 1.0 - 1e-9 {
                let mut loops = Vec::new();
                for w in f.boundaries() {
                    let mut segs = Vec::new();
                    for e in w.edge_iter() {
                        segs.extend(crate::prism_curve::segment(e, plane, tol, &mut curve_work)?);
                    }
                    loops.push(Loop2 { segs });
                }
                let (outer, holes) = split_outer(loops)?;
                let region = Region2 { outer, holes };
                events.push((plane.height(from_p3(p.origin())), n.dot(d) > 0.0, oriented(&[region])));
                continue;
            }
            if n.dot(d).abs() > 1e-9 {
                return None;
            }
        } else {
            if !wall_along(&surf, d, tol, &mut wall_work) {
                return None;
            }
        }
    }
    let mut levels: Vec<f64> = events.iter().map(|(z, _, _)| *z).collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup_by(|x, y| (*x - *y).abs() < tol);
    if levels.len() < 2 || levels.len() > 64 || levels.iter().any(|z| !z.is_finite()) {
        return None;
    }
    let mut current = Vec::new();
    let mut stack = Vec::new();
    for (i, z) in levels.iter().enumerate() {
        // Remove terminating material before adding any material starting here.
        for up in [true, false] {
            for (_, _, section) in events.iter().filter(|(h, u, _)| (*h - *z).abs() < tol && *u == up) {
                current = match (current.is_empty(), up) {
                    (true, false) => section.clone(),
                    (true, true) => return None,
                    (false, _) => boolean2d(&current, section, if up { BoolOp::Cut } else { BoolOp::Union }, tol)?,
                };
            }
        }
        if let Some(next) = levels.get(i + 1)
            && !current.is_empty()
        {
            stack.push(Prism { lo: *z, hi: *next, regions: oriented(&current) });
        }
    }
    (current.is_empty() && !stack.is_empty()).then_some(stack)
}

fn stack_boolean(a: &Body, b: &Body, op: BoolOp) -> Option<Result<Option<Body>>> {
    // Common-cap joins and overlapping partial-depth cuts of aligned stepped prisms can
    // retain their exact section boundaries instead of intersecting meshes.
    // Nonaligned bodies retain their existing B-rep path.
    if !matches!(op, BoolOp::Union | BoolOp::Cut) {
        return None;
    }
    if op == BoolOp::Cut && a.face_count().saturating_add(b.face_count()) > 2048 {
        return None;
    }
    fn section(ps: &[Prism], lo: f64, hi: f64, tol: f64) -> &[Region2] {
        ps.iter().find(|p| p.lo <= lo + tol && p.hi >= hi - tol).map(|p| p.regions.as_slice()).unwrap_or(&[])
    }
    let tol = (a.size().max(b.size()) * 1e-7).max(1e-9) * 10.0;
    let mut dirs: Vec<Vec3> = Vec::new();
    for f in a.solid.face_iter() {
        if let mt::Surface::Plane(p) = f.oriented_surface() {
            let n = p.normal();
            if let Some(n) = Vec3::new(n.x, n.y, n.z).normalized()
                && !dirs.iter().any(|m| m.dot(n).abs() > 1.0 - 1e-9)
            {
                dirs.push(n);
            }
        }
    }
    let ends = |ps: &[Prism]| ps.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.lo), hi.max(p.hi)));
    let (d, plane, pa, pb) = dirs.into_iter().take(8).find_map(|d| {
        let plane = Plane::from_normal(Vec3::ZERO, d)?;
        let pa = stack_of(a, d, &plane, tol)?;
        let pb = stack_of(b, d, &plane, tol)?;
        let (ea, eb) = (ends(&pa), ends(&pb));
        match op {
            BoolOp::Union if (ea.0 - eb.0).abs() >= tol && (ea.1 - eb.1).abs() >= tol => return None,
            // A cut tool may extend past a cap into empty space. Exact slab sections
            // discard that space rather than intersecting coincident cylinder walls.
            BoolOp::Cut if ea.1 <= eb.0 + tol || eb.1 <= ea.0 + tol => return None,
            _ => {}
        }
        Some((d, plane, pa, pb))
    })?;
    let mut levels: Vec<f64> = pa.iter().chain(&pb).flat_map(|p| [p.lo, p.hi]).collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup_by(|x, y| (*x - *y).abs() < tol);
    let mut slabs = Vec::new();
    let mut work = 0usize;
    let mut output_edges = 0usize;
    let edges = |regions: &[Region2]| regions.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).map(|l| l.segs.len()).sum::<usize>();
    for w in levels.windows(2) {
        let (lo, hi) = (*w.first()?, *w.get(1)?);
        let (x, y) = (section(&pa, lo, hi, tol), section(&pb, lo, hi, tol));
        if op == BoolOp::Cut {
            let count = edges(x).saturating_add(edges(y));
            work = work.saturating_add(count.saturating_mul(count));
            if work > 32_000_000 {
                return None;
            }
        }
        let s = match (x.is_empty(), y.is_empty(), op) {
            (true, _, BoolOp::Cut | BoolOp::Intersect) | (_, true, BoolOp::Intersect) => Vec::new(),
            (true, _, BoolOp::Union) => y.to_vec(),
            (_, true, _) => x.to_vec(),
            _ => boolean2d(x, y, op, tol)?,
        };
        if op == BoolOp::Cut {
            output_edges = output_edges.saturating_add(edges(&s));
            if output_edges > 8192 {
                return None;
            }
        }
        slabs.push(oriented(&s));
    }
    if slabs.iter().all(Vec::is_empty) {
        return Some(Ok(None));
    }
    sew_sections(
        &plane,
        d,
        &levels,
        &slabs,
        tol,
        op == BoolOp::Cut
            && !a.regularization_protected
            && !b.regularization_protected
            && a.split_keep().is_empty()
            && b.split_keep().is_empty()
            && a.paint().is_none_or(|p| p.faces.is_empty())
            && b.paint().is_none_or(|p| p.faces.is_empty()),
    )
    .map(|r| r.map(Some))
}

pub(crate) fn prism_boolean(a: &Body, b: &Body, op: BoolOp) -> Option<Result<Option<Body>>> {
    crate::guard("prism boolean", || Ok(simple_prism_boolean(a, b, op).or_else(|| stack_boolean(a, b, op)))).ok().flatten()
}

#[cfg(test)]
mod planar_tests {
    use super::*;
    fn rectangle(a: Vec2, b: Vec2) -> Region2 {
        Region2 { outer: Loop2::polygon(&[a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)]), holes: vec![] }
    }
    #[test]
    fn simple_prism_retains_large_analytic_section_budget() {
        let count = 2050;
        let radius = 1000.;
        let points: Vec<_> = (0..count)
            .map(|i| {
                let angle = std::f64::consts::TAU * i as f64 / count as f64;
                Vec2::new(radius * angle.cos(), radius * angle.sin())
            })
            .collect();
        let region = Region2 { outer: Loop2::polygon(&points), holes: vec![] };
        let stock = crate::extrude(&Plane::XY, &[region], 0., 2.).unwrap().pop().unwrap();
        assert_eq!(stock.face_count(), count + 2);
        assert!(stock.face_count() > 2048 && stock.validity().is_empty());
        let before: Vec<_> = stock.solid.vertex_iter().map(|v| (v.id(), v.point())).collect();
        let recovered = prism_of(&stock, Vec3::Z, &Plane::XY, 1e-5).expect("large simple line prism remains supported");
        assert_eq!(recovered.regions.len(), 1);
        assert_eq!(recovered.regions[0].outer.segs.len(), count);
        assert!(recovered.regions[0].outer.segs.iter().all(|s| matches!(s, Seg2::Line { .. })));
        let area = count as f64 * radius * radius * (std::f64::consts::TAU / count as f64).sin() / 2.;
        assert!((recovered.regions[0].area() - area).abs() < area * 1e-12);
        assert!((recovered.lo - 0.).abs() < 1e-8 && (recovered.hi - 2.).abs() < 1e-8);
        let cutter = crate::box_solid(Vec3::new(-1001., -1001., -1.), Vec3::new(1001., 1001., 3.)).unwrap();
        assert!(
            simple_prism_boolean(&stock, &cutter, BoolOp::Cut)
                .expect("large analytic section routes through the exact simple path")
                .unwrap()
                .is_none()
        );
        assert_eq!(stock.solid.vertex_iter().map(|v| (v.id(), v.point())).collect::<Vec<_>>(), before);
    }
    #[test]
    fn curved_stop_cut_constructs_fresh_current_cap_for_only_uniform_lump() {
        let cylinder = crate::cylinder(Vec3::new(25., 25., 0.), Vec3::Z, 2., 20.).unwrap();
        let stop = crate::box_solid(Vec3::new(19., 19., 1.), Vec3::new(31., 31., 2.)).unwrap();
        let roof = crate::boolean(&cylinder, &stop, BoolOp::Union).unwrap().unwrap();
        let original = rectangle(Vec2::new(20., 20.), Vec2::new(30., 30.));
        let tool = crate::extrude(&Plane::XY, &[original], 0., 25.).unwrap().pop().unwrap();
        let before_tool = crate::step_export(&[&tool], "tool").unwrap();
        let before_roof = crate::step_export(&[&roof], "roof").unwrap();
        let trimmed = crate::boolean(&tool, &roof, BoolOp::Cut).unwrap().unwrap();
        let lumps = trimmed.lumps().unwrap();
        assert_eq!(lumps.len(), 2);
        let mut lower = 0;
        let mut upper = 0;
        for lump in &lumps {
            assert!(lump.validity().is_empty());
            let mesh = lump.tessellate(1e-5).unwrap();
            if mesh.contains(Vec3::new(21., 21., 0.5)) {
                lower += 1;
                assert!(!mesh.contains(Vec3::new(25., 25., 0.5)));
                assert!(!mesh.contains(Vec3::new(21., 21., 1.5)));
                assert!((mesh.measure().volume - (100. - 4. * std::f64::consts::PI)).abs() < 0.002);
                let sources = lump.face_sources().expect("fresh actual extrusion has complete sources");
                assert_eq!(sources.len(), lump.face_count());
                assert!(sources.iter().all(Option::is_some));
                let mut caps = 0;
                for source in sources.iter().flatten() {
                    match source {
                        crate::FaceSource::ExtrudeCap { region, plane, .. } => {
                            assert_eq!(region.holes.len(), 1);
                            assert!((region.area() - (100. - 4. * std::f64::consts::PI)).abs() < 1e-8);
                            assert!(plane.normal().z.abs() > 1. - 1e-10);
                            assert!((plane.origin.z - if plane.normal().z > 0. { 1. } else { 0. }).abs() < 1e-8);
                            caps += 1;
                        }
                        crate::FaceSource::ExtrudeSide { .. } => {}
                        _ => panic!("old producer lineage must not survive a changed Boolean footprint"),
                    }
                }
                assert_eq!(caps, 2);
            } else {
                upper += 1;
                assert!(mesh.contains(Vec3::new(21., 21., 3.)));
                assert!(!mesh.contains(Vec3::new(25., 25., 3.)));
                assert!(mesh.contains(Vec3::new(25., 25., 22.)));
                assert!(
                    lump.face_sources().is_none_or(|s| s
                        .iter()
                        .all(|s| s.is_none() || matches!(s, Some(crate::FaceSource::ExtrudeSide { coalesced: true, plane: None, .. })))),
                    "genuinely stepped geometry has no durable certificate"
                );
                assert!((mesh.measure().volume - (18. * (100. - 4. * std::f64::consts::PI) + 500.)).abs() < 0.02);
            }
        }
        assert_eq!((lower, upper), (1, 1));
        assert_eq!(crate::step_export(&[&tool], "tool").unwrap(), before_tool);
        assert_eq!(crate::step_export(&[&roof], "roof").unwrap(), before_roof);
    }
    fn stepped_tab_stock() -> Body {
        let stock = crate::box_solid(Vec3::new(-20., -15., 0.), Vec3::new(20., 15., 24.)).unwrap();
        let tab = crate::box_solid(Vec3::new(-12., -25., 8.), Vec3::new(-4., -15., 16.)).unwrap();
        crate::boolean(&stock, &tab, BoolOp::Union).unwrap().unwrap()
    }

    fn coplanar_joins(body: &Body) -> usize {
        let faces: Vec<_> = body.solid.face_iter().collect();
        let mut adjacency = std::collections::HashMap::<_, Vec<usize>>::new();
        for (i, face) in faces.iter().enumerate() {
            for edge in face.edge_iter() {
                adjacency.entry(edge.id()).or_default().push(i);
            }
        }
        adjacency
            .values()
            .filter(|faces| {
                let [a, b] = faces.as_slice() else { return false };
                let (Some(pa), Some(pb)) = (body.planar_face(*a), body.planar_face(*b)) else { return false };
                let (na, nb) = (pa.normal(), pb.normal());
                na.dot(nb) > 1. - 1e-12 && (pb.origin - pa.origin).dot(na).abs() < 1e-10
            })
            .count()
    }

    #[test]
    fn shell_of_joined_stepped_tabs_has_no_artificial_planar_stations() {
        let stock = stepped_tab_stock();
        let snapshot = crate::step_export(&[&stock], "unchanged stock").unwrap();
        assert_eq!(coplanar_joins(&stock), 0);
        let shell = crate::shell(&stock, &[Vec3::new(0., 0., 24.)], 1.).unwrap();
        assert_eq!(coplanar_joins(&shell), 0, "Shell must not add internal edges across continuous planar walls");
        assert!(shell.validity().is_empty());
        let mesh = shell.tessellate(1e-5).unwrap();
        let expected = 40. * 30. * 24. + 8. * 10. * 8. - (38. * 28. * 23. + 6. * 10. * 6.);
        assert!((mesh.measure().volume - expected).abs() < 1e-6);
        for (point, material) in [
            (Vec3::new(0., 0., 0.5), true),
            (Vec3::new(0., 0., 10.), false),
            (Vec3::new(0., -14.5, 10.), true),
            (Vec3::new(19.5, 0., 10.), true),
            (Vec3::new(-8., -24.5, 12.), true),
            (Vec3::new(-8., -20., 12.), false),
            (Vec3::new(0., -18., 9.), false),
        ] {
            assert_eq!(mesh.contains(point), material, "wrong material at {point:?}");
        }
        let step = crate::step_export(&[&shell], "stepped tab shell").unwrap();
        let reopened = crate::step_import(&step).unwrap();
        let body = &reopened.bodies[0].body;
        assert_eq!(body.face_count(), shell.face_count());
        assert_eq!(coplanar_joins(body), 0);
        assert!((body.tessellate(1e-5).unwrap().measure().volume - expected).abs() < 1e-6);
        assert_eq!(crate::step_export(&[&stock], "unchanged stock").unwrap(), snapshot);
    }

    #[test]
    fn contiguous_analytic_walls_remove_faces_but_retain_vertical_stations() {
        for curved in [false, true] {
            let outer = if curved {
                Region2 { outer: Loop2::circle(Vec2::ZERO, 10.), holes: vec![] }
            } else {
                rectangle(Vec2::new(-10., -10.), Vec2::new(10., 10.))
            };
            let mut upper = outer.clone();
            upper.holes.push(if curved {
                Loop2::circle(Vec2::ZERO, 5.).reversed()
            } else {
                rectangle(Vec2::new(-5., -5.), Vec2::new(5., 5.)).outer.reversed()
            });
            let slabs = vec![vec![outer], vec![upper]];
            let old = sew_sections(&Plane::XY, Vec3::Z, &[0., 2., 20.], &slabs, 1e-7, false).unwrap().unwrap();
            let merged = sew_sections(&Plane::XY, Vec3::Z, &[0., 2., 20.], &slabs, 1e-7, true).unwrap().unwrap();
            let count = if curved { 2 } else { 4 };
            assert_eq!(old.face_count() - merged.face_count(), count);
            let marked: Vec<_> = merged.solid.face_iter().enumerate().filter(|(i, _)| merged.face_was_coalesced(*i)).collect();
            assert_eq!(marked.len(), count);
            for (_, face) in marked {
                assert_eq!(face.boundaries()[0].len(), 6, "all axial station vertices are retained");
                let heights: Vec<_> = face.vertex_iter().map(|v| v.point().z).collect();
                for z in [0., 2., 20.] {
                    assert!(heights.contains(&z));
                }
            }
            assert!(old.face_sources().is_none_or(|s| s.iter().all(Option::is_none)));
            let expected = if curved { std::f64::consts::PI * (100. * 20. - 25. * 18.) } else { 400. * 20. - 100. * 18. };
            let mesh = merged.tessellate(1e-5).unwrap();
            assert!((mesh.measure().volume - expected).abs() < 0.03);
            assert!(mesh.contains(Vec3::new(0., 0., 1.)));
            assert!(!mesh.contains(Vec3::new(0., 0., 10.)));
            assert!(merged.validity().is_empty());
            assert_ne!(wall_key(&slabs[0][0].outer.segs[0]), wall_key(&slabs[0][0].outer.segs[0].reversed()));
        }
    }

    #[test]
    fn cut_run_coalescing_respects_authored_splits_and_face_paint() {
        let region = Region2 { outer: Loop2::circle(Vec2::ZERO, 10.), holes: vec![] };
        let tool_region = Region2 { outer: Loop2::circle(Vec2::ZERO, 5.), holes: vec![] };
        let stock = crate::extrude(&Plane::XY, &[region], 0., 20.).unwrap().pop().unwrap();
        let cutter = crate::extrude(&Plane::XY, &[tool_region], 2., 22.).unwrap().pop().unwrap();
        let merged = stack_boolean(&stock, &cutter, BoolOp::Cut).unwrap().unwrap().unwrap();
        let protected = [
            stock.clone().with_split_lines(Some(std::sync::Arc::new(vec![vec![Vec3::new(10., 0., 0.), Vec3::new(10., 0., 20.)]]))),
            stock.with_paint(Some(crate::Paint { opacity: 1., faces: vec![crate::FacePaint { face: 0, color: [1., 0., 0.], opacity: 1. }] })),
        ];
        for input in protected {
            let output = stack_boolean(&input, &cutter, BoolOp::Cut).unwrap().unwrap().unwrap();
            assert_eq!(output.face_count(), merged.face_count() + 2);
            assert!((0..output.face_count()).all(|i| !output.face_was_coalesced(i)));
            // Independent circular chord-volume bound: 2*pi*(10*20+5*18)*1e-5 < .019.
            let expected = std::f64::consts::PI * (100. * 20. - 25. * 18.);
            for body in [&output, &merged] {
                let mesh = body.tessellate(1e-5).unwrap();
                assert!((mesh.measure().volume - expected).abs() < 0.03);
                assert!(mesh.contains(Vec3::new(0., 0., 1.)));
                assert!(!mesh.contains(Vec3::new(0., 0., 10.)));
                assert!(mesh.contains(Vec3::new(7., 0., 10.)));
            }
        }
    }

    #[test]
    fn annular_inward_tool_recovers_exact_stacks_before_cap_policy() {
        let plane = Plane::XY;
        let mut region = rectangle(Vec2::ZERO, Vec2::new(40.0, 30.0));
        region.holes.push(Loop2::circle(Vec2::new(20.0, 15.0), 5.0));
        let mut stock = crate::extrude(&plane, std::slice::from_ref(&region), 0.0, 20.0).unwrap().pop().unwrap();
        for (lo, hi) in [(20.0, 30.0), (30.0, 35.0)] {
            let positive = crate::extrude(&plane, std::slice::from_ref(&region), lo, hi).unwrap().pop().unwrap();
            stock = crate::boolean(&stock, &positive, BoolOp::Union).unwrap().unwrap();
        }
        let mut grown = rectangle(Vec2::new(-0.3, -0.3), Vec2::new(40.3, 30.3));
        grown.holes = region.holes;
        let tool = crate::extrude(&plane, &[grown], 30.0, 35.6).unwrap().pop().unwrap();
        let tol = stock.size().max(tool.size()) * 1e-6;
        let a = stack_of(&stock, Vec3::Z, &plane, tol).expect("annular stock qualifies");
        let b = stack_of(&tool, Vec3::Z, &plane, tol).expect("overrun tool qualifies");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert!((a[0].lo - 0.0).abs() < tol && (a[0].hi - 35.0).abs() < tol);
        assert!((b[0].lo - 30.0).abs() < tol && (b[0].hi - 35.6).abs() < tol);
        assert_eq!(a[0].regions[0].holes.len(), 1);
        assert_eq!(b[0].regions[0].holes.len(), 1);
        assert!((a[0].regions[0].area() - (1200.0 - 25.0 * std::f64::consts::PI)).abs() < 1e-8);
        assert!(simple_prism_boolean(&stock, &tool, BoolOp::Cut).is_none());
        let result = stack_boolean(&stock, &tool, BoolOp::Cut).expect("qualified overlapping stacks").unwrap().unwrap();
        assert_eq!(result.shell_count(), 1);
        assert!(result.validity().is_empty());
        let mesh = result.tessellate(1e-5).unwrap();
        let expected = (1200.0 - 25.0 * std::f64::consts::PI) * 30.0;
        // An inscribed circular mesh at radial chord error 1e-5 has volume
        // deficit below 2*pi*R*height*error, conservatively 0.01 mm^3 here.
        assert!((mesh.measure().volume - expected).abs() < 0.02);
        for (point, present) in [
            (Vec3::new(5.0, 5.0, 29.0), true),
            (Vec3::new(5.0, 5.0, 31.0), false),
            (Vec3::new(20.0, 15.0, 29.0), false),
            (Vec3::new(24.8, 15.0, 29.0), false),
            (Vec3::new(25.2, 15.0, 29.0), true),
        ] {
            assert_eq!(mesh.contains(point), present, "{point:?}");
        }
    }
    #[test]
    fn annular_partial_cut_overruns_and_interior_lumps_preserve_material() {
        let area = 1200.0 - 25.0 * std::f64::consts::PI;
        for seam in [0.0, 0.37] {
            for plane in [Plane::XY, Plane::new(Vec3::new(30.0, 20.0, 10.0), Vec3::X, Vec3::Z).unwrap()] {
                let mut region = rectangle(Vec2::ZERO, Vec2::new(40.0, 30.0));
                region.holes.push(Loop2::circle_from(Vec2::new(20.0, 15.0), 5.0, seam));
                let stock = crate::extrude(&plane, std::slice::from_ref(&region), 0.0, 35.0).unwrap().pop().unwrap();
                let before = crate::step_export(&[&stock], "source").unwrap();
                for (lo, hi, retained, shells) in [(30.0, 35.6, 30.0, 1), (-0.6, 5.0, 30.0, 1), (10.0, 15.0, 30.0, 2), (30.0, 35.0, 30.0, 1)] {
                    let tool = crate::extrude(&plane, std::slice::from_ref(&region), lo, hi).unwrap().pop().unwrap();
                    let result = stack_boolean(&stock, &tool, BoolOp::Cut).expect("exact aligned cut path").unwrap().unwrap();
                    assert_eq!(result.shell_count(), shells);
                    assert!(result.validity().is_empty());
                    let mesh = result.tessellate(1e-5).unwrap();
                    assert!((mesh.measure().volume - retained * area).abs() < 0.02);
                    let at = |x: f64, y: f64, z: f64| plane.to_world(Vec2::new(x, y)) + plane.normal() * z;
                    for z in [2.5, 12.5, 32.5] {
                        let material = !(lo < z && z < hi);
                        assert_eq!(mesh.contains(at(5.0, 5.0, z)), material);
                        assert!(!mesh.contains(at(20.0, 15.0, z)));
                        assert_eq!(mesh.contains(at(25.2, 15.0, z)), material);
                    }
                    let public = crate::boolean(&stock, &tool, BoolOp::Cut).unwrap().unwrap();
                    assert!((public.tessellate(1e-5).unwrap().measure().volume - retained * area).abs() < 0.02);
                    if lo == 30.0 && hi > 35.0 {
                        let imported = crate::step_import(&crate::step_export(&[&result], "inward annular cut").unwrap()).unwrap();
                        assert!(imported.warnings.is_empty());
                        let saved = &imported.bodies.first().unwrap().body;
                        assert!((saved.tessellate(1e-5).unwrap().measure().volume - 30.0 * area).abs() < 0.02);
                        let next = crate::extrude(&plane, std::slice::from_ref(&region), 25.0, 31.0).unwrap().pop().unwrap();
                        let shortened = crate::boolean(saved, &next, BoolOp::Cut).unwrap().unwrap();
                        let next_volume = shortened.tessellate(1e-5).unwrap().measure().volume;
                        assert!(
                            (next_volume - 25.0 * area).abs() < 0.02,
                            "seam {seam}, plane {plane:?}, next cut volume {next_volume}, expected {}",
                            25.0 * area
                        );
                        assert!(shortened.contains(at(5.0, 5.0, 24.0)));
                        assert!(!shortened.contains(at(5.0, 5.0, 26.0)));
                        assert!(!shortened.contains(at(20.0, 15.0, 24.0)));
                    }
                }
                let clear = crate::extrude(&plane, std::slice::from_ref(&region), 36.0, 40.0).unwrap().pop().unwrap();
                assert!(stack_boolean(&stock, &clear, BoolOp::Cut).is_none());
                let unchanged = crate::boolean(&stock, &clear, BoolOp::Cut).unwrap().unwrap();
                assert!(std::sync::Arc::ptr_eq(&stock.solid, &unchanged.solid), "disjoint Cut must retain exact source topology");
                let unchanged_mesh = unchanged.tessellate(1e-5).unwrap();
                let source_mesh = stock.tessellate(1e-5).unwrap();
                assert_eq!(unchanged_mesh.measure().volume, source_mesh.measure().volume);
                // tessellate() floors tolerance at body.size()*1e-6. The arc
                // polygon's area error is at most perimeter*radial chord error.
                let bound = std::f64::consts::TAU * 5.0 * 35.0 * (stock.size() * 1e-6).max(1e-5) + 1e-5;
                assert!((unchanged_mesh.measure().volume - 35.0 * area).abs() < bound);
                assert!(!unchanged_mesh.contains(plane.to_world(Vec2::new(20.0, 15.0)) + plane.normal() * 12.5));
                let all = crate::extrude(&plane, std::slice::from_ref(&region), -1.0, 36.0).unwrap().pop().unwrap();
                assert!(stack_boolean(&stock, &all, BoolOp::Cut).unwrap().unwrap().is_none());
                assert_eq!(crate::step_export(&[&stock], "source").unwrap(), before);
            }
        }
    }
    #[test]
    fn annular_partial_cut_preserves_unequal_radii_and_conservative_refusals() {
        let mut region = rectangle(Vec2::ZERO, Vec2::new(40.0, 30.0));
        region.holes.push(Loop2::circle(Vec2::new(20.0, 15.0), 5.0));
        let stock = crate::extrude(&Plane::XY, std::slice::from_ref(&region), 0.0, 35.0).unwrap().pop().unwrap();
        let before = crate::step_export(&[&stock], "unchanged source").unwrap();
        let mut larger_hole = region.clone();
        larger_hole.holes = vec![Loop2::circle(Vec2::new(20.0, 15.0), 5.1)];
        let tool = crate::extrude(&Plane::XY, &[larger_hole], 30.0, 36.0).unwrap().pop().unwrap();
        let result = stack_boolean(&stock, &tool, BoolOp::Cut).unwrap().unwrap().unwrap();
        let expected = (1200.0 - 25.0 * std::f64::consts::PI) * 30.0 + std::f64::consts::PI * (5.1_f64.powi(2) - 25.0) * 5.0;
        let mesh = result.tessellate(1e-5).unwrap();
        assert!((mesh.measure().volume - expected).abs() < 0.02);
        assert!(mesh.contains(Vec3::new(25.05, 15.0, 32.0)));
        assert!(!mesh.contains(Vec3::new(25.2, 15.0, 32.0)));
        assert!(!mesh.contains(Vec3::new(20.0, 15.0, 32.0)));
        let interior = crate::extrude(&Plane::XY, std::slice::from_ref(&region), 10.0, 15.0).unwrap().pop().unwrap();
        for d in [Vec3::X, Vec3::Y] {
            let plane = Plane::from_normal(Vec3::ZERO, d).unwrap();
            let tol = stock.size() * 1e-6;
            assert!(stack_of(&stock, d, &plane, tol).is_none(), "transverse cylinder axis must not fill the hole");
            assert!(stack_of(&interior, d, &plane, tol).is_none());
        }
        assert!(stack_boolean(&stock, &interior, BoolOp::Union).is_none(), "Union retains its common-cap policy");
        let tapered = crate::extrude_tapered(&Plane::XY.offset(30.0), &rectangle(Vec2::ZERO, Vec2::new(40.0, 30.0)), 6.0, 1.0, 0.05).unwrap();
        assert!(stack_boolean(&stock, &tapered, BoolOp::Cut).is_none(), "slanted walls are not aligned prisms");
        let curved_cap = crate::sphere(Vec3::new(20.0, 15.0, 35.0), 5.0).unwrap();
        assert!(stack_boolean(&stock, &curved_cap, BoolOp::Cut).is_none());
        let pieces =
            (0..350).map(|i| crate::box_solid(Vec3::new(i as f64 * 3.0, 0.0, 0.0), Vec3::new(i as f64 * 3.0 + 1.0, 1.0, 1.0)).unwrap()).collect();
        let over_budget = crate::ops::join_pieces(pieces).unwrap().unwrap();
        assert!(over_budget.face_count() > 2048);
        assert!(stack_boolean(&over_budget, &interior, BoolOp::Cut).is_none());
        assert_eq!(crate::step_export(&[&stock], "unchanged source").unwrap(), before);
    }
    #[test]
    fn curved_prism_wall_requires_positive_separable_rational_weights() {
        let surface = |mixed: bool| {
            let weight = if mixed { 2.0 } else { 1.0 };
            let points = vec![
                vec![mt::Vector4::new(0.0, 0.0, 0.0, 1.0), mt::Vector4::new(0.0, 0.0, 10.0, 1.0)],
                vec![mt::Vector4::new(5.0, 0.0, 0.0, 1.0), mt::Vector4::new(5.0 * weight, 0.0, 10.0 * weight, weight)],
            ];
            mt::Surface::NurbsSurface(mt::NurbsSurface::new(mt::BSplineSurface::new(
                (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
                points,
            )))
        };
        assert!(wall_along(&surface(false), Vec3::Z, 1e-5, &mut 1000));
        assert!(!wall_along(&surface(true), Vec3::Z, 1e-5, &mut 1000), "parallel Euclidean controls alone do not certify a rational sweep");
        assert!(!wall_along(&surface(false), Vec3::Y, 1e-5, &mut 1000));
        assert!(!wall_along(&surface(false), Vec3::Z, 1e-5, &mut 0));
    }
    #[test]
    fn partial_depth_cuts_touching_a_through_hole_use_exact_stacked_sections() {
        let plane = Plane::named("XY").unwrap();
        let mut plate = rectangle(Vec2::ZERO, Vec2::new(100., 60.));
        plate.holes.push(rectangle(Vec2::new(25., 20.), Vec2::new(30., 30.)).outer);
        let stock = crate::extrude(&plane, &[plate], 0., 2.5).unwrap().pop().unwrap();
        let low = crate::extrude(&plane, &[rectangle(Vec2::new(20., 20.), Vec2::new(30., 30.))], 0., 1.5).unwrap().pop().unwrap();
        let high = crate::extrude(&plane, &[rectangle(Vec2::new(25., 20.), Vec2::new(35., 30.))], 1., 2.5).unwrap().pop().unwrap();
        for tools in [[&low, &high], [&high, &low]] {
            let mut result = stock.clone();
            for tool in tools {
                result = stack_boolean(&result, tool, BoolOp::Cut).expect("exact stack path").unwrap().unwrap();
            }
            assert!(result.validity().is_empty());
            assert_eq!(result.shell_count(), 1);
            assert!((crate::measure(&result).unwrap().volume - 14_725.).abs() < 1e-6);
            let mesh = result.tessellate(1e-4).unwrap();
            for (point, material) in [
                (Vec3::new(22.5, 25., 0.5), false),
                (Vec3::new(22.5, 25., 2.), true),
                (Vec3::new(32.5, 25., 0.5), true),
                (Vec3::new(32.5, 25., 2.), false),
                (Vec3::new(27.5, 25., 1.25), false),
                (Vec3::new(10., 10., 1.25), true),
            ] {
                assert_eq!(mesh.contains(point), material, "{point:?}");
            }
            let imported = crate::step_import(&crate::step_export(&[&result], "layered cut").unwrap()).unwrap();
            assert!(imported.warnings.is_empty());
            assert!((crate::measure(&imported.bodies.first().unwrap().body).unwrap().volume - 14_725.).abs() < 1e-6);
            let next = crate::extrude(&plane, &[rectangle(Vec2::new(20., 20.), Vec2::new(25., 30.))], 1.5, 2.5).unwrap().pop().unwrap();
            let opened = crate::boolean(&imported.bodies.first().unwrap().body, &next, BoolOp::Cut).unwrap().unwrap();
            assert!((crate::measure(&opened).unwrap().volume - 14_675.).abs() < 1e-6);
            assert!(opened.validity().is_empty());
        }
    }
    #[test]
    fn compound_planar_extrusion_normalizes_overlap_and_retains_islands() {
        let plane = Plane::named("XY").unwrap();
        let a = rectangle(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let b = rectangle(Vec2::new(5.0, 0.0), Vec2::new(15.0, 10.0));
        let body = extrude_planar_regions(&plane, &[a.clone(), b], 0.0, 2.0).unwrap();
        assert!((crate::measure(&body).unwrap().volume - 300.0).abs() < 1e-6);
        let inner = Region2 { outer: Loop2::circle(Vec2::new(5.0, 5.0), 2.0), holes: vec![] };
        let body = extrude_planar_regions(&plane, &[a.clone(), inner.clone()], 0.0, 2.0).unwrap();
        assert!((crate::measure(&body).unwrap().volume - 200.0).abs() < 1e-6);
        let annulus = Region2 { outer: a.outer, holes: vec![Loop2::circle(Vec2::new(5.0, 5.0), 3.0)] };
        let body = extrude_planar_regions(&plane, &[annulus, inner], 0.0, 2.0).unwrap();
        let expected = (100.0 - std::f64::consts::PI * 5.0) * 2.0;
        assert!((crate::measure(&body).unwrap().volume - expected).abs() < 0.1);
        assert_eq!(body.solid.boundaries().len(), 2);
        let a = rectangle(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let b = rectangle(Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
        assert!(extrude_planar_regions(&plane, &[a, b], 0.0, 2.0).is_err());
        let circles = [Vec2::ZERO, Vec2::new(10.0, 0.0)].map(|center| Region2 { outer: Loop2::circle(center, 5.0), holes: vec![] });
        assert!(extrude_planar_regions(&plane, &circles, 0.0, 2.0).is_err());
    }

    #[test]
    fn analytic_circle_classification_respects_signed_sweeps_and_seams() {
        for seam in [0.0, 0.13, std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
            for reversed in [false, true] {
                let lp = Loop2::circle_from(Vec2::ZERO, 10.0, seam);
                let lp = if reversed { lp.reversed() } else { lp };
                for p in [Vec2::ZERO, Vec2::new(9.9998, 0.0), Vec2::new(0.0, 9.9998), Vec2::new(0.0, -9.9998)] {
                    assert!(inside_loop(&lp, p), "{seam} {reversed} {p:?}");
                }
                for p in [Vec2::new(11.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(0.0, -10.0)] {
                    assert!(!inside_loop(&lp, p), "{seam} {reversed} {p:?}");
                }
            }
        }
        let half = Loop2 {
            segs: vec![
                Seg2::Arc { center: Vec2::ZERO, radius: 10.0, start: -std::f64::consts::FRAC_PI_2, sweep: std::f64::consts::PI },
                Seg2::Line { a: Vec2::new(0.0, 10.0), b: Vec2::new(0.0, -10.0) },
            ],
        };
        for lp in [half.clone(), half.reversed()] {
            assert!(inside_loop(&lp, Vec2::new(5.0, 0.0)));
            assert!(!inside_loop(&lp, Vec2::new(-5.0, 0.0)));
            assert!(!inside_loop(&lp, Vec2::new(-5.0, -10.0)));
        }
    }
    #[test]
    fn tiny_tool_between_circle_and_display_chord_is_not_lost() {
        let circle = Region2 { outer: Loop2::circle(Vec2::ZERO, 10.0), holes: vec![] };
        let sampled = circle.outer.polyline(1e-3);
        let direction = (sampled[0] + sampled[1]).normalized().unwrap();
        let at = direction * 9.9998;
        let half = Vec2::new(0.000025, 0.000025);
        let square = rectangle(at - half, at + half);
        assert!(square.outer.segs.iter().all(|s| s.start().len() < 10.0));
        assert!(!circle.contains(at), "fixture must lie outside the display chord");
        let cut = planar_regions_boolean(std::slice::from_ref(&circle), std::slice::from_ref(&square), BoolOp::Cut, 1e-7).unwrap();
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].holes.len(), 1);
        assert!((cut[0].holes[0].signed_area().abs() - 2.5e-9).abs() < 1e-13);
        let intersect = planar_regions_boolean(&[circle], &[square], BoolOp::Intersect, 1e-7).unwrap();
        assert_eq!(intersect.len(), 1);
        assert!((intersect[0].area() - 2.5e-9).abs() < 1e-13);
    }
    #[test]
    fn planar_region_api_rejects_hostile_values_and_budgets() {
        let plate = rectangle(Vec2::ZERO, Vec2::new(10.0, 10.0));
        for tolerance in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
            assert!(planar_regions_boolean(std::slice::from_ref(&plate), &[], BoolOp::Union, tolerance).is_err());
        }
        let mut bad = plate.clone();
        bad.outer.segs[0] = Seg2::Line { a: Vec2::new(f64::NAN, 0.0), b: Vec2::X };
        assert!(planar_regions_boolean(&[bad], &[], BoolOp::Union, 1e-7).is_err());
        let many = vec![plate; MAX_PLANAR_SEGMENTS / 4 + 1];
        assert!(planar_regions_boolean(&many, &[], BoolOp::Union, 1e-7).is_err());
    }
}
