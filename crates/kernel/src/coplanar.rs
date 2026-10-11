//! Booleans of bodies with coincident planar faces, by moving faces apart first.
//!
//! The mesh-based intersection cannot handle faces that lie in the same plane, nor the edges
//! lying in them. A face whose neighbours stand perpendicular to it (any extruded or
//! box-like part) can be pushed along its normal exactly, by rebuilding it and stretching or
//! shrinking its neighbours. Then:
//! - cut and intersect: the tool's coincident face moves out of the way (outward when the
//!   bodies are on the same side of the plane, away from the target when they touch back to
//!   back), which leaves the result unchanged;
//! - union of faces facing the same way: both faces move outward by different amounts, the
//!   union is taken, and the slab beyond the plane is cut off (now a transversal cut);
//! - union of faces back to back: the tool's face moves into the target, which overlaps it.

use std::collections::HashMap;

use solvecraft_geom::Vec3;
use truck_modeling::{self as mt, builder};

use crate::body::{Body, Solid, from_p3, p3, v3};
use crate::{BoolOp, KernelError, Result, guard};

#[derive(Clone, Copy, Debug)]
struct PlaneFace {
    n: Vec3,
    d: f64,
}

fn planes(b: &Body) -> Vec<(usize, PlaneFace)> {
    b.solid
        .face_iter()
        .enumerate()
        .filter_map(|(i, f)| match f.oriented_surface() {
            mt::Surface::Plane(p) => {
                let n = p.normal();
                let n = Vec3::new(n.x, n.y, n.z).normalized()?;
                Some((i, PlaneFace { n, d: n.dot(from_p3(p.origin())) }))
            }
            _ => None,
        })
        .collect()
}

/// Push the planar faces of `b` lying in the plane (n, d) — n their outward normal — along n
/// by `delta` (negative: inward). Their neighbours must stand perpendicular to the plane.
pub(crate) fn push_faces(b: &Body, n: Vec3, d: f64, delta: f64) -> Result<Body> {
    // The exact rebuild, unless it came out unsound (a curved side it couldn't re-sweep).
    match push_faces_exact(b, n, d, delta)
        .and_then(|r| if r.validity().is_empty() { Ok(r) } else { Err(KernelError::Failed("pushing a face: an unsound result".into())) })
    {
        Ok(r) => Ok(r),
        // Faces of planes and cylinders: every face follows (any neighbours).
        Err(e) => {
            let size = b.size();
            let tol = (size * 1e-7).max(1e-9) * 10.0;
            let planes: Vec<bool> = b
                .solid
                .face_iter()
                .map(|f| match f.oriented_surface() {
                    mt::Surface::Plane(p) => {
                        let m = p.normal();
                        let m = Vec3::new(m.x, m.y, m.z);
                        m.dot(n) > 1.0 - 1e-9 && (n.dot(from_p3(p.origin())) - d).abs() < tol
                    }
                    _ => false,
                })
                .collect();
            crate::offset::offset_body(b, |i, _| if planes.get(i).copied().unwrap_or(false) { delta } else { 0.0 }).map_err(|_| e)
        }
    }
}

