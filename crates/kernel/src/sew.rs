//! Sewing a solid from face descriptions: each face is its boundary loops (lines and circular
//! arcs, counter-clockwise about the outward normal; holes clockwise) and its surface (planar,
//! revolved from a line, or extruded from an arc). Vertices and edges at the same places are
//! shared, so the faces close into one shell without booleans — for solids whose faces are
//! known exactly, such as folded sheet metal.

use std::collections::HashMap;

use mt::builder;
use solvecraft_geom::Vec3;
use truck_modeling as mt;

use crate::body::{Body, Solid, p3, v3};
use crate::{KernelError, Result, guard};

/// A boundary edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeSpec {
    Line {
        a: Vec3,
        b: Vec3,
    },
    /// A circular arc from `a` to `b` through `mid`.
    Arc {
        a: Vec3,
        b: Vec3,
        mid: Vec3,
    },
}

impl EdgeSpec {
    fn ends(&self) -> (Vec3, Vec3) {
        match *self {
            EdgeSpec::Line { a, b } | EdgeSpec::Arc { a, b, .. } => (a, b),
        }
    }
}

/// A face's surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfSpec {
    /// The plane of the boundary.
    Plane,
    /// The surface the line `a`→`b` sweeps turning `angle` about the axis.
    Revolved { a: Vec3, b: Vec3, axis_origin: Vec3, axis: Vec3, angle: f64 },
    /// The surface an arc sweeps moving along `dir`.
    Extruded { a: Vec3, b: Vec3, mid: Vec3, dir: Vec3 },
}

/// A face: boundary loops, surface, and a point on it with its outward normal (curved faces).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceSpec {
    pub loops: Vec<Vec<EdgeSpec>>,
    pub surface: SurfSpec,
    pub probe: Option<(Vec3, Vec3)>,
}

type Key = (i64, i64, i64);

fn key(p: Vec3) -> Key {
    let q = |x: f64| (x * 1e6).round() as i64;
    (q(p.x), q(p.y), q(p.z))
}

#[derive(Default)]
struct Cache {
    verts: HashMap<Key, mt::Vertex>,
    edges: HashMap<(Key, Key, Option<Key>, usize), mt::Edge>,
}

impl Cache {
    fn vertex(&mut self, p: Vec3) -> mt::Vertex {
        self.verts.entry(key(p)).or_insert_with(|| builder::vertex(p3(p))).clone()
    }
    fn edge(&mut self, e: &EdgeSpec, sheet: usize) -> mt::Edge {
        let (a, b) = e.ends();
        let mid = match e {
            EdgeSpec::Arc { mid, .. } => Some(key(*mid)),
            EdgeSpec::Line { .. } => None,
        };
        let (ka, kb) = (key(a), key(b));
        if let Some(found) = self.edges.get(&(ka, kb, mid, sheet)) {
            return found.clone();
        }
        if let Some(found) = self.edges.get(&(kb, ka, mid, sheet)) {
            return found.inverse();
        }
        let (va, vb) = (self.vertex(a), self.vertex(b));
        let made = match *e {
            EdgeSpec::Line { .. } => builder::line(&va, &vb),
            EdgeSpec::Arc { mid, .. } => builder::circle_arc(&va, &vb, p3(mid)),
        };
        self.edges.insert((ka, kb, mid, sheet), made.clone());
        made
    }
}

/// Sew the faces into a closed solid.
pub fn sew(faces: &[FaceSpec]) -> Result<Body> {
    sew_impl(faces, false, None)
}

/// Separate boundary sheets at a shared geometric edge without moving it.
/// Optional transient sources follow only unchanged produced face identities.
pub(crate) fn sew_touching_with_sources(faces: &[FaceSpec], sources: &[Option<crate::FaceSource>]) -> Result<Body> {
    if sources.len() != faces.len() {
        return Err(KernelError::Invalid("sew source alignment".into()));
    }
    sew_impl(faces, true, Some(sources))
}

type EdgeUse = (usize, usize, usize);

