use serde::Serialize;
use solvecraft_geom::{Mesh, Vec3};
use truck_meshalgo::prelude::*;
use truck_modeling as mt;

use crate::{KernelError, Result, guard};

#[path = "planar_regularize.rs"]
mod planar_regularize;
#[path = "planar_sources.rs"]
mod planar_sources;
#[path = "shell_groups.rs"]
mod shell_groups;

pub(crate) type Solid = mt::Solid;

/// A solid body (one or more closed shells), or a mesh body (triangles only, like an imported
/// 3MF or STL: it renders, measures and exports, but B-rep features cannot use it). Cheap to
/// clone; never mutated in place.
#[derive(Clone, Debug)]
pub struct Body {
    pub(crate) solid: std::sync::Arc<Solid>,
    /// Missing-face check from the orientation mesh, reusable at exactly its tolerance.
    pub(crate) coarse_missing: Option<(f64, usize)>,
    /// Set for mesh bodies (`solid` is then empty).
    pub(crate) mesh: Option<std::sync::Arc<crate::meshbody::TriMesh>>,
    /// Display colour (linear RGB 0..1) from an imported file; operations drop it.
    pub(crate) color: Option<[f32; 3]>,
    /// Opacity and face colours for export (set by the document; operations drop it).
    pub(crate) paint: Option<std::sync::Arc<Paint>>,
    /// Lines the user split faces along (Split Face), as polylines: export keeps the faces on
    /// either side apart where it would otherwise join pieces of one surface.
    pub(crate) splits: Option<std::sync::Arc<Vec<Vec<Vec3>>>>,
    /// Exact construction/import source identities, in this body's face order.
    pub(crate) face_sources: Option<std::sync::Arc<Vec<Option<FaceSource>>>>,
    /// Sticky transient veto: authored face boundaries must survive later rebuilds.
    pub(crate) regularization_protected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FaceSource {
    ExtrudeSide { segment: solvecraft_geom::Seg2, plane: Option<solvecraft_geom::Plane>, coalesced: bool },
    CertifiedExtrudeSide { producer: u64, sketch: u64, curve: String, plane: Option<solvecraft_geom::Plane> },
    ExtrudeCap { high: bool, region: solvecraft_geom::Region2, plane: solvecraft_geom::Plane },
    CertifiedExtrudeCap { producer: u64, sketch: u64, high: bool, outer: Vec<String>, holes: Vec<Vec<String>>, plane: solvecraft_geom::Plane },
    Box { producer: Option<u64>, axis: u8, high: bool, plane: solvecraft_geom::Plane },
    ShellPlanar { producer: Option<u64>, source: Box<FaceSource>, inner: bool, plane: solvecraft_geom::Plane },
    Imported { item: u64, face: u64, payload_sha256: Option<String>, producer: Option<u64> },
}

pub type CapSignature = (Vec<String>, Vec<Vec<String>>);
type FaceSources = std::sync::Arc<Vec<Option<FaceSource>>>;

impl FaceSource {
    fn plane_mut(&mut self) -> Option<&mut solvecraft_geom::Plane> {
        match self {
            Self::ExtrudeSide { plane, .. } | Self::CertifiedExtrudeSide { plane, .. } => plane.as_mut(),
            Self::ExtrudeCap { plane, .. } | Self::CertifiedExtrudeCap { plane, .. } | Self::Box { plane, .. } | Self::ShellPlanar { plane, .. } => {
                Some(plane)
            }
            Self::Imported { .. } => None,
        }
    }
}

/// How a body looks beyond its colour, for export: its opacity and its faces' own colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    /// 1 = opaque.
    pub opacity: f32,
    pub faces: Vec<FacePaint>,
}

/// A face's own colour: the B-rep face index (`face_iter` order), sRGB as 0..1, opacity.
#[derive(Clone, Debug, PartialEq)]
pub struct FacePaint {
    pub face: usize,
    pub color: [f32; 3],
    pub opacity: f32,
}

/// A B-rep edge as seen by the rest of the application.
#[derive(Clone, Debug, Serialize)]
pub struct EdgeInfo {
    pub index: usize,
    /// Polyline approximation.
    pub points: Vec<Vec3>,
    /// Point halfway along the polyline (used to re-find the edge).
    pub mid: Vec3,
    pub length: f64,
}

/// A B-rep face.
#[derive(Clone, Debug, Serialize)]
pub struct FaceInfo {
    pub index: usize,
    pub area: f64,
    pub centroid: Vec3,
    /// Unit normal if the face is planar.
    pub plane_normal: Option<Vec3>,
}

type MeshedShell = truck_topology::Shell<mt::Point3, PolylineCurve<mt::Point3>, Option<PolygonMesh>>;

/// Triangulate each shell (not the solid: truck's solid constructor panics on the open or
/// non-manifold shells an import can produce). Boundaries that lie on their surface only within
/// tolerance (imports, boolean results) are projected onto it.
fn mesh_shells(solid: &Solid, tol: f64) -> Vec<MeshedShell> {
    solid
        .boundaries()
        .iter()
        // The robust search tries the exact one first, so exact geometry costs nothing extra; a
        // face can also come back as an empty mesh rather than none (boolean results with
        // approximate intersection curves), so always use it.
        .map(|sh| sh.robust_triangulation(tol))
        .collect()
}

/// A face's triangles (corner positions and normals, in the mesh's winding) with every edge
/// whose middle sags more than `tol` from the surface split there, found through the mesh's
/// surface parameters. A split depends on the edge alone, so neighbours stay conforming.
/// `None` when nothing needs splitting or the parameters don't match the surface.
#[allow(clippy::type_complexity)]
fn refined(pm: &PolygonMesh, surf: &mt::Surface, tol: f64) -> Option<Vec<[(Vec3, Vec3); 3]>> {
    refined_with_cache(pm, surf, tol, true)
}