pub(crate) fn push_faces_exact(b: &Body, n: Vec3, d: f64, delta: f64) -> Result<Body> {
    let size = b.size();
    let tol = (size * 1e-7).max(1e-9);
    let fail = |m: &str| KernelError::Failed(format!("pushing a face: {m}"));
    guard("push face", || {
        let solid = b.deep_copy();
        let shift = n * delta;
        let mat = mt::Matrix4::from_translation(v3(shift));
        let mut shells = Vec::new();
        for sh in solid.boundaries() {
            let faces: Vec<mt::Face> = sh.face_iter().cloned().collect();
            let on_plane = |f: &mt::Face| match f.oriented_surface() {
                mt::Surface::Plane(p) => {
                    let m = p.normal();
                    let m = Vec3::new(m.x, m.y, m.z);
                    m.dot(n) > 1.0 - 1e-9 && (n.dot(from_p3(p.origin())) - d).abs() < tol * 10.0
                }
                _ => false,
            };
            let moving: Vec<usize> = (0..faces.len()).filter(|i| faces.get(*i).is_some_and(on_plane)).collect();
            if moving.is_empty() {
                shells.push(sh.clone());
                continue;
            }
            // Vertices and edges of the moving faces.
            let mut vmap: HashMap<mt::VertexID, mt::Vertex> = HashMap::new();
            let mut emap: HashMap<mt::EdgeID, mt::Edge> = HashMap::new();
            for i in &moving {
                let f = faces.get(*i).ok_or_else(|| fail("face"))?;
                for v in f.vertex_iter() {
                    vmap.entry(v.id()).or_insert_with(|| builder::vertex(p3(from_p3(v.point()) + shift)));
                }
            }
            for i in &moving {
                let f = faces.get(*i).ok_or_else(|| fail("face"))?;
                for e in f.edge_iter() {
                    if emap.contains_key(&e.id()) {
                        continue;
                    }
                    let (a, bb) = (
                        vmap.get(&e.absolute_front().id()).ok_or_else(|| fail("vertex"))?,
                        vmap.get(&e.absolute_back().id()).ok_or_else(|| fail("vertex"))?,
                    );
                    let curve = mt::Transformed::transformed(&e.curve(), mat);
                    emap.insert(e.id(), mt::Edge::new(a, bb, curve));
                }
            }
            // Edges leaving the plane at moved vertices: straight, along n.
            for f in &faces {
                for e in f.edge_iter() {
                    if emap.contains_key(&e.id()) {
                        continue;
                    }
                    let (fa, fb) = (e.absolute_front(), e.absolute_back());
                    let (ma, mb) = (vmap.get(&fa.id()), vmap.get(&fb.id()));
                    if ma.is_none() && mb.is_none() {
                        continue;
                    }
                    if !matches!(e.curve(), mt::Curve::Line(_)) {
                        return Err(fail("a neighbouring edge is not straight"));
                    }
                    let dir = (from_p3(fb.point()) - from_p3(fa.point())).normalized().ok_or_else(|| fail("zero edge"))?;
                    if dir.cross(n).len() > 1e-9 {
                        return Err(fail("a neighbouring face is not perpendicular"));
                    }
                    let (na, nb) = (ma.cloned().unwrap_or_else(|| fa.clone()), mb.cloned().unwrap_or_else(|| fb.clone()));
                    if (from_p3(nb.point()) - from_p3(na.point())).dot(dir) <= tol {
                        return Err(fail("the face would pass its opposite side"));
                    }
                    emap.insert(e.id(), builder::line(&na, &nb));
                }
            }
            let rebuild_wires = |f: &mt::Face| -> Vec<mt::Wire> {
                f.absolute_boundaries()
                    .iter()
                    .map(|w| {
                        w.edge_iter()
                            .map(|e| match emap.get(&e.id()) {
                                Some(ne) => {
                                    if e.front() == e.absolute_front() {
                                        ne.clone()
                                    } else {
                                        ne.inverse()
                                    }
                                }
                                None => e.clone(),
                            })
                            .collect::<Vec<_>>()
                            .into()
                    })
                    .collect()
            };
            use mt::{ParametricSurface3D, SearchNearestParameter};
            let mut out: Vec<mt::Face> = Vec::new();
            for (i, f) in faces.iter().enumerate() {
                let touches = f.edge_iter().any(|e| emap.contains_key(&e.id()));
                if !touches {
                    out.push(f.clone());
                    continue;
                }
                let wires = rebuild_wires(f);
                let surface = if moving.contains(&i) {
                    mt::Transformed::transformed(&f.surface(), mat)
                } else if matches!(f.surface(), mt::Surface::Plane(_)) {
                    f.surface()
                } else {
                    // A curved side: sweep its edge in the plane along the new height.
                    let base = f.edge_iter().find_map(|e| {
                        emap.get(&e.id()).filter(|_| {
                            let (a, bb) = (from_p3(e.absolute_front().point()), from_p3(e.absolute_back().point()));
                            (n.dot(a) - d).abs() < tol * 10.0 && (n.dot(bb) - d).abs() < tol * 10.0
                        })
                    });
                    let base = base.ok_or_else(|| fail("a curved side without an edge in the plane"))?;
                    let height = f.vertex_iter().map(|v| n.dot(from_p3(v.point())) - d).fold(0.0f64, |m, x| if x.abs() > m.abs() { x } else { m });
                    let swept: mt::Face = builder::tsweep(base, v3(n * (height - delta)));
                    let mut s = swept.surface();
                    // Same normal as the old side at a point both share.
                    let probe = f.vertex_iter().map(|v| from_p3(v.point())).fold(Vec3::ZERO, |a, x| a + x) / f.vertex_iter().count().max(1) as f64;
                    let old = f.surface();
                    if let (Some((u0, v0)), Some((u1, v1))) =
                        (old.search_nearest_parameter(p3(probe), None, 100), s.search_nearest_parameter(p3(probe), None, 100))
                    {
                        let (a, bb) = (old.normal(u0, v0), s.normal(u1, v1));
                        if mt::InnerSpace::dot(a, bb) < 0.0 {
                            s = mt::Invertible::inverse(&s);
                        }
                    }
                    s
                };
                let mut nf = mt::Face::try_new(wires, surface).map_err(|e| fail(&e.to_string()))?;
                if !f.orientation() {
                    nf.invert();
                }
                out.push(nf);
            }
            shells.push(out.into());
        }
        let s = Solid::try_new(shells).map_err(|e| fail(&e.to_string()))?;
        Body::new(s)
    })
}

