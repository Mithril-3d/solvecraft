//! Bounded, chronological checks of a joint trajectory. This samples tessellated geometry;
//! it is deliberately not advertised as exact continuous collision detection.
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use solidcraft_doc::joints;
use solidcraft_doc::{IDENTITY, Mat, apply_point, mat_mul};
use solidcraft_geom::{Aabb3, Mesh, Vec3};

use crate::params::bad;
use crate::{Result, Session};

const MAX_SAMPLES: usize = 4096;
const MAX_PLACEMENTS: usize = 4096;
const MAX_BODIES: usize = 256;
const MAX_TRIANGLES: usize = 50_000;
const MAX_WORK: usize = 50_000_000;

fn limit(message: &str) -> crate::EngineError {
    bad("contact.sweep", message)
}

struct CachedBody {
    path: Vec<u64>,
    offset: Mat,
    mesh: Arc<Mesh>,
    size: f64,
}

/// A member of a contact set: (occurrence id, name).
pub(super) type Member = (u64, String);

pub(super) struct ContactGeometry {
    bodies: Vec<CachedBody>,
    /// Body indices, and the set members (occurrence id, name) they are under.
    pairs: Vec<(usize, usize, Member, Member)>,
    step: f64,
}

impl ContactGeometry {
    pub(super) fn new(s: &Session) -> Result<Self> {
        let assembly = &s.doc.assembly;
        let sets: Vec<Vec<u64>> = if assembly.contact_all {
            vec![s.doc.occurrences.iter().map(|o| o.id).collect()]
        } else {
            assembly.contact_sets.iter().filter(|set| !set.suppressed).map(|set| set.occurrences.clone()).collect()
        };
        Self::of_sets(s, &sets)
    }

    /// The geometry of the bodies under `sets` (occurrence ids), paired as contact sets pair
    /// them: bodies under different members of one set.
    pub(super) fn of_sets(s: &Session, sets: &[Vec<u64>]) -> Result<Self> {
        let mut out = Self { bodies: Vec::new(), pairs: Vec::new(), step: f64::INFINITY };
        if sets.is_empty() {
            return Ok(out);
        }
        let state = s.model.state();
        let mut meshes: HashMap<String, Arc<Mesh>> = HashMap::new();
        let mut stack = vec![(0, Vec::<u64>::new())];
        let mut count = 0;
        while let Some((component, path)) = stack.pop() {
            count += 1;
            if count > MAX_PLACEMENTS || path.len() > 64 {
                return Err(limit("assembly exceeds contact sweep placement budget"));
            }
            for occurrence in s.doc.occurrences.iter().filter(|o| o.parent == component) {
                let mut child = path.clone();
                child.push(occurrence.id);
                stack.push((occurrence.component, child));
                if stack.len() > MAX_PLACEMENTS {
                    return Err(limit("assembly exceeds contact sweep placement budget"));
                }
            }
            if !sets.iter().any(|set| path.iter().any(|id| set.contains(id))) {
                continue;
            }
            for body in state.bodies.iter().filter(|b| s.doc.model_body_component(b) == component) {
                if body.body.is_mesh() {
                    return Err(limit("contact sweep does not support mesh bodies; disable their contact set to move them"));
                }
                if out.bodies.len() >= MAX_BODIES {
                    return Err(limit("too many bodies for a contact sweep"));
                }
                let mesh = if let Some(mesh) = meshes.get(&body.name) {
                    mesh.clone()
                } else {
                    let mesh = body.body.tessellate((body.body.size() * 1e-3).max(1e-3))?;
                    if mesh.triangles.len() > MAX_TRIANGLES || mesh.positions.len().saturating_add(mesh.triangles.len().saturating_mul(4)) > 200_000 {
                        return Err(limit("body exceeds contact sweep tessellation budget"));
                    }
                    let extent = mesh.bounds().size();
                    // Whole-body widths along bounded distinct facet normals catch tilted
                    // plates without interpreting skinny mesh triangles as thin material.
                    // Local thin walls of a wider body remain a sampling limitation.
                    let mut thickness = extent.x.min(extent.y).min(extent.z);
                    let mut directions: Vec<Vec3> = Vec::new();
                    for triangle in &mesh.triangles {
                        let Some([a, b, c]) = mesh.tri(triangle) else {
                            return Err(limit("invalid contact tessellation"));
                        };
                        let Some(direction) = (b - a).cross(c - a).normalized() else { continue };
                        if directions.len() < 32 && !directions.iter().any(|d| d.dot(direction).abs() > 1.0 - 1e-6) {
                            directions.push(direction);
                        }
                    }
                    for direction in directions {
                        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
                        for point in &mesh.positions {
                            let projected = point.dot(direction);
                            low = low.min(projected);
                            high = high.max(projected);
                        }
                        thickness = thickness.min(high - low);
                    }
                    if !thickness.is_finite() || thickness <= 1e-9 {
                        return Err(limit("body is too thin for a bounded contact sweep"));
                    }
                    out.step = out.step.min(thickness * 0.25);
                    let mesh = Arc::new(mesh);
                    meshes.insert(body.name.clone(), mesh.clone());
                    mesh
                };
                out.bodies.push(CachedBody {
                    path: path.clone(),
                    offset: s.doc.model_body_offset(body).copied().unwrap_or(IDENTITY),
                    mesh,
                    size: body.body.size(),
                });
            }
        }
        let mut pairs = BTreeSet::new();
        for set in sets {
            let tagged: Vec<(usize, u64)> =
                out.bodies.iter().enumerate().filter_map(|(i, b)| b.path.iter().rev().find(|id| set.contains(id)).map(|id| (i, *id))).collect();
            for (i, (a, oa)) in tagged.iter().enumerate() {
                for (b, ob) in tagged.iter().skip(i + 1) {
                    if oa != ob {
                        pairs.insert((*a, *b, *oa, *ob));
                    }
                }
            }
        }
        let name = |id| s.doc.occurrences.iter().find(|o| o.id == id).map(|o| o.name.clone()).unwrap_or_default();
        out.pairs = pairs.into_iter().map(|(a, b, oa, ob)| (a, b, (oa, name(oa)), (ob, name(ob)))).collect();
        Ok(out)
    }