#[allow(clippy::type_complexity)]
fn refined_with_cache(pm: &PolygonMesh, surf: &mt::Surface, tol: f64, memoize: bool) -> Option<Vec<[(Vec3, Vec3); 3]>> {
    use mt::{ParametricSurface, ParametricSurface3D};
    let (pos, uvs, nor) = (pm.positions(), pm.uv_coords(), pm.normals());
    if uvs.is_empty() {
        return None;
    }
    type V = (Vec3, (f64, f64), Vec3);
    let mut tris: Vec<[V; 3]> = Vec::new();
    for tri in pm.faces().triangle_iter() {
        let mut t = [(Vec3::ZERO, (0.0, 0.0), Vec3::Z); 3];
        for (slot, v) in t.iter_mut().zip(tri.iter()) {
            let p = pos.get(v.pos)?;
            let uv = uvs.get(v.uv?)?;
            let n = v.nor.and_then(|i| nor.get(i)).map(|n| Vec3::new(n.x, n.y, n.z)).unwrap_or(Vec3::Z);
            *slot = (Vec3::new(p.x, p.y, p.z), (uv.x, uv.y), n);
        }
        tris.push(t);
    }
    // The parameters must be the surface's own.
    let check = tris.iter().take(8).flat_map(|t| t.iter()).all(|(p, (u, v), _)| from_p3(surf.subs(*u, *v)).dist(*p) < tol * 4.0 + 1e-9);
    // Bounded work: a face may grow to a few times its triangles (wild surfaces sag
    // everywhere and are left as meshed).
    let cap = (tris.len() * 8).clamp(2_000, 60_000);
    if !check || tris.len() > 50_000 {
        return None;
    }
    // Edges on the face's boundary are shared with the next face: never split them.
    let k = |x: &V| ((x.1.0 * 1e9).round() as i64, (x.1.1 * 1e9).round() as i64);
    let ek = |a: &V, b: &V| if k(a) < k(b) { (k(a), k(b)) } else { (k(b), k(a)) };
    // Only exact finite values identify reusable evaluations. In particular,
    // the rounded UV keys used for boundary incidence cannot identify a point
    // whose derivatives or midpoint may be reused.
    let vertex_key = |v: &V| {
        let values = [v.0.x, v.0.y, v.0.z, v.1.0, v.1.1, v.2.x, v.2.y, v.2.z];
        values.iter().all(|x| x.is_finite()).then(|| values.map(f64::to_bits))
    };
    let cache_limit = cap * 3;
    let mut singular_cache = std::collections::HashMap::<(u64, u64), bool>::new();
    let mut changed = false;
    for _ in 0..4 {
        let mut uses: std::collections::HashMap<((i64, i64), (i64, i64)), u32> = std::collections::HashMap::new();
        for [a, b, c] in &tris {
            for e in [ek(a, b), ek(b, c), ek(c, a)] {
                *uses.entry(e).or_default() += 1;
            }
        }
        let mut edge_cache = std::collections::HashMap::<([u64; 8], [u64; 8]), Option<V>>::new();
        let mut at = |a: &V, b: &V| -> Option<V> {
            let key = if memoize { vertex_key(a).zip(vertex_key(b)).map(|(a, b)| if a < b { (a, b) } else { (b, a) }) } else { None };
            if let Some(key) = key
                && let Some(value) = edge_cache.get(&key)
            {
                return *value;
            }
            let mut calculate = || -> Option<V> {
                if uses.get(&ek(a, b)).copied().unwrap_or(0) < 2 {
                    return None;
                }
                // Not at a degenerate point (a cone's apex: its parameters there are arbitrary).
                let mut singular = |x: &V| {
                    let key = (x.1.0.to_bits(), x.1.1.to_bits());
                    let cacheable = memoize && x.1.0.is_finite() && x.1.1.is_finite();
                    if cacheable && let Some(value) = singular_cache.get(&key) {
                        return *value;
                    }
                    let (du, dv) = (surf.uder(x.1.0, x.1.1), surf.vder(x.1.0, x.1.1));
                    let c = du.cross(dv);
                    let value = c.x * c.x + c.y * c.y + c.z * c.z
                        <= 1e-12 * (du.x * du.x + du.y * du.y + du.z * du.z + dv.x * dv.x + dv.y * dv.y + dv.z * dv.z).powi(2) + 1e-300;
                    if cacheable && singular_cache.len() < cache_limit {
                        singular_cache.insert(key, value);
                    }
                    value
                };
                if singular(a) || singular(b) {
                    return None;
                }
                let (u, v) = ((a.1.0 + b.1.0) / 2.0, (a.1.1 + b.1.1) / 2.0);
                let q = from_p3(surf.subs(u, v));
                // Within tolerance, or so far off that the parameters wrapped (a seam).
                let sag = q.dist((a.0 + b.0) * 0.5);
                if sag <= tol || sag >= 0.5 * a.0.dist(b.0) {
                    return None;
                }
                let n = surf.normal(u, v);
                let mut n = Vec3::new(n.x, n.y, n.z);
                // Keep the mesh's normal side.
                if n.dot(a.2 + b.2) < 0.0 {
                    n = -n;
                }
                Some((q, (u, v), n))
            };
            let value = calculate();
            if let Some(key) = key
                && edge_cache.len() < cache_limit
                && value.is_none_or(|v| vertex_key(&v).is_some())
            {
                edge_cache.insert(key, value);
            }
            value
        };
        let mut next = Vec::with_capacity(tris.len());
        let mut any = false;
        for t in &tris {
            let [a, b, c] = t;
            let m = [at(a, b), at(b, c), at(c, a)];
            any |= m.iter().any(Option::is_some);
            match m {
                [None, None, None] => next.push(*t),
                [Some(x), Some(y), Some(z)] => {
                    next.extend([[*a, x, z], [x, *b, y], [z, y, *c], [x, y, z]]);
                }
                [Some(x), None, None] => next.extend([[*a, x, *c], [x, *b, *c]]),
                [None, Some(y), None] => next.extend([[*a, *b, y], [*a, y, *c]]),
                [None, None, Some(z)] => next.extend([[*a, *b, z], [z, *b, *c]]),
                [Some(x), Some(y), None] => next.extend([[*a, x, *c], [x, *b, y], [x, y, *c]]),
                [None, Some(y), Some(z)] => next.extend([[*a, *b, z], [z, *b, y], [z, y, *c]]),
                [Some(x), None, Some(z)] => next.extend([[*a, x, z], [x, *b, *c], [x, *c, z]]),
            }
        }
        tris = next;
        changed |= any;
        if !any || tris.len() > cap {
            break;
        }
    }
    changed.then(|| tris.into_iter().map(|t| t.map(|(p, _, n)| (p, n))).collect())
}

