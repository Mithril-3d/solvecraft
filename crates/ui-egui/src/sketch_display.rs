//! Display-only sampling. CAD curves, profiles and constraints are never changed here.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use solvecraft_engine::doc::ModelState;
use solvecraft_engine::geom::{Loop2, Plane, Region2, Vec2, Vec3};
use solvecraft_engine::render::{PixelCurveView, SampledPath, SamplingBudget, SamplingQuality, sample_segment, sample_segment_unclipped};
use solvecraft_engine::sketch::Sketch;

use crate::SolveApp;
use crate::viewport::Proj;

pub fn projection_key(proj: &Proj, ppp: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for n in proj.vp.0.iter().flatten() {
        n.to_bits().hash(&mut h);
    }
    for n in [proj.rect.left(), proj.rect.top(), proj.rect.width(), proj.rect.height(), ppp] {
        n.to_bits().hash(&mut h);
    }
    h.finish()
}

pub fn pixel_view(proj: &Proj, ppp: f32) -> PixelCurveView {
    PixelCurveView { vp: proj.vp, width_px: f64::from(proj.rect.width() * ppp), height_px: f64::from(proj.rect.height() * ppp) }
}

pub fn curve_with_budget(sketch: &Sketch, index: usize, plane: &Plane, proj: &Proj, ppp: f32, budget: &mut SamplingBudget) -> SampledPath {
    let segments = sketch.exact_segs(index).unwrap_or_else(|| sketch.segs(index));
    let mut local = SamplingBudget::new(budget.remaining_vertices.min(4096), budget.remaining_nodes.min(65_536));
    let initial = local.clone();
    let mut out = SampledPath { parts: Vec::new(), quality: SamplingQuality::Complete, vertex_count: 0 };
    for segment in segments {
        let sampled = sample_segment(&segment, plane, &pixel_view(proj, ppp), &mut local);
        if sampled.quality != SamplingQuality::Complete {
            out.quality = sampled.quality;
        }
        out.vertex_count += sampled.vertex_count;
        out.parts.extend(sampled.parts);
    }
    budget.remaining_vertices -= initial.remaining_vertices - local.remaining_vertices;
    budget.remaining_nodes -= initial.remaining_nodes - local.remaining_nodes;
    out
}

pub fn curve_unclipped(sketch: &Sketch, index: usize, plane: &Plane, proj: &Proj, ppp: f32) -> SampledPath {
    let mut budget = SamplingBudget::new(4096, 65_536);
    let mut out = SampledPath { parts: Vec::new(), quality: SamplingQuality::Complete, vertex_count: 0 };
    for segment in sketch.exact_segs(index).unwrap_or_else(|| sketch.segs(index)) {
        let sampled = sample_segment_unclipped(&segment, plane, &pixel_view(proj, ppp), &mut budget);
        if sampled.quality != SamplingQuality::Complete {
            out.quality = sampled.quality;
        }
        out.vertex_count += sampled.vertex_count;
        out.parts.extend(sampled.parts);
    }
    out
}

#[cfg(test)]
pub fn curve_parts(sketch: &Sketch, index: usize, plane: &Plane, proj: &Proj, ppp: f32) -> Vec<Vec<Vec3>> {
    curve_with_budget(sketch, index, plane, proj, ppp, &mut SamplingBudget::new(4096, 65_536)).parts
}

/// A closed display region made from complete, unclipped adaptive boundaries. Near-plane
/// clipping or an exhausted budget suppresses the fill instead of inventing closing edges.
#[cfg(test)]
pub fn display_region(region: &Region2, plane: &Plane, proj: &Proj, ppp: f32) -> Option<Region2> {
    let mut budget = SamplingBudget::new(4096, 65_536);
    display_region_with_budget(region, plane, proj, ppp, &mut budget)
}