fn touching_sheets(faces: &[FaceSpec]) -> Result<HashMap<EdgeUse, usize>> {
    type Use = (EdgeUse, Vec3, Vec3, Vec3);
    let mut uses: HashMap<(Key, Key), Vec<Use>> = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        let Some(outer) = face.loops.first() else { continue };
        let normal = outer
            .iter()
            .fold(Vec3::ZERO, |n, e| {
                let (a, b) = e.ends();
                n + a.cross(b)
            })
            .normalized();
        for (li, lp) in face.loops.iter().enumerate() {
            for (ei, e) in lp.iter().enumerate() {
                if matches!(e, EdgeSpec::Line { .. }) && matches!(face.surface, SurfSpec::Plane) {
                    let (a, b) = e.ends();
                    let (ka, kb) = (key(a), key(b));
                    let k = if ka < kb { (ka, kb) } else { (kb, ka) };
                    if let Some(n) = normal {
                        uses.entry(k).or_default().push(((fi, li, ei), a, b, n));
                    }
                }
            }
        }
    }
    let mut sheets = HashMap::new();
    for group in uses.values().filter(|g| g.len() > 2) {
        if group.len() != 4 {
            return Err(KernelError::Invalid("sew: unsupported edge contact".into()));
        }
        let mut paired = std::collections::HashSet::new();
        for (i, (use_a, a, b, n)) in group.iter().enumerate() {
            if paired.contains(&i) {
                continue;
            }
            let t = (*b - *a).normalized().ok_or_else(|| KernelError::Invalid("sew: a degenerate contact".into()))?;
            // Pair the two faces bounding each material wedge, not the empty
            // wedge between them. Each pair uses the edge in opposite directions.
            let j = group
                .iter()
                .enumerate()
                .find_map(|(j, (_, c, d, m))| {
                    (i != j
                        && !paired.contains(&j)
                        && key(*a) == key(*d)
                        && key(*b) == key(*c)
                        && n.cross(t).dot(*m) < -1e-8
                        && m.cross(-t).dot(*n) < -1e-8)
                        .then_some(j)
                })
                .ok_or_else(|| KernelError::Invalid("sew: ambiguous edge contact".into()))?;
            let use_b = group.get(j).ok_or_else(|| KernelError::Invalid("sew: edge contact".into()))?.0;
            let sheet = paired.len() / 2;
            sheets.insert(*use_a, sheet);
            sheets.insert(use_b, sheet);
            paired.insert(i);
            paired.insert(j);
        }
    }
    Ok(sheets)
}