/// Revolved surfaces are periodic in their angle; a face whose angles start at 0 is found at 0
/// or 2π, which confuses the boolean's loop projection. Each revolved face's surface is turned
/// (same geometry) so the face sits around angle π, well away from the wrap.
fn rephase_revolved(solid: Solid) -> Solid {
    use mt::{BoundedCurve, ParametricCurve, SearchNearestParameter};
    let r = guard("rephase", || {
        let mut changed = false;
        let mut shells = Vec::new();
        for sh in solid.boundaries() {
            let mut faces = Vec::new();
            for f in sh.face_iter() {
                let mt::Surface::RevolutedCurve(proc_) = f.surface() else {
                    faces.push(f.clone());
                    continue;
                };
                let surf = f.surface();
                // The face's middle angle.
                let mut acc = Vec3::ZERO;
                let mut n = 0.0f64;
                for w in f.absolute_boundaries() {
                    for e in w.edge_iter() {
                        let c = e.oriented_curve();
                        let (t0, t1) = c.range_tuple();
                        for k in 0..4 {
                            acc += from_p3(c.subs(t0 + (t1 - t0) * k as f64 / 4.0));
                            n += 1.0;
                        }
                    }
                }
                let Some((_, v)) = surf.search_nearest_parameter(p3(acc / n.max(1.0)), None, 100) else {
                    faces.push(f.clone());
                    continue;
                };
                let phi = v.rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
                if phi.abs() < 0.3 {
                    faces.push(f.clone());
                    continue;
                }
                let new_proc = proc_.map_ref(|rc| {
                    let (o, a) = (rc.origin(), rc.axis());
                    let rot = mt::Matrix4::from_translation(mt::EuclideanSpace::to_vec(o))
                        * mt::Matrix4::from_axis_angle(a, mt::Rad(phi))
                        * mt::Matrix4::from_translation(-mt::EuclideanSpace::to_vec(o));
                    mt::RevolutedCurve::by_revolution(mt::Transformed::transformed(rc.entity_curve(), rot), o, a)
                });
                match mt::Face::try_new(f.absolute_boundaries().clone(), mt::Surface::RevolutedCurve(new_proc)) {
                    Ok(mut nf) => {
                        if !f.orientation() {
                            nf.invert();
                        }
                        faces.push(nf);
                        changed = true;
                    }
                    Err(_) => faces.push(f.clone()),
                }
            }
            shells.push(mt::Shell::from(faces));
        }
        if !changed {
            return Ok(None);
        }
        Ok(Solid::try_new(shells).ok())
    });
    match r {
        Ok(Some(s)) => s,
        _ => solid,
    }
}

pub(crate) fn p3(v: Vec3) -> mt::Point3 {
    mt::Point3::new(v.x, v.y, v.z)
}
pub(crate) fn v3(v: Vec3) -> mt::Vector3 {
    mt::Vector3::new(v.x, v.y, v.z)
}
pub(crate) fn from_p3(p: mt::Point3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn polyline_mid(pts: &[Vec3]) -> (Vec3, f64) {
    let len: f64 = pts.windows(2).map(|w| w.first().zip(w.get(1)).map(|(a, b)| a.dist(*b)).unwrap_or(0.0)).sum();
    let mut acc = 0.0;
    for w in pts.windows(2) {
        if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
            let l = a.dist(*b);
            if acc + l >= len * 0.5 && l > 0.0 {
                return (a.lerp(*b, (len * 0.5 - acc) / l), len);
            }
            acc += l;
        }
    }
    (pts.first().copied().unwrap_or_default(), len)
}

impl Body {
    /// The very same body (shared data), not just an equal-looking one.
    pub fn same(&self, other: &Body) -> bool {
        std::sync::Arc::ptr_eq(&self.solid, &other.solid)
            && match (&self.mesh, &other.mesh) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.color == other.color
            && self.paint == other.paint
            && self.splits == other.splits
            && self.face_sources == other.face_sources
            && self.regularization_protected == other.regularization_protected
    }

    pub(crate) fn new(solid: Solid) -> Result<Body> {
        if solid.boundaries().is_empty() {
            return Err(KernelError::Failed("empty result".into()));
        }
        let mut solid = rephase_revolved(solid);
        // Orient outward: a closed solid must have positive volume (meshed finely enough that
        // every face comes out: a face missing from a coarse mesh can flip the sign).
        let span = {
            let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
            let mut hi = -lo;
            for v in solid.vertex_iter() {
                let p = from_p3(v.point());
                lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
            }
            if lo.x.is_finite() { (hi - lo).len() } else { 1.0 }
        };
        let tol = (span * 0.01).clamp(1e-3, 1.0);
        let (v, missing) = guard("orient", || {
            let shells = mesh_shells(&solid, tol);
            let volume = shells.iter().map(|s| s.to_polygon().volume()).sum::<f64>();
            let missing =
                shells.iter().flat_map(|s| s.face_iter()).filter(|f| f.surface().is_none_or(|m| m.faces().triangle_iter().next().is_none())).count();
            Ok((volume, missing))
        })?;
        if v < 0.0 {
            solid.not();
        }
        Ok(Body {
            solid: std::sync::Arc::new(solid),
            coarse_missing: (v >= 0.0).then_some((tol, missing)),
            mesh: None,
            color: None,
            paint: None,
            splits: None,
            face_sources: None,
            regularization_protected: false,
        })
    }