fn display_region_with_budget(region: &Region2, plane: &Plane, proj: &Proj, ppp: f32, shared: &mut SamplingBudget) -> Option<Region2> {
    let mut budget = SamplingBudget::new(shared.remaining_vertices.min(4096), shared.remaining_nodes.min(65_536));
    let initial = budget.clone();
    let result = (|| {
        let mut boundary = |loop_: &Loop2| -> Option<Loop2> {
            let mut points: Vec<Vec2> = Vec::new();
            for segment in &loop_.segs {
                let sampled = sample_segment_unclipped(segment, plane, &pixel_view(proj, ppp), &mut budget);
                if sampled.quality != SamplingQuality::Complete || sampled.parts.len() != 1 {
                    return None;
                }
                let part = sampled.parts.first()?;
                // Both exact endpoints must survive; a clipped boundary cannot close a region.
                if part.first()?.dist(plane.to_world(segment.start())) > 1e-8 || part.last()?.dist(plane.to_world(segment.end())) > 1e-8 {
                    return None;
                }
                for point in part {
                    let point = plane.to_local(*point);
                    if points.last().is_none_or(|last| last.dist(point) > 1e-12) {
                        points.push(point);
                    }
                }
            }
            if points.len() < 3 || points.first()?.dist(*points.last()?) > 1e-8 {
                return None;
            }
            points.pop();
            Some(Loop2::polygon(&points))
        };
        let outer = boundary(&region.outer)?;
        let holes = region.holes.iter().map(&mut boundary).collect::<Option<Vec<_>>>()?;
        Some(Region2 { outer, holes })
    })();
    shared.remaining_vertices -= initial.remaining_vertices - budget.remaining_vertices;
    shared.remaining_nodes -= initial.remaining_nodes - budget.remaining_nodes;
    result
}

#[cfg(test)]
pub fn region_triangles(region: &Region2, plane: &Plane, proj: &Proj, ppp: f32) -> Vec<[Vec2; 3]> {
    display_region(region, plane, proj, ppp).map_or_else(Vec::new, |display| fill_triangles(&display, &mut 65_536))
}

/// Bound the slab triangulator's allocation before calling it. Every slab can emit
/// at most one triangle per edge; complex fills are omitted rather than cut into holes.
pub fn fill_triangles(display: &Region2, budget: &mut usize) -> Vec<[Vec2; 3]> {
    let mut heights = Vec::new();
    let mut edges = 0usize;
    for loop_ in std::iter::once(&display.outer).chain(&display.holes) {
        for segment in &loop_.segs {
            let (a, b) = (segment.start(), segment.end());
            if (a.y - b.y).abs() > 1e-12 {
                edges += 1;
                heights.extend([a.y, b.y]);
            }
        }
    }
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    if edges.saturating_mul(heights.len().saturating_sub(1)) > *budget {
        return Vec::new();
    }
    let triangles = display.triangulate(1.0);
    *budget = budget.saturating_sub(triangles.len());
    triangles
}

pub fn boundary_parts(loop_: &Loop2, plane: &Plane, proj: &Proj, ppp: f32) -> Vec<Vec<Vec3>> {
    let mut budget = SamplingBudget::new(4096, 65_536);
    loop_.segs.iter().flat_map(|segment| sample_segment(segment, plane, &pixel_view(proj, ppp), &mut budget).parts).collect()
}

pub struct DisplayCache {
    state: Option<Arc<ModelState>>,
    view: u64,
    curves: HashMap<(u64, usize, [u64; 9]), Arc<SampledPath>>,
    profiles: HashMap<(u64, usize, [u64; 9]), Arc<Option<Region2>>>,
    budget: SamplingBudget,
}

impl Default for DisplayCache {
    fn default() -> Self {
        Self { state: None, view: 0, curves: HashMap::new(), profiles: HashMap::new(), budget: SamplingBudget::new(131_072, 1_048_576) }
    }
}