/// A box covering everything beyond the plane (n, d) by up to `depth`.
fn slab_beyond(n: Vec3, d: f64, depth: f64, extent: f64) -> Result<Body> {
    let u = n.any_perp();
    let w = n.cross(u);
    let o = n * d;
    let pl = solvecraft_geom::Plane::new(o, u, w).ok_or_else(|| KernelError::Failed("slab plane".into()))?;
    let sq = solvecraft_geom::Loop2::polygon(&[
        solvecraft_geom::Vec2::new(-extent, -extent),
        solvecraft_geom::Vec2::new(extent, -extent),
        solvecraft_geom::Vec2::new(extent, extent),
        solvecraft_geom::Vec2::new(-extent, extent),
    ]);
    let r = solvecraft_geom::Region2 { outer: sq, holes: vec![] };
    crate::extrude(&pl, &[r], 0.0, depth)?.pop().ok_or_else(|| KernelError::Failed("slab".into()))
}

/// Do the bodies have planar faces in a common plane?
pub(crate) fn has_coincident_faces(a: &Body, b: &Body) -> bool {
    let tol = (a.size().max(b.size()) * 1e-7).max(1e-9);
    let (pa, pb) = (planes(a), planes(b));
    pa.iter().any(|(_, x)| pb.iter().any(|(_, y)| x.n.dot(y.n).abs() > 1.0 - 1e-9 && (x.d - y.d * x.n.dot(y.n)).abs() < tol * 10.0))
}

