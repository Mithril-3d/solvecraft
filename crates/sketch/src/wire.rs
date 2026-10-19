//! 3D sketch curves (wires): polylines with explicit coordinates that a sketch carries besides its
//! planar geometry — included 3D edges, curves projected onto surfaces, intersection curves.
//! They are reference geometry: drawn and usable as paths, not part of profiles or the solver.

use serde::{Deserialize, Serialize};
use solidcraft_geom::Vec3;

use crate::link::{Link, LinkGeom, LinkKind, LinkSource};
use crate::model::{Result, Sketch, SketchError, in_range3};

/// Most points one wire may have.
pub const MAX_WIRE_POINTS: usize = 100_000;

/// Coordinate meaning of a stored wire and its editable fit points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireSpace {
    /// Legacy drawn or detached geometry remains physically fixed in the world.
    #[default]
    World,
    /// Geometry follows the sketch's owning component placement.
    Component,
}

impl WireSpace {
    /// Coordinate semantics of a retained legacy producer, including a currently lost link.
    pub fn from_legacy_link(link: &Link) -> Self {
        match (&link.kind, &link.source) {
            (LinkKind::Include, LinkSource::Edge { .. } | LinkSource::SketchCurve { .. })
            | (_, LinkSource::OnSurface { .. } | LinkSource::Intersection { .. } | LinkSource::Iso { .. }) => Self::Component,
            _ => Self::World,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: String,
    /// Polyline in `space` coordinates.
    pub pts: Vec<Vec3>,
    #[serde(default)]
    pub space: WireSpace,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Drawn in a 3D sketch: the points the curve was drawn through (one: a point; two: a
    /// line; more: a smooth spline). `pts` is made from them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fit: Vec<Vec3>,
}

/// Most fit points of a drawn 3D curve.
pub const MAX_FIT_POINTS: usize = 500;

/// Samples per span of a drawn 3D spline.
const SPAN_SAMPLES: usize = 24;

/// The polyline of a drawn 3D curve through `fit`: the point itself, the line, or a centripetal
/// Catmull–Rom spline through the points (smooth, no overshoot loops).
pub fn fit_polyline(fit: &[Vec3]) -> Vec<Vec3> {
    if fit.len() <= 2 {
        return fit.to_vec();
    }
    let n = fit.len();
    // A curve that ends where it starts is smooth through that point (its neighbours wrap).
    let closed = n > 3 && fit[0].dist(fit[n - 1]) < 1e-9;
    let at = |i: isize| -> Vec3 {
        match usize::try_from(i) {
            Ok(i) if i < n => fit[i],
            Ok(_) if closed => fit[1],
            Err(_) if closed => fit[n - 2],
            // Mirrored end tangents.
            Ok(_) => fit[n - 1] * 2.0 - fit[n - 2],
            Err(_) => fit[0] * 2.0 - fit[1],
        }
    };
    let mut out = vec![fit[0]];
    for k in 0..n - 1 {
        let k = k as isize;
        let (p0, p1, p2, p3) = (at(k - 1), at(k), at(k + 1), at(k + 2));
        let tj = |a: Vec3, b: Vec3| a.dist(b).sqrt().max(1e-9);
        let t0 = 0.0;
        let t1 = t0 + tj(p0, p1);
        let t2 = t1 + tj(p1, p2);
        let t3 = t2 + tj(p2, p3);
        for s in 1..=SPAN_SAMPLES {
            let t = t1 + (t2 - t1) * s as f64 / SPAN_SAMPLES as f64;
            let a1 = p0 * ((t1 - t) / (t1 - t0)) + p1 * ((t - t0) / (t1 - t0));
            let a2 = p1 * ((t2 - t) / (t2 - t1)) + p2 * ((t - t1) / (t2 - t1));
            let a3 = p2 * ((t3 - t) / (t3 - t2)) + p3 * ((t - t2) / (t3 - t2));
            let b1 = a1 * ((t2 - t) / (t2 - t0)) + a2 * ((t - t0) / (t2 - t0));
            let b2 = a2 * ((t3 - t) / (t3 - t1)) + a3 * ((t - t1) / (t3 - t1));
            out.push(b1 * ((t2 - t) / (t2 - t1)) + b2 * ((t - t1) / (t2 - t1)));
        }
    }
    out
}

fn clean(w: &[Vec3]) -> Option<Vec<Vec3>> {
    if w.len() > MAX_WIRE_POINTS {
        return None;
    }
    let mut out: Vec<Vec3> = Vec::new();
    for p in w {
        if !in_range3(*p) {
            return None;
        }
        if out.last().is_none_or(|l| l.dist(*p) > 1e-9) {
            out.push(*p);
        }
    }
    (out.len() >= 2).then_some(out)
}

fn clean_wires(wires: &[Vec<Vec3>]) -> Result<Vec<Vec<Vec3>>> {
    if wires.len() > crate::link::MAX_LINK_ENTITIES || wires.iter().fold(0usize, |n, w| n.saturating_add(w.len())) > 1_000_000 {
        return Err(SketchError::Invalid("3D linked curves exceed the stored geometry budget".into()));
    }
    wires.iter().map(|w| clean(w).ok_or_else(|| SketchError::Invalid("3D linked curve points out of range or coincident".into()))).collect()
}

fn drawn_polyline(fit: &[Vec3]) -> Result<Vec<Vec3>> {
    if fit.is_empty() || fit.len() > MAX_FIT_POINTS {
        return Err(SketchError::Invalid(format!("a 3D curve needs 1 to {MAX_FIT_POINTS} points")));
    }
    if fit.iter().any(|p| !in_range3(*p)) {
        return Err(SketchError::Invalid("3D point out of range".into()));
    }
    if fit.windows(2).any(|w| w[0].dist(w[1]) < 1e-9) {
        return Err(SketchError::Invalid("the points of a 3D curve must differ".into()));
    }
    let pts = fit_polyline(fit);
    if pts.iter().any(|p| !in_range3(*p)) {
        return Err(SketchError::Invalid("3D spline samples out of range".into()));
    }
    Ok(pts)
}

impl Sketch {
    fn wire_budget(&self, replacing: Option<&str>, new_count: usize, new_points: usize) -> Result<()> {
        let mut count = new_count;
        let mut points = new_points;
        for w in self.wires.iter().filter(|w| replacing.is_none() || w.link.as_deref() != replacing) {
            count = count.saturating_add(1);
            points = points.saturating_add(w.pts.len()).saturating_add(w.fit.len());
        }
        if count > crate::link::MAX_LINK_ENTITIES || points > 1_000_000 {
            return Err(SketchError::Invalid("3D curves exceed the stored geometry budget".into()));
        }
        Ok(())
    }

    /// Validate saved arrays without regenerating legacy geometry or changing point values.
    pub fn validate_wires(&self) -> Result<()> {
        if self.wires.len() > crate::link::MAX_LINK_ENTITIES {
            return Err(SketchError::Invalid("too many 3D curves".into()));
        }
        let mut work = 0usize;
        let mut ids = std::collections::BTreeSet::new();
        for w in &self.wires {
            work = work.saturating_add(w.pts.len()).saturating_add(w.fit.len());
            if work > 1_000_000
                || w.id.is_empty()
                || w.id.len() > 512
                || !ids.insert(&w.id)
                || w.link.as_ref().is_some_and(|id| id.is_empty() || id.len() > 512)
                || w.pts.is_empty()
                || w.pts.len() > MAX_WIRE_POINTS
                || w.fit.len() > MAX_FIT_POINTS
                || (w.pts.len() == 1 && w.fit.len() != 1)
                || w.pts.iter().chain(&w.fit).any(|p| !in_range3(*p))
            {
                return Err(SketchError::Invalid("3D curve arrays, ids or point bounds are invalid".into()));
            }
        }
        Ok(())
    }

    /// Producer semantics for legacy wires with retained association provenance. Breaking a
    /// link removes this evidence, so detached wires conservatively retain world coordinates.
    pub fn legacy_wire_space(&self, wire: &Wire) -> WireSpace {
        let Some(link) = wire.link.as_deref().and_then(|id| self.link(id)) else { return WireSpace::World };
        WireSpace::from_legacy_link(link)
    }

    pub fn wire_index(&self, id: &str) -> Option<usize> {
        self.wires.iter().position(|w| w.id == id)
    }

    /// Wires owned by a link, in creation order.
    pub fn link_wires(&self, id: &str) -> Vec<usize> {
        self.wires.iter().enumerate().filter(|(_, w)| w.link.as_deref() == Some(id)).map(|(i, _)| i).collect()
    }

    /// Add a link with planar geometry and/or 3D wires.
    pub fn add_link_with_wires(&mut self, kind: LinkKind, source: LinkSource, geom: &[LinkGeom], wires: &[Vec<Vec3>]) -> Result<String> {
        let ws = clean_wires(wires)?;
        self.wire_budget(None, ws.len(), ws.iter().map(Vec::len).sum())?;
        if ws.is_empty() {
            return self.add_link(kind, source, geom);
        }
        let id = if geom.is_empty() {
            let id = self.fresh("j")?;
            self.links.push(Link { id: id.clone(), kind, source, face_name: None, edge_name: None, auto_face: false, lost: false });
            id
        } else {
            self.add_link(kind, source, geom)?
        };
        self.set_link_wires(&id, &ws)?;
        Ok(id)
    }

    /// Replace a link's wires (ids are kept in order; extra wires get new ids).
    pub fn set_link_wires(&mut self, id: &str, wires: &[Vec<Vec3>]) -> Result<()> {
        if self.link(id).is_none() {
            return Err(SketchError::Unknown(id.into()));
        }
        let ws = clean_wires(wires)?;
        self.wire_budget(Some(id), ws.len(), ws.iter().map(Vec::len).sum())?;
        let old: Vec<String> = self.link_wires(id).iter().filter_map(|i| self.wires.get(*i).map(|w| w.id.clone())).collect();
        self.wires.retain(|w| w.link.as_deref() != Some(id));
        for (k, pts) in ws.into_iter().enumerate() {
            let wid = match old.get(k) {
                Some(o) => o.clone(),
                None => self.fresh("w")?,
            };
            self.wires.push(Wire { id: wid, pts, space: WireSpace::Component, link: Some(id.to_string()), fit: Vec::new() });
        }
        Ok(())
    }

    /// Add a curve drawn in a 3D sketch through `fit` (see [`Wire::fit`]).
    pub fn add_drawn_wire(&mut self, fit: &[Vec3]) -> Result<String> {
        self.add_drawn_wire_in(fit, WireSpace::World)
    }

    /// Add a drawn curve without changing the coordinate meaning supplied by its author.
    pub fn add_drawn_wire_in(&mut self, fit: &[Vec3], space: WireSpace) -> Result<String> {
        let pts = drawn_polyline(fit)?;
        self.wire_budget(None, 1, pts.len() + fit.len())?;
        let id = self.fresh(if fit.len() == 1 { "q" } else { "w" })?;
        self.wires.push(Wire { id: id.clone(), pts, space, link: None, fit: fit.to_vec() });
        Ok(id)
    }

    /// Move a fit point of a drawn 3D curve, and every other drawn point at the same place
    /// (curves drawn end to end stay joined). Returns how many points moved.
    pub fn move_drawn_point(&mut self, wire: &str, index: usize, to: Vec3) -> Result<usize> {
        if !in_range3(to) {
            return Err(SketchError::Invalid("3D point out of range".into()));
        }
        let w = self.wire_index(wire).ok_or_else(|| SketchError::Unknown(wire.into()))?;
        let space = self
            .wires
            .get(w)
            .filter(|w| w.link.is_none())
            .map(|w| w.space)
            .ok_or_else(|| SketchError::Invalid("linked 3D geometry cannot be moved".into()))?;
        let from = self.wires.get(w).and_then(|w| w.fit.get(index)).copied().ok_or_else(|| SketchError::Unknown(format!("{wire} point {index}")))?;
        let mut n = 0;
        let mut changes = Vec::new();
        for (index, w) in self.wires.iter().enumerate().filter(|(_, w)| !w.fit.is_empty() && w.link.is_none() && w.space == space) {
            let mut fit = w.fit.clone();
            let mut changed = false;
            for p in fit.iter_mut().filter(|p| p.dist(from) < 1e-9) {
                *p = to;
                changed = true;
                n += 1;
            }
            if changed {
                let pts = drawn_polyline(&fit)?;
                changes.push((index, fit, pts));
            }
        }
        for (index, fit, pts) in changes {
            if let Some(w) = self.wires.get_mut(index) {
                w.fit = fit;
                w.pts = pts;
            }
        }
        Ok(n)
    }

    /// Remove wires by id.
    pub fn remove_wires(&mut self, ids: &[String]) {
        self.wires.retain(|w| !ids.contains(&w.id));
        self.prune_links();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_space_defaults_and_preserves_detached_provenance() {
        let mut sk = Sketch::new();
        let id = sk.add_drawn_wire(&[Vec3::new(1.0, 2.0, 3.0)]).unwrap();
        assert_eq!(sk.wires[0].space, WireSpace::World);
        let mut raw = serde_json::to_value(&sk.wires[0]).unwrap();
        raw.as_object_mut().unwrap().remove("space");
        assert_eq!(serde_json::from_value::<Wire>(raw).unwrap().space, WireSpace::World);
        let link = sk
            .add_link_with_wires(
                LinkKind::Include,
                LinkSource::Edge { body: "Missing".into(), at: Vec3::ZERO },
                &[],
                &[vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 4.0)]],
            )
            .unwrap();
        sk.set_link_lost(&link, true);
        let wire = sk.wires.iter().find(|w| w.link.as_deref() == Some(&link)).unwrap();
        assert_eq!(sk.legacy_wire_space(wire), WireSpace::Component);
        let wid = wire.id.clone();
        sk.break_link(&link).unwrap();
        let wire = sk.wires.iter().find(|w| w.id == wid).unwrap();
        assert_eq!(wire.space, WireSpace::Component);
        assert_eq!(sk.legacy_wire_space(wire), WireSpace::World);
        assert_eq!(sk.wires.iter().find(|w| w.id == id).unwrap().space, WireSpace::World);
    }

    #[test]
    fn wire_space_invalid_producers_and_move_are_atomic() {
        let mut sk = Sketch::new();
        let wire = sk.add_drawn_wire_in(&[Vec3::ZERO, Vec3::Z], WireSpace::Component).unwrap();
        sk.add_drawn_wire(&[Vec3::ZERO]).unwrap();
        assert_eq!(sk.move_drawn_point(&wire, 0, Vec3::X).unwrap(), 1);
        assert_eq!(sk.wires[1].fit[0], Vec3::ZERO);
        let before = sk.clone();
        assert!(sk.move_drawn_point(&wire, 0, Vec3::Z).is_err());
        assert_eq!(sk, before);
        assert!(sk.add_link_with_wires(LinkKind::Include, LinkSource::Origin, &[], &[vec![Vec3::ZERO, Vec3::new(f64::NAN, 0.0, 0.0)]]).is_err());
        assert_eq!(sk, before);
        assert!(sk.add_link_with_wires(LinkKind::Include, LinkSource::Origin, &[], &[vec![Vec3::ZERO; MAX_WIRE_POINTS + 1]]).is_err());
        assert_eq!(sk, before);
        sk.validate_wires().unwrap();
    }

    /// Through points around a circle that end where they start, the spline turns as smoothly at
    /// the closing point as everywhere else; an open one keeps its mirrored end tangents.
    #[test]
    fn a_closed_spline_is_smooth_where_it_closes() {
        let ring: Vec<Vec3> = (0..=12).map(|k| f64::from(k) * std::f64::consts::TAU / 12.0).map(|a| Vec3::new(a.cos(), a.sin(), 0.0)).collect();
        let pts = fit_polyline(&ring);
        let dir = |a: Vec3, b: Vec3| (b - a).normalized().unwrap();
        let n = pts.len();
        let (start, end) = (dir(pts[0], pts[1]), dir(pts[n - 2], pts[n - 1]));
        let mid = |i: usize| dir(pts[i - 1], pts[i]).dot(dir(pts[i], pts[i + 1]));
        assert!(start.dot(end) >= mid(SPAN_SAMPLES) - 1e-9, "turn at the close {}, at a fit point {}", start.dot(end), mid(SPAN_SAMPLES));
        let open = fit_polyline(&ring[..12]);
        assert_eq!(open.first(), ring.first());
        assert_eq!(open.last(), ring.get(11));
    }
}