fn faces_connected(faces: &[mt::Face]) -> bool {
    let mut by_edge: HashMap<mt::EdgeID, Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for e in f.edge_iter() {
            by_edge.entry(e.id()).or_default().push(i);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut todo = vec![0];
    while let Some(i) = todo.pop() {
        if seen.insert(i)
            && let Some(f) = faces.get(i)
        {
            for e in f.edge_iter() {
                if let Some(adjacent) = by_edge.get(&e.id()) {
                    todo.extend(adjacent.iter().copied().filter(|j| !seen.contains(j)));
                }
            }
        }
    }
    seen.len() == faces.len()
}

/// Keep disconnected closed boundaries as distinct shells, preserving orientation.
fn closed_components(faces: Vec<mt::Face>) -> Result<Vec<mt::Shell>> {
    const MAX_EDGE_USES: usize = 500_000;
    const MAX_COMPONENTS: usize = 1024;
    let fail = |s: &str| KernelError::Invalid(format!("sew: {s}"));
    let mut by_edge: HashMap<mt::EdgeID, Vec<usize>> = HashMap::new();
    let mut edge_uses = 0usize;
    for (i, face) in faces.iter().enumerate() {
        for edge in face.edge_iter() {
            edge_uses = edge_uses.saturating_add(1);
            if edge_uses > MAX_EDGE_USES {
                return Err(fail("edge budget exceeded"));
            }
            let uses = by_edge.entry(edge.id()).or_default();
            if uses.len() >= 2 {
                return Err(fail("nonmanifold edge contact"));
            }
            uses.push(i);
        }
    }
    if by_edge.values().any(|uses| uses.len() != 2) {
        return Err(fail("the faces do not close"));
    }
    let mut seen = vec![false; faces.len()];
    let mut vertices = HashMap::new();
    let mut components = Vec::new();
    for first in 0..faces.len() {
        if seen.get(first).copied().unwrap_or(true) {
            continue;
        }
        if components.len() >= MAX_COMPONENTS {
            return Err(fail("component budget exceeded"));
        }
        let component = components.len();
        let mut todo = vec![first];
        if let Some(slot) = seen.get_mut(first) {
            *slot = true;
        }
        let mut indices = Vec::new();
        while let Some(i) = todo.pop() {
            let face = faces.get(i).ok_or_else(|| fail("missing component face"))?;
            indices.push(i);
            for vertex in face.vertex_iter() {
                if vertices.insert(vertex.id(), component).is_some_and(|previous| previous != component) {
                    return Err(fail("bodies touch only at a vertex"));
                }
            }
            for edge in face.edge_iter() {
                let adjacent = by_edge.get(&edge.id()).ok_or_else(|| fail("missing component edge"))?;
                for j in adjacent {
                    let slot = seen.get_mut(*j).ok_or_else(|| fail("missing adjacent face"))?;
                    if !*slot {
                        *slot = true;
                        todo.push(*j);
                    }
                }
            }
        }
        indices.sort_unstable();
        let component_faces =
            indices.into_iter().map(|i| faces.get(i).cloned().ok_or_else(|| fail("missing component face"))).collect::<Result<Vec<_>>>()?;
        components.push(component_faces.into());
    }
    Ok(components)
}

/// A contact's edge sheets can terminate in separate vertex fans. Give each fan
/// its own vertex at the same coordinates so the boundary remains a manifold.
fn split_vertex_fans(faces: Vec<mt::Face>) -> Result<Vec<mt::Face>> {
    let mut by_vertex: HashMap<mt::VertexID, Vec<usize>> = HashMap::new();
    let mut by_edge: HashMap<mt::EdgeID, Vec<usize>> = HashMap::new();
    let mut vertices = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for v in f.vertex_iter() {
            vertices.insert(v.id(), v.clone());
            by_vertex.entry(v.id()).or_default().push(i);
        }
        for e in f.edge_iter() {
            by_edge.entry(e.id()).or_default().push(i);
        }
    }
    let mut replacements: HashMap<(mt::VertexID, usize), mt::Vertex> = HashMap::new();
    for (id, members) in &mut by_vertex {
        members.sort_unstable();
        members.dedup();
        let mut seen = std::collections::HashSet::new();
        let mut fans = Vec::new();
        for start in members.iter().copied() {
            if seen.contains(&start) {
                continue;
            }
            let mut todo = vec![start];
            let mut fan = Vec::new();
            while let Some(i) = todo.pop() {
                if !seen.insert(i) {
                    continue;
                }
                fan.push(i);
                if let Some(f) = faces.get(i) {
                    for e in f.edge_iter().filter(|e| e.absolute_front().id() == *id || e.absolute_back().id() == *id) {
                        if let Some(adjacent) = by_edge.get(&e.id()) {
                            todo.extend(adjacent.iter().copied().filter(|j| !seen.contains(j)));
                        }
                    }
                }
            }
            fans.push(fan);
        }
        if fans.len() > 1 {
            let original = vertices.get(id).ok_or_else(|| KernelError::Failed("sew: missing contact vertex".into()))?;
            for fan in fans {
                let vertex = builder::vertex(original.point());
                for i in fan {
                    replacements.insert((*id, i), vertex.clone());
                }
            }
        }
    }
    let mut edges = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for e in f.edge_iter() {
            edges.entry(e.id()).or_insert_with(|| {
                let (a, b) = (e.absolute_front(), e.absolute_back());
                let a = replacements.get(&(a.id(), i)).unwrap_or(a);
                let b = replacements.get(&(b.id(), i)).unwrap_or(b);
                mt::Edge::new(a, b, e.curve())
            });
        }
    }
    faces
        .into_iter()
        .map(|f| {
            let wires: Vec<mt::Wire> = f
                .absolute_boundaries()
                .iter()
                .map(|w| {
                    w.edge_iter()
                        .map(|e| {
                            let rebuilt = edges.get(&e.id()).ok_or_else(|| KernelError::Failed("sew: missing contact edge".into()))?;
                            Ok(if e.front() == e.absolute_front() { rebuilt.clone() } else { rebuilt.inverse() })
                        })
                        .collect::<Result<Vec<_>>>()
                        .map(Into::into)
                })
                .collect::<Result<_>>()?;
            let mut rebuilt = mt::Face::try_new(wires, f.surface()).map_err(|e| KernelError::Failed(format!("sew: contact face: {e}")))?;
            if !f.orientation() {
                rebuilt.invert();
            }
            Ok(rebuilt)
        })
        .collect()
}