/// Retry a failed boolean with coincident planar faces pushed apart. `None` when there are no
/// coincident faces or the faces can't be pushed.
pub(crate) fn boolean_apart(a: &Body, b: &Body, op: BoolOp) -> Option<Result<Option<Body>>> {
    let size = a.size().max(b.size());
    let tol = (size * 1e-7).max(1e-9);
    let (pa, pb) = (planes(a), planes(b));
    let mut same: Vec<(Vec3, f64)> = Vec::new();
    let mut back: Vec<(Vec3, f64)> = Vec::new();
    for (_, x) in &pa {
        for (_, y) in &pb {
            if (x.d - y.d * x.n.dot(y.n)).abs() < tol * 10.0 && x.n.dot(y.n).abs() > 1.0 - 1e-9 {
                let list = if x.n.dot(y.n) > 0.0 { &mut same } else { &mut back };
                if !list.iter().any(|(n, d)| n.dot(x.n) > 1.0 - 1e-9 && (d - x.d).abs() < tol * 10.0) {
                    list.push((x.n, x.d));
                }
            }
        }
    }
    if same.is_empty() && back.is_empty() {
        return None;
    }
    let (d1, d2) = (size * 0.0137, size * 0.0229);
    let run = || -> Result<Option<Body>> {
        let (mut a2, mut b2) = (a.clone(), b.clone());
        for (n, d) in &same {
            // b's face points along n too.
            b2 = push_faces(&b2, *n, *d, d2)?;
            if op == BoolOp::Union {
                a2 = push_faces(&a2, *n, *d, d1)?;
            }
        }
        for (n, d) in &back {
            // a's face points along n; b's face points along −n at the same plane (−d).
            let shift = if op == BoolOp::Union && crate::ops::narrowing_bores() {
                // Pushed into `a`, b's face must have a's material behind it all over and that
                // deep: over a hole in a's face (a narrowed bore's rim over the other body's
                // bore) it would fill the hole, through a thin wall it would stick out the far
                // side. Only checked inside `bore_union`, which then tries its other way; a
                // general check turns some joins that come out slightly wrong today (a pad
                // half past an edge) into failures, until such faces can be split first.
                [d2, d2 * 0.25, d2 * 0.0625]
                    .into_iter()
                    .find(|depth| backed(a, b, -*n, -*d, *depth))
                    .ok_or_else(|| KernelError::Failed("pushing a face: part of it lies over a hole or past the edge of the other body".into()))?
            } else if op == BoolOp::Union {
                d2
            } else {
                -d2
            };
            b2 = push_faces(&b2, -*n, -*d, shift)?;
        }
        let Some(mut r) = crate::ops::boolean(&a2, &b2, op)? else { return Ok(None) };
        if op == BoolOp::Union {
            for (n, d) in &same {
                let slab = slab_beyond(*n, *d, d1.max(d2) * 3.0, size * 4.0)?;
                match crate::ops::boolean(&r, &slab, BoolOp::Cut)? {
                    Some(x) => r = x,
                    None => return Ok(None),
                }
            }
        }
        Ok(Some(r))
    };
    Some(run())
}

/// Has `a` material behind every part of b's faces in the plane (m, d) (m their outward
/// normal, pointing into `a`), to `depth`? Checked at points inside the faces' triangles.
fn backed(a: &Body, b: &Body, m: Vec3, d: f64, depth: f64) -> bool {
    let size = a.size().max(b.size());
    let tol = (size * 1e-7).max(1e-9) * 10.0;
    let faces: Vec<usize> = planes(b).into_iter().filter(|(_, p)| p.n.dot(m) > 1.0 - 1e-9 && (p.d - d).abs() < tol).map(|(i, _)| i).collect();
    let (Ok(ma), Ok(mb)) = (a.tessellate_with(size * 1e-3, true), b.tessellate_with(size * 1e-3, true)) else { return false };
    let inside = ma.inside_index();
    let mut points = Vec::new();
    for (t, fi) in mb.triangles.iter().zip(&mb.tri_face) {
        if !faces.contains(&(*fi as usize)) {
            continue;
        }
        let Some([p, q, r]) = mb.tri(t) else { continue };
        let c = (p + q + r) / 3.0;
        points.extend([c, c.lerp(p, 0.8), c.lerp(q, 0.8), c.lerp(r, 0.8)]);
        if points.len() > 4000 {
            break;
        }
    }
    !points.is_empty() && points.iter().all(|p| [0.05, 0.5, 1.0].iter().all(|k| inside.contains(*p + m * (depth * k))))
}