    pub(crate) fn inherit_regularization_protection(mut self, input: &Body) -> Self {
        self.regularization_protected |= input.regularization_protected;
        self
    }

    pub fn face_sources(&self) -> Option<&[Option<FaceSource>]> {
        self.face_sources.as_deref().map(Vec::as_slice)
    }

    /// A current wall reconstructed from an exact contiguous section run.
    /// This transient witness grants no durable source/producer identity.
    pub fn face_was_coalesced(&self, index: usize) -> bool {
        matches!(self.face_sources().and_then(|s| s.get(index)), Some(Some(FaceSource::ExtrudeSide { coalesced: true, .. })))
    }

    /// An exact construction/import plane; never inferred from tessellation.
    pub fn face_plane(&self, index: usize) -> Option<solvecraft_geom::Plane> {
        if index >= 50_000 {
            return None;
        }
        let source = self.face_sources()?.get(index)?.as_ref()?;
        match source {
            FaceSource::ExtrudeSide { plane, .. } | FaceSource::CertifiedExtrudeSide { plane, .. } => *plane,
            FaceSource::ExtrudeCap { plane, .. }
            | FaceSource::CertifiedExtrudeCap { plane, .. }
            | FaceSource::Box { plane, .. }
            | FaceSource::ShellPlanar { plane, .. } => Some(*plane),
            FaceSource::Imported { .. } => self.planar_face(index),
        }
    }

    /// Exact current planar surface for an explicitly fixed snapshot, without source lineage.
    pub fn planar_face(&self, index: usize) -> Option<solvecraft_geom::Plane> {
        if index >= 50_000 {
            return None;
        }
        guard("exact planar face", || {
            let plane = self.solid.face_iter().nth(index).and_then(|face| match face.oriented_surface() {
                mt::Surface::Plane(plane) => {
                    let u = plane.u_axis();
                    let v = plane.v_axis();
                    solvecraft_geom::Plane::new(from_p3(plane.origin()), Vec3::new(u.x, u.y, u.z), Vec3::new(v.x, v.y, v.z))
                }
                _ => None,
            });
            Ok(plane)
        })
        .ok()
        .flatten()
    }

    pub fn with_box_certificate(mut self, producer: u64) -> Result<Self> {
        if producer == 0 {
            return Err(KernelError::Invalid("invalid box producer".into()));
        }
        if let Some(sources) = &self.face_sources {
            let mut sources = sources.as_ref().clone();
            for source in sources.iter_mut().flatten() {
                if let FaceSource::Box { producer: id, .. } = source {
                    *id = Some(producer);
                }
            }
            self.face_sources = Some(std::sync::Arc::new(sources));
        }
        Ok(self)
    }

    /// Cap signatures are in face order; unknown or ambiguous source curves stay uncertified.
    pub fn with_extrude_cap_certificates(mut self, producer: u64, sketch: u64, signatures: &[Option<CapSignature>]) -> Result<Self> {
        if producer == 0 || sketch == 0 {
            return Err(KernelError::Invalid("invalid cap producer".into()));
        }
        if let Some(sources) = &self.face_sources {
            if sources.len() != signatures.len() {
                return Err(KernelError::Invalid("cap certificate count mismatch".into()));
            }
            let mut sources = sources.as_ref().clone();
            for (source, signature) in sources.iter_mut().zip(signatures) {
                if let Some(FaceSource::ExtrudeCap { high, plane, .. }) = source {
                    let Some((outer, holes)) = signature else {
                        *source = None;
                        continue;
                    };
                    if outer.len() > 4096 || holes.len() > 4096 || holes.iter().any(|hole| hole.len() > 4096) {
                        return Err(KernelError::Invalid("oversized cap curve signature".into()));
                    }
                    let count = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
                    if outer.is_empty()
                        || count > 4096
                        || holes.len() > 4096
                        || outer.iter().chain(holes.iter().flatten()).map(String::len).sum::<usize>() > 65_536
                        || outer.iter().chain(holes.iter().flatten()).any(|id| id.is_empty() || id.len() > 128)
                    {
                        return Err(KernelError::Invalid("invalid cap curve signature".into()));
                    }
                    let mut outer = outer.clone();
                    outer.sort();
                    outer.dedup();
                    let mut holes = holes.clone();
                    for hole in &mut holes {
                        if hole.is_empty() {
                            return Err(KernelError::Invalid("empty cap hole signature".into()));
                        }
                        hole.sort();
                        hole.dedup();
                    }
                    holes.sort();
                    if holes.windows(2).any(|pair| pair[0] == pair[1]) || holes.iter().flatten().any(|id| outer.binary_search(id).is_ok()) {
                        return Err(KernelError::Invalid("ambiguous cap hole signature".into()));
                    }
                    *source = Some(FaceSource::CertifiedExtrudeCap { producer, sketch, high: *high, outer, holes, plane: *plane });
                }
            }
            self.face_sources = Some(std::sync::Arc::new(sources));
        }
        Ok(self)
    }

