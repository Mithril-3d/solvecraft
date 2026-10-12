//! Exact topological removal of construction-induced planar subdivisions.
//! Shared edge identity and complete construction lineage select the groups.
use super::{Body, from_p3};
use crate::{Result, guard};
use solvecraft_geom::Plane;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use truck_modeling as mt;

fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn merged_face(faces: &[&mt::Face], members: &[usize], plane: Plane, eligible: &HashSet<mt::EdgeID>) -> Option<mt::Face> {
    let mut counts = HashMap::<mt::EdgeID, (usize, bool)>::new();
    for i in members {
        for edge in faces.get(*i)?.boundary_iters().into_iter().flatten() {
            // Whole linear supports, not a sampled plane fit.
            let mt::Curve::Line(line) = edge.curve() else { return None };
            let (a, b) = (from_p3(line.0), from_p3(line.1));
            if !a.is_finite() || !b.is_finite() || a.len().max(b.len()) > 1e9 {
                return None;
            }
            let front = from_p3(edge.absolute_front().point());
            let back = from_p3(edge.absolute_back().point());
            if !front.is_finite() || !back.is_finite() {
                return None;
            }
            let error = 4096. * f64::EPSILON * (1. + a.len() + b.len() + plane.origin.len());
            if plane.height(a).abs() > error || plane.height(b).abs() > error || a.dist(front) > error || b.dist(back) > error {
                return None;
            }
            let count = counts.entry(edge.id()).or_insert((0, edge.orientation()));
            if count.0 == 1 && (!eligible.contains(&edge.id()) || count.1 == edge.orientation()) {
                return None;
            }
            count.0 += 1;
            if count.0 > 2 {
                return None;
            }
        }
    }
    let mut boundary = Vec::new();
    for i in members {
        for edge in faces.get(*i)?.boundary_iters().into_iter().flatten() {
            if counts.get(&edge.id()).is_some_and(|(n, _)| *n == 1) {
                boundary.push(edge);
            }
        }
    }
    let mut next = HashMap::new();
    let mut incoming = HashMap::new();
    for (i, edge) in boundary.iter().enumerate() {
        if next.insert(edge.front().id(), i).is_some() || incoming.insert(edge.back().id(), i).is_some() {
            return None;
        }
    }
    if next.len() != incoming.len() || next.keys().any(|id| !incoming.contains_key(id)) {
        return None;
    }
    let mut used = vec![false; boundary.len()];
    let mut outer = None;
    let mut holes = Vec::new();
    for first in 0..boundary.len() {
        if used[first] {
            continue;
        }
        let start = boundary.get(first)?.front().id();
        let origin = from_p3(boundary.get(first)?.front().point());
        let mut index = first;
        let mut edges = Vec::new();
        let (mut area, mut magnitude, mut arithmetic_error) = (0., 0., 0.);
        loop {
            if *used.get(index)? {
                return None;
            }
            used[index] = true;
            let edge = boundary.get(index)?.clone();
            let da = from_p3(edge.front().point()) - origin;
            let db = from_p3(edge.back().point()) - origin;
            let a = solvecraft_geom::Vec2::new(da.dot(plane.x), da.dot(plane.y));
            let b = solvecraft_geom::Vec2::new(db.dot(plane.x), db.dot(plane.y));
            let term = a.cross(b);
            area += term;
            magnitude += term.abs();
            let coordinate_error =
                128. * f64::EPSILON * (1. + origin.len() + from_p3(edge.front().point()).len() + from_p3(edge.back().point()).len());
            arithmetic_error += coordinate_error * (da.len() + db.len() + coordinate_error) * 16.;
            let back = edge.back().id();
            edges.push(edge);
            if back == start {
                break;
            }
            index = *next.get(&back)?;
        }
        if edges.len() < 3 || !area.is_finite() || area.abs() <= arithmetic_error + 64. * f64::EPSILON * (edges.len() as f64 + 1.) * magnitude.max(1.)
        {
            return None;
        }
        let wire = mt::Wire::from(edges);
        if area > 0. {
            if outer.replace(wire).is_some() {
                return None;
            }
        } else {
            holes.push(wire);
        }
    }
    let mut wires = vec![outer?];
    wires.extend(holes);
    mt::Face::try_new(wires, faces.get(*members.first()?)?.oriented_surface()).ok()
}