/// Union of bodies that touch along one identical planar face, back to back (a body joined
/// with its mirror image): the two faces go and the shells are stitched along their edges.
/// `None` when no such pair of faces exists.
pub(crate) fn glue(a: &Body, b: &Body) -> Option<Result<Body>> {
    use mt::{BoundedCurve, ParametricCurve};
    let size = a.size().max(b.size());
    let tol = (size * 1e-7).max(1e-9) * 10.0;
    let key = |p: Vec3| ((p.x / tol).round() as i64, (p.y / tol).round() as i64, (p.z / tol).round() as i64);
    let mid = |e: &mt::Edge| {
        let c = e.curve();
        let (t0, t1) = c.range_tuple();
        from_p3(c.subs((t0 + t1) * 0.5))
    };
    // Matching faces: same plane, opposite normals, the same edges.
    let (pa, pb) = (planes(a), planes(b));
    let fa_list: Vec<&mt::Face> = a.solid.face_iter().collect();
    let fb_list: Vec<&mt::Face> = b.solid.face_iter().collect();
    for (ia, x) in &pa {
        for (ib, y) in &pb {
            if x.n.dot(y.n) > -1.0 + 1e-9 || (x.d + y.d).abs() > tol {
                continue;
            }
            let (Some(fa), Some(fb)) = (fa_list.get(*ia), fb_list.get(*ib)) else { continue };
            let ea: Vec<mt::Edge> = fa.edge_iter().collect();
            let eb: Vec<mt::Edge> = fb.edge_iter().collect();
            if ea.len() != eb.len() {
                continue;
            }
            // B edge → the matching A edge.
            let mut map: HashMap<mt::EdgeID, mt::Edge> = HashMap::new();
            let mut vmap: HashMap<mt::VertexID, mt::Vertex> = HashMap::new();
            let mut ok = true;
            for e in &eb {
                let ends = (key(from_p3(e.absolute_front().point())), key(from_p3(e.absolute_back().point())));
                let m = key(mid(e));
                let hit = ea.iter().find(|f| {
                    let fe = (key(from_p3(f.absolute_front().point())), key(from_p3(f.absolute_back().point())));
                    (fe == ends || fe == (ends.1, ends.0)) && key(mid(f)) == m
                });
                let Some(f) = hit else {
                    ok = false;
                    break;
                };
                let same = key(from_p3(f.absolute_front().point())) == ends.0;
                map.insert(e.id(), if same { f.absolute_clone() } else { f.absolute_clone().inverse() });
                let (fa_v, fb_v) = if same {
                    (f.absolute_front().clone(), f.absolute_back().clone())
                } else {
                    (f.absolute_back().clone(), f.absolute_front().clone())
                };
                vmap.insert(e.absolute_front().id(), fa_v);
                vmap.insert(e.absolute_back().id(), fb_v);
            }
            if !ok {
                continue;
            }
            return Some(guard("glue", || {
                let fail = |m: &str| KernelError::Failed(format!("joining touching faces: {m}"));
                // B's other edges, rebuilt on A's vertices where they meet the seam.
                let mut edges: HashMap<mt::EdgeID, mt::Edge> = map.clone();
                for e in b.solid.edge_iter() {
                    if edges.contains_key(&e.id()) {
                        continue;
                    }
                    let (f0, f1) = (e.absolute_front(), e.absolute_back());
                    if !vmap.contains_key(&f0.id()) && !vmap.contains_key(&f1.id()) {
                        edges.insert(e.id(), e.absolute_clone());
                        continue;
                    }
                    let v0 = vmap.get(&f0.id()).cloned().unwrap_or_else(|| f0.clone());
                    let v1 = vmap.get(&f1.id()).cloned().unwrap_or_else(|| f1.clone());
                    edges.insert(e.id(), mt::Edge::new(&v0, &v1, e.curve()));
                }
                let mut faces: Vec<mt::Face> = fa_list.iter().enumerate().filter(|(i, _)| i != ia).map(|(_, f)| (*f).clone()).collect();
                for (i, f) in fb_list.iter().enumerate() {
                    if i == *ib {
                        continue;
                    }
                    let wires: Vec<mt::Wire> = f
                        .absolute_boundaries()
                        .iter()
                        .map(|w| {
                            w.edge_iter()
                                .filter_map(|e| edges.get(&e.id()).map(|ne| if e.front() == e.absolute_front() { ne.clone() } else { ne.inverse() }))
                                .collect::<Vec<_>>()
                                .into()
                        })
                        .collect();
                    let mut nf = mt::Face::try_new(wires, f.surface()).map_err(|e| fail(&e.to_string()))?;
                    if !f.orientation() {
                        nf.invert();
                    }
                    faces.push(nf);
                }
                let solid = Solid::try_new(vec![faces.into()]).map_err(|e| fail(&e.to_string()))?;
                Body::new(crate::heal::heal(solid, size))
            }));
        }
    }
    None
}