    pub(crate) fn transformed_sources(&self, m: [[f64; 4]; 4]) -> Result<Option<FaceSources>> {
        let Some(sources) = &self.face_sources else { return Ok(None) };
        let vector = |p: Vec3| {
            Vec3::new(
                m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z,
                m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z,
                m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z,
            )
        };
        let translation = Vec3::new(m[3][0], m[3][1], m[3][2]);
        let reflected = vector(Vec3::X).dot(vector(Vec3::Y).cross(vector(Vec3::Z))) < 0.;
        let mut sources = sources.as_ref().clone();
        for source in sources.iter_mut().flatten() {
            if let Some(plane) = source.plane_mut() {
                *plane = solvecraft_geom::Plane::new(
                    vector(plane.origin) + translation,
                    vector(plane.x),
                    vector(plane.y) * if reflected { -1. } else { 1. },
                )
                .ok_or_else(|| KernelError::Invalid("degenerate transformed face plane".into()))?;
            }
        }
        Ok(Some(std::sync::Arc::new(sources)))
    }

    /// Bind constructed edges to uniquely proven source curves without changing face names.
    pub fn with_extrude_certificates(mut self, producer: u64, sketch: u64, curves: &[Option<String>]) -> Result<Self> {
        if producer == 0 || sketch == 0 || curves.iter().flatten().any(|curve| curve.is_empty() || curve.len() > 128) {
            return Err(KernelError::Invalid("invalid constructed side certificate".into()));
        }
        if let Some(sources) = &self.face_sources {
            if sources.len() != curves.len() {
                return Err(KernelError::Invalid("side certificate count mismatch".into()));
            }
            let sources = sources
                .iter()
                .zip(curves)
                .map(|(source, curve)| match (source, curve) {
                    (Some(FaceSource::ExtrudeSide { plane, .. }), Some(curve)) => {
                        Some(FaceSource::CertifiedExtrudeSide { producer, sketch, curve: curve.clone(), plane: *plane })
                    }
                    (Some(FaceSource::ExtrudeSide { .. }), None) => None,
                    _ => source.clone(),
                })
                .collect();
            self.face_sources = Some(std::sync::Arc::new(sources));
        }
        Ok(self)
    }

