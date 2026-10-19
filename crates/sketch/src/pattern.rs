//! Stored parameters and stable entity ids for editing an existing sketch pattern.

use serde::{Deserialize, Serialize};
use solidcraft_geom::Vec2;

use crate::{CurveKind, Sketch, SketchError};

/// Placement relative to the original, along a pattern direction's oriented line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternAlignment {
    Left,
    Center,
    #[default]
    Right,
}

impl PatternAlignment {
    /// Slot zero is always the original. Center alternates sides to retain stable copy ids.
    pub fn multiplier(self, index: usize, reversed: bool) -> f64 {
        let value = match self {
            Self::Left => -(index as f64),
            Self::Right => index as f64,
            Self::Center if index.is_multiple_of(2) => -((index / 2) as f64),
            Self::Center => index.div_ceil(2) as f64,
        };
        if reversed { -value } else { value }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SketchPatternKind {
    Circular {
        center: String,
        count: usize,
        angle: f64,
        #[serde(default)]
        alignment: PatternAlignment,
        #[serde(default)]
        reversed: bool,
    },
    Rectangular {
        dir: Vec2,
        #[serde(default)]
        dir2: Option<Vec2>,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default)]
        direction2: Option<String>,
        count: usize,
        spacing: f64,
        count2: usize,
        spacing2: f64,
        #[serde(default)]
        alignment: PatternAlignment,
        #[serde(default)]
        alignment2: PatternAlignment,
        #[serde(default)]
        reversed: bool,
        #[serde(default)]
        reversed2: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatternInstance {
    pub index: [usize; 2],
    pub curves: Vec<String>,
    /// Seed point id and instance point id, in the order used to create the copy.
    pub points: Vec<(String, String)>,
    pub constraints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchPattern {
    pub id: String,
    pub entities: Vec<String>,
    pub kind: SketchPatternKind,
    pub instances: Vec<PatternInstance>,
}

impl SketchPattern {
    pub fn contains(&self, entity: &str) -> bool {
        self.entities.iter().any(|id| id == entity)
            || self
                .instances
                .iter()
                .any(|instance| instance.curves.iter().any(|id| id == entity) || instance.points.iter().any(|(_, id)| id == entity))
            || matches!(&self.kind, SketchPatternKind::Circular { center, .. } if center == entity)
            || matches!(&self.kind, SketchPatternKind::Rectangular { direction, direction2, .. }
                if direction.as_deref() == Some(entity) || direction2.as_deref() == Some(entity))
    }
}

impl Sketch {
    /// Start an explicit Move/Scale of seed geometry with its copies already transformed.
    /// Interactive drags instead leave the ordinary relations to the held-point solver,
    /// which also supports dragging an instance back through its seed.
    pub fn follow_pattern_seeds(&mut self) {
        let constraint_indices: std::collections::BTreeMap<_, _> = self.constraints.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
        for pattern in &self.patterns {
            for instance in &pattern.instances {
                for cid in &instance.constraints {
                    let Some(kind) = constraint_indices.get(cid.as_str()).and_then(|i| self.constraints.get(*i)).map(|c| c.kind.clone()) else {
                        continue;
                    };
                    match kind {
                        crate::ConstraintKind::PatternPoint { p, q, center, angle, mut offset, directions } => {
                            for (l, distance) in directions.into_iter().flatten() {
                                if let Some(crate::Shape::Line { a, b }) = self.shape(l)
                                    && let Some(u) = (b - a).normalized()
                                {
                                    offset += u * distance.x + u.perp() * distance.y;
                                }
                            }
                            let Some(seed) = self.point(p) else { continue };
                            let c = center.and_then(|c| self.point(c)).unwrap_or(Vec2::ZERO);
                            let d = seed - c;
                            let (sin, cos) = angle.sin_cos();
                            if let Some(copy) = self.points.get_mut(q) {
                                copy.pos = c + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos) + offset;
                            }
                        }
                        crate::ConstraintKind::PatternRadius { a, b } => {
                            let r = match self.curves.get(a).map(|c| &c.kind) {
                                Some(CurveKind::Circle { r, .. } | CurveKind::Ellipse { r, .. }) => Some(*r),
                                _ => None,
                            };
                            if let (Some(r), Some(CurveKind::Circle { r: copy, .. } | CurveKind::Ellipse { r: copy, .. })) =
                                (r, self.curves.get_mut(b).map(|c| &mut c.kind))
                            {
                                *copy = r;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        self.refresh_pattern_shapes();
    }
    /// Deleting a seed, instance or relation breaks that pattern's association. Remaining
    /// geometry stays in the sketch, without stale metadata that could resurrect a deletion.
    pub fn prune_patterns(&mut self) {
        if self.patterns.is_empty() {
            return;
        }
        let curves: std::collections::BTreeSet<_> = self.curves.iter().map(|c| c.id.as_str()).collect();
        let points: std::collections::BTreeSet<_> = self.points.iter().map(|p| p.id.as_str()).collect();
        let constraint_ids: std::collections::BTreeSet<_> = self.constraints.iter().map(|c| c.id.as_str()).collect();
        let retired: Vec<_> = self
            .patterns
            .iter()
            .filter(|pattern| {
                pattern.entities.iter().any(|id| !curves.contains(id.as_str()) && !points.contains(id.as_str()))
                    || matches!(&pattern.kind, SketchPatternKind::Circular { center, .. } if !points.contains(center.as_str()))
                    || matches!(&pattern.kind, SketchPatternKind::Rectangular { direction, direction2, .. }
                        if direction.iter().chain(direction2).any(|id| !curves.contains(id.as_str())))
                    || pattern.instances.iter().any(|instance| {
                        instance.curves.iter().any(|id| !curves.contains(id.as_str()))
                            || instance.points.iter().any(|(p, q)| !points.contains(p.as_str()) || !points.contains(q.as_str()))
                            || instance.constraints.iter().any(|id| !constraint_ids.contains(id.as_str()))
                    })
            })
            .map(|pattern| pattern.id.clone())
            .collect();
        let constraints: std::collections::BTreeSet<_> =
            self.patterns.iter().filter(|p| retired.contains(&p.id)).flat_map(|p| &p.instances).flat_map(|i| i.constraints.iter().cloned()).collect();
        self.patterns.retain(|p| !retired.contains(&p.id));
        self.constraints.retain(|c| !constraints.contains(&c.id));
    }

    /// Conic rho is an authored shape parameter rather than a solver unknown. Like its
    /// defining points, the copies follow the seed's shape when that parameter is edited.
    pub(crate) fn refresh_pattern_shapes(&mut self) {
        if !self.curves.iter().any(|c| matches!(c.kind, CurveKind::Conic { .. } | CurveKind::Arc { .. })) {
            return;
        }
        for pattern in &self.patterns {
            let sources: Vec<_> = pattern.entities.iter().filter_map(|id| self.curve_index(id)).collect();
            for instance in &pattern.instances {
                for (seed, copy) in sources.iter().zip(&instance.curves) {
                    if let Some(CurveKind::Conic { rho, .. }) = self.curves.get(*seed).map(|c| &c.kind)
                        && let Some(copy) = self.curve_index(copy)
                    {
                        let rho = *rho;
                        if let Some(CurveKind::Conic { rho: slot, .. }) = self.curves.get_mut(copy).map(|c| &mut c.kind) {
                            *slot = rho;
                        }
                    }
                    let Some(source) = self.curves.get(*seed).cloned() else { continue };
                    if let CurveKind::Arc { c, a, b } = source.kind {
                        let mapped = |p: usize| {
                            self.points
                                .get(p)
                                .and_then(|p| instance.points.iter().find(|(id, _)| *id == p.id))
                                .and_then(|(_, id)| self.point_index(id))
                        };
                        if let (Some(c), Some(a), Some(b), Some(copy)) = (mapped(c), mapped(a), mapped(b), self.curve_index(copy))
                            && let Some(curve) = self.curves.get_mut(copy)
                        {
                            curve.kind = CurveKind::Arc { c, a, b };
                            curve.reversed = source.reversed;
                        }
                    }
                }
            }
        }
    }

    pub fn validate_patterns(&self) -> Result<(), SketchError> {
        if self.patterns.len() > crate::MAX_CONSTRAINTS {
            return Err(SketchError::TooLarge);
        }
        let mut ids = std::collections::BTreeSet::new();
        for pattern in &self.patterns {
            if pattern.id.is_empty()
                || pattern.id.len() > 512
                || !ids.insert(&pattern.id)
                || pattern.entities.is_empty()
                || pattern.entities.len() > crate::MAX_CURVES + crate::MAX_POINTS
                || pattern.instances.len() >= 500
            {
                return Err(SketchError::Invalid("invalid sketch pattern".into()));
            }
            let valid_count = |n| (1..=500).contains(&n);
            let finite = |v: f64| v.is_finite() && v.abs() <= crate::MAX_COORD;
            let valid = match &pattern.kind {
                SketchPatternKind::Circular { count, angle, center, .. } => {
                    valid_count(*count) && angle.is_finite() && angle.abs() > 1e-9 && angle.abs() <= 360.0 && self.resolve_point(center).is_some()
                }
                SketchPatternKind::Rectangular { count, count2, spacing, spacing2, dir, dir2, direction, direction2, .. } => {
                    valid_count(*count)
                        && valid_count(*count2)
                        && count.checked_mul(*count2).is_some_and(|n| n <= 500)
                        && finite(*spacing)
                        && finite(*spacing2)
                        && finite(dir.x)
                        && finite(dir.y)
                        && (dir.len() - 1.0).abs() < 1e-6
                        && dir2.is_none_or(|d| finite(d.x) && finite(d.y) && (d.len() - 1.0).abs() < 1e-6)
                        && direction.iter().chain(direction2).all(|id| {
                            self.curve_index(id).is_some_and(|l| matches!(self.curves.get(l).map(|c| &c.kind), Some(CurveKind::Line { .. })))
                        })
                }
            };
            if !valid {
                return Err(SketchError::Invalid("invalid sketch pattern settings".into()));
            }
            let (n1, n2) = match pattern.kind {
                SketchPatternKind::Circular { count, .. } => (count, 1),
                SketchPatternKind::Rectangular { count, count2, .. } => (count, count2),
            };
            if pattern.instances.len() != n1 * n2 - 1 {
                return Err(SketchError::Invalid("incomplete sketch pattern".into()));
            }
            let mut slots = std::collections::BTreeSet::new();
            for instance in &pattern.instances {
                let [i, j] = instance.index;
                let valid_id = |id: &String| !id.is_empty() && id.len() <= 512;
                if i >= n1
                    || j >= n2
                    || (i == 0 && j == 0)
                    || !slots.insert(instance.index)
                    || instance.points.len() > crate::MAX_POINTS
                    || instance.curves.len() > crate::MAX_CURVES
                    || instance.constraints.len() > crate::MAX_CONSTRAINTS
                    || instance.curves.iter().any(|id| !valid_id(id) || self.curve_index(id).is_none())
                    || instance
                        .points
                        .iter()
                        .any(|(p, q)| !valid_id(p) || !valid_id(q) || self.point_index(p).is_none() || self.point_index(q).is_none())
                    || instance.constraints.iter().any(|id| {
                        !valid_id(id)
                            || !self.constraints.iter().any(|c| {
                                c.id == *id
                                    && matches!(c.kind, crate::ConstraintKind::PatternPoint { .. } | crate::ConstraintKind::PatternRadius { .. })
                            })
                    })
                {
                    return Err(SketchError::Invalid("invalid sketch pattern instance".into()));
                }
            }
        }
        for constraint in &self.constraints {
            if matches!(constraint.kind, crate::ConstraintKind::PatternPoint { .. } | crate::ConstraintKind::PatternRadius { .. }) {
                self.validate(&constraint.kind)?;
            }
        }
        Ok(())
    }
}