/// A cylindrical face: its index, a point on the axis, the unit axis and the radius.
#[derive(Clone, Copy, Debug)]
struct CylFace {
    index: usize,
    origin: Vec3,
    axis: Vec3,
    radius: f64,
}

fn cylinders(b: &Body) -> Vec<CylFace> {
    (0..b.face_count())
        .filter_map(|index| match crate::face_surface(b, index)? {
            crate::FaceSurface::Cylinder { origin, axis, radius } => Some(CylFace { index, origin, axis, radius }),
            _ => None,
        })
        .collect()
}

/// Do the two cylinders lie on the same surface?
fn same_cylinder(x: &CylFace, y: &CylFace, tol: f64) -> bool {
    let off = y.origin - x.origin;
    x.axis.dot(y.axis).abs() > 1.0 - 1e-9 && (x.radius - y.radius).abs() < tol && (off - x.axis * off.dot(x.axis)).len() < tol
}

/// Do the bodies have cylindrical faces on a common cylinder?
pub(crate) fn has_coincident_cylinders(a: &Body, b: &Body) -> bool {
    let tol = (a.size().max(b.size()) * 1e-7).max(1e-9) * 10.0;
    let cb = cylinders(b);
    cylinders(a).iter().any(|x| cb.iter().any(|y| same_cylinder(x, y, tol)))
}