fn sew_impl(faces: &[FaceSpec], touching: bool, sources: Option<&[Option<crate::FaceSource>]>) -> Result<Body> {
    if faces.len() < 4 || faces.len() > 200_000 {
        return Err(KernelError::Invalid("a solid needs 4…200000 faces".into()));
    }
    let mut edge_uses = 0usize;
    for f in faces {
        for l in &f.loops {
            edge_uses = edge_uses.saturating_add(l.len().max(1));
            if edge_uses > 500_000 {
                return Err(KernelError::Invalid("sew: edge budget exceeded".into()));
            }
            for e in l {
                let (a, b) = e.ends();
                if !a.is_finite() || !b.is_finite() || a.dist(b) < 1e-9 {
                    return Err(KernelError::Invalid("sew: a degenerate edge".into()));
                }
            }
        }
    }
    guard("sew", || {
        let sheets = if touching { touching_sheets(faces)? } else { HashMap::new() };
        let mut cache = Cache::default();
        let mut out: Vec<mt::Face> = Vec::with_capacity(faces.len());
        for (fi, f) in faces.iter().enumerate() {
            let wires: Vec<mt::Wire> = f
                .loops
                .iter()
                .enumerate()
                .map(|(li, l)| {
                    l.iter().enumerate().map(|(ei, e)| cache.edge(e, sheets.get(&(fi, li, ei)).copied().unwrap_or(0))).collect::<Vec<_>>().into()
                })
                .collect();
            let face = match f.surface {
                SurfSpec::Plane => builder::try_attach_plane(&wires).map_err(|e| KernelError::Failed(format!("sew: face {fi}: {e}")))?,
                SurfSpec::Revolved { a, b, axis_origin, axis, angle } => {
                    let k = axis.normalized().ok_or_else(|| KernelError::Invalid("sew: axis".into()))?;
                    let (va, vb) = (builder::vertex(p3(a)), builder::vertex(p3(b)));
                    let swept: mt::Shell = builder::rsweep(&builder::line(&va, &vb), p3(axis_origin), v3(k), mt::Rad(angle));
                    let tmp = swept.face_iter().next().cloned().ok_or_else(|| KernelError::Failed("sew: revolved surface".into()))?;
                    curved(tmp.oriented_surface(), wires, f.probe, fi)?
                }
                SurfSpec::Extruded { a, b, mid, dir } => {
                    let (va, vb) = (builder::vertex(p3(a)), builder::vertex(p3(b)));
                    let tmp: mt::Face = builder::tsweep(&builder::circle_arc(&va, &vb, p3(mid)), v3(dir));
                    curved(tmp.oriented_surface(), wires, f.probe, fi)?
                }
            };
            out.push(face);
        }
        let source_ids: HashMap<_, _> = out
            .iter()
            .enumerate()
            .filter_map(|(i, f)| sources.and_then(|s| s.get(i)).and_then(Option::as_ref).map(|s| (f.id(), (f.orientation(), s))))
            .collect();
        if !sheets.is_empty() && !faces_connected(&out) {
            return Err(KernelError::Invalid("sew: bodies touch only along an edge".into()));
        }
        if !sheets.is_empty() {
            out = split_vertex_fans(out)?;
        }
        let shells = closed_components(out)?;
        let solid = Solid::try_new(shells).map_err(|e| KernelError::Failed(format!("sew: the faces do not close: {e}")))?;
        let body = Body::new(solid)?;
        let mapped =
            body.solid.face_iter().map(|f| source_ids.get(&f.id()).filter(|(o, _)| *o == f.orientation()).map(|(_, s)| (*s).clone())).collect();
        Ok(body.with_face_sources(sources.map(|_| std::sync::Arc::new(mapped))))
    })
}