    pub(super) fn has_pairs(&self) -> bool {
        !self.pairs.is_empty()
    }

    /// Keeps only the pairs of set members (occurrence ids) `keep` takes.
    pub(super) fn keep_pairs(&mut self, keep: impl Fn(u64, u64) -> bool) {
        self.pairs.retain(|(_, _, a, b)| keep(a.0, b.0));
    }

    /// How many mesh points are placed for each check.
    pub(super) fn vertex_count(&self) -> usize {
        self.bodies.iter().map(|b| b.mesh.positions.len()).fold(0, usize::saturating_add)
    }

    /// How many pairs of set members are checked against each other.
    pub(super) fn pair_count(&self) -> usize {
        self.pairs.iter().map(|(_, _, a, b)| (a.0.min(b.0), a.0.max(b.0))).collect::<BTreeSet<_>>().len()
    }

    fn frame(body: &CachedBody, s: &Session) -> Result<Mat> {
        let mut frame = IDENTITY;
        for id in &body.path {
            let occurrence = s.doc.occurrences.iter().find(|o| o.id == *id).ok_or_else(|| limit("contact occurrence disappeared"))?;
            frame = mat_mul(&frame, &occurrence.transform);
        }
        Ok(mat_mul(&frame, &body.offset))
    }

    fn bounds(&self, s: &Session) -> Result<Aabb3> {
        let mut bounds = Aabb3::EMPTY;
        for body in &self.bodies {
            let frame = Self::frame(body, s)?;
            for point in &body.mesh.positions {
                bounds.add(apply_point(&frame, *point));
            }
        }
        Ok(bounds)
    }