    pub fn with_import_certificate(mut self, digest: &str, producer: u64) -> Result<Self> {
        if producer == 0 || digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(KernelError::Invalid("invalid immutable import certificate".into()));
        }
        if let Some(sources) = &self.face_sources {
            let mut sources = sources.as_ref().clone();
            for source in sources.iter_mut().flatten() {
                if let FaceSource::Imported { payload_sha256, producer: origin, .. } = source {
                    *payload_sha256 = Some(digest.into());
                    *origin = Some(producer);
                }
            }
            self.face_sources = Some(std::sync::Arc::new(sources));
        }
        Ok(self)
    }

    pub(crate) fn with_face_sources(mut self, sources: Option<std::sync::Arc<Vec<Option<FaceSource>>>>) -> Self {
        self.face_sources = sources;
        self
    }

    /// A deep copy of the solid that can be mutated without affecting other bodies.
    pub(crate) fn deep_copy(&self) -> Solid {
        mt::builder::clone(&*self.solid)
    }

    /// Is this a mesh body (no B-rep)?
    pub fn is_mesh(&self) -> bool {
        self.mesh.is_some()
    }

    /// Error for operations that need a B-rep.
    pub(crate) fn require_brep(&self, what: &str) -> Result<()> {
        if self.is_mesh() {
            Err(KernelError::Invalid(format!("{what} needs a solid body; mesh bodies can only be moved, measured and exported")))
        } else {
            Ok(())
        }
    }

    /// Display colour from an imported file.
    pub fn color(&self) -> Option<[f32; 3]> {
        self.color
    }

    /// The same body with a display colour.
    pub fn with_color(mut self, c: Option<[f32; 3]>) -> Body {
        self.color = c.filter(|c| c.iter().all(|x| x.is_finite())).map(|c| c.map(|x| x.clamp(0.0, 1.0)));
        self
    }

    /// Lines faces were split along on purpose (Split Face), as polylines.
    pub fn split_lines(&self) -> Option<&std::sync::Arc<Vec<Vec<Vec3>>>> {
        self.splits.as_ref()
    }

    pub(crate) fn split_keep(&self) -> &[Vec<Vec3>] {
        self.splits.as_ref().map(|l| l.as_slice()).unwrap_or(&[])
    }

    /// The same body with the lines its faces were split along on purpose.
    pub fn with_split_lines(mut self, s: Option<std::sync::Arc<Vec<Vec<Vec3>>>>) -> Body {
        self.splits = s.filter(|l| !l.is_empty());
        self.regularization_protected |= self.splits.is_some();
        self
    }

    /// Opacity and face colours for export, if any.
    pub fn paint(&self) -> Option<&Paint> {
        self.paint.as_deref()
    }

    /// The same body with opacity and face colours for export (`None` or opaque with no face
    /// colours clears them).
    pub fn with_paint(mut self, p: Option<Paint>) -> Body {
        let clean = |x: f32| if x.is_finite() { x.clamp(0.0, 1.0) } else { 1.0 };
        self.paint = p
            .map(|mut p| {
                p.opacity = clean(p.opacity);
                p.faces.retain(|f| f.color.iter().all(|x| x.is_finite()));
                for f in &mut p.faces {
                    f.color = f.color.map(|x| x.clamp(0.0, 1.0));
                    f.opacity = clean(f.opacity);
                }
                p
            })
            .filter(|p| p.opacity < 1.0 || !p.faces.is_empty())
            .map(std::sync::Arc::new);
        self.regularization_protected |= self.paint().is_some_and(|paint| !paint.faces.is_empty());
        self
    }

    /// Characteristic size (bounding box diagonal) for tolerances.
    pub fn size(&self) -> f64 {
        let mut b = solvecraft_geom::Aabb3::EMPTY;
        if let Some(m) = &self.mesh {
            for p in &m.positions {
                b.add(*p);
            }
        }
        for v in self.solid.vertex_iter() {
            b.add(from_p3(v.point()));
        }
        let d = b.diagonal();
        if d.is_finite() && d > 0.0 { d } else { 1.0 }
    }

    /// Distinct edges in a stable order (the order of `edges()` and `tessellate().edges`).
    pub(crate) fn unique_edges(solid: &Solid) -> Vec<mt::Edge> {
        let mut seen = std::collections::HashSet::new();
        solid.edge_iter().filter(|e| seen.insert(e.id())).collect()
    }

    /// Triangulate with chord tolerance `tol` (mm). Triangles carry their face index.
    pub fn tessellate(&self, tol: f64) -> Result<Mesh> {
        self.tessellate_with(tol, false)
    }

    /// Like [`Body::tessellate`]; with `refine`, triangles that cut through curved faces are
    /// split until within `tol` of the surface (for measuring).
    pub fn tessellate_with(&self, tol: f64, refine: bool) -> Result<Mesh> {
        let tol = if tol.is_finite() && tol > 0.0 { tol } else { 0.05 };
        let tol = tol.max(self.size() * 1e-6);
        if let Some(m) = &self.mesh {
            return Ok(m.to_mesh());
        }
        guard("tessellate", || {
            let meshed = mesh_shells(&self.solid, tol);
            let mut out = Mesh::default();
            let solid_faces: Vec<&mt::Face> = self.solid.face_iter().collect();
            for (fi, face) in meshed.iter().flat_map(|s| s.face_iter()).enumerate() {
                let Some(pm) = face.surface() else { continue };
                let flip = !face.orientation();
                let base = u32::try_from(out.positions.len()).unwrap_or(u32::MAX);
                // Curved faces: split triangles that cut through the surface.
                let refinement = if refine {
                    solid_faces.get(fi).and_then(|sf| {
                        let surface = sf.surface();
                        if matches!(surface, mt::Surface::Plane(_)) { None } else { refined(&pm, &surface, tol) }
                    })
                } else {
                    None
                };
                if let Some(tris) = refinement {
                    let fid = u32::try_from(fi).unwrap_or(u32::MAX);
                    for t in tris {
                        let mut idx = [0u32; 3];
                        for (slot, (p, n)) in idx.iter_mut().zip(t) {
                            out.positions.push(p);
                            out.normals.push(if flip { -n } else { n });
                            *slot = u32::try_from(out.positions.len() - 1).unwrap_or(0);
                        }
                        out.triangles.push(if flip { [idx[0], idx[2], idx[1]] } else { idx });
                        out.tri_face.push(fid);
                    }
                    continue;
                }
                let pos = pm.positions();
                let nor = pm.normals();
                // Expand to per-corner vertices so normals stay per face (crisp edges).
                for tri in pm.faces().triangle_iter() {
                    let mut idx = [0u32; 3];
                    for (k, v) in tri.iter().enumerate() {
                        let Some(p) = pos.get(v.pos) else { continue };
                        let n = v.nor.and_then(|ni| nor.get(ni)).map(|n| Vec3::new(n.x, n.y, n.z)).unwrap_or(Vec3::Z);
                        let n = if flip { -n } else { n };
                        out.positions.push(Vec3::new(p.x, p.y, p.z));
                        out.normals.push(n);
                        if let Some(slot) = idx.get_mut(k) {
                            *slot = u32::try_from(out.positions.len() - 1).unwrap_or(0);
                        }
                    }
                    let t = if flip { [idx[0], idx[2], idx[1]] } else { idx };
                    out.triangles.push(t);
                    out.tri_face.push(u32::try_from(fi).unwrap_or(u32::MAX));
                }
                let _ = base;
            }
            // Fill missing normals from geometry.
            for t in &out.triangles {
                let (Some(a), Some(b), Some(c)) =
                    (out.positions.get(t[0] as usize), out.positions.get(t[1] as usize), out.positions.get(t[2] as usize))
                else {
                    continue;
                };
                let fnrm = (b - a).cross(c - a).normalized().unwrap_or(Vec3::Z);
                for k in t {
                    if let Some(n) = out.normals.get_mut(*k as usize)
                        && (n.dot(fnrm) < -0.2 || n.len() < 0.5)
                    {
                        *n = fnrm;
                    }
                }
            }
            let mut seen = std::collections::HashMap::new();
            for e in meshed.iter().flat_map(|s| s.edge_iter()) {
                if seen.contains_key(&e.id()) {
                    continue;
                }
                seen.insert(e.id(), out.edges.len());
                let c = e.oriented_curve();
                out.edges.push(c.0.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect());
            }
            // Which faces each edge bounds.
            out.edge_faces = vec![Vec::new(); out.edges.len()];
            for (fi, face) in meshed.iter().flat_map(|s| s.face_iter()).enumerate() {
                for e in face.edge_iter() {
                    if let Some(slot) = seen.get(&e.id()).and_then(|i| out.edge_faces.get_mut(*i)) {
                        let f = u32::try_from(fi).unwrap_or(u32::MAX);
                        if !slot.contains(&f) {
                            slot.push(f);
                        }
                    }
                }
            }
            Ok(out)
        })
    }

    /// Faces with no triangles in a coarse tessellation (the mesher failed or gave up).
    pub fn unmeshed_faces(&self) -> usize {
        if self.is_mesh() {
            return 0;
        }
        let tol = (self.size() * 1e-2).max(1e-3);
        if let Some((cached_tol, missing)) = self.coarse_missing
            && cached_tol == tol
        {
            return missing;
        }
        let Ok(m) = self.tessellate(tol) else { return self.face_count() };
        let mut has = vec![false; self.face_count()];
        for f in &m.tri_face {
            if let Some(h) = has.get_mut(*f as usize) {
                *h = true;
            }
        }
        has.iter().filter(|h| !**h).count()
    }

    /// Display mesh: like [`Body::tessellate`], with seam edges flagged.
    pub fn display_mesh(&self, tol: f64) -> Result<Mesh> {
        let mut m = self.tessellate(tol)?;
        if self.is_mesh() {
            return Ok(m);
        }
        m.seams = crate::topo::seam_flags(self, &m);
        (m.smooth_edges, m.sharp_edges) = crate::smooth_edges::flags(self, &m);
        Ok(m)
    }

    /// Edges with polylines (index = position in [`Body::tessellate`]'s `edges`).
    pub fn edges(&self, tol: f64) -> Result<Vec<EdgeInfo>> {
        let m = self.tessellate(tol)?;
        Ok(m.edges
            .into_iter()
            .enumerate()
            .map(|(index, points)| {
                let (mid, length) = polyline_mid(&points);
                EdgeInfo { index, points, mid, length }
            })
            .collect())
    }

    /// Faces with area and centroid.
    pub fn faces(&self, tol: f64) -> Result<Vec<FaceInfo>> {
        let m = self.tessellate(tol)?;
        let nf = self.face_count();
        let mut acc: Vec<(f64, Vec3, Vec3, bool)> = vec![(0.0, Vec3::ZERO, Vec3::ZERO, true); nf];
        for (t, f) in m.triangles.iter().zip(&m.tri_face) {
            let Some([a, b, c]) = m.tri(t) else { continue };
            let cr = (b - a).cross(c - a);
            let ar = cr.len() * 0.5;
            if let Some(slot) = acc.get_mut(*f as usize) {
                slot.0 += ar;
                slot.1 += (a + b + c) * (ar / 3.0);
                if let Some(n) = cr.normalized() {
                    if slot.2 == Vec3::ZERO {
                        slot.2 = n;
                    } else if slot.2.dot(n) < 1.0 - 1e-6 {
                        slot.3 = false;
                    }
                }
            }
        }
        Ok(acc
            .into_iter()
            .enumerate()
            .map(|(index, (area, c, n, planar))| FaceInfo {
                index,
                area,
                centroid: if area > 0.0 { c / area } else { Vec3::ZERO },
                plane_normal: (planar && n != Vec3::ZERO).then_some(n),
            })
            .collect())
    }

    /// Index of the edge closest to `p` (by polyline distance).
    pub fn nearest_edge(&self, p: Vec3, tol: f64) -> Result<Option<(usize, f64)>> {
        let edges = self.edges(tol)?;
        let mut best: Option<(usize, f64)> = None;
        for e in &edges {
            for w in e.points.windows(2) {
                if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
                    let d = p.dist_to_segment(*a, *b);
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((e.index, d));
                    }
                }
            }
        }
        Ok(best)
    }

    /// Is the point inside the solid?
    pub fn contains(&self, p: Vec3) -> bool {
        self.tessellate(self.size() * 2e-3).map(|m| m.contains(p)).unwrap_or(false)
    }

    /// B-rep faces (a mesh body is one face; [`crate::measure`] counts its triangles).
    pub fn face_count(&self) -> usize {
        match &self.mesh {
            Some(_) => 1,
            None => self.solid.face_iter().count(),
        }
    }
    /// B-rep edges (none for a mesh body).
    pub fn edge_count(&self) -> usize {
        match &self.mesh {
            Some(_) => 0,
            None => Self::unique_edges(&self.solid).len(),
        }
    }
    /// B-rep vertices (none for a mesh body).
    pub fn vertex_count(&self) -> usize {
        if self.mesh.is_some() {
            return 0;
        }
        let mut seen = std::collections::HashSet::new();
        self.solid.vertex_iter().filter(|v| seen.insert(v.id())).count()
    }
    /// What is wrong with the body as a solid: shells that aren't closed and consistently
    /// oriented, faces that don't mesh, a volume that isn't positive. Empty when it is sound.
    pub fn validity(&self) -> Vec<String> {
        if let Some(m) = &self.mesh {
            return if crate::meshbody::is_closed(m) { Vec::new() } else { vec!["the mesh is not closed".into()] };
        }
        let mut out = Vec::new();
        for (i, sh) in self.solid.boundaries().iter().enumerate() {
            let c = sh.shell_condition();
            if c != truck_topology::shell::ShellCondition::Closed {
                out.push(format!("shell {i} is {c:?}, not closed"));
            }
        }
        let missing = self.unmeshed_faces();
        if missing > 0 {
            out.push(format!("{missing} face(s) don't mesh"));
        }
        match crate::measure(self) {
            Ok(m) if m.volume.is_finite() && m.volume > 0.0 => {}
            Ok(m) => out.push(format!("volume {}", m.volume)),
            Err(e) => out.push(format!("no measure: {e}")),
        }
        out
    }

    pub fn shell_count(&self) -> usize {
        match &self.mesh {
            Some(_) => 1,
            None => self.solid.boundaries().len(),
        }
    }
}