/// A curved face on `surf`, turned so its normal at the probe points outward.
fn curved(mut surf: mt::Surface, wires: Vec<mt::Wire>, probe: Option<(Vec3, Vec3)>, fi: usize) -> Result<mt::Face> {
    use mt::{ParametricSurface3D, SearchParameter};
    if let Some((p, outward)) = probe
        && let Some((u, v)) = surf.search_parameter(p3(p), None, 100)
    {
        let n = surf.normal(u, v);
        if Vec3::new(n.x, n.y, n.z).dot(outward) < 0.0 {
            surf = mt::Invertible::inverse(&surf);
        }
    }
    mt::Face::try_new(wires, surf).map_err(|e| KernelError::Failed(format!("sew: face {fi}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> FaceSpec {
        FaceSpec {
            loops: vec![vec![EdgeSpec::Line { a, b }, EdgeSpec::Line { a: b, b: c }, EdgeSpec::Line { a: c, b: d }, EdgeSpec::Line { a: d, b: a }]],
            surface: SurfSpec::Plane,
            probe: None,
        }
    }

    fn box_faces(origin: Vec3, size: Vec3) -> Vec<FaceSpec> {
        let p = |x: f64, y: f64, z: f64| origin + Vec3::new(x * size.x, y * size.y, z * size.z);
        let (a, b, c, d) = (p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.));
        let (e, f, g, h) = (p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.));
        vec![quad(a, d, c, b), quad(e, f, g, h), quad(a, b, f, e), quad(b, c, g, f), quad(c, d, h, g), quad(d, a, e, h)]
    }

    #[test]
    fn disconnected_closed_faces_keep_every_material_component() {
        let mut faces = box_faces(Vec3::ZERO, Vec3::new(2.0, 3.0, 4.0));
        faces.extend(box_faces(Vec3::X * 10.0, Vec3::new(2.0, 3.0, 4.0)));
        for reverse in [false, true] {
            if reverse {
                faces.reverse();
            }
            let body = sew(&faces).unwrap();
            assert_eq!(body.shell_count(), 2);
            assert!(body.validity().is_empty());
            assert!((crate::measure(&body).unwrap().volume - 48.0).abs() < 1e-9);
            let lumps = body.lumps().unwrap();
            assert_eq!(lumps.len(), 2);
            assert!(lumps.iter().all(|b| (crate::measure(b).unwrap().volume - 24.0).abs() < 1e-9));
            let mesh = body.tessellate(0.01).unwrap();
            assert!(mesh.contains(Vec3::new(1.0, 1.0, 1.0)));
            assert!(mesh.contains(Vec3::new(11.0, 1.0, 1.0)));
            assert!(!mesh.contains(Vec3::new(5.0, 1.0, 1.0)));
            let imported = crate::step_import(&crate::step_export(&[&body], "sewn pieces").unwrap()).unwrap();
            assert!(imported.warnings.is_empty());
            let volume: f64 = imported.bodies.iter().map(|b| crate::measure(&b.body).unwrap().volume).sum();
            assert!((volume - 48.0).abs() < 1e-6);
        }
    }

    #[test]
    fn sewing_preserves_inward_cavity_orientation_and_refuses_vertex_contact() {
        let outer = box_faces(Vec3::ZERO, Vec3::new(2.0, 3.0, 4.0));
        let mut inner = box_faces(Vec3::new(0.5, 0.75, 1.0), Vec3::new(1.0, 1.5, 2.0));
        for face in &mut inner {
            for lp in &mut face.loops {
                lp.reverse();
                for edge in lp {
                    if let EdgeSpec::Line { a, b } = edge {
                        std::mem::swap(a, b);
                    }
                }
            }
        }
        let cavity = sew(&outer.iter().chain(&inner).cloned().collect::<Vec<_>>()).unwrap();
        assert_eq!(cavity.shell_count(), 2);
        assert_eq!(cavity.lumps().unwrap().len(), 1);
        assert!((crate::measure(&cavity).unwrap().volume - 21.0).abs() < 1e-9);
        assert!(!cavity.tessellate(0.01).unwrap().contains(Vec3::new(1.0, 1.5, 2.0)));
        let touching = box_faces(Vec3::new(2.0, 3.0, 4.0), Vec3::new(2.0, 3.0, 4.0));
        assert!(sew(&outer.iter().chain(&touching).cloned().collect::<Vec<_>>()).unwrap_err().to_string().contains("vertex"));
        let mut hostile = outer.clone();
        hostile[0].loops = vec![Vec::new(); 500_001];
        assert!(sew(&hostile).unwrap_err().to_string().contains("budget"));
    }

    #[test]
    fn a_sewn_box() {
        let p = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
        let (a, b, c, d) = (p(0., 0., 0.), p(2., 0., 0.), p(2., 3., 0.), p(0., 3., 0.));
        let (e, f, g, h) = (p(0., 0., 4.), p(2., 0., 4.), p(2., 3., 4.), p(0., 3., 4.));
        let faces = vec![quad(a, d, c, b), quad(e, f, g, h), quad(a, b, f, e), quad(b, c, g, f), quad(c, d, h, g), quad(d, a, e, h)];
        let body = sew(&faces).unwrap();
        let m = crate::measure(&body).unwrap();
        assert!((m.volume - 24.0).abs() < 1e-9, "{}", m.volume);
        assert_eq!(body.face_count(), 6);
        assert!(sew(&faces[..5]).is_err());
    }
}
