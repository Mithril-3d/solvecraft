//! Associative offsets keep their output entity ids while their source curves move.
use crate::{CurveKind, Shape, Sketch, SketchError, intersections};
use serde::{Deserialize, Serialize};
use solvecraft_geom::Vec2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffsetChain {
    pub sources: Vec<(String, bool)>,
    pub outputs: Vec<String>,
    pub distance: f64,
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<crate::offset_topology::OffsetTopology>,
}

impl Sketch {
    /// Removing an output removes its relationship. A partial analytic chain
    /// deletion releases the surviving outputs, while retained dependents keep
    /// their source references so the command's refresh refuses broken sources.
    pub(crate) fn prune_offsets_for_removed_curves(&mut self, which: &[usize]) {
        let removed = which.iter().filter_map(|i| self.curves.get(*i).map(|curve| curve.id.clone())).collect::<std::collections::BTreeSet<_>>();
        if removed.is_empty() {
            return;
        }
        let released = self
            .offsets
            .iter()
            .filter(|offset| offset.outputs.iter().any(|id| removed.contains(id)))
            .flat_map(|offset| offset.outputs.iter().cloned())
            .chain(self.freeform_offsets.iter().filter(|offset| removed.contains(&offset.output)).map(|offset| offset.output.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        self.offsets.retain(|offset| !offset.outputs.iter().any(|id| removed.contains(id)));
        self.freeform_offsets.retain(|offset| !removed.contains(&offset.output));
        let points = self
            .curves
            .iter()
            .filter(|curve| released.contains(&curve.id))
            .flat_map(|curve| curve.kind.point_ids())
            .collect::<std::collections::BTreeSet<_>>();
        for index in points {
            if index != 0
                && self.curves.iter().all(|curve| !curve.kind.uses(index) || released.contains(&curve.id))
                && let Some(point) = self.points.get_mut(index)
            {
                point.fixed = false;
            }
        }
    }

    pub fn refresh_offsets(&mut self) -> Result<(), SketchError> {
        if self.offsets.is_empty() && self.freeform_offsets.is_empty() {
            return Ok(());
        }
        let mut candidate = self.clone();
        candidate.refresh_analytic_offsets()?;
        candidate.refresh_freeform_offsets()?;
        *self = candidate;
        Ok(())
    }

    fn refresh_analytic_offsets(&mut self) -> Result<(), SketchError> {
        for index in 0..self.offsets.len() {
            if self.refresh_convex_offset(index)? {
                continue;
            }
            let offset = self.offsets.get(index).cloned().ok_or_else(|| SketchError::Invalid("missing offset chain".into()))?;
            if offset.sources.len() != offset.outputs.len() || offset.sources.len() > 10_000 || !offset.distance.is_finite() {
                return Err(SketchError::Invalid("damaged offset chain".into()));
            }
            let bad = || SketchError::Invalid("an offset source vanished or changed shape".into());
            let mut pieces = Vec::new();
            for (id, forward) in &offset.sources {
                let i = self.curve_index(id).ok_or_else(bad)?;
                let kind = &self.curves.get(i).ok_or_else(bad)?.kind;
                let piece = match *kind {
                    CurveKind::Line { a, b } => {
                        let (a, b) = (self.point(a).ok_or_else(bad)?, self.point(b).ok_or_else(bad)?);
                        let (a, b) = if *forward { (a, b) } else { (b, a) };
                        let n = (b - a).normalized().ok_or_else(bad)?.perp() * offset.distance;
                        (Shape::Line { a: a + n, b: b + n }, a + n, b + n)
                    }
                    CurveKind::Arc { c, a, b } => {
                        let (c, a, b) = (self.point(c).ok_or_else(bad)?, self.point(a).ok_or_else(bad)?, self.point(b).ok_or_else(bad)?);
                        let r = c.dist(a) - if *forward { offset.distance } else { -offset.distance };
                        if r <= 1e-9 {
                            return Err(SketchError::Invalid("the offset collapses an arc".into()));
                        }
                        let a = c + (a - c).normalized().ok_or_else(bad)? * r;
                        let b = c + (b - c).normalized().ok_or_else(bad)? * r;
                        let (a, b) = if *forward { (a, b) } else { (b, a) };
                        (Shape::Round { c, r, start: 0.0, sweep: std::f64::consts::TAU }, a, b)
                    }
                    CurveKind::Circle { c, r } => {
                        let c = self.point(c).ok_or_else(bad)?;
                        let r = r - offset.distance;
                        if r <= 1e-9 {
                            return Err(SketchError::Invalid("the offset collapses a circle".into()));
                        }
                        let out = offset.outputs.first().and_then(|id| self.curve_index(id)).ok_or_else(bad)?;
                        if let Some(crate::Curve { kind: CurveKind::Circle { r: target, .. }, .. }) = self.curves.get_mut(out) {
                            *target = r;
                        } else {
                            return Err(bad());
                        }
                        let _ = c;
                        continue;
                    }
                    _ => return Err(bad()),
                };
                pieces.push(piece);
            }
            if pieces.is_empty() {
                continue;
            }
            let joint = |i: usize, j: usize| -> Result<Vec2, SketchError> {
                let (x, y) = (pieces.get(i).ok_or_else(bad)?, pieces.get(j).ok_or_else(bad)?);
                if x.2.dist(y.1) < 1e-6 {
                    return Ok(x.2);
                }
                let guess = (x.2 + y.1) * 0.5;
                intersections(&x.0, &y.0)
                    .into_iter()
                    .min_by(|a, b| a.dist(guess).total_cmp(&b.dist(guess)))
                    .ok_or_else(|| SketchError::Invalid("the offset curves no longer meet".into()))
            };
            let n = pieces.len();
            let mut joints = vec![if offset.closed { joint(n - 1, 0)? } else { pieces.first().ok_or_else(bad)?.1 }];
            for i in 0..n.saturating_sub(1) {
                joints.push(joint(i, i + 1)?);
            }
            if !offset.closed {
                joints.push(pieces.last().ok_or_else(bad)?.2);
            }
            for (i, id) in offset.outputs.iter().enumerate() {
                let a = *joints.get(i).ok_or_else(bad)?;
                let b = *joints.get(if offset.closed { (i + 1) % n } else { i + 1 }).ok_or_else(bad)?;
                let shape = &pieces.get(i).ok_or_else(bad)?.0;
                if let Shape::Line { a: p, b: q } = shape
                    && (b - a).dot(*q - *p) <= 1e-9
                {
                    return Err(SketchError::Invalid("the offset curves would cross".into()));
                }
                let ci = self.curve_index(id).ok_or_else(bad)?;
                let kind = self.curves.get(ci).ok_or_else(bad)?.kind.clone();
                let (pa, pb) = match kind {
                    CurveKind::Line { a, b } => (a, b),
                    CurveKind::Arc { a, b, .. } => {
                        if offset.sources.get(i).is_some_and(|x| x.1) {
                            (a, b)
                        } else {
                            (b, a)
                        }
                    }
                    _ => return Err(bad()),
                };
                for (pi, pos) in [(pa, a), (pb, b)] {
                    if let Some(p) = self.points.get_mut(pi) {
                        p.pos = pos;
                        p.fixed = true;
                    }
                }
            }
        }
        Ok(())
    }
}