impl Body {
    /// The separate pieces of a body: each outer shell with the voids inside it. A body in one
    /// piece comes back as itself.
    pub fn lumps(&self) -> Result<Vec<Body>> {
        self.lumps_with_clearance(0.0)
    }

    /// Whether this body and `other` share material or touch (within a fine mesh tolerance).
    pub fn meets(&self, other: &Body) -> Result<bool> {
        self.require_brep("contact test")?;
        other.require_brep("contact test")?;
        let tol = (self.size().max(other.size()) * 1e-4).max(1e-6);
        let (a, b) = (self.tessellate(tol)?, other.tessellate(tol)?);
        shell_groups::meshes_meet(&a, &b, 2.0 * tol)
    }

    /// A boolean shortcut needs clearance larger than both shell mesh deflections.
    pub(crate) fn separated_lumps(&self) -> Result<Vec<Body>> {
        self.lumps_with_clearance(4.0 * (self.size() * 1e-3).max(1e-3))
    }

    fn lumps_with_clearance(&self, clearance: f64) -> Result<Vec<Body>> {
        if self.mesh.is_some() || self.solid.boundaries().len() < 2 {
            return Ok(vec![self.clone()]);
        }
        let tol = (self.size() * 1e-3).max(1e-3);
        guard("lumps", || {
            let shells: Vec<mt::Shell> = self.solid.boundaries().clone();
            if shells.len() > shell_groups::MAX_SHELLS
                || shells.iter().flat_map(mt::Shell::vertex_iter).take(shell_groups::MAX_POINTS + 1).count() > shell_groups::MAX_POINTS
                || shells.iter().flat_map(mt::Shell::face_iter).take(shell_groups::MAX_TRIANGLES + 1).count() > shell_groups::MAX_TRIANGLES
            {
                return Err(KernelError::Failed("lumps: topology exceeds the shell grouping limit".into()));
            }
            let (mut point_count, mut triangle_count) = (0usize, 0usize);
            let data: Vec<(f64, Mesh)> = shells
                .iter()
                .map(|sh| {
                    let meshed = sh.robust_triangulation(tol);
                    if meshed.face_iter().any(|f| f.surface().is_none_or(|m| m.faces().triangle_iter().next().is_none())) {
                        return Err(KernelError::Failed("lumps: cannot mesh a complete shell boundary".into()));
                    }
                    let pm = meshed.to_polygon();
                    point_count = point_count
                        .checked_add(pm.positions().len())
                        .ok_or_else(|| KernelError::Failed("lumps: mesh exceeds the shell grouping limit".into()))?;
                    triangle_count = triangle_count
                        .checked_add(pm.faces().triangle_iter().take(shell_groups::MAX_TRIANGLES + 1).count())
                        .ok_or_else(|| KernelError::Failed("lumps: mesh exceeds the shell grouping limit".into()))?;
                    if point_count > shell_groups::MAX_POINTS || triangle_count > shell_groups::MAX_TRIANGLES {
                        return Err(KernelError::Failed("lumps: mesh exceeds the shell grouping limit".into()));
                    }
                    let mut m = Mesh::default();
                    for p in pm.positions() {
                        m.positions.push(Vec3::new(p.x, p.y, p.z));
                    }
                    for t in pm.faces().triangle_iter() {
                        let idx = |k: usize| {
                            t.get(k).and_then(|x| u32::try_from(x.pos).ok()).ok_or_else(|| KernelError::Failed("lumps: invalid mesh index".into()))
                        };
                        let indices = [idx(0)?, idx(1)?, idx(2)?];
                        let [a, b, c] = m.tri(&indices).ok_or_else(|| KernelError::Failed("lumps: invalid mesh triangle".into()))?;
                        // STEP sphere poles can produce zero-area duplicate triangles.
                        // They have no boundary area or volume and are not ray crossings.
                        if (b - a).cross(c - a) == Vec3::ZERO {
                            continue;
                        }
                        m.triangles.push(indices);
                        m.tri_face.push(0);
                    }
                    let origin = m.positions.first().copied().ok_or_else(|| KernelError::Failed("lumps: empty mesh".into()))?;
                    let v = m
                        .triangles
                        .iter()
                        .map(|t| {
                            m.tri(t)
                                .map(|[a, b, c]| (a - origin).dot((b - origin).cross(c - origin)) / 6.0)
                                .ok_or_else(|| KernelError::Failed("lumps: invalid mesh triangle".into()))
                        })
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .sum();
                    Ok((v, m))
                })
                .collect::<Result<_>>()?;
            let (volumes, meshes): (Vec<_>, Vec<_>) = data.into_iter().unzip();
            let groups = shell_groups::groups(&meshes, &volumes, (self.size() * 1e-8).max(1e-9).max(clearance))?;
            if groups.len() == 1 {
                return Ok(vec![self.clone()]);
            }
            // Grouping keeps existing shells; certificates belong to their
            // exact face identities, not the new lump's positional face index.
            // This is not propagation through a geometry-changing operation.
            let sources_by_face = if let Some(sources) = &self.face_sources {
                if sources.len() != self.solid.face_iter().count() {
                    return Err(KernelError::Invalid("lumps: face source table does not match the body".into()));
                }
                let mut by_id = std::collections::HashMap::new();
                for (face, source) in self.solid.face_iter().zip(sources.iter()) {
                    if by_id.insert(face.id(), Some((face.orientation(), source))).is_some() {
                        // A repeated identity has no unique source association.
                        by_id.insert(face.id(), None);
                    }
                }
                Some(by_id)
            } else {
                None
            };
            groups
                .into_iter()
                .map(|g| {
                    let shells = g
                        .into_iter()
                        .map(|i| shells.get(i).cloned().ok_or_else(|| KernelError::Failed("lumps: missing shell".into())))
                        .collect::<Result<Vec<_>>>()?;
                    let result = Solid::try_new(shells).map_err(|e| KernelError::Failed(format!("lump: {e}"))).and_then(Body::new)?;
                    let sources = sources_by_face.as_ref().map(|old| {
                        result
                            .solid
                            .face_iter()
                            .map(|face| match old.get(&face.id()) {
                                Some(Some((orientation, source))) if *orientation == face.orientation() => (*source).clone(),
                                _ => None,
                            })
                            .collect()
                    });
                    Ok(result.with_face_sources(sources.map(std::sync::Arc::new)).inherit_regularization_protection(self))
                })
                .collect()
        })
    }
}

#[cfg(test)]
#[path = "body_lump_tests.rs"]
mod lump_tests;

#[cfg(test)]
#[path = "refine_cache_tests.rs"]
mod refine_cache_tests;

#[cfg(test)]
#[path = "regularization_protection_tests.rs"]
mod regularization_protection_tests;