impl DisplayCache {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.curves.len()
    }

    pub fn prepare(&mut self, state: Arc<ModelState>, proj: &Proj, ppp: f32) {
        self.reset(&state, proj, ppp);
    }

    fn reset(&mut self, state: &Arc<ModelState>, proj: &Proj, ppp: f32) {
        let view = projection_key(proj, ppp);
        if view != self.view || self.state.as_ref().is_none_or(|old| !Arc::ptr_eq(old, state)) {
            self.curves.clear();
            self.profiles.clear();
            self.budget = SamplingBudget::new(131_072, 1_048_576);
            self.view = view;
            self.state = Some(Arc::clone(state));
        }
    }

    fn curve(&mut self, state: Arc<ModelState>, sketch: u64, index: usize, plane: &Plane, proj: &Proj, ppp: f32) -> Arc<SampledPath> {
        self.reset(&state, proj, ppp);
        let plane_key =
            [plane.origin.x, plane.origin.y, plane.origin.z, plane.x.x, plane.x.y, plane.x.z, plane.y.x, plane.y.y, plane.y.z].map(f64::to_bits);
        let key = (sketch, index, plane_key);
        if !self.curves.contains_key(&key) && (self.curves.len() >= 4096 || self.budget.remaining_vertices == 0 || self.budget.remaining_nodes == 0) {
            return Arc::new(SampledPath { parts: Vec::new(), quality: SamplingQuality::BudgetLimited, vertex_count: 0 });
        }
        self.curves
            .entry(key)
            .or_insert_with(|| {
                Arc::new(state.sketch(sketch).map_or_else(
                    || SampledPath { parts: Vec::new(), quality: SamplingQuality::Invalid, vertex_count: 0 },
                    |ss| curve_with_budget(&ss.sketch, index, plane, proj, ppp, &mut self.budget),
                ))
            })
            .clone()
    }

    fn profile(&mut self, state: Arc<ModelState>, sketch: u64, index: usize, plane: &Plane, proj: &Proj, ppp: f32) -> Arc<Option<Region2>> {
        self.reset(&state, proj, ppp);
        let plane_key =
            [plane.origin.x, plane.origin.y, plane.origin.z, plane.x.x, plane.x.y, plane.x.z, plane.y.x, plane.y.y, plane.y.z].map(f64::to_bits);
        let key = (sketch, index, plane_key);
        if !self.profiles.contains_key(&key)
            && (self.profiles.len() >= 1024 || self.budget.remaining_vertices == 0 || self.budget.remaining_nodes == 0)
        {
            return Arc::new(None);
        }
        self.profiles
            .entry(key)
            .or_insert_with(|| {
                Arc::new(
                    state
                        .sketch(sketch)
                        .and_then(|ss| ss.profiles.get(index))
                        .and_then(|profile| display_region_with_budget(&profile.region, plane, proj, ppp, &mut self.budget)),
                )
            })
            .clone()
    }
}

pub fn profile_region(app: &SolveApp, sketch: u64, index: usize, plane: &Plane, proj: &Proj) -> Arc<Option<Region2>> {
    let state = app.session.world_state();
    let ppp = app.viewport.pixels_per_point();
    match app.viewport.display.lock() {
        Ok(mut cache) => cache.profile(state, sketch, index, plane, proj, ppp),
        Err(_) => Arc::new(None),
    }
}

pub fn curve_sample(app: &SolveApp, sketch: u64, index: usize, plane: &Plane, proj: &Proj) -> Arc<SampledPath> {
    let state = app.session.world_state();
    let ppp = app.viewport.pixels_per_point();
    match app.viewport.display.lock() {
        Ok(mut cache) => cache.curve(state, sketch, index, plane, proj, ppp),
        Err(_) => Arc::new(state.sketch(sketch).map_or_else(
            || SampledPath { parts: Vec::new(), quality: SamplingQuality::Invalid, vertex_count: 0 },
            |ss| curve_with_budget(&ss.sketch, index, plane, proj, ppp, &mut SamplingBudget::new(4096, 65_536)),
        )),
    }
}