    pub(super) fn samples(&self, start: &Session, end: &Session) -> Result<usize> {
        let a = joints::joint_values(&start.doc.assembly);
        let b = joints::joint_values(&end.doc.assembly);
        let bounds = self.bounds(start)?.union(&self.bounds(end)?);
        let state = start.model.state();
        let (mut linear, mut angular, mut radius) = (0.0, 0.0, 0.0_f64);
        for joint in start.doc.assembly.joints.iter().filter(|j| !j.suppressed) {
            for (index, _) in joint.kind.dofs().iter().enumerate() {
                let delta = (a.get(&joint.id).and_then(|q| q.get(index)).copied().unwrap_or(0.0)
                    - b.get(&joint.id).and_then(|q| q.get(index)).copied().unwrap_or(0.0))
                .abs();
                if joint.kind.is_angle(index) {
                    angular += delta;
                    if delta == 0.0 {
                        continue;
                    }
                    for session in [start, end] {
                        for origin in [&joint.a, &joint.b] {
                            let frame = session.doc.origin_world(&state, origin)?;
                            let pivot = apply_point(&frame, Vec3::ZERO);
                            let far = Vec3::new(
                                (bounds.min.x - pivot.x).abs().max((bounds.max.x - pivot.x).abs()),
                                (bounds.min.y - pivot.y).abs().max((bounds.max.y - pivot.y).abs()),
                                (bounds.min.z - pivot.z).abs().max((bounds.max.z - pivot.z).abs()),
                            );
                            radius = radius.max(far.len());
                        }
                    }
                } else {
                    linear += delta;
                }
            }
        }
        let required = ((linear + angular * (radius + linear)) / self.step).max(angular / 1_f64.to_radians()).ceil().max(1.0);
        if !required.is_finite() || required > MAX_SAMPLES as f64 {
            return Err(limit("motion exceeds contact sweep sampling budget; drive the joint in smaller increments"));
        }
        let vertices: usize = self.bodies.iter().map(|body| body.mesh.positions.len()).sum();
        if (required as usize).saturating_mul(vertices.saturating_add(self.pairs.len())) > MAX_WORK {
            return Err(limit("motion exceeds contact sweep transform budget; simplify the contact set or use smaller increments"));
        }
        Ok(required as usize)
    }

    pub(super) fn first_hit(&self, s: &Session, work: &mut usize) -> Result<Option<(String, String, f64)>> {
        Ok(self.hits(s, work, MAX_WORK, false)?.into_iter().next().map(|(a, b, depth)| (a.1, b.1, depth)))
    }

    /// The pairs of set members whose bodies pass through each other in `s`, with how deep the
    /// first two bodies found go (every pair, or the first found when `all` is false). `work`
    /// adds up the cost, refused past `budget`.
    pub(super) fn hits(&self, s: &Session, work: &mut usize, budget: usize, all: bool) -> Result<Vec<(Member, Member, f64)>> {
        let mut out: Vec<(Member, Member, f64)> = Vec::new();
        // The member pairs found so far (lower id first).
        let mut found: BTreeSet<(u64, u64)> = BTreeSet::new();
        let meshes: Vec<Mesh> = self
            .bodies
            .iter()
            .map(|body| {
                let frame = Self::frame(body, s)?;
                let positions = body.mesh.positions.iter().map(|p| apply_point(&frame, *p)).collect();
                Ok(Mesh { positions, triangles: body.mesh.triangles.clone(), ..Mesh::default() })
            })
            .collect::<Result<_>>()?;
        // Each mesh's box once, not once per pair it is in.
        let boxes: Vec<Aabb3> = meshes.iter().map(Mesh::bounds).collect();
        for (a, b, oa, ob) in &self.pairs {
            // Members already found colliding need no more of their bodies tested.
            if found.contains(&(oa.0.min(ob.0), oa.0.max(ob.0))) {
                continue;
            }
            let (Some(ma), Some(mb), Some(ba), Some(bb)) = (meshes.get(*a), meshes.get(*b), self.bodies.get(*a), self.bodies.get(*b)) else {
                return Err(limit("invalid contact geometry pair"));
            };
            let (Some(aabb), Some(bbb)) = (boxes.get(*a), boxes.get(*b)) else {
                return Err(limit("invalid contact geometry pair"));
            };
            if aabb.max.x < bbb.min.x
                || bbb.max.x < aabb.min.x
                || aabb.max.y < bbb.min.y
                || bbb.max.y < aabb.min.y
                || aabb.max.z < bbb.min.z
                || bbb.max.z < aabb.min.z
            {
                continue;
            }
            let cost = ma
                .positions
                .len()
                .saturating_add(ma.triangles.len().saturating_mul(4))
                .saturating_mul(mb.triangles.len())
                .saturating_add(mb.positions.len().saturating_add(mb.triangles.len().saturating_mul(4)).saturating_mul(ma.triangles.len()));
            *work = work.saturating_add(cost);
            if *work > budget {
                return Err(limit("motion exceeds contact sweep collision budget; simplify the contact set or use smaller increments"));
            }
            if let Some(depth) = super::mesh_overlap(ma, mb, 1e-4 * ba.size.max(bb.size).max(1.0)) {
                found.insert((oa.0.min(ob.0), oa.0.max(ob.0)));
                out.push((oa.clone(), ob.clone(), depth));
                if !all {
                    break;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "contact_sweep_tests.rs"]
mod tests;