impl Body {
    pub(crate) fn regularize_certified_planar_faces(self) -> Result<Self> {
        self.regularize_planar_faces(false)
    }

    /// Only newly produced Cut walls call this path. The temporary planes carry
    /// construction support, never persistent ancestor/source-role authority.
    pub(crate) fn regularize_generated_planar_faces(self) -> Result<Self> {
        let body = self.regularize_planar_faces(true)?;
        let sources = body.face_sources().map(|sources| {
            sources
                .iter()
                .map(|source| {
                    let mut source = source.clone();
                    if let Some(super::FaceSource::ExtrudeSide { plane, coalesced, .. }) = &mut source {
                        if plane.is_some() && !*coalesced {
                            source = None;
                        } else {
                            *plane = None;
                        }
                    }
                    source
                })
                .collect()
        });
        Ok(body.with_face_sources(sources.map(Arc::new)))
    }

    fn regularize_planar_faces(self, generated: bool) -> Result<Self> {
        if self.regularization_protected {
            return Ok(self);
        }
        let Some(sources) = self.face_sources() else { return Ok(self) };
        if sources.len() > 16_384 || self.paint().is_some_and(|p| !p.faces.is_empty()) {
            return Ok(self);
        }
        let split_points = self.split_keep().iter().try_fold(0usize, |sum, line| sum.checked_add(line.len()));
        let Some(split_points) = split_points.filter(|n| *n <= 65_536) else { return Ok(self) };
        let result = guard("certified planar topology regularization", || {
            let mut faces = Vec::new();
            let mut shell_owner = Vec::new();
            let mut use_budget = 65_536usize;
            for (shell_index, shell) in self.solid.boundaries().iter().enumerate() {
                for face in shell.face_iter() {
                    if faces.len() >= sources.len() {
                        return Ok(None);
                    }
                    for _ in face.edge_iter() {
                        let Some(left) = use_budget.checked_sub(1) else { return Ok(None) };
                        use_budget = left;
                    }
                    faces.push(face);
                    shell_owner.push(shell_index);
                }
            }
            if faces.len() != sources.len() {
                return Ok(None);
            }
            let valid: Vec<_> = faces
                .iter()
                .zip(sources)
                .map(|(f, s)| {
                    s.as_ref()
                        .and_then(|source| {
                            if generated {
                                match source {
                                    super::FaceSource::ExtrudeSide { segment: solvecraft_geom::Seg2::Line { .. }, plane: Some(p), .. } => Some(*p),
                                    _ => None,
                                }
                            } else {
                                super::planar_sources::certified_plane(source)
                            }
                        })
                        .filter(|p| super::planar_sources::actual_plane(f).is_some_and(|actual| super::planar_sources::same_support(p, &actual)))
                })
                .collect();
            let mut adjacency = HashMap::<mt::EdgeID, Vec<(usize, mt::Edge)>>::new();
            for (i, face) in faces.iter().enumerate() {
                for edge in face.boundary_iters().into_iter().flatten() {
                    adjacency.entry(edge.id()).or_default().push((i, edge));
                }
            }
            let mut eligible = HashSet::new();
            let mut parent: Vec<_> = (0..faces.len()).collect();
            let mut identity = 16_000_000usize;
            let mut work = 2_000_000usize;
            for uses in adjacency.values() {
                let [(a, ea), (b, eb)] = uses.as_slice() else { continue };
                if a == b || shell_owner[*a] != shell_owner[*b] || ea.orientation() == eb.orientation() {
                    continue;
                }
                let (Some(pa), Some(pb)) = (valid[*a], valid[*b]) else { continue };
                if pa != pb {
                    continue;
                }
                let (Some(sa), Some(sb)) = (sources[*a].as_ref(), sources[*b].as_ref()) else { continue };
                let Some(cost) = super::planar_sources::descriptor_cost(sa)
                    .and_then(|a| super::planar_sources::descriptor_cost(sb).and_then(|b| a.checked_add(b)))
                else {
                    continue;
                };
                let Some(left) = identity.checked_sub(cost) else { return Ok(None) };
                identity = left;
                if !generated && sa != sb {
                    continue;
                }
                let Some(cost) = split_points.checked_mul(6).and_then(|n| n.checked_add(1)) else { return Ok(None) };
                let Some(left) = work.checked_sub(cost) else { return Ok(None) };
                work = left;
                if crate::heal::on_split_line(&crate::heal::edge_points(ea), self.split_keep()) {
                    continue;
                }
                eligible.insert(ea.id());
                let (ra, rb) = (root(&mut parent, *a), root(&mut parent, *b));
                parent[ra.max(rb)] = ra.min(rb);
            }
            let mut groups = BTreeMap::<usize, Vec<usize>>::new();
            for i in 0..faces.len() {
                groups.entry(root(&mut parent, i)).or_default().push(i);
            }
            let mut replacements = HashMap::new();
            let mut changed = false;
            let mut metadata = 2_000_000usize;
            for (leader, members) in &groups {
                if members.len() < 2 {
                    continue;
                }
                let Some(face) = valid[*leader].and_then(|p| merged_face(&faces, members, p, &eligible)) else { continue };
                replacements.insert(*leader, face);
                changed = true;
            }
            if !changed {
                return Ok(None);
            }
            let mut shells = Vec::new();
            let mut output_sources = Vec::new();
            let mut output_ids = Vec::new();
            let mut offset = 0;
            for shell in self.solid.boundaries() {
                let mut output = Vec::new();
                for face in shell.face_iter() {
                    let leader = root(&mut parent, offset);
                    if let Some(merged) = replacements.get(&leader) {
                        if offset == leader {
                            output.push(merged.clone());
                        } else {
                            offset += 1;
                            continue;
                        }
                    } else {
                        output.push(face.clone());
                    }
                    output_ids.push(output.last().ok_or_else(|| crate::KernelError::Failed("missing regularized face".into()))?.id());
                    let source = sources[offset].as_ref();
                    if let Some(source) = source {
                        let Some(cost) = super::planar_sources::descriptor_cost(source) else { return Ok(None) };
                        let Some(left) = metadata.checked_sub(cost) else { return Ok(None) };
                        metadata = left;
                    }
                    let mut source = source.cloned();
                    if generated
                        && replacements.contains_key(&leader)
                        && let Some(super::FaceSource::ExtrudeSide { coalesced, .. }) = &mut source
                    {
                        *coalesced = true;
                    }
                    output_sources.push(source);
                    offset += 1;
                }
                shells.push(mt::Shell::from(output));
            }
            let solid = mt::Solid::try_new(shells).map_err(|e| crate::KernelError::Failed(format!("planar regularization does not close: {e}")))?;
            let body = Body::new(solid)?.with_color(self.color()).with_paint(self.paint().cloned()).with_split_lines(self.split_lines().cloned());
            if !body.solid.face_iter().map(|f| f.id()).eq(output_ids) {
                return Ok(None);
            }
            Ok(Some(body.with_face_sources(Some(Arc::new(output_sources)))))
        })?;
        Ok(result.unwrap_or(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternate_adjacency_path_cannot_remove_protected_internal_edge() {
        let vertices: Vec<_> =
            (0..9).map(|i| mt::builder::vertex(super::super::p3(solvecraft_geom::Vec3::new((i % 3) as f64, (i / 3) as f64, 0.)))).collect();
        let mut edges = HashMap::<(usize, usize), mt::Edge>::new();
        let mut faces = Vec::new();
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let ids = [y * 3 + x, y * 3 + x + 1, (y + 1) * 3 + x + 1, (y + 1) * 3 + x];
            let wire: mt::Wire = (0..4)
                .map(|i| {
                    let (a, b) = (ids[i], ids[(i + 1) % 4]);
                    let key = (a.min(b), a.max(b));
                    let edge = edges.entry(key).or_insert_with(|| mt::builder::line(&vertices[key.0], &vertices[key.1]));
                    if a == key.0 { edge.clone() } else { edge.inverse() }
                })
                .collect();
            faces.push(mt::builder::try_attach_plane(&[wire]).unwrap());
        }
        let refs: Vec<_> = faces.iter().collect();
        let mut eligible: HashSet<_> = edges.values().map(|e| e.id()).collect();
        assert!(merged_face(&refs, &[0, 1, 2, 3], Plane::XY, &eligible).is_some());
        eligible.remove(&edges[&(1, 4)].id());
        assert!(
            merged_face(&refs, &[0, 1, 2, 3], Plane::XY, &eligible).is_none(),
            "remaining three internal joins cannot bypass authored split protection"
        );
    }
}