/// Union of bodies with bores on a common cylinder (a drain pad's bore around the drain hole
/// it is joined over). The mesh intersection can't take the coincident walls, so the tool's
/// bore is narrowed (its walls pushed toward the axis, now clear of the target's), the bodies
/// are joined, and the cylinder of the original bore is cut out again over the tool's length.
/// Recutting is valid only where that cylinder holds no material of either body. The
/// samples are plausibility checks; a whole-boundary guard rejects additional boundaries
/// crossing the recut. Original common-bore trims retain the legacy eligibility policy.
/// This is not a general emptiness or Boolean-equivalence certificate. Each direction
/// returns a result only when `accept` takes it; otherwise this returns `None`.
pub(crate) fn bore_union(a: &Body, b: &Body, accept: &dyn Fn(&Body) -> bool) -> Option<Body> {
    let size = a.size().max(b.size());
    let tol = (size * 1e-7).max(1e-9) * 10.0;
    let mtol = size * 2e-3;
    let (ma, mb) = (a.tessellate_with(mtol, true).ok()?, b.tessellate_with(mtol, true).ok()?);
    let (ia, ib) = (ma.inside_index(), mb.inside_index());
    let empty = |p: Vec3| !ia.contains(p) && !ib.contains(p);
    let (ca, cb) = (cylinders(a), cylinders(b));
    // (body to narrow, its mesh and inside test, the other body, their faces on the cylinder).
    let ways = [(b, &mb, &ib, a, &cb, &ca), (a, &ma, &ia, b, &ca, &cb)];
    for (tool, mesh, inside, target, ct, co) in ways {
        for c in ct {
            if !co.iter().any(|o| same_cylinder(c, o, tol)) {
                continue;
            }
            let bore: Vec<usize> = ct.iter().filter(|x| same_cylinder(c, x, tol)).map(|x| x.index).collect();
            // The tool's extent along the axis on that cylinder, and whether it is a bore (the
            // material outside it) at a point of the face.
            let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut probe = None;
            for (t, fi) in mesh.triangles.iter().zip(&mesh.tri_face) {
                if !bore.contains(&(*fi as usize)) {
                    continue;
                }
                let Some(ps) = mesh.tri(t) else { continue };
                for p in ps {
                    let s = (p - c.origin).dot(c.axis);
                    t0 = t0.min(s);
                    t1 = t1.max(s);
                }
                if probe.is_none() {
                    probe = Some((ps[0] + ps[1] + ps[2]) / 3.0);
                }
            }
            let Some(q) = probe else { continue };
            if !(t1 - t0 > tol && c.radius > tol) {
                continue;
            }
            let along = c.origin + c.axis * (q - c.origin).dot(c.axis);
            let Some(radial) = (q - along).normalized() else { continue };
            let on = along + radial * c.radius;
            let e = (c.radius * 0.05).max(size * 1e-4);
            if !inside.contains(on + radial * e) || inside.contains(on - radial * e) {
                continue;
            }
            // The cylinder over the tool's length holds no material of either body.
            let (u, w) = (c.axis.any_perp(), c.axis.cross(c.axis.any_perp()));
            let at = |s: f64, r: f64, ang: f64| c.origin + c.axis * s + (u * ang.cos() + w * ang.sin()) * r;
            let clear = (1..=9).all(|i| {
                let s = t0 + (t1 - t0) * f64::from(i) / 10.0;
                empty(at(s, 0.0, 0.0))
                    && (0..8).all(|k| {
                        let ang = f64::from(k) * std::f64::consts::FRAC_PI_4 + 0.3;
                        empty(at(s, c.radius * 0.5, ang)) && empty(at(s, c.radius * 0.9, ang))
                    })
            });
            if !clear {
                continue;
            }
            // Past an end where the space is empty well beyond the radius too, the cut runs on a
            // little (so its cap doesn't lie in a face's plane). Not where the other body's bore
            // goes on: the cut's side would lie on its wall.
            let ext = size * 0.01;
            let past = |s: f64| {
                empty(at(s, 0.0, 0.0))
                    && (0..8).all(|k| {
                        let ang = f64::from(k) * std::f64::consts::FRAC_PI_4 + 0.3;
                        empty(at(s, c.radius * 0.9, ang)) && empty(at(s, c.radius * 1.1, ang)) && empty(at(s, c.radius * 1.5, ang))
                    })
            };
            let lo = if past(t0 - ext * 0.5) && past(t0 - ext) { t0 - ext } else { t0 };
            let hi = if past(t1 + ext * 0.5) && past(t1 + ext) { t1 + ext } else { t1 };
            // The samples above are plausibility checks, not proof that an
            // arbitrarily thin web is absent. Every additional current boundary
            // must be wholly clear of the recut; unsupported trims refuse.
            let other_bore: Vec<usize> = co.iter().filter(|x| same_cylinder(c, x, tol)).map(|x| x.index).collect();
            if crate::bore_guard::boundaries_clear(tool, &bore, c.origin, c.axis, c.radius, lo, hi, tol).is_none()
                || crate::bore_guard::boundaries_clear(target, &other_bore, c.origin, c.axis, c.radius, lo, hi, tol).is_none()
            {
                continue;
            }
            let d = (c.radius * 0.2).min(size * 0.01);
            let run = || -> Result<Option<Body>> {
                let narrowed = crate::offset::offset_body(tool, |i, _| if bore.contains(&i) { d } else { 0.0 })?;
                if !narrowed.validity().is_empty() {
                    return Err(KernelError::Failed("narrowing a bore: an unsound result".into()));
                }
                let Some(joined) = crate::ops::boolean(target, &narrowed, BoolOp::Union)? else { return Ok(None) };
                let cut = crate::build::cylinder(at(lo, 0.0, 0.0), c.axis, c.radius, hi - lo)?;
                crate::ops::boolean(&joined, &cut, BoolOp::Cut)
            };
            match run() {
                Ok(Some(r)) if accept(&r) => return Some(r),
                _ => continue,
            }
        }
    }
    None
}
